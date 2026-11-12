#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
mod layer_shell;
mod bundle;
mod layout;
mod minimap;
mod restart;
mod tracker;

use std::str::FromStr;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const DEFAULT_TOGGLE_SHORTCUT: &str = "Ctrl+Space";
/// Ticks the checklist's current step (most steps have no log line to tick them).
const SETTINGS_FILE: &str = "settings.json";

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    visible: bool,
    /// false = locked: click-through except the header buttons, and not draggable.
    interactive: bool,
    toggle_shortcut: String,
    shortcut_error: Option<String>,
    last_error: Option<String>,
    /// Wayland layer-shell surface: moved by margins (move_overlay_by), not window dragging.
    layer_shell: bool,
}

struct OverlayState(Mutex<Status>);

/// An area of the overlay, in its CSS pixels.
#[derive(Clone, Copy, Default, Debug, serde::Deserialize)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Rect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

/// The only areas that take mouse input; everything else is click-through, locked or not.
/// The page sends them: the header buttons when locked, the whole top bar when unlocked,
/// plus the settings menu while it is open.
struct InputRects(Mutex<Vec<Rect>>);

/// Set while the top bar is being dragged on Hyprland (see `start_overlay_drag`).
struct Dragging(std::sync::Arc<std::sync::atomic::AtomicBool>);

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
struct Settings {
    toggle_shortcut: String,
    /// Overlay offset from the screen's top-left corner (layer-shell only).
    overlay_x: i32,
    overlay_y: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            toggle_shortcut: DEFAULT_TOGGLE_SHORTCUT.into(),
            overlay_x: 20,
            overlay_y: 170,
        }
    }
}

fn settings_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_data_dir().ok().map(|dir| dir.join(SETTINGS_FILE))
}

fn load_settings(app: &AppHandle) -> Settings {
    settings_path(app)
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_settings(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_path(app).ok_or("No settings folder")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

fn overlay(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window("overlay").ok_or_else(|| "Overlay window missing".into())
}

/// The compass around the player: shown and hidden with the overlay, never takes input.
fn show_compass(app: &AppHandle, visible: bool) {
    let Some(window) = app.get_webview_window("compass") else { return };
    if !visible {
        let _ = window.hide();
        return;
    }
    let _ = window.show();
    #[cfg(target_os = "linux")]
    if snapshot(app).is_ok_and(|s| s.layer_shell) {
        let shown = window.clone();
        let _ = app.run_on_main_thread(move || {
            if let Ok(gtk_window) = shown.gtk_window() {
                layer_shell::place_compass(&gtk_window);
            }
        });
        return;
    }
    let _ = window.set_ignore_cursor_events(true);
    let _ = window.center();
}

/// The checklist panel's rectangle in global logical pixels (x, y, width, height), for the
/// map reader to leave out of its icon search; None when the overlay is hidden or not a
/// layer surface. Callable from any thread: GTK is asked on the main thread.
pub fn panel_rect(app: &AppHandle) -> Option<[f64; 4]> {
    #[cfg(target_os = "linux")]
    {
        let status = snapshot(app).ok()?;
        if !status.visible || !status.layer_shell {
            return None;
        }
        let window = overlay(app).ok()?;
        let (send, receive) = std::sync::mpsc::channel();
        app.run_on_main_thread(move || {
            use gtk::prelude::{MonitorExt, WidgetExt};
            let rect = window.gtk_window().ok().and_then(|gtk_window| {
                let (left, top) = layer_shell::position(&gtk_window);
                let (width, height) = layer_shell::content_size(&gtk_window);
                let area = gtk_window.display().monitor_at_window(&gtk_window.window()?)?.geometry();
                Some([(area.x() + left) as f64, (area.y() + top) as f64, width as f64, height as f64])
            });
            let _ = send.send(rect);
        })
        .ok()?;
        receive.recv_timeout(std::time::Duration::from_millis(200)).ok().flatten()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        None
    }
}

fn snapshot(app: &AppHandle) -> Result<Status, String> {
    app.state::<OverlayState>().0.lock().map(|s| s.clone()).map_err(|e| e.to_string())
}

fn publish(app: &AppHandle) {
    if let Ok(status) = snapshot(app) {
        let _ = app.emit("overlay-status", status);
    }
}

/// Linux: GTK's input shape limits mouse input to the input rects. On a layer surface the
/// keyboard is also only taken while unlocked (for recording a shortcut).
#[cfg(target_os = "linux")]
fn apply_input(app: &AppHandle, window: &WebviewWindow, interactive: bool) -> Result<(), String> {
    let rects = app.state::<InputRects>().0.lock().map_err(|e| e.to_string())?.clone();
    let layer = snapshot(app)?.layer_shell;
    let window = window.clone();
    // GTK objects are not Send, so fetch and change the GTK window on the main thread.
    app.run_on_main_thread(move || {
        use gtk::prelude::WidgetExt;
        let Ok(gtk_window) = window.gtk_window() else { return };
        if layer {
            layer_shell::set_interactive(&gtk_window, interactive);
        }
        let region = gtk::cairo::Region::create();
        for rect in &rects {
            let _ = region.union_rectangle(&gtk::cairo::RectangleInt::new(
                rect.x.floor() as i32,
                rect.y.floor() as i32,
                rect.width.ceil() as i32,
                rect.height.ceil() as i32,
            ));
        }
        gtk_window.input_shape_combine_region(Some(&region));
    })
    .map_err(|e| e.to_string())
}

/// Elsewhere the whole window ignores the mouse; `watch_input_hover` lifts that while the
/// cursor is over an input rect.
#[cfg(not(target_os = "linux"))]
fn apply_input(_app: &AppHandle, window: &WebviewWindow, _interactive: bool) -> Result<(), String> {
    window.set_ignore_cursor_events(true).map_err(|e| e.to_string())
}

/// Not Linux: poll the cursor and make the window clickable only over the input rects.
/// Untested on Windows so far.
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn watch_input_hover(app: AppHandle) {
    std::thread::spawn(move || {
        let mut over = false;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(30));
            let Ok(status) = snapshot(&app) else { continue };
            let Ok(window) = overlay(&app) else { continue };
            if !status.visible {
                over = false;
                continue;
            }
            let rects = app.state::<InputRects>().0.lock().map(|r| r.clone()).unwrap_or_default();
            let (Ok(cursor), Ok(origin), Ok(scale)) =
                (app.cursor_position(), window.inner_position(), window.scale_factor())
            else {
                continue;
            };
            let x = (cursor.x - origin.x as f64) / scale;
            let y = (cursor.y - origin.y as f64) / scale;
            let inside = rects.iter().any(|rect| rect.contains(x, y));
            if inside != over {
                over = inside;
                let _ = window.set_ignore_cursor_events(!inside);
            }
        }
    });
}

