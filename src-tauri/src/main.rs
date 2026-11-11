#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
fn quit(app: AppHandle) {
    app.exit(0);
}

fn main() {
    let modifiers = Some(Modifiers::CONTROL | Modifiers::ALT);
    let interaction = Shortcut::new(modifiers, Code::KeyI);
    let visibility = Shortcut::new(modifiers, Code::KeyO);

    tauri::Builder::default()
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
            quit
        ])
        .setup(move |app| {
            // Shortcut failure is recoverable through the regular controls window.
            let plugin = tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    if let Ok(status) = snapshot(app) {
                        if shortcut == &interaction {
                            let _ = report(app, apply_interaction(app, !status.interactive));
                        } else if shortcut == &visibility {
                            let _ = report(app, apply_visibility(app, !status.visible));
                        }
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
