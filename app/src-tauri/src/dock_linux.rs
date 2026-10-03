//! Linux only: dock the pet window via the `wlr-layer-shell` protocol — the same mechanism bars
//! like waybar use — on compositors that implement it (niri, Sway, Hyprland, and other
//! wlroots/Smithay-based Wayland compositors), so it reads as part of the desktop shell rather
//! than an ordinary floating window, and grows in place into the panel instead of opening a
//! second, centered window. GNOME, KDE and plain X11 sessions do not implement this protocol:
//! there, `is_supported()` returns false and `main.rs`'s `docked()` falls back to the classic
//! two-window layout, unchanged.
//!
//! Verified live on this machine's own niri/Wayland session (not blind, unlike the macOS and
//! Windows modules): `gtk_layer_is_supported()` returns true here, and `init_for_window` /
//! `set_layer` / `set_anchor` / `set_keyboard_mode` all ran without error against a real
//! `gtk::ApplicationWindow` upcast to `gtk::Window` — exactly the type and path Tauri's
//! `WebviewWindow::gtk_window()` gives us. The high-level `gtk-layer-shell` crate (0.8.2) does
//! not re-export those functions at all (checked its source), so this calls the `-sys` crate's
//! raw FFI directly instead, the same calls that wrapper would have made internally.

use glib::translate::ToGlibPtr;
use gtk::prelude::*;
use gtk_layer_shell_sys as gls;
use std::sync::Mutex;
use tauri::WebviewWindow;

/// The panel's content size when expanded (mirrors what `tauri.conf.json`'s "panel" window is,
/// for the platforms that still use it; keep the two in sync by hand if that ever changes).
const PANEL_SIZE: (f64, f64) = (900.0, 600.0);

/// The pet pill's last fitted size, so collapsing after an expand restores it exactly instead
/// of recomputing (and possibly jittering) it.
static LAST_PILL: Mutex<(f64, f64)> = Mutex::new((300.0, 96.0));

/// Whether this session's compositor speaks `wlr-layer-shell` at all. May block briefly for a
/// Wayland roundtrip the first time it runs (per the library's own doc comment), so `main.rs`
/// calls this once and caches the answer rather than asking on every command.
pub fn is_supported() -> bool {
    unsafe { gls::gtk_layer_is_supported() != 0 }
}

fn gtk_window(window: &WebviewWindow) -> Option<gtk::Window> {
    let w = window.gtk_window().ok()?;
    Some(w.upcast())
}

/// Turn the window into a layer-shell surface, anchored to the top of the screen, above normal
/// windows, on every workspace (layer-shell surfaces live outside the workspace model
/// entirely, so there is no per-workspace pinning to do). Keyboard mode is "on demand" rather
/// than "none" so the panel's chat input can still be typed into once expanded. Call once when
/// the window is created, before it is first shown.
pub fn style(window: &WebviewWindow) {
    let Some(w) = gtk_window(window) else { return };
    let ptr = w.to_glib_none().0;
    unsafe {
        gls::gtk_layer_init_for_window(ptr);
        gls::gtk_layer_set_layer(ptr, gls::GTK_LAYER_SHELL_LAYER_OVERLAY);
        gls::gtk_layer_set_anchor(ptr, gls::GTK_LAYER_SHELL_EDGE_TOP, 1);
        gls::gtk_layer_set_keyboard_mode(ptr, gls::GTK_LAYER_SHELL_KEYBOARD_MODE_ON_DEMAND);
    }
}

fn remember_pill(width: f64, height: f64) {
    *LAST_PILL.lock().unwrap_or_else(|e| e.into_inner()) = (width, height);
}

/// Anchored only to the top edge, the compositor centers the surface horizontally on its own
/// (the standard layer-shell convention for an axis with no anchor), so sizing the window is
/// all that is needed — no manual x/y placement like the macOS and Windows modules need.
///
/// Tauri's own `set_size` alone turned out not to be enough to actually resize a layer-shell
/// surface once realized (confirmed live: the window stayed invisible/zero-sized without this);
/// going through GTK directly — request, resize and a forced relayout — is what reliably works,
/// both for the window's first size and for a later live resize (collapsing/expanding).
fn apply_size(window: &WebviewWindow, width: f64, height: f64) {
    let size = tauri::LogicalSize::new(width, height);
    let _ = window.set_min_size(Some(size));
    let _ = window.set_max_size(Some(size));
    let _ = window.set_size(size);
    if let Some(w) = gtk_window(window) {
        let (w_px, h_px) = (width.ceil() as i32, height.ceil() as i32);
        w.set_size_request(w_px, h_px);
        w.resize(w_px, h_px);
        w.queue_resize();
    }
}

/// Resize the pill to its natural content size. Called from `fit_pet`, the same spot that
/// already sizes the pet window on every platform.
pub fn place_pill(window: &WebviewWindow, width: f64, height: f64) {
    remember_pill(width, height);
    apply_size(window, width, height);
}

/// Remember a new pill content size without touching the window: used while the panel is
/// expanded, when the pill's own fitting logic (still running in the background, see
/// `notch-window.js`) must not be allowed to resize the window out from under the panel.
pub fn remember_pill_size(width: f64, height: f64) {
    remember_pill(width, height);
}

/// Grow the window down into the full panel, or shrink it back to the last pill size, so it
/// reads as a dropdown growing out of the top of the screen instead of a second window opening
/// elsewhere.
pub fn set_expanded(window: &WebviewWindow, expanded: bool) {
    let (w, h) = if expanded { PANEL_SIZE } else { *LAST_PILL.lock().unwrap_or_else(|e| e.into_inner()) };
    apply_size(window, w, h);
}