/// Hyprland's global cursor position, in the same logical pixels as layer margins.
#[cfg(target_os = "linux")]
fn hyprland_cursor() -> Option<(f64, f64)> {
    let reply = layer_shell::hyprland_request("cursorpos")?;
    let (x, y) = reply.trim().split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

// All input behavior belongs to our window; no keys or mouse actions are sent to the game.
fn apply_interaction(app: &AppHandle, interactive: bool) -> Result<Status, String> {
    let window = overlay(app)?;
    apply_input(app, &window, interactive)?;
    let state = app.state::<OverlayState>();
    let mut status = state.0.lock().map_err(|e| e.to_string())?;
    window.set_focusable(interactive).map_err(|e| e.to_string())?;
    status.interactive = interactive;
    status.last_error = None;
    Ok(status.clone())
}

fn apply_visibility(app: &AppHandle, visible: bool) -> Result<Status, String> {
    // Showing always returns to locked; a hidden window cannot leave an input trap.
    // On Linux a window that was never shown has no native GdkWindow yet, so show first.
    let window = overlay(app)?;
    if visible {
        // Hyprland: open on the game's monitor rather than whichever one has focus.
        #[cfg(target_os = "linux")]
        if snapshot(app)?.layer_shell {
            let target = window.clone();
            let _ = app.run_on_main_thread(move || {
                if let (Ok(gtk_window), Some((x, y))) = (target.gtk_window(), layer_shell::game_center()) {
                    layer_shell::set_monitor_at(&gtk_window, x, y);
                }
            });
        }
        window.show().map_err(|e| e.to_string())?;
        // Now that the layer surface is on a monitor, keep it within that monitor.
        #[cfg(target_os = "linux")]
        if snapshot(app)?.layer_shell {
            let shown = window.clone();
            let _ = app.run_on_main_thread(move || {
                if let Ok(gtk_window) = shown.gtk_window() {
                    let (x, y) = layer_shell::position(&gtk_window);
                    layer_shell::move_to(&gtk_window, x, y);
                }
            });
        }
        apply_interaction(app, false)?;
        show_compass(app, true);
    } else {
        apply_interaction(app, false)?;
        window.hide().map_err(|e| e.to_string())?;
        show_compass(app, false);
    }
    let state = app.state::<OverlayState>();
    let mut status = state.0.lock().map_err(|e| e.to_string())?;
    status.visible = visible;
    Ok(status.clone())
}

fn report(app: &AppHandle, result: Result<Status, String>) -> Result<Status, String> {
    if let Err(error) = &result {
        eprintln!("Overlay: {error}");
        if let Ok(mut status) = app.state::<OverlayState>().0.lock() {
            status.last_error = Some(error.clone());
        }
    }
    publish(app);
    result
}

fn toggle_visibility(app: &AppHandle) {
    if let Ok(status) = snapshot(app) {
        // The owner's own choice: the watcher no longer owes a show.
        app.state::<Presence>().hidden_by_watcher.store(false, std::sync::atomic::Ordering::SeqCst);
        if status.visible {
            let _ = report(app, apply_visibility(app, false));
        } else {
            show_overlay(app);
        }
    }
}

/// Hides the overlay while the game is not on screen and brings it back when it is
/// (Hyprland; elsewhere the game is taken to be on screen).
struct Presence {
    /// The setting "Hide when the game is off screen".
    auto_hide: std::sync::atomic::AtomicBool,
    /// The watcher hid the overlay and will show it again.
    hidden_by_watcher: std::sync::atomic::AtomicBool,
    /// The overlay page has loaded: the window may be shown.
    page_ready: std::sync::atomic::AtomicBool,
    /// The setting "Hide during boss fights".
    fight_hide: std::sync::atomic::AtomicBool,
    /// A boss fight hid the overlay; it shows again when the fight is over.
    hidden_by_fight: std::sync::atomic::AtomicBool,
}

/// A boss fight started or ended (`tracker::Fight`). The overlay hides for its length,
/// so nothing sits over the fight, and comes back with the next target.
pub fn set_fight(app: &AppHandle, fighting: bool) {
    use std::sync::atomic::Ordering;
    let presence = app.state::<Presence>();
    if fighting {
        if presence.fight_hide.load(Ordering::SeqCst) && snapshot(app).is_ok_and(|status| status.visible) {
            presence.hidden_by_fight.store(true, Ordering::SeqCst);
            let _ = report(app, apply_visibility(app, false));
        }
    } else if presence.hidden_by_fight.swap(false, Ordering::SeqCst) {
        show_after_fight(app);
    }
}

/// Shows an overlay a fight hid, unless the game has left the screen meanwhile: then
/// the game watcher shows it when the game is back.
fn show_after_fight(app: &AppHandle) {
    use std::sync::atomic::Ordering;
    let presence = app.state::<Presence>();
    if presence.auto_hide.load(Ordering::SeqCst) && game_on_screen() == Some(false) {
        presence.hidden_by_watcher.store(true, Ordering::SeqCst);
    } else {
        show_overlay(app);
    }
}

/// The setting "Hide during boss fights". Switched off, an overlay a fight hid comes back.
#[tauri::command]
fn set_fight_hide(app: AppHandle, enabled: bool) {
    use std::sync::atomic::Ordering;
    let presence = app.state::<Presence>();
    presence.fight_hide.store(enabled, Ordering::SeqCst);
    if !enabled && presence.hidden_by_fight.swap(false, Ordering::SeqCst) {
        show_after_fight(&app);
    }
}

/// Shows the overlay; the very first time with `first_show`'s extra hide-and-show.
fn show_overlay(app: &AppHandle) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static FIRST_DONE: AtomicBool = AtomicBool::new(false);
    if FIRST_DONE.swap(true, Ordering::SeqCst) {
        let _ = report(app, apply_visibility(app, true));
    } else {
        first_show(app);
    }
}

