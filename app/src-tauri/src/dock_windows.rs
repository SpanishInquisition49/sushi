//! Windows only: keep the pet window above normal windows and anchored to the top-center of the
//! primary monitor, instead of wherever it happened to land, and grow it in place into the
//! panel instead of opening a second, centered window.
//!
//! Uses only `SetWindowPos`/`HWND_TOPMOST` (the same mechanism widgets, overlays and tools like
//! Rainmeter use) — there is no public, documented Win32 API for "visible over every virtual
//! desktop" or "above exclusive full-screen apps" the way macOS's collection behavior and
//! status-level windows give for free, so those two guarantees are weaker here than on macOS
//! or the Linux layer-shell path. Signatures verified by cross-compiling this file for
//! `x86_64-pc-windows-gnu` on this machine (via `mingw-w64-gcc`); the actual windowing
//! behavior has not been run on real Windows.

use std::sync::Mutex;
use tauri::WebviewWindow;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromWindow};
use windows::Win32::UI::WindowsAndMessaging::{HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetWindowPos};

/// The panel's content size when expanded (mirrors what `tauri.conf.json`'s "panel" window is,
/// for the platforms that still use it; keep the two in sync by hand if that ever changes).
const PANEL_SIZE: (f64, f64) = (900.0, 600.0);

/// The pet pill's last fitted size, so collapsing after an expand restores it exactly instead
/// of recomputing (and possibly jittering) it.
static LAST_PILL: Mutex<(f64, f64)> = Mutex::new((300.0, 96.0));

/// Always true: `SetWindowPos`/`HWND_TOPMOST` is available on every Windows version this app
/// targets, unlike the Linux layer-shell path which depends on the compositor.
pub fn is_supported() -> bool {
    true
}

fn hwnd(window: &WebviewWindow) -> Option<HWND> {
    window.hwnd().ok()
}

/// Nothing to set up once on Windows (no window-level "collection behavior" to configure the
/// way macOS and Linux need): `apply_frame` re-asserts `HWND_TOPMOST` on every place/resize.
pub fn style(_window: &WebviewWindow) {}

fn remember_pill(width: f64, height: f64) {
    *LAST_PILL.lock().unwrap_or_else(|e| e.into_inner()) = (width, height);
}

fn apply_frame(window: &WebviewWindow, width: f64, height: f64) {
    let Some(h) = hwnd(window) else { return };
    let scale = window.scale_factor().unwrap_or(1.0);
    let (w_px, h_px) = ((width * scale).round() as i32, (height * scale).round() as i32);
    unsafe {
        let monitor = MonitorFromWindow(h, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(monitor, &mut mi).as_bool() {
            return;
        }
        let rc: RECT = mi.rcMonitor;
        let x = rc.left + ((rc.right - rc.left) - w_px) / 2;
        let y = rc.top;
        let _ = SetWindowPos(h, Some(HWND_TOPMOST), x, y, w_px, h_px, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }
}

/// Resize/reposition the pill to its natural content size, anchored at the top-center of the
/// primary monitor. Called from `fit_pet`, the same spot that already sizes the pet window on
/// every platform.
pub fn place_pill(window: &WebviewWindow, width: f64, height: f64) {
    remember_pill(width, height);
    apply_frame(window, width, height);
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
    apply_frame(window, w, h);
}
