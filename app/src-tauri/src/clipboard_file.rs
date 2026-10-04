//! Read a file path from the system clipboard, the way a file manager leaves it there after
//! "Copy" — used when middle-clicking the pet window feeds it a file (see `feed_clipboard` in
//! `main.rs`), a fallback for desktops where dragging a file onto the window is awkward. Real
//! drag & drop (`on_drag_drop` in `main.rs`) is still the primary way to feed the app; this only
//! covers the gesture the Noctalia plugin already has (`plugin/lib/feed.luau`, which shells out
//! to `wl-paste` instead of linking GTK directly).
//!
//! Linux is verified live on this machine: `gtk::Clipboard::wait_for_uris` correctly reads a
//! `file://` URI a real `wl-copy` put on the compositor's clipboard, resolved through
//! `glib::filename_from_uri` into the same path. That round trip only completed once the probe
//! had a realized, presented window and kept the GLib main loop pumped while waiting — the pet
//! window always has both (it is shown as soon as it exists, and the app's own main loop is
//! always running), so this is not a caveat for the real feature, just a note for whoever next
//! pokes at this function from a standalone probe of their own. Windows is verified to at least
//! compile (`cargo check --target x86_64-pc-windows-gnu`, what `justfile`'s `app-windows` builds
//! with), not run on real Windows. macOS is a best reading of the public `NSPasteboard` API with
//! no SDK here to build it at all (the same situation `notch.rs` documents) — treat it as
//! unverified until it has run on a real Mac.

use std::path::PathBuf;

#[cfg(target_os = "linux")]
pub fn path_from_clipboard() -> Option<PathBuf> {
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    if let Some(uri) = clipboard.wait_for_uris().into_iter().next()
        && let Ok((path, _)) = gtk::glib::filename_from_uri(&uri)
    {
        return Some(path);
    }
    // Some file managers put a plain path on the clipboard instead of a `text/uri-list`.
    let text = clipboard.wait_for_text()?;
    let text = text.trim();
    text.starts_with('/').then(|| PathBuf::from(text))
}

#[cfg(windows)]
pub fn path_from_clipboard() -> Option<PathBuf> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard};
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
    const CF_HDROP: u32 = 15;
    // SAFETY: a plain open/read/close cycle of the clipboard, with every handle checked before use.
    unsafe {
        IsClipboardFormatAvailable(CF_HDROP).ok()?;
        OpenClipboard(Some(HWND::default())).ok()?;
        let path = (|| {
            let handle = GetClipboardData(CF_HDROP).ok()?;
            let hdrop = HDROP(handle.0);
            if DragQueryFileW(hdrop, 0xFFFF_FFFF, None) == 0 {
                return None;
            }
            let mut buf = [0u16; 32 * 1024];
            let len = DragQueryFileW(hdrop, 0, Some(&mut buf));
            (len > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
        })();
        let _ = CloseClipboard();
        path
    }
}

#[cfg(target_os = "macos")]
pub fn path_from_clipboard() -> Option<PathBuf> {
    use objc2_app_kit::NSPasteboard;
    use objc2_foundation::NSString;

    // SAFETY: plain reads of the general pasteboard's current contents.
    unsafe {
        let pb = NSPasteboard::generalPasteboard();
        let items = pb.pasteboardItems()?;
        let file_url_type = NSString::from_str("public.file-url");
        for item in items.iter() {
            let Some(s) = item.stringForType(&file_url_type) else { continue };
            if let Some(encoded) = s.to_string().strip_prefix("file://") {
                return Some(PathBuf::from(percent_decode(encoded)));
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(byte) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