#[cfg(target_os = "linux")]
fn game_on_screen() -> Option<bool> {
    layer_shell::game_on_screen()
}
#[cfg(not(target_os = "linux"))]
fn game_on_screen() -> Option<bool> {
    None
}

/// Every two seconds: when the game leaves the screen (closed, or its workspace is not
/// shown) the overlay hides; when it comes back, an overlay hidden that way shows again.
fn watch_game(app: AppHandle) {
    use std::sync::atomic::Ordering;
    std::thread::spawn(move || {
        let mut last: Option<bool> = None;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let presence = app.state::<Presence>();
            if !presence.page_ready.load(Ordering::SeqCst) {
                continue;
            }
            let Some(on) = game_on_screen() else { continue };
            if last == Some(on) {
                continue;
            }
            last = Some(on);
            if !presence.auto_hide.load(Ordering::SeqCst) {
                continue;
            }
            if on {
                if presence.hidden_by_watcher.swap(false, Ordering::SeqCst) {
                    show_overlay(&app);
                }
            } else if snapshot(&app).is_ok_and(|status| status.visible) {
                presence.hidden_by_watcher.store(true, Ordering::SeqCst);
                let _ = report(&app, apply_visibility(&app, false));
            }
        }
    });
}

/// The setting "Hide when the game is off screen". Switched off, an overlay the watcher
/// hid comes back.
#[tauri::command]
fn set_auto_hide(app: AppHandle, enabled: bool) {
    use std::sync::atomic::Ordering;
    let presence = app.state::<Presence>();
    presence.auto_hide.store(enabled, Ordering::SeqCst);
    if !enabled && presence.hidden_by_watcher.swap(false, Ordering::SeqCst) {
        show_overlay(&app);
    }
}

