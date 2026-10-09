//! macOS notch geometry and window placement. All AppKit work runs on the main thread.

use block2::RcBlock;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_app_kit::{
    NSImage, NSImageScaling, NSImageView, NSScreen, NSStatusWindowLevel, NSView, NSWindow,
    NSWindowCollectionBehavior, NSWorkspace,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use tauri::{Emitter, WebviewWindow};

use super::Mode;
const LEFT_WING: f64 = 88.0;
const RIGHT_WING: f64 = 0.0;
// Content dimensions only: never remember the added camera inset.
static LAST_PILL: Mutex<(f64, f64)> = Mutex::new((300.0, 96.0));
static ATTENTION_HEIGHT: Mutex<f64> = Mutex::new(380.0);
static LAST_GEOMETRY: Mutex<Option<Geometry>> = Mutex::new(None);
// Restore only windows hidden because their camera geometry was unavailable, not by the user.
static GEOMETRY_HIDDEN: AtomicBool = AtomicBool::new(false);
static DISPLAY_REVISION: AtomicU64 = AtomicU64::new(0);
static FRAME_GENERATION: AtomicU64 = AtomicU64::new(0);
static RENDER_REVISION: AtomicU64 = AtomicU64::new(0);

// A native sibling of WKWebView survives its backing-store resize. It never consumes input.
define_class!(
    #[unsafe(super(NSImageView))]
    #[thread_kind = MainThreadOnly]
    struct NotchSnapshotView;

    impl NotchSnapshotView {
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView {
            std::ptr::null_mut()
        }
    }
);

#[derive(Clone, Copy, Debug, PartialEq)]
struct RenderToken {
    id: u64,
    revision: u64,
}

impl RenderToken {
    fn is_current(self, id: u64, revision: u64) -> bool {
        self.id == id && self.revision == revision
    }
}

struct RenderProtection {
    token: RenderToken,
    view: Retained<NotchSnapshotView>,
}

thread_local! {
    // AppKit objects are created, accessed and released only on the main thread.
    static RENDER_PROTECTION: RefCell<Option<RenderProtection>> = const { RefCell::new(None) };
}

fn clear_render_protection(token: Option<RenderToken>) {
    RENDER_PROTECTION.with_borrow_mut(|current| {
        if token.is_some_and(|token| current.as_ref().map(|p| p.token) != Some(token)) {
            return;
        }
        if let Some(protection) = current.take() {
            protection.view.removeFromSuperview();
        }
    });
}
static PRESENTATION: Mutex<Presentation> = Mutex::new(Presentation {
    settled: Mode::Collapsed,
    pending: None,
});

#[derive(Clone, Copy)]
struct Transition {
    id: u64,
    from: Mode,
    to: Mode,
    target: NSRect,
    source: NSRect,
}
impl Transition {
    fn payload(self, viewport: NSRect) -> Value {
        json!({ "id": self.id, "fromMode": self.from.name(), "toMode": self.to.name(),
            "durationMs": 160, "from": relative(self.source, viewport), "to": relative(self.target, viewport) })
    }
}

struct Presentation {
    settled: Mode,
    pending: Option<Transition>,
}
impl Presentation {
    fn complete(&mut self, id: u64) -> Option<NSRect> {
        let pending = self.pending.filter(|pending| pending.id == id)?;
        self.settled = pending.to;
        self.pending = None;
        Some(pending.target)
    }
}

fn envelope(a: NSRect, b: NSRect) -> NSRect {
    let x = a.origin.x.min(b.origin.x);
    let y = a.origin.y.min(b.origin.y);
    NSRect {
        origin: NSPoint { x, y },
        size: NSSize {
            width: (a.origin.x + a.size.width).max(b.origin.x + b.size.width) - x,
            height: (a.origin.y + a.size.height).max(b.origin.y + b.size.height) - y,
        },
    }
}

fn relative(frame: NSRect, viewport: NSRect) -> Value {
    json!({ "x": frame.origin.x - viewport.origin.x,
        "y": viewport.origin.y + viewport.size.height - frame.origin.y - frame.size.height,
        "width": frame.size.width, "height": frame.size.height })
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Geometry {
    frame: NSRect,
    center: f64,
    notch_width: f64,
    top_inset: f64,
    scale: f64,
    lateral_valid: bool,
}

impl Geometry {
    fn from_areas(frame: NSRect, top: f64, left: NSRect, right: NSRect, scale: f64) -> Self {
        let has_notch = top.is_finite() && top > 0.0;
        let gap_start = left.origin.x + left.size.width;
        let gap = right.origin.x - gap_start;
        let valid_area = |area: NSRect| {
            [
                area.origin.x,
                area.origin.y,
                area.size.width,
                area.size.height,
            ]
            .iter()
            .all(|v| v.is_finite())
                && area.origin.x >= frame.origin.x
                && area.origin.x + area.size.width <= frame.origin.x + frame.size.width
                && area.size.height >= top
                && (area.origin.y + area.size.height - (frame.origin.y + frame.size.height)).abs()
                    < 0.5
        };
        let valid_gap = has_notch
            && valid_area(left)
            && valid_area(right)
            && gap > 0.0
            && gap < frame.size.width / 2.0
            && left.size.width >= LEFT_WING
            && right.size.width >= RIGHT_WING;
        Self {
            frame,
            center: if has_notch && valid_gap {
                gap_start + gap / 2.0
            } else {
                frame.origin.x + frame.size.width / 2.0
            },
            notch_width: if valid_gap { gap } else { 0.0 },
            top_inset: if has_notch { top } else { 0.0 },
            scale,
            lateral_valid: valid_gap,
        }
    }

    fn payload(self) -> Value {
        json!({ "hasNotch": self.top_inset > 0.0,
            "notchWidth": self.notch_width, "topInset": self.top_inset,
            "lateralAvailable": self.lateral_valid,
            "leftWingWidth": if self.lateral_valid { LEFT_WING } else { 0.0 },
            "rightWingWidth": if self.lateral_valid { RIGHT_WING } else { 0.0 },
            "mode": super::dock_mode().name(),
            "displayRevision": DISPLAY_REVISION.load(Ordering::SeqCst) })
    }

    #[cfg(test)]
    fn target(self, content: (f64, f64), expanded: bool) -> Option<NSRect> {
        self.mode_target(
            content,
            if expanded {
                Mode::Full
            } else {
                Mode::Collapsed
            },
        )
    }

    fn mode_target(self, content: (f64, f64), mode: Mode) -> Option<NSRect> {
        let attention_height = *ATTENTION_HEIGHT.lock().unwrap_or_else(|e| e.into_inner());
        self.mode_target_with_attention(content, mode, attention_height)
    }

    fn mode_target_with_attention(self, content: (f64, f64), mode: Mode, attention_height: f64) -> Option<NSRect> {
        if mode == Mode::Collapsed && self.top_inset > 0.0 {
            return self.lateral_valid.then(|| NSRect {
                origin: NSPoint {
                    x: self.center - self.notch_width / 2.0 - LEFT_WING,
                    y: self.frame.origin.y + self.frame.size.height - self.top_inset,
                },
                size: NSSize {
                    width: LEFT_WING + self.notch_width + RIGHT_WING,
                    height: self.top_inset,
                },
            });
        }
        let (width, mut content_height) = mode.size().unwrap_or(content);
        if mode == Mode::Attention && self.lateral_valid {
            content_height = attention_height.clamp(1.0, 380.0)
                .min((self.frame.size.height - self.top_inset).max(1.0));
        }
        let height = content_height + self.top_inset;
        Some(NSRect {
            origin: NSPoint {
                x: self.center - width / 2.0,
                y: self.frame.origin.y + self.frame.size.height - height,
            },
            size: NSSize { width, height },
        })
    }
}

pub fn is_supported() -> bool {
    true
}

// Only call inside on_main. The marker prevents accidental background AppKit access.
fn with_window<R>(
    window: &WebviewWindow,
    f: impl FnOnce(&NSWindow, MainThreadMarker) -> R,
) -> Option<R> {
    let mtm = MainThreadMarker::new()?;
    let ptr = window.ns_window().ok()? as *mut NSWindow;
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { f(&*ptr, mtm) })
}

fn on_main(window: &WebviewWindow, f: impl FnOnce(WebviewWindow) + Send + 'static) {
    let owned = window.clone();
    if MainThreadMarker::new().is_some() {
        f(owned);
    } else if let Err(error) = window.run_on_main_thread(move || f(owned)) {
        eprintln!("notch: failed to schedule window update: {error}");
    }
}

fn geometry(w: &NSWindow, mtm: MainThreadMarker) -> Option<Geometry> {
    let screen = w.screen().or_else(|| NSScreen::mainScreen(mtm))?;
    Some(Geometry::from_areas(
        screen.frame(),
        screen.safeAreaInsets().top,
        screen.auxiliaryTopLeftArea(),
        screen.auxiliaryTopRightArea(),
        screen.backingScaleFactor(),
    ))
}

// Native bounds and child webview bounds must change together throughout an animation.
fn set_frame(window: &WebviewWindow, w: &NSWindow, frame: NSRect) {
    if w.frame() == frame {
        return;
    }
    w.setContentMinSize(NSSize {
        width: 1.0,
        height: 1.0,
    });
    w.setContentMaxSize(frame.size);
    w.setContentMinSize(frame.size);
    // Let the caller rebase its snapshot before AppKit flushes the new frame.
    w.setFrame_display_animate(frame, false, false);
    let bounds = tauri::Rect {
        position: tauri::LogicalPosition::new(0.0, 0.0).into(),
        size: tauri::LogicalSize::new(frame.size.width, frame.size.height).into(),
    };
    if let Err(error) = window.as_ref().set_bounds(bounds) {
        eprintln!("notch: failed to synchronize webview bounds: {error}");
    }
}

fn snapshot_wing(
    window: &WebviewWindow,
    wing: NSRect,
    done: impl Fn(&WebviewWindow, Option<&NSImage>, MainThreadMarker) + Send + 'static,
) -> tauri::Result<()> {
    let owned = window.clone();
    window.with_webview(move |platform| {
        let Some(mtm) = MainThreadMarker::new() else { return; };
        let ptr = platform.inner().cast::<WKWebView>();
        if ptr.is_null() { done(&owned, None, mtm); return; }
        // Tauri owns this WKWebView for the lifetime of the platform callback.
        let webview = unsafe { &*ptr };
        let Some(frame) = with_window(&owned, |w, _| w.frame()) else { return; };
        let rect = local_wing_rect(wing, frame, webview.isFlipped());
        let config = unsafe { WKSnapshotConfiguration::new(mtm) };
        unsafe {
            config.setRect(rect);
            config.setAfterScreenUpdates(true);
        }
        let callback = RcBlock::new(move |image: *mut NSImage, error: *mut objc2_foundation::NSError| {
            let Some(mtm) = MainThreadMarker::new() else { return; };
            if !error.is_null() {
                eprintln!("notch: snapshot failed: {}", unsafe { &*error });
            }
            // WebKit keeps the image alive for the duration of its completion handler.
            done(&owned, unsafe { image.as_ref() }, mtm);
        });
        unsafe { webview.takeSnapshotWithConfiguration_completionHandler(Some(&config), &callback); }
    })
}

fn contains_rect(outer: NSRect, inner: NSRect) -> bool {
    inner.origin.x >= outer.origin.x && inner.origin.y >= outer.origin.y
        && inner.origin.x + inner.size.width <= outer.origin.x + outer.size.width
        && inner.origin.y + inner.size.height <= outer.origin.y + outer.size.height
}

fn local_wing_rect(wing: NSRect, viewport: NSRect, flipped: bool) -> NSRect {
    NSRect {
        origin: NSPoint {
            x: wing.origin.x - viewport.origin.x,
            y: if flipped {
                viewport.origin.y + viewport.size.height - wing.origin.y - wing.size.height
            } else {
                wing.origin.y - viewport.origin.y
            },
        },
        size: wing.size,
    }
}

fn token_is_current(token: RenderToken) -> bool {
    token.is_current(FRAME_GENERATION.load(Ordering::SeqCst), RENDER_REVISION.load(Ordering::SeqCst))
}

fn present_frame(window: &WebviewWindow, g: Geometry, target: NSRect, id: u64) {
    let protect = with_window(window, |w, _| {
        let wing = g.mode_target((300.0, 96.0), Mode::Collapsed)?;
        (g.lateral_valid && w.isVisible() && w.frame() != target
            && contains_rect(w.frame(), wing)).then_some(wing)
    }).flatten();
    let revision = RENDER_REVISION.fetch_add(1, Ordering::SeqCst) + 1;
    let token = RenderToken { id, revision };
    if let Some(wing) = protect {
        let capture = snapshot_wing(window, wing, move |window, image, mtm| {
            if !token_is_current(token) { return; }
            with_window(window, |w, _| {
                if geometry(w, mtm) != Some(g) {
                    apply_frame(window, false, false);
                    return;
                }
                clear_render_protection(None);
                let overlay = image.and_then(|image| {
                    let parent = w.contentView()?;
                    let view: Retained<NotchSnapshotView> = unsafe {
                        msg_send![NotchSnapshotView::alloc(mtm), initWithFrame: local_wing_rect(wing, w.frame(), parent.isFlipped())]
                    };
                    view.setImage(Some(image));
                    view.setImageScaling(NSImageScaling::ScaleNone);
                    parent.addSubview(&view);
                    Some(view)
                });
                set_frame(window, w, target);
                if let Some(view) = overlay {
                    let parent_flipped = w.contentView().is_some_and(|parent| parent.isFlipped());
                    // Rebase in native coordinates in the same main-thread turn as setFrame.
                    view.setFrame(local_wing_rect(wing, target, parent_flipped));
                    RENDER_PROTECTION.with_borrow_mut(|p| *p = Some(RenderProtection { token, view }));
                    let owned = window.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(1500));
                        on_main(&owned, move |_| clear_render_protection(Some(token)));
                    });
                }
                w.displayIfNeeded();
                emit_presentation(window, g, target);
            });
        });
        if let Err(error) = capture {
            eprintln!("notch: failed to schedule snapshot: {error}");
            with_window(window, |w, _| { set_frame(window, w, target); w.displayIfNeeded(); });
            emit_presentation(window, g, target);
        }
    } else {
        with_window(window, |w, _| { set_frame(window, w, target); w.displayIfNeeded(); });
        emit_presentation(window, g, target);
    }
}

