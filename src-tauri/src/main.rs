#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod tracker;

use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    visible: bool,
    interactive: bool,
    shortcut_errors: Vec<String>,
    last_error: Option<String>,
}

struct OverlayState(Mutex<Status>);

fn snapshot(app: &AppHandle) -> Result<Status, String> {
    app.state::<OverlayState>()
        .0
        .lock()
        .map(|s| s.clone())
        .map_err(|e| e.to_string())
}

fn publish(app: &AppHandle) {
    if let Ok(status) = snapshot(app) {
        let _ = app.emit("overlay-status", status);
    }
}

// All input behavior belongs to our windows; no keys or mouse actions are sent to the game.
fn apply_interaction(app: &AppHandle, interactive: bool) -> Result<Status, String> {
    let window = app
        .get_webview_window("overlay")
        .ok_or("Overlay window missing")?;
    let state = app.state::<OverlayState>();
    let mut status = state.0.lock().map_err(|e| e.to_string())?;
    let old = status.interactive;
    let result = (|| {
        window.set_focusable(interactive)?;
        window.set_ignore_cursor_events(!interactive)?;
        Ok::<_, tauri::Error>(())
    })();
    if let Err(error) = result {
        let _ = window.set_focusable(old);
        let _ = window.set_ignore_cursor_events(!old);
        return Err(error.to_string());
    }
    status.interactive = interactive;
    status.last_error = None;
    Ok(status.clone())
}

fn apply_visibility(app: &AppHandle, visible: bool) -> Result<Status, String> {
    // Showing always returns to click-through; a hidden window cannot leave an input trap.
    // On Linux a window that was never shown has no native GdkWindow yet, and tao panics
    // when click-through is set on it, so show first and switch to click-through after.
    let window = app
        .get_webview_window("overlay")
        .ok_or("Overlay window missing")?;
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

#[tauri::command]
fn reset_overlay(app: AppHandle) -> Result<Status, String> {
    let result = (|| {
        apply_interaction(&app, false)?;
        app.get_webview_window("overlay")
            .ok_or("Overlay window missing")?
            .center()
            .map_err(|e| e.to_string())?;
        apply_visibility(&app, true)
    })();
    report(&app, result)
}

#[tauri::command]
fn open_controls(app: AppHandle) -> Result<(), String> {
    let controls = app
        .get_webview_window("controls")
        .ok_or("Controls window missing")?;
    let result = (|| {
        controls.unminimize()?;
        controls.show()?;
        controls.set_focus()
    })();
    result.map_err(|e| e.to_string())
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
fn quit(app: AppHandle) {
    app.exit(0);
}

fn toggle_visibility(app: &AppHandle) {
    if let Ok(status) = snapshot(app) {
        let _ = report(app, apply_visibility(app, !status.visible));
    }
}

fn toggle_interaction(app: &AppHandle) {
    if let Ok(status) = snapshot(app) {
        let _ = report(app, apply_interaction(app, !status.interactive));
    }
}

// On Wayland, X11 global shortcuts only fire while an XWayland window has focus, so the
// compositor binds the keys instead and signals us: `pkill -USR1 -x poe-batonga` toggles
// visibility, `pkill -USR2 -x poe-batonga` toggles interaction.
#[cfg(unix)]
fn listen_for_signals(app: AppHandle) {
    use signal_hook::consts::{SIGUSR1, SIGUSR2};
    match signal_hook::iterator::Signals::new([SIGUSR1, SIGUSR2]) {
        Ok(mut signals) => {
            std::thread::spawn(move || {
                for signal in signals.forever() {
                    match signal {
                        SIGUSR1 => toggle_visibility(&app),
                        SIGUSR2 => toggle_interaction(&app),
                        _ => {}
                    }
                }
            });
        }
        Err(error) => eprintln!("Overlay: signal toggles unavailable: {error}"),
    }
}

fn main() {
    let modifiers = Some(Modifiers::CONTROL | Modifiers::ALT);
    let interaction = Shortcut::new(modifiers, Code::KeyI);
    let visibility = Shortcut::new(modifiers, Code::KeyO);

    tauri::Builder::default()
        // Only one copy runs: a second launch shows the existing overlay and exits.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = report(app, apply_visibility(app, true));
        }))
        .manage(OverlayState(Mutex::new(Status {
            visible: false,
            interactive: false,
            shortcut_errors: vec![],
            last_error: None,
        })))
        .invoke_handler(tauri::generate_handler![
            overlay_status,
            set_interaction,
            set_visibility,
            reset_overlay,
            open_controls,
            tracker_state,
            toggle_step,
            reset_progress,
            quit
        ])
        .setup(move |app| {
            app.manage(tracker::init(app.handle()));
            tracker::spawn(app.handle().clone());
            #[cfg(unix)]
            listen_for_signals(app.handle().clone());
            // Shortcut failure is recoverable through the regular controls window.
            let plugin = tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    if shortcut == &interaction {
                        toggle_interaction(app);
                    } else if shortcut == &visibility {
                        toggle_visibility(app);
                    }
                })
                .build();
            match app.handle().plugin(plugin) {
                Ok(()) => {
                    for (label, shortcut) in
                        [("Ctrl+Alt+I", interaction), ("Ctrl+Alt+O", visibility)]
                    {
                        if let Err(error) = app.global_shortcut().register(shortcut) {
                            app.state::<OverlayState>()
                                .0
                                .lock()
                                .unwrap()
                                .shortcut_errors
                                .push(format!("{label}: {error}"));
                        }
                    }
                }
                Err(error) => app
                    .state::<OverlayState>()
                    .0
                    .lock()
                    .unwrap()
                    .shortcut_errors
                    .push(error.to_string()),
            }
            // Start with the guide over the desktop; recovery controls remain in the taskbar.
            if let Some(controls) = app.get_webview_window("controls") {
                controls.minimize()?;
            }
            let _ = report(app.handle(), apply_visibility(app.handle(), true));
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "controls"
                && matches!(event, tauri::WindowEvent::CloseRequested { .. })
            {
                window.app_handle().exit(0);
            }
        })
        .run(tauri::generate_context!())
        .expect("Unable to start Batonga overlay test");
}