fn parse_shortcut(accelerator: &str) -> Result<Shortcut, String> {
    Shortcut::from_str(accelerator).map_err(|e| format!("{accelerator}: {e}"))
}

#[tauri::command]
fn overlay_status(state: State<'_, OverlayState>) -> Result<Status, String> {
    state.0.lock().map(|s| s.clone()).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_interaction(app: AppHandle, interactive: bool) -> Result<Status, String> {
    report(&app, apply_interaction(&app, interactive))
}

#[tauri::command]
fn set_visibility(app: AppHandle, visible: bool) -> Result<Status, String> {
    app.state::<Presence>().hidden_by_watcher.store(false, std::sync::atomic::Ordering::SeqCst);
    report(&app, apply_visibility(&app, visible))
}

/// Layer-shell only: moves the overlay by (dx, dy) pixels. Sync commands run on the main
/// thread, so GTK can be used directly.
#[tauri::command]
fn move_overlay_by(app: AppHandle, dx: i32, dy: i32) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let gtk_window = overlay(&app)?.gtk_window().map_err(|e| e.to_string())?;
        let (x, y) = layer_shell::position(&gtk_window);
        layer_shell::move_to(&gtk_window, x + dx, y + dy);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (app, dx, dy);
    Ok(())
}

/// Fits the overlay to its panel (and open menu). Sync commands run on the main thread,
/// so GTK can be used directly.
#[tauri::command]
fn resize_overlay(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    let window = overlay(&app)?;
    #[cfg(target_os = "linux")]
    if snapshot(&app)?.layer_shell {
        // The layer surface keeps its fixed size; record the panel's and keep it on-screen.
        layer_shell::set_content_size(width.ceil() as i32, height.ceil() as i32);
        let gtk_window = window.gtk_window().map_err(|e| e.to_string())?;
        let (x, y) = layer_shell::position(&gtk_window);
        layer_shell::move_to(&gtk_window, x, y);
        return Ok(());
    }
    window.set_size(tauri::LogicalSize::new(width, height)).map_err(|e| e.to_string())
}

/// Layer-shell only: keeps the current position for the next start.
#[tauri::command]
fn save_overlay_position(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        let gtk_window = overlay(&app)?.gtk_window().map_err(|e| e.to_string())?;
        let (overlay_x, overlay_y) = layer_shell::position(&gtk_window);
        save_settings(&app, &Settings { overlay_x, overlay_y, ..load_settings(&app) })?;
    }
    #[cfg(not(target_os = "linux"))]
    let _ = app;
    Ok(())
}

#[tauri::command]
fn set_input_rects(app: AppHandle, rects: Vec<Rect>) -> Result<(), String> {
    *app.state::<InputRects>().0.lock().map_err(|e| e.to_string())? = rects;
    let status = snapshot(&app)?;
    if status.visible {
        apply_input(&app, &overlay(&app)?, status.interactive)?;
    }
    Ok(())
}