/// DOM acknowledgement alone precedes WebKit's native backing-store commit. A snapshot
/// with afterScreenUpdates fences that commit before uncovering the live camera band.
pub fn ack_render(window: &WebviewWindow, id: u64, revision: u64) {
    on_main(window, move |window| {
        let token = RenderToken { id, revision };
        let protected = RENDER_PROTECTION.with_borrow(|p| p.as_ref().is_some_and(|p| p.token == token));
        if !protected || !token_is_current(token) { return; }
        let wing = with_window(&window, |w, mtm| {
            geometry(w, mtm)?.mode_target((300.0, 96.0), Mode::Collapsed)
        }).flatten();
        if let Some(wing) = wing {
            if let Err(error) = snapshot_wing(&window, wing, move |_, _, _| clear_render_protection(Some(token))) {
                eprintln!("notch: failed to confirm rendered layout: {error}");
                clear_render_protection(Some(token));
            }
        } else {
            clear_render_protection(Some(token));
        }
    });
}

fn apply_frame(window: &WebviewWindow, animate: bool, only_if_changed: bool) -> bool {
    with_window(window, |w, mtm| {
        let Some(g) = geometry(w, mtm) else {
            clear_render_protection(None);
            FRAME_GENERATION.fetch_add(1, Ordering::SeqCst);
            PRESENTATION
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pending = None;
            *LAST_GEOMETRY.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return false;
        };
        let changed = {
            let mut last = LAST_GEOMETRY.lock().unwrap_or_else(|e| e.into_inner());
            let changed = *last != Some(g);
            *last = Some(g);
            changed
        };
        if changed {
            clear_render_protection(None);
            FRAME_GENERATION.fetch_add(1, Ordering::SeqCst);
            PRESENTATION
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pending = None;
            DISPLAY_REVISION.fetch_add(1, Ordering::SeqCst);
            super::DOCK_STATE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .display_changed(g.lateral_valid, super::setting_on("autoOpen"));
        }
        if only_if_changed && !changed {
            return true;
        }
        let mode = super::dock_mode();
        let content = *LAST_PILL.lock().unwrap_or_else(|e| e.into_inner());
        let Some(target) = g.mode_target(content, mode) else {
            if w.isVisible() {
                GEOMETRY_HIDDEN.store(true, Ordering::SeqCst);
                let _ = window.hide();
            }
            let _ = window.emit("dockLayout", g.payload());
            return false;
        };
        let start = w.frame();
        let reduced = NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        let mut presentation = PRESENTATION.lock().unwrap_or_else(|e| e.into_inner());
        if !changed
            && animate
            && !reduced
            && presentation
                .pending
                .is_some_and(|t| t.to == mode && t.target == target)
        {
            return true;
        }
        let from = presentation.pending.map_or(presentation.settled, |t| t.to);
        let source = presentation.pending.map_or(start, |t| t.target);
        let id = FRAME_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        clear_render_protection(None);
        let animate = animate
            && g.lateral_valid
            && !changed
            && !reduced
            && (start != target || presentation.pending.is_some());
        let viewport = if animate {
            envelope(start, target)
        } else {
            target
        };
        presentation.pending = animate.then_some(Transition {
            id,
            from,
            to: mode,
            target,
            source,
        });
        if !animate {
            presentation.settled = mode;
        }
        drop(presentation);
        present_frame(window, g, viewport, id);
        if animate {
            // Recovery only: rendering frames belong to WebKit's compositor, not a worker timer.
            let owned = window.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(500));
                finish_transition(&owned, id);
            });
        }
        if GEOMETRY_HIDDEN.swap(false, Ordering::SeqCst) {
            w.orderFrontRegardless();
        }
        true
    })
    .unwrap_or(false)
}

