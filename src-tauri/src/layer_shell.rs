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
    use gtk::prelude::{GtkWindowExt, MonitorExt};
    let (width, height) = window.size();
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
    use gtk::prelude::{GtkWindowExt, MonitorExt, WidgetExt};
    let (width, height) = window.size();
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

/// The current offset from the output's top-left corner.
pub fn position(window: &gtk::ApplicationWindow) -> (i32, i32) {
    (window.layer_shell_margin(Edge::Left), window.layer_shell_margin(Edge::Top))
}
