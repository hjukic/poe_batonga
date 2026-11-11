#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
mod layer_shell;
mod layout;
mod minimap;
mod tracker;

use std::str::FromStr;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const DEFAULT_TOGGLE_SHORTCUT: &str = "Ctrl+Space";
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

/// Where the header buttons are, in the overlay's CSS pixels. While locked, only this
/// area takes mouse input, so the lock button can always be clicked.
#[derive(Clone, Copy, Default, Debug, serde::Deserialize)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

struct ControlsRect(Mutex<Rect>);

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
struct Settings {
    toggle_shortcut: String,
    /// Live minimap reading (screenshots of the minimap corner only).
    read_minimap: bool,
    /// Overlay offset from the screen's top-left corner (layer-shell only).
    overlay_x: i32,
    overlay_y: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            toggle_shortcut: DEFAULT_TOGGLE_SHORTCUT.into(),
            read_minimap: true,
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

fn snapshot(app: &AppHandle) -> Result<Status, String> {
    app.state::<OverlayState>().0.lock().map(|s| s.clone()).map_err(|e| e.to_string())
}

fn publish(app: &AppHandle) {
    if let Ok(status) = snapshot(app) {
        let _ = app.emit("overlay-status", status);
    }
}

/// Linux: GTK's input shape limits mouse input to the header buttons while locked
/// (an empty region would make the lock button unreachable).
#[cfg(target_os = "linux")]
fn set_click_through(app: &AppHandle, window: &WebviewWindow, through: bool) -> Result<(), String> {
    let rect = *app.state::<ControlsRect>().0.lock().map_err(|e| e.to_string())?;
    let layer = snapshot(app)?.layer_shell;
    let window = window.clone();
    // GTK objects are not Send, so fetch and change the GTK window on the main thread.
    app.run_on_main_thread(move || {
        use gtk::prelude::WidgetExt;
        let Ok(gtk_window) = window.gtk_window() else { return };
        if layer {
            layer_shell::set_interactive(&gtk_window, !through);
        }
        if through {
            let area = gtk::cairo::RectangleInt::new(
                rect.x.floor() as i32,
                rect.y.floor() as i32,
                rect.width.ceil() as i32,
                rect.height.ceil() as i32,
            );
            gtk_window.input_shape_combine_region(Some(&gtk::cairo::Region::create_rectangle(&area)));
        } else {
            gtk_window.input_shape_combine_region(None);
        }
    })
    .map_err(|e| e.to_string())
}

/// Elsewhere the whole window is click-through while locked; `watch_controls_hover`
/// lifts it while the cursor is over the header buttons.
#[cfg(not(target_os = "linux"))]
fn set_click_through(_app: &AppHandle, window: &WebviewWindow, through: bool) -> Result<(), String> {
    window.set_ignore_cursor_events(through).map_err(|e| e.to_string())
}

/// Not Linux: poll the cursor and make the window clickable only over the header buttons
/// while locked. Untested on Windows so far.
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn watch_controls_hover(app: AppHandle) {
    std::thread::spawn(move || {
        let mut over = false;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let Ok(status) = snapshot(&app) else { continue };
            let Ok(window) = overlay(&app) else { continue };
            if !status.visible || status.interactive {
                over = false;
                continue;
            }
            let rect = app.state::<ControlsRect>().0.lock().map(|r| *r).unwrap_or_default();
            let (Ok(cursor), Ok(origin), Ok(scale)) =
                (app.cursor_position(), window.inner_position(), window.scale_factor())
            else {
                continue;
            };
            let x = (cursor.x - origin.x as f64) / scale;
            let y = (cursor.y - origin.y as f64) / scale;
            let inside = x >= rect.x && x <= rect.x + rect.width && y >= rect.y && y <= rect.y + rect.height;
            if inside != over {
                over = inside;
                let _ = window.set_ignore_cursor_events(!inside);
            }
        }
    });
}

