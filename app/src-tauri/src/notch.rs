//! macOS only: dock the pet window into (or just under) the camera notch, above the menu bar
//! and visible on every Space and over full-screen apps, instead of letting it float as an
//! ordinary always-on-top window — and grow it in place into the full panel size instead of
//! opening a second, centered window.
//!
//! Written against the public AppKit selectors below (`NSScreen.safeAreaInsets`,
//! `.auxiliaryTopLeftArea`/`RightArea`, `NSWindow` levels and collection behavior); nothing
//! private. This cannot be built or run from here (no macOS SDK on this machine), so treat it
//! as unverified until it has been through a real `cargo tauri dev` on a Mac. If something is
//! off, the fix is local to this file.

use objc2::MainThreadMarker;
use objc2_app_kit::{NSScreen, NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::sync::Mutex;
use tauri::WebviewWindow;

/// The panel's content size when expanded (mirrors what `tauri.conf.json`'s old "panel" window
/// used to be; keep the two in sync by hand if that ever changes).
const PANEL_SIZE: (f64, f64) = (900.0, 600.0);

/// A sane notch width to fall back on if the aux-area probe below looks wrong (current MacBook
/// Pro notches are all close to this).
const FALLBACK_NOTCH_WIDTH: f64 = 220.0;

/// The pet pill's last fitted size, so collapsing after an expand restores it exactly instead
/// of recomputing (and possibly jittering) it.
static LAST_PILL: Mutex<(f64, f64)> = Mutex::new((300.0, 96.0));

/// Always true: the notch APIs used below (`safeAreaInsets` etc.) degrade to sane defaults on
/// their own on a Mac with no physical notch, unlike Linux's layer-shell, which depends on the
/// compositor and has to be probed at runtime.
pub fn is_supported() -> bool {
    true
}

fn with_window<R>(window: &WebviewWindow, f: impl FnOnce(&NSWindow) -> R) -> Option<R> {
    let ptr = window.ns_window().ok()? as *mut NSWindow;
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { f(&*ptr) })
}

fn behavior() -> NSWindowCollectionBehavior {
    NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::Stationary
        | NSWindowCollectionBehavior::IgnoresCycle
        | NSWindowCollectionBehavior::FullScreenAuxiliary
}

/// Pin the window above the menu bar, on every Space and over full-screen apps, so it reads as
/// a piece of system chrome rather than an ordinary floating app window. Call once when the
/// window is created.
pub fn style(window: &WebviewWindow) {
    with_window(window, |w| unsafe {
        w.setLevel(NSStatusWindowLevel);
        w.setCollectionBehavior(behavior());
        w.setHasShadow(false);
        w.setOpaque(false);
    });
}

/// The notch's rect in screen coordinates (bottom-left origin, as every `NSScreen` frame is),
/// or `None` on a screen with no physical notch (older MacBook, external display, pre-Monterey).
fn notch_rect(screen: &NSScreen) -> Option<NSRect> {
    let insets = unsafe { screen.safeAreaInsets() };
    if insets.top <= 0.0 {
        return None;
    }
    let frame = unsafe { screen.frame() };
    let left = unsafe { screen.auxiliaryTopLeftArea() };
    let right = unsafe { screen.auxiliaryTopRightArea() };
    let gap = right.origin.x - (left.origin.x + left.size.width);
    let width = if gap > 10.0 && gap < frame.size.width / 2.0 { gap } else { FALLBACK_NOTCH_WIDTH };
    Some(NSRect {
        origin: NSPoint { x: frame.origin.x + (frame.size.width - width) / 2.0, y: frame.origin.y + frame.size.height - insets.top },
        size: NSSize { width, height: insets.top },
    })
}

/// Where the pill/panel's top-center should sit: inside the notch when there is one, otherwise
/// centered just under the menu bar of the given screen (same window, same code path).
fn anchor(screen: &NSScreen) -> (f64, f64) {
    let frame = unsafe { screen.frame() };
    match notch_rect(screen) {
        Some(r) => (r.origin.y + r.size.height, r.origin.x + r.size.width / 2.0),
        None => (frame.origin.y + frame.size.height, frame.origin.x + frame.size.width / 2.0),
    }
}

fn remember_pill(width: f64, height: f64) {
    *LAST_PILL.lock().unwrap_or_else(|e| e.into_inner()) = (width, height);
}

fn apply_frame(window: &WebviewWindow, width: f64, height: f64, animate: bool) {
    // Keep the size constraints matched to whatever we are about to set: a stale min/max from
    // the previous state (pill vs. panel) would otherwise clamp this resize.
    let size = tauri::LogicalSize::new(width, height);
    let _ = window.set_min_size(Some(size));
    let _ = window.set_max_size(Some(size));
    with_window(window, |w| unsafe {
        let screen = w.screen().or_else(|| MainThreadMarker::new().and_then(NSScreen::mainScreen));
        let Some(screen) = screen else { return };
        let (top, mid_x) = anchor(&screen);
        let target = NSRect { origin: NSPoint { x: mid_x - width / 2.0, y: top - height }, size: NSSize { width, height } };
        w.setFrame_display_animate(target, true, animate);
    });
}

/// Resize/reposition the pill to its natural content size, anchored at the notch. Called from
/// `fit_pet`, the same spot that already sizes the pet window on every platform — this just
/// replaces "wherever the window manager left it" with "anchored at the notch" for the target.
pub fn place_pill(window: &WebviewWindow, width: f64, height: f64) {
    remember_pill(width, height);
    apply_frame(window, width, height, false);
}

/// Remember a new pill content size without touching the window: used while the panel is
/// expanded, when the pill's own fitting logic (still running in the background, see
/// `notch-window.js`) must not be allowed to resize the window out from under the panel.
pub fn remember_pill_size(width: f64, height: f64) {
    remember_pill(width, height);
}

/// Grow the window down into the full panel, or shrink it back to the last pill size — an
/// animated native resize, so it reads as a dropdown growing out of the notch instead of a
/// second window opening elsewhere on the screen.
pub fn set_expanded(window: &WebviewWindow, expanded: bool) {
    let (w, h) = if expanded { PANEL_SIZE } else { *LAST_PILL.lock().unwrap_or_else(|e| e.into_inner()) };
    apply_frame(window, w, h, true);
}