/// Hyprland + layer-shell: the overlay follows the real cursor until `end_overlay_drag`,
/// so dragging is as smooth as the cursor. Returns false when unavailable; the page then
/// falls back to moving the overlay by pointer deltas.
#[tauri::command]
fn start_overlay_drag(app: AppHandle) -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    {
        use std::sync::atomic::Ordering;
        if !snapshot(&app)?.layer_shell {
            return Ok(false);
        }
        let Some(start) = hyprland_cursor() else { return Ok(false) };
        let gtk_window = overlay(&app)?.gtk_window().map_err(|e| e.to_string())?;
        let origin = layer_shell::position(&gtk_window);
        let dragging = app.state::<Dragging>().0.clone();
        if dragging.swap(true, Ordering::SeqCst) {
            return Ok(true); // already following the cursor
        }
        let handle = app.clone();
        std::thread::spawn(move || {
            let mut last = origin;
            while dragging.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(4));
                let Some(cursor) = hyprland_cursor() else { continue };
                let target = (
                    origin.0 + (cursor.0 - start.0).round() as i32,
                    origin.1 + (cursor.1 - start.1).round() as i32,
                );
                if target == last {
                    continue;
                }
                last = target;
                let window = handle.get_webview_window("overlay");
                let _ = handle.run_on_main_thread(move || {
                    if let Some(Ok(gtk_window)) = window.map(|w| w.gtk_window()) {
                        layer_shell::move_to(&gtk_window, target.0, target.1);
                    }
                });
            }
        });
        Ok(true)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        Ok(false)
    }
}

/// Stops following the cursor and keeps the position for the next start.
#[tauri::command]
fn end_overlay_drag(app: AppHandle) -> Result<(), String> {
    app.state::<Dragging>().0.store(false, std::sync::atomic::Ordering::SeqCst);
    // Queued after the drag thread's last move, so the saved position is the final one.
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = save_overlay_position(handle);
    })
    .map_err(|e| e.to_string())
}

/// Changes the show/hide shortcut. On failure the previous one stays registered.
#[tauri::command]
fn set_toggle_shortcut(app: AppHandle, accelerator: String) -> Result<Status, String> {
    let new = parse_shortcut(&accelerator)?;
    let old = snapshot(&app)?.toggle_shortcut;
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    if let Err(error) = shortcuts.register(new) {
        let _ = parse_shortcut(&old).map(|s| shortcuts.register(s));
        return Err(format!("{accelerator} can't be used: {error}"));
    }
    save_settings(&app, &Settings { toggle_shortcut: accelerator.clone(), ..load_settings(&app) })?;
    {
        let state = app.state::<OverlayState>();
        let mut status = state.0.lock().map_err(|e| e.to_string())?;
        status.toggle_shortcut = accelerator;
        status.shortcut_error = None;
    }
    publish(&app);
    snapshot(&app)
}

// Off the UI thread: it waits on a lock the background readers may hold for a while.
#[tauri::command(async)]
fn tracker_state(tracker: State<'_, tracker::Tracker>) -> Result<tracker::TrackerView, String> {
    tracker.0.lock().map(|inner| inner.view()).map_err(|e| e.to_string())
}

/// Ticks or unticks a step by hand (recovery when the log misses or misreads one).
#[tauri::command]
fn toggle_step(app: AppHandle, id: String) -> Result<tracker::TrackerView, String> {
    let tracker = app.state::<tracker::Tracker>();
    let mut inner = tracker.0.lock().map_err(|e| e.to_string())?;
    let tracker::Inner { routes, progress, .. } = &mut *inner;
    tracker::toggle(routes, progress, &id)?;
    if inner.progress.completed.contains_key(&id) {
        inner.note_hand_tick(&id);
    }
    inner.sync_navigation(&app);
    inner.save();
    let view = inner.view();
    let _ = app.emit("tracker-state", view.clone());
    // Ticking the boss's step by hand ends its fight: the overlay comes back.
    let fight_over = inner.settle_fight();
    drop(inner);
    if fight_over {
        set_fight(&app, false);
    }
    Ok(view)
}

/// Whether the compass also heads for optional steps (on by default).
#[tauri::command]
fn set_compass_optional(app: AppHandle, follow: bool) -> Result<(), String> {
    let tracker = app.state::<tracker::Tracker>();
    let mut inner = tracker.0.lock().map_err(|e| e.to_string())?;
    inner.skip_optional = !follow;
    inner.sync_navigation(&app);
    Ok(())
}

