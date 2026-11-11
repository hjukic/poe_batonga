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
    move_to(window, left, top);
}

/// Locked: no keyboard focus ever, so the game keeps it. Unlocked: focus on click,
/// so the settings menu can record a shortcut.
pub fn set_interactive(window: &gtk::ApplicationWindow, interactive: bool) {
    window.set_keyboard_mode(if interactive { KeyboardMode::OnDemand } else { KeyboardMode::None });
}

pub fn move_to(window: &gtk::ApplicationWindow, left: i32, top: i32) {
    window.set_layer_shell_margin(Edge::Left, left.max(0));
    window.set_layer_shell_margin(Edge::Top, top.max(0));
}

/// The current offset from the output's top-left corner.
pub fn position(window: &gtk::ApplicationWindow) -> (i32, i32) {
    (window.layer_shell_margin(Edge::Left), window.layer_shell_margin(Edge::Top))
}