fn layout_payload(g: Geometry, viewport: NSRect) -> Value {
    let mut payload = g.payload();
    payload["transitionId"] = json!(FRAME_GENERATION.load(Ordering::SeqCst));
    // Keep displays without usable camera geometry on their existing presentation path.
    if !g.lateral_valid {
        return payload;
    }
    RENDER_PROTECTION.with_borrow(|p| {
        if let Some(p) = p.as_ref() {
            payload["renderAck"] = json!({ "transitionId": p.token.id, "revision": p.token.revision });
        }
    });
    let presentation = PRESENTATION.lock().unwrap_or_else(|e| e.into_inner());
    let content = *LAST_PILL.lock().unwrap_or_else(|e| e.into_inner());
    payload["viewport"] = json!({ "x": viewport.origin.x, "y": viewport.origin.y,
        "width": viewport.size.width, "height": viewport.size.height });
    for mode in [Mode::Collapsed, Mode::Preview, Mode::Attention, Mode::Full] {
        if let Some(frame) = g.mode_target(content, mode) {
            payload["frames"][mode.name()] = relative(frame, viewport);
        }
    }
    if let Some(t) = presentation.pending {
        payload["transition"] = t.payload(viewport);
    }
    payload
}

fn emit_presentation(window: &WebviewWindow, g: Geometry, viewport: NSRect) {
    let payload = layout_payload(g, viewport);
    let _ = window.emit("dockLayout", &payload);
    let _ = window.emit("dockMode", &payload);
}