// All input behavior belongs to our window; no keys or mouse actions are sent to the game.
fn apply_interaction(app: &AppHandle, interactive: bool) -> Result<Status, String> {
    let window = overlay(app)?;
    set_click_through(app, &window, !interactive)?;
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
        window.show().map_err(|e| e.to_string())?;
        apply_interaction(app, false)?;
    } else {
        apply_interaction(app, false)?;
        window.hide().map_err(|e| e.to_string())?;
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
        let _ = report(app, apply_visibility(app, !status.visible));
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
fn set_controls_rect(app: AppHandle, rect: Rect) -> Result<(), String> {
    *app.state::<ControlsRect>().0.lock().map_err(|e| e.to_string())? = rect;
    let status = snapshot(&app)?;
    if status.visible && !status.interactive {
        set_click_through(&app, &overlay(&app)?, true)?;
    }
    Ok(())
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

#[tauri::command]
fn tracker_state(tracker: State<'_, tracker::Tracker>) -> Result<tracker::TrackerView, String> {
    tracker.0.lock().map(|inner| inner.view()).map_err(|e| e.to_string())
}

/// Ticks or unticks a step by hand (recovery when the log misses or misreads one).
#[tauri::command]
fn toggle_step(app: AppHandle, id: String) -> Result<tracker::TrackerView, String> {
    let tracker = app.state::<tracker::Tracker>();
    let mut inner = tracker.0.lock().map_err(|e| e.to_string())?;
    let tracker::Inner { route, progress, .. } = &mut *inner;
    tracker::toggle(route, progress, &id)?;
    inner.save();
    let view = inner.view();
    let _ = app.emit("tracker-state", view.clone());
    Ok(view)
}

/// Clears all ticks. Log reading continues from where it is; past lines are not replayed.
#[tauri::command]
fn reset_progress(app: AppHandle) -> Result<tracker::TrackerView, String> {
    let tracker = app.state::<tracker::Tracker>();
    let mut inner = tracker.0.lock().map_err(|e| e.to_string())?;
    inner.progress.completed.clear();
    inner.progress.dismissed.clear();
    inner.save();
    let view = inner.view();
    let _ = app.emit("tracker-state", view.clone());
    Ok(view)
}

#[tauri::command]
fn minimap_state(minimap: State<'_, minimap::Minimap>) -> Result<minimap::MinimapView, String> {
    minimap.last.lock().map(|view| view.clone()).map_err(|e| e.to_string())
}

/// Turns live minimap reading on or off; the choice is kept across restarts.
#[tauri::command]
fn set_minimap_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    save_settings(&app, &Settings { read_minimap: enabled, ..load_settings(&app) })?;
    app.state::<minimap::Minimap>().enabled.store(enabled, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn layout_state(recorder: State<'_, layout::Recorder>) -> Option<layout::LayoutView> {
    recorder.view()
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

fn main() {
    tauri::Builder::default()
        // Only one copy runs: a second launch shows the existing overlay and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = report(app, apply_visibility(app, true));
        }))
        .manage(ControlsRect(Mutex::new(Rect::default())))
        .invoke_handler(tauri::generate_handler![
            overlay_status,
            set_interaction,
            set_visibility,
            move_overlay_by,
            save_overlay_position,
            set_controls_rect,
            set_toggle_shortcut,
            tracker_state,
            toggle_step,
            reset_progress,
            minimap_state,
            set_minimap_enabled,
            layout_state,
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
            tracker::spawn(app.handle().clone());
            let reader = minimap::init(settings.read_minimap);
            minimap::spawn(app.handle().clone(), &reader);
            app.manage(reader);
            #[cfg(not(target_os = "linux"))]
            watch_controls_hover(app.handle().clone());

            // Only the show/hide shortcut is global; a failure is shown in the settings menu.
            let plugin = tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_visibility(app);
                    }
                })
                .build();
            let registered = app
                .handle()
                .plugin(plugin)
                .map_err(|e| e.to_string())
                .and_then(|()| parse_shortcut(&settings.toggle_shortcut))
                .and_then(|shortcut| app.global_shortcut().register(shortcut).map_err(|e| e.to_string()));
            if let Err(error) = registered {
                app.state::<OverlayState>().0.lock().unwrap().shortcut_error = Some(error);
            }
            let _ = report(app.handle(), apply_visibility(app.handle(), true));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Unable to start Batonga overlay");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_accelerators_the_settings_menu_records() {
        for accelerator in ["Ctrl+Space", "Ctrl+Alt+KeyO", "Shift+F5", "Super+Digit1", "F9"] {
            assert!(parse_shortcut(accelerator).is_ok(), "{accelerator}");
        }
        assert!(parse_shortcut("Ctrl+").is_err());
    }
}