/// Clears all ticks. Log reading continues from where it is; past lines are not replayed.
#[tauri::command]
fn reset_progress(app: AppHandle) -> Result<tracker::TrackerView, String> {
    let tracker = app.state::<tracker::Tracker>();
    let mut inner = tracker.0.lock().map_err(|e| e.to_string())?;
    inner.progress.completed.clear();
    inner.progress.dismissed.clear();
    inner.sync_navigation(&app);
    inner.save();
    let view = inner.view();
    let _ = app.emit("tracker-state", view.clone());
    Ok(view)
}

// Off the UI thread: it waits on a lock the background readers may hold for a while.
#[tauri::command(async)]
fn minimap_state(minimap: State<'_, minimap::Minimap>) -> Result<minimap::MinimapView, String> {
    minimap.last.lock().map(|view| view.clone()).map_err(|e| e.to_string())
}

/// Whether the player is in a zone (not at the login or character screen).
#[tauri::command]
fn in_zone(tracker: State<'_, tracker::Tracker>) -> bool {
    tracker.0.lock().map(|inner| inner.in_zone).unwrap_or(true)
}

// Off the UI thread: it waits on a lock the background readers may hold for a while.
#[tauri::command(async)]
fn layout_state(recorder: State<'_, layout::Recorder>) -> Option<layout::LayoutView> {
    recorder.view()
}

/// Page errors and notes, printed in the app's terminal (the webview has no visible console).
#[tauri::command]
fn client_log(window: tauri::Window, message: String) {
    eprintln!("Page [{}]: {message}", window.label());
}

/// The first time the layer surfaces are mapped they stay blank (Hyprland, seen
/// 2026-11-12: listed by the compositor, nothing drawn); hidden and shown again they
/// draw. So the first show is followed by one hide-and-show, off the main thread so
/// the compositor handles each step.
fn first_show(app: &AppHandle) {
    let _ = report(app, apply_visibility(app, true));
    let handle = app.clone();
    std::thread::spawn(move || {
        let pause = std::time::Duration::from_millis(250);
        std::thread::sleep(pause);
        let _ = report(&handle, apply_visibility(&handle, false));
        std::thread::sleep(pause);
        let _ = report(&handle, apply_visibility(&handle, true));
    });
}