pub fn finish_transition(window: &WebviewWindow, id: u64) {
    on_main(window, move |window| {
        let target = PRESENTATION
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .complete(id);
        let Some(target) = target else {
            return;
        };
        with_window(&window, |w, mtm| {
            let Some(g) = geometry(w, mtm) else {
                return;
            };
            // A display change invalidates the old target even if its refresh event is queued.
            let previous = *LAST_GEOMETRY.lock().unwrap_or_else(|e| e.into_inner());
            if previous != Some(g) {
                apply_frame(&window, false, false);
                return;
            }
            present_frame(&window, g, target, id);
        });
    });
}

pub fn style(window: &WebviewWindow) {
    on_main(window, |window| {
        with_window(&window, |w, _mtm| {
            w.setLevel(NSStatusWindowLevel);
            w.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::Stationary
                    | NSWindowCollectionBehavior::IgnoresCycle
                    | NSWindowCollectionBehavior::FullScreenAuxiliary,
            );
            w.setHasShadow(false);
            w.setOpaque(false);
        });
        apply_frame(&window, false, false);
    });
}

pub async fn get_layout(window: WebviewWindow) -> Result<Value, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let owned = window.clone();
    window
        .run_on_main_thread(move || {
            let layout = with_window(&owned, |w, mtm| {
                geometry(w, mtm).map(|g| layout_payload(g, w.frame()))
            })
            .flatten();
            let _ = tx.send(layout);
        })
        .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        rx.recv()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "notch: no screen available".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Measured content height excludes the camera band and never changes the pill dimensions.
