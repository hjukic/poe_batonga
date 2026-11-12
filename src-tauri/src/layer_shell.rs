//! Wayland: run the overlay as a wlr-layer-shell surface instead of a normal window.
//!
//! Compositors such as Hyprland stack normal windows under a fullscreen game and send
//! every click over a fullscreen game to the game. A layer surface on the overlay layer
//! draws above fullscreen windows and gets input only inside its input region, so the
//! locked overlay passes clicks to the game while its header buttons still work. It
//! never takes keyboard focus while locked. The layer surface is placed by margins from
//! the top-left of the output, because a client can't drag a layer surface the way it
//! drags a normal window.

use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

/// True when the session is Wayland and the compositor supports layer-shell.
pub fn supported() -> bool {
    gtk_layer_shell::is_supported()
}

/// Turns the window into a layer surface. Call on the main thread, before the window
/// is first shown (layer-shell must be set up before the GTK window is realized).
pub fn init(window: &gtk::ApplicationWindow, left: i32, top: i32) {
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace("poe-batonga");
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    // 0: keep clear of the top bar's reserved space instead of drawing under it.
    window.set_exclusive_zone(0);
    window.set_keyboard_mode(KeyboardMode::None);
    // Not on a monitor yet: keep the saved position within the smallest monitor, since
    // the compositor may place the overlay on any of them.
    use gtk::prelude::MonitorExt;
    // The panel's size isn't known yet; assume a typical one.
    let (width, height) = (360, 420);
    let _ = window;
    let display = gtk::gdk::Display::default();
    let smallest = display
        .iter()
        .flat_map(|d| (0..d.n_monitors()).filter_map(move |i| d.monitor(i)))
        .map(|m| m.geometry())
        .min_by_key(|area| area.width() * area.height());
    let (left, top) = match smallest {
        Some(area) => (left.clamp(0, (area.width() - width).max(0)), top.clamp(0, (area.height() - height).max(0))),
        None => (left, top),
    };
    move_to(window, left, top);
}

/// Locked: no keyboard focus ever, so the game keeps it. Unlocked: focus on click,
/// so the settings menu can record a shortcut.
pub fn set_interactive(window: &gtk::ApplicationWindow, interactive: bool) {
    window.set_keyboard_mode(if interactive { KeyboardMode::OnDemand } else { KeyboardMode::None });
}

/// Places the overlay, kept fully on its monitor (a layer surface is clipped to its
/// output, so a saved position from a wider setup could hide it).
pub fn move_to(window: &gtk::ApplicationWindow, left: i32, top: i32) {
    use gtk::prelude::{MonitorExt, WidgetExt};
    let (width, height) = content_size(window);
    let monitor = window
        .window()
        .and_then(|gdk_window| window.display().monitor_at_window(&gdk_window))
        .or_else(|| window.display().primary_monitor());
    let (max_left, max_top) = match monitor {
        Some(monitor) => {
            let area = monitor.geometry();
            ((area.width() - width).max(0), (area.height() - height).max(0))
        }
        None => (i32::MAX, i32::MAX),
    };
    window.set_layer_shell_margin(Edge::Left, left.clamp(0, max_left));
    window.set_layer_shell_margin(Edge::Top, top.clamp(0, max_top));
}

/// Sends one request to Hyprland's IPC socket and returns the reply (None elsewhere).
pub fn hyprland_request(request: &str) -> Option<String> {
    use std::io::{Read, Write};
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    // Started outside the desktop session (e.g. detached), the instance variable is
    // missing: use the most recently started Hyprland instance instead.
    let instance = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok().or_else(|| {
        std::fs::read_dir(format!("{runtime}/hypr"))
            .ok()?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().join(".socket.sock").exists())
            .max_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
    })?;
    let mut socket =
        std::os::unix::net::UnixStream::connect(format!("{runtime}/hypr/{instance}/.socket.sock")).ok()?;
    socket.write_all(request.as_bytes()).ok()?;
    let mut reply = String::new();
    socket.read_to_string(&mut reply).ok()?;
    Some(reply)
}