/// The overlay page has loaded: show the windows for the first time.
#[tauri::command]
fn page_ready(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let presence = app.state::<Presence>();
    if window.label() == "overlay" && !presence.page_ready.swap(true, Ordering::SeqCst) && !snapshot(&app)?.visible {
        // With the game off screen the overlay stays hidden until the game shows up.
        if presence.auto_hide.load(Ordering::SeqCst) && game_on_screen() == Some(false) {
            presence.hidden_by_watcher.store(true, Ordering::SeqCst);
        } else {
            show_overlay(&app);
        }
    }
    Ok(())
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

/// Linux: with nvidia-utils installed but no NVIDIA driver loaded (e.g. an AMD-only
/// machine), glvnd tries NVIDIA's EGL first and WebKit's page process crashes inside
/// libnvidia-gpucomp or draws nothing. Use Mesa then. Runs before any thread or GL use;
/// an explicit __EGL_VENDOR_LIBRARY_FILENAMES from the user always wins.
#[cfg(target_os = "linux")]
fn prefer_mesa_egl() {
    const MESA: &str = "/usr/share/glvnd/egl_vendor.d/50_mesa.json";
    let nvidia_loaded = std::path::Path::new("/proc/driver/nvidia").exists();
    if std::env::var_os("__EGL_VENDOR_LIBRARY_FILENAMES").is_none()
        && !nvidia_loaded
        && std::path::Path::new(MESA).exists()
    {
        std::env::set_var("__EGL_VENDOR_LIBRARY_FILENAMES", MESA);
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    prefer_mesa_egl();
    tauri::Builder::default()
        // Only one copy runs: a second launch shows the existing overlay and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second launch asks for the overlay: show it whatever the watcher did.
            app.state::<Presence>().hidden_by_watcher.store(false, std::sync::atomic::Ordering::SeqCst);
            show_overlay(app);
        }))
        .manage(Presence {
            auto_hide: std::sync::atomic::AtomicBool::new(true),
            hidden_by_watcher: Default::default(),
            page_ready: Default::default(),
            fight_hide: std::sync::atomic::AtomicBool::new(true),
            hidden_by_fight: Default::default(),
        })
        .manage(InputRects(Mutex::new(Vec::new())))
        .manage(Dragging(Default::default()))
        .invoke_handler(tauri::generate_handler![
            overlay_status,
            set_interaction,
            set_visibility,
            move_overlay_by,
            save_overlay_position,
            resize_overlay,
            set_input_rects,
            start_overlay_drag,
            end_overlay_drag,
            set_toggle_shortcut,
            tracker_state,
            toggle_step,
            reset_progress,
            set_compass_optional,
            set_auto_hide,
            set_fight_hide,
            minimap_state,
            layout_state,
            in_zone,
            restart::restart_latest,
            client_log,
            page_ready,
            quit
        ])
        .setup(|app| {
            let settings = load_settings(app.handle());
            // Wayland: make the overlay a layer surface before it is first shown.
            #[cfg(target_os = "linux")]
            let layer_shell = layer_shell::supported()
                && overlay(app.handle())
                    .and_then(|window| window.gtk_window().map_err(|e| e.to_string()))
                    .map(|gtk_window| layer_shell::init(&gtk_window, settings.overlay_x, settings.overlay_y))
                    .is_ok();
            #[cfg(target_os = "linux")]
            if layer_shell {
                if let Some(Ok(gtk_window)) = app.get_webview_window("compass").map(|w| w.gtk_window()) {
                    layer_shell::init_compass(&gtk_window);
                }
            }
            #[cfg(not(target_os = "linux"))]
            let layer_shell = false;
            app.manage(OverlayState(Mutex::new(Status {
                visible: false,
                interactive: false,
                toggle_shortcut: settings.toggle_shortcut.clone(),
                shortcut_error: None,
                last_error: None,
                layer_shell,
            })));
            // Before the tracker and minimap threads, which both feed it.
            let layouts = app.path().app_data_dir().ok().map(|dir| dir.join("layouts"));
            app.manage(layout::Recorder::new(layouts));
            app.manage(tracker::init(app.handle()));
            if let Ok(inner) = app.state::<tracker::Tracker>().0.lock() {
                inner.sync_navigation(app.handle());
            }
            tracker::spawn(app.handle().clone());
            let reader = minimap::init();
            minimap::spawn(app.handle().clone(), &reader);
            app.manage(reader);
            #[cfg(not(target_os = "linux"))]
            watch_input_hover(app.handle().clone());

            // One global shortcut: show/hide. A failure is shown in the settings menu.
            let plugin = tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    let _ = shortcut;
                    toggle_visibility(app);
                })
                .build();
            let registered = app
                .handle()
                .plugin(plugin)
                .map_err(|e| e.to_string())
                .and_then(|()| parse_shortcut(&settings.toggle_shortcut))
                .and_then(|shortcut| app.global_shortcut().register(shortcut).map_err(|e| e.to_string()));
            if let Err(error) = registered {
                eprintln!("Overlay: global shortcut not registered: {error}");
                app.state::<OverlayState>().0.lock().unwrap().shortcut_error = Some(error);
            }
            // The overlay is shown once its page has loaded (`page_ready`), and hidden
            // while the game is off screen.
            watch_game(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Unable to start Batonga overlay");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_capture_settings_do_not_disable_reading_or_change_other_preferences() {
        let old: Settings = serde_json::from_str(r#"{"readMinimap":false,"windowsCaptureOptIn":false,"toggleShortcut":"F9","overlayX":80}"#).unwrap();
        assert_eq!(old.toggle_shortcut, "F9");
        assert_eq!(old.overlay_x, 80);
        let saved = serde_json::to_value(old).unwrap();
        assert!(saved.get("readMinimap").is_none());
        assert!(saved.get("windowsCaptureOptIn").is_none());
        assert!(minimap::init().last.lock().unwrap().enabled);
    }

    #[test]
    fn accepts_the_accelerators_the_settings_menu_records() {
        for accelerator in ["Ctrl+Space", "Ctrl+Alt+KeyO", "Shift+F5", "Super+Digit1", "F9"] {
            assert!(parse_shortcut(accelerator).is_ok(), "{accelerator}");
        }
        assert!(parse_shortcut("Ctrl+").is_err());
    }
}