pub fn fit_attention(window: &WebviewWindow, height: f64) {
    if !height.is_finite() || height < 1.0 { return; }
    let height = height.ceil().min(380.0);
    on_main(window, move |window| {
        let changed = {
            let mut previous = ATTENTION_HEIGHT.lock().unwrap_or_else(|e| e.into_inner());
            if *previous == height { false } else { *previous = height; true }
        };
        if changed && super::dock_mode() == Mode::Attention {
            apply_frame(&window, true, false);
        }
    });
}

/// AppKit reports the live pointer even when DOM replacement generated a spurious mouseleave.
pub async fn get_pointer(window: WebviewWindow) -> Result<Value, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let owned = window.clone();
    window.run_on_main_thread(move || {
        let point = with_window(&owned, |w, _| {
            let point = w.mouseLocationOutsideOfEventStream();
            json!({ "x": point.x, "y": w.frame().size.height - point.y,
                "transitionId": FRAME_GENERATION.load(Ordering::SeqCst) })
        });
        let _ = tx.send(point);
    }).map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        rx.recv().map_err(|e| e.to_string())?
            .ok_or_else(|| "notch: no window available".to_string())
    }).await.map_err(|e| e.to_string())?
}

pub fn place_pill(window: &WebviewWindow, width: f64, height: f64, animate: bool) {
    remember_pill_size(width, height);
    on_main(window, move |window| {
        apply_frame(&window, animate, false);
    });
}