/// Hyprland: the centre of the Path of Exile 2 window, in global logical pixels.
pub fn game_center() -> Option<(i32, i32)> {
    let clients: serde_json::Value = serde_json::from_str(&hyprland_request("j/clients")?).ok()?;
    let game = clients.as_array()?.iter().find(|client| {
        client["class"].as_str().is_some_and(|class| class == "steam_app_2694490")
            || client["title"].as_str() == Some("Path of Exile 2")
    })?;
    let coord = |key: &str, i: usize| game[key][i].as_i64().map(|v| v as i32);
    Some((coord("at", 0)? + coord("size", 0)? / 2, coord("at", 1)? + coord("size", 1)? / 2))
}

/// Puts the overlay on the monitor that contains the point (global logical pixels).
pub fn set_monitor_at(window: &gtk::ApplicationWindow, x: i32, y: i32) {
    use gtk::prelude::{MonitorExt, WidgetExt};
    let display = window.display();
    let monitor = (0..display.n_monitors()).filter_map(|i| display.monitor(i)).find(|monitor| {
        let area = monitor.geometry();
        x >= area.x() && x < area.x() + area.width() && y >= area.y() && y < area.y() + area.height()
    });
    if let Some(monitor) = monitor {
        window.set_monitor(&monitor);
    }
}

/// The panel's real size (it is smaller than the layer surface). GTK doesn't pass a
/// resize of a mapped layer window on to the compositor, so the surface keeps its fixed,
/// generous size from tauri.conf.json; the area around the panel is transparent and
/// click-through. This size keeps the panel itself on its monitor.
static CONTENT_SIZE: std::sync::Mutex<Option<(i32, i32)>> = std::sync::Mutex::new(None);

pub fn set_content_size(width: i32, height: i32) {
    if let Ok(mut size) = CONTENT_SIZE.lock() {
        *size = Some((width, height));
    }
}

pub fn content_size(window: &gtk::ApplicationWindow) -> (i32, i32) {
    use gtk::prelude::GtkWindowExt;
    CONTENT_SIZE.lock().ok().and_then(|size| *size).unwrap_or_else(|| window.size())
}

/// The current offset from the output's top-left corner.
pub fn position(window: &gtk::ApplicationWindow) -> (i32, i32) {
    (window.layer_shell_margin(Edge::Left), window.layer_shell_margin(Edge::Top))
}

/// Where the character stands on the game's screen, as a share of its height (the
/// camera keeps the player horizontally centred, a little above the middle).
pub const PLAYER_Y: f64 = 0.45;

/// The compass: a layer surface that never takes input, centred on the player.
/// Call before the window is first shown, like `init`.
pub fn init_compass(window: &gtk::ApplicationWindow) {
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace("poe-batonga-compass");
    // Anchored to the top edge only: centred horizontally, placed down by its margin.
    window.set_anchor(Edge::Top, true);
    // -1: measure from the monitor's edge, ignoring the top bar's reserved space.
    window.set_exclusive_zone(-1);
    window.set_keyboard_mode(KeyboardMode::None);
}

/// Puts the compass on the game's monitor around the player, and makes it click-through.
/// Call on the main thread after the window is shown.
pub fn place_compass(window: &gtk::ApplicationWindow) {
    use gtk::prelude::{GtkWindowExt, MonitorExt, WidgetExt};
    let center = game_center();
    if let Some((x, y)) = center {
        set_monitor_at(window, x, y);
    }
    let display = window.display();
    let monitor = center
        .and_then(|(x, y)| display.monitor_at_point(x, y))
        .or_else(|| window.window().and_then(|w| display.monitor_at_window(&w)));
    if let Some(monitor) = monitor {
        let (_, height) = window.size();
        let area = monitor.geometry();
        let top = (area.height() as f64 * PLAYER_Y) as i32 - height / 2;
        window.set_layer_shell_margin(Edge::Top, top.max(0));
    }
    window.input_shape_combine_region(Some(&gtk::cairo::Region::create()));
}