pub fn remember_pill_size(width: f64, height: f64) {
    *LAST_PILL.lock().unwrap_or_else(|e| e.into_inner()) = (width, height);
}

pub fn set_mode(window: &WebviewWindow, focus: bool) {
    on_main(window, move |window| {
        if apply_frame(&window, true, false) && super::dock_mode() != Mode::Collapsed {
            // Tauri's show() makes the NSWindow key and would steal the typing focus.
            with_window(&window, |w, _| w.orderFrontRegardless());
            if focus {
                let _ = window.set_focus();
            }
        }
    });
}

/// Reposition even when the page's content size has not changed (display/scale changes).
pub fn refresh(window: &WebviewWindow) {
    on_main(window, |window| {
        apply_frame(&window, false, true);
    });
}

/// The fallback must anchor the window before showing it, even if the page never fitted it.
pub fn show_fallback(window: &WebviewWindow) {
    on_main(window, |window| {
        if apply_frame(&window, false, false) {
            with_window(&window, |w, _| w.orderFrontRegardless());
        } else {
            GEOMETRY_HIDDEN.store(true, Ordering::SeqCst);
        }
    });
}

pub fn toggle_visibility(window: &WebviewWindow) {
    on_main(window, |window| {
        if window.is_visible().unwrap_or(false) || GEOMETRY_HIDDEN.load(Ordering::SeqCst) {
            GEOMETRY_HIDDEN.store(false, Ordering::SeqCst);
            let _ = window.hide();
        } else {
            show_fallback(&window);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
        NSRect {
            origin: NSPoint { x, y },
            size: NSSize { width, height },
        }
    }
    fn notched() -> Geometry {
        Geometry::from_areas(
            rect(100.0, -200.0, 1500.0, 1000.0),
            32.0,
            rect(100.0, 768.0, 650.0, 32.0),
            rect(950.0, 768.0, 650.0, 32.0),
            2.0,
        )
    }
    #[test]
    fn temporary_envelope_keeps_camera_edges_identical_to_final_left_wing() {
        let g = notched();
        let wings = g.mode_target((300.0, 96.0), Mode::Collapsed).unwrap();
        for mode in [Mode::Preview, Mode::Attention, Mode::Full] {
            let panel = g.mode_target((300.0, 96.0), mode).unwrap();
            let temporary = envelope(panel, wings);
            assert_eq!(temporary, panel);
            assert_eq!(envelope(wings, panel), temporary);
            let local = relative(wings, temporary);
            assert_eq!(
                temporary.origin.x + local["x"].as_f64().unwrap() + LEFT_WING,
                750.0
            );
            assert_eq!(
                temporary.origin.x + local["x"].as_f64().unwrap() + wings.size.width,
                950.0
            );
            assert_eq!(local["y"], 0.0);
            let final_local = relative(wings, wings);
            assert_eq!(final_local["x"], 0.0);
            assert_eq!(final_local["y"], 0.0);
        }
    }

    #[test]
    fn native_snapshot_rebases_without_moving_on_screen_in_flipped_or_unflipped_views() {
        let g = notched();
        let wing = g.mode_target((300.0, 96.0), Mode::Collapsed).unwrap();
        for mode in [Mode::Preview, Mode::Attention, Mode::Full, Mode::Collapsed] {
            let viewport = envelope(wing, g.mode_target((300.0, 96.0), mode).unwrap());
            assert!(contains_rect(viewport, wing));
            for flipped in [false, true] {
                let local = local_wing_rect(wing, viewport, flipped);
                assert_eq!(viewport.origin.x + local.origin.x, wing.origin.x);
                let screen_y = if flipped {
                    viewport.origin.y + viewport.size.height - local.origin.y - local.size.height
                } else {
                    viewport.origin.y + local.origin.y
                };
                assert_eq!(screen_y, wing.origin.y);
                assert_eq!(local.size, wing.size);
            }
        }
        assert!(!contains_rect(rect(100.0, 0.0, 300.0, 96.0), wing));
    }

    #[test]
    fn render_confirmation_cannot_uncover_a_newer_settlement_or_transition() {
        let opening = RenderToken { id: 3, revision: 10 };
        let closing = RenderToken { id: 3, revision: 11 };
        assert!(opening.is_current(3, 10));
        // Final contraction reuses the transition id but needs its own render confirmation.
        assert!(!opening.is_current(3, 11));
        assert!(closing.is_current(3, 11));
        // Reentry and a monitor refresh both replace the transition generation.
        assert!(!closing.is_current(4, 11));
        assert!(!closing.is_current(4, 12));
    }

    #[test]
    fn only_current_completion_can_apply_final_native_bounds() {
        let target = notched()
            .mode_target((300.0, 96.0), Mode::Collapsed)
            .unwrap();
        let mut presentation = Presentation {
            settled: Mode::Preview,
            pending: Some(Transition {
                id: 3,
                from: Mode::Preview,
                to: Mode::Collapsed,
                target,
                source: target,
            }),
        };
        assert_eq!(presentation.complete(2), None);
        assert_eq!(presentation.settled, Mode::Preview);
        assert_eq!(presentation.pending.unwrap().id, 3);
        assert_eq!(presentation.complete(3), Some(target));
        assert_eq!(presentation.settled, Mode::Collapsed);
        assert_eq!(presentation.complete(3), None);
        presentation.pending = Some(Transition {
            id: 4,
            from: Mode::Collapsed,
            to: Mode::Preview,
            target,
            source: target,
        });
        // Monitor changes cancel the pending target before any old completion arrives.
        presentation.pending = None;
        assert_eq!(presentation.complete(4), None);
    }
    #[test]
    fn left_wing_ends_at_camera_base_without_extending_past_its_right_edge() {
        let g = notched();
        assert_eq!((g.center, g.notch_width, g.top_inset), (850.0, 200.0, 32.0));
        let target = g.target((400.0, 96.0), false).unwrap();
        assert_eq!(target, rect(662.0, 768.0, 288.0, 32.0));
        assert_eq!(target.origin.x + LEFT_WING, 750.0);
        assert_eq!(target.origin.x + target.size.width, 950.0);
        assert_eq!(
            target.origin.y,
            g.frame.origin.y + g.frame.size.height - g.top_inset
        );
        let mut other_scale = g;
        other_scale.scale = 1.0;
        assert_eq!(other_scale.target((50.0, 40.0), false), Some(target));
        assert_ne!(
            g, other_scale,
            "scale changes must trigger viewport synchronization"
        );
    }

    #[test]
    fn invalid_geometry_hides_only_collapsed_content() {
        let g = Geometry::from_areas(
            notched().frame,
            32.0,
            rect(0.0, 0.0, 0.0, 0.0),
            rect(0.0, 0.0, 0.0, 0.0),
            2.0,
        );
        assert_eq!(g.notch_width, 0.0);
        assert_eq!(g.target((300.0, 96.0), false), None);
        assert_eq!(
            g.target((300.0, 96.0), true),
            Some(rect(400.0, 168.0, 900.0, 632.0))
        );
        assert_eq!(g.payload()["lateralAvailable"], false);
        assert_eq!(g.payload()["leftWingWidth"], 0.0);
    }

    #[test]
    fn rejects_nan_wrong_vertical_band_and_insufficient_wing_space() {
        let frame = notched().frame;
        let right = rect(950.0, 768.0, 650.0, 32.0);
        for left in [
            rect(f64::NAN, 768.0, 650.0, 32.0),
            rect(100.0, 700.0, 650.0, 32.0),
            rect(663.0, 768.0, 87.0, 32.0),
        ] {
            assert!(
                Geometry::from_areas(frame, 32.0, left, right, 2.0)
                    .target((300.0, 96.0), false)
                    .is_none()
            );
        }
        let right = rect(1570.0, 768.0, 30.0, 32.0);
        // No space is needed on the right now; it only identifies the camera edge.
        let g = Geometry::from_areas(frame, 32.0, rect(100.0, 768.0, 1450.0, 32.0), right, 2.0);
        assert!(g.lateral_valid);
        let target = g.target((300.0, 96.0), false).unwrap();
        assert_eq!(target.origin.x + target.size.width, right.origin.x);
    }

    #[test]
    fn nap_and_content_updates_do_not_resize_left_wing() {
        let g = notched();
        assert_eq!(
            g.target((50.0, 40.0), false),
            g.target((400.0, 96.0), false)
        );
        assert_eq!(
            g.target((50.0, 40.0), true),
            Some(rect(400.0, 168.0, 900.0, 632.0))
        );
        assert_eq!(
            g.payload(),
            json!({ "hasNotch": true, "notchWidth": 200.0, "topInset": 32.0,
            "lateralAvailable": true, "leftWingWidth": 88.0, "rightWingWidth": 0.0,
            "mode": super::super::dock_mode().name(),
            "displayRevision": DISPLAY_REVISION.load(Ordering::SeqCst) })
        );
    }

    #[test]
    fn display_changes_update_camera_edges_and_height() {
        let g = Geometry::from_areas(
            rect(-1800.0, -500.0, 1800.0, 1200.0),
            38.0,
            rect(-1800.0, 662.0, 790.0, 38.0),
            rect(-750.0, 662.0, 750.0, 38.0),
            2.0,
        );
        assert_ne!(g, notched());
        assert_eq!(
            g.target((300.0, 96.0), false),
            Some(rect(-1098.0, 662.0, 348.0, 38.0))
        );
        assert_eq!(g.payload()["notchWidth"], 260.0);
        assert_eq!(g.payload()["topInset"], 38.0);
    }

    #[test]
    fn no_notch_preserves_standard_content_size() {
        let g = Geometry::from_areas(notched().frame, 0.0, notched().frame, notched().frame, 1.0);
        assert_eq!(
            g.target((50.0, 40.0), false),
            Some(rect(825.0, 760.0, 50.0, 40.0))
        );
        assert_eq!(
            g.target((50.0, 40.0), true),
            Some(rect(400.0, 200.0, 900.0, 600.0))
        );
        assert_eq!(
            g.payload(),
            json!({"hasNotch": false, "notchWidth": 0.0, "topInset": 0.0,
            "lateralAvailable": false, "leftWingWidth": 0.0, "rightWingWidth": 0.0,
            "mode": super::super::dock_mode().name(),
            "displayRevision": DISPLAY_REVISION.load(Ordering::SeqCst) })
        );
    }

    #[test]
    fn attention_height_fits_content_and_caps_to_the_display_without_changing_other_modes() {
        let g = notched();
        let target = g.mode_target_with_attention((300.0, 96.0), Mode::Attention, 184.0).unwrap();
        assert_eq!(target, rect(570.0, 584.0, 560.0, 216.0));
        assert_eq!(g.mode_target_with_attention((300.0, 96.0), Mode::Attention, 900.0).unwrap().size.height, 412.0);
        for mode in [Mode::Collapsed, Mode::Preview, Mode::Full] {
            assert_eq!(g.mode_target_with_attention((300.0, 96.0), mode, 184.0), g.mode_target((300.0, 96.0), mode));
        }
        let mut small = g;
        small.frame.size.height = 240.0;
        assert_eq!(small.mode_target_with_attention((300.0, 96.0), Mode::Attention, 380.0).unwrap().size.height, 240.0);
        let no_notch = Geometry::from_areas(g.frame, 0.0, g.frame, g.frame, 1.0);
        assert_eq!(no_notch.mode_target_with_attention((300.0, 96.0), Mode::Attention, 184.0).unwrap().size.height, 380.0);
    }

    #[test]
    fn resizing_attention_preserves_the_previous_height_as_the_animation_source() {
        let g = notched();
        let source = g.mode_target_with_attention((300.0, 96.0), Mode::Attention, 380.0).unwrap();
        let target = g.mode_target_with_attention((300.0, 96.0), Mode::Attention, 184.0).unwrap();
        let transition = Transition { id: 10, from: Mode::Attention, to: Mode::Attention, source, target };
        let viewport = envelope(source, target);
        let payload = transition.payload(viewport);
        assert_eq!(payload["from"]["height"], 412.0);
        assert_eq!(payload["to"]["height"], 216.0);
        assert_eq!(payload["from"]["y"], 0.0);
        assert_eq!(payload["to"]["y"], 0.0);
        assert_eq!(payload["to"]["width"], 560.0);
    }

    #[test]
    fn compact_panels_are_centered_below_the_camera_and_keep_fixed_left_wing() {
        let g = notched();
        assert_eq!(
            g.mode_target((300.0, 96.0), Mode::Preview),
            Some(rect(640.0, 548.0, 420.0, 252.0))
        );
        assert_eq!(
            g.mode_target((300.0, 96.0), Mode::Attention),
            Some(rect(570.0, 388.0, 560.0, 412.0))
        );
        let no_notch = Geometry::from_areas(g.frame, 0.0, g.frame, g.frame, 1.0);
        assert_eq!(
            no_notch.mode_target((300.0, 96.0), Mode::Attention),
            Some(rect(570.0, 420.0, 560.0, 380.0))
        );
        let mut invalid = g;
        invalid.lateral_valid = false;
        assert!(
            invalid
                .mode_target((300.0, 96.0), Mode::Collapsed)
                .is_none()
        );
        assert!(
            invalid
                .mode_target((300.0, 96.0), Mode::Attention)
                .is_some()
        );
    }
}
