// Sushi desktop app: a small always-on-top pet window and a panel window, both web views of
// `app/ui`. The daemon (`sushi daemon`) stays the single source of truth; this process follows its
// state over the local socket, forwards it to the windows and turns Allow / Deny clicks back
// into requests. It works the same on macOS, Linux and Windows.

#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use sushi::client::{request, watch};
use sushi::paths::config_home;
use sushi::protocol::Request;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, DragDropEvent, Emitter, Manager, WindowEvent};

// Docks the pet window (into the camera notch on macOS, above everything via `HWND_TOPMOST` on
// Windows, via `wlr-layer-shell` where the Linux compositor speaks it) and grows it in place
// into the panel, instead of the two-window model below — see `docked()` and each module's own
// doc comment for why, and the caveats. All three expose the same four functions, so the rest
// of this file calls `dock::` without caring which platform it is.
#[cfg(target_os = "macos")]
#[path = "notch.rs"]
mod dock;
#[cfg(target_os = "linux")]
#[path = "dock_linux.rs"]
mod dock;
#[cfg(windows)]
#[path = "dock_windows.rs"]
mod dock;

mod clipboard_file;

const TIMEOUT: Duration = Duration::from_secs(5);

/// The latest `{ up, snapshot }` sent to the windows, so a window that opens later can catch up.
#[derive(Default, Clone)]
struct Latest(Arc<Mutex<Value>>);

fn settings_path() -> PathBuf {
    config_home().join("sushi").join("app.json")
}

fn read_settings() -> Value {
    std::fs::read_to_string(settings_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

fn setting_on(key: &str) -> bool {
    read_settings().get(key).and_then(Value::as_bool).unwrap_or(true)
}

/// Run a request for a window command; the daemon's own error text goes back to the page.
fn send(req: Request) -> Result<(), String> {
    let reply = request(&req, TIMEOUT)?;
    if reply.ok { Ok(()) } else { Err(reply.error.unwrap_or_else(|| "the daemon refused the request".into())) }
}

#[tauri::command]
fn get_state(latest: tauri::State<Latest>) -> Value {
    latest.0.lock().map(|v| v.clone()).unwrap_or(Value::Null)
}

#[tauri::command]
async fn decide(action: String, id: u64) -> Result<(), String> {
    // "approve-edits": approve a plan and let the agent edit without asking.
    send(match action.as_str() {
        "approve" => Request::Approve { id, accept_edits: false },
        "approve-edits" => Request::Approve { id, accept_edits: true },
        _ => Request::Deny { id },
    })
}

#[tauri::command]
async fn answer(id: u64, answers: Value) -> Result<(), String> {
    send(Request::Answer { id, answers })
}

#[tauri::command]
async fn chat_send(text: String, model: Option<String>, agent: Option<String>, file: Option<String>) -> Result<(), String> {
    send(Request::ChatSend { text, model, agent, file })
}

#[tauri::command]
async fn chat_stop() -> Result<(), String> {
    send(Request::ChatStop)
}

#[tauri::command]
async fn chat_clear() -> Result<(), String> {
    send(Request::ChatClear)
}

#[tauri::command]
async fn care_feed() -> Result<(), String> {
    send(Request::CareFeed)
}

#[tauri::command]
async fn care_pet() -> Result<(), String> {
    send(Request::CarePet)
}

#[tauri::command]
async fn care_nap() -> Result<(), String> {
    send(Request::CareNap)
}

#[tauri::command]
async fn care_play(score: u32) -> Result<(), String> {
    send(Request::CarePlay { score })
}

#[tauri::command]
async fn care_buy(id: String) -> Result<(), String> {
    send(Request::CareBuy { id })
}

#[tauri::command]
async fn care_equip(id: String) -> Result<(), String> {
    send(Request::CareEquip { id })
}

#[tauri::command]
fn get_settings() -> Value {
    read_settings()
}

#[tauri::command]
fn set_settings(app: AppHandle, settings: Value) -> Result<(), String> {
    let path = settings_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let _ = app.emit("settings", settings);
    Ok(())
}

/// Whether this run docks the pet window instead of using the classic, freely draggable
/// two-window layout: off if the user turned the "dockedWindow" setting off (checked once at
/// startup — switching it live would mean tearing down and rebuilding the window from a
/// completely different model, so it takes a restart instead, like the setting says), otherwise
/// always true on macOS and Windows, probed on Linux (its compositor may not speak
/// `wlr-layer-shell`) since the probe itself may block briefly.
fn docked() -> bool {
    static DOCKED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DOCKED.get_or_init(|| setting_on("dockedWindow") && dock::is_supported())
}

fn show_panel(app: &AppHandle) {
    if docked() {
        if let Some(w) = app.get_webview_window("pet") {
            DOCK_EXPANDED.store(true, Ordering::SeqCst);
            dock::set_expanded(&w, true);
            let _ = w.set_focus();
            let _ = app.emit("dockMode", json!({ "expanded": true }));
        }
        return;
    }
    if let Some(w) = app.get_webview_window("panel") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[tauri::command]
fn toggle_panel(app: AppHandle) {
    if docked() {
        if DOCK_EXPANDED.load(Ordering::SeqCst) { close_panel(app) } else { show_panel(&app) }
        return;
    }
    match app.get_webview_window("panel") {
        Some(w) if w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) => {
            let _ = w.hide();
        }
        _ => show_panel(&app),
    }
}

#[tauri::command]
fn open_panel(app: AppHandle) {
    show_panel(&app);
}

#[tauri::command]
fn close_panel(app: AppHandle) {
    if docked() {
        if let Some(w) = app.get_webview_window("pet") {
            DOCK_EXPANDED.store(false, Ordering::SeqCst);
            dock::set_expanded(&w, false);
            let _ = app.emit("dockMode", json!({ "expanded": false }));
        }
        return;
    }
    if let Some(w) = app.get_webview_window("panel") {
        let _ = w.hide();
    }
}

/// Set once the pet window has been sized by its page (or given up on): it is shown from then on.
static PET_SHOWN: AtomicBool = AtomicBool::new(false);

/// Whether the docked window is currently showing the panel rather than the pill (see
/// `docked()`). There is no second window in that mode whose visibility could answer this.
static DOCK_EXPANDED: AtomicBool = AtomicBool::new(false);

/// The pet window takes the size of what it can show. Its minimum and maximum are that size too: a
/// fixed-size window is one tiling compositors (niri) open floating instead of giving it a column,
/// and they take its size only when it opens, so it stays hidden until it is sized.
#[tauri::command]
fn fit_pet(window: tauri::WebviewWindow, width: f64, height: f64) {
    if window.label() != "pet" || !(width >= 1.0 && height >= 1.0) {
        return;
    }
    if docked() {
        // While expanded this only remembers the size for when it collapses back: resizing the
        // window now would fight with it currently showing the panel.
        if DOCK_EXPANDED.load(Ordering::SeqCst) {
            dock::remember_pill_size(width.ceil(), height.ceil());
        } else {
            dock::place_pill(&window, width.ceil(), height.ceil());
        }
    } else {
        let size = tauri::LogicalSize::new(width.ceil(), height.ceil());
        let _ = window.set_min_size(Some(size));
        let _ = window.set_max_size(Some(size));
        let _ = window.set_size(size);
    }
    if !PET_SHOWN.swap(true, Ordering::SeqCst) {
        let _ = window.show();
    }
}

/// Show the pet window at its configured size if its page never sized it.
fn show_pet_anyway(app: &AppHandle) {
    if !PET_SHOWN.swap(true, Ordering::SeqCst)
        && let Some(w) = app.get_webview_window("pet")
    {
        let _ = w.show();
    }
}

/// Builds the pet window, and (when not docked, see `docked()`) the panel alongside it. Both are
/// built here rather than left to the config's own auto-create so a docked pet window can be
/// handed a different page (`?view=dock`) and skip the panel window entirely, growing itself
/// into it instead. Where it is not docked this is the same two windows as before: on Linux the
/// panel is made transient for the pet window because tiling compositors (niri) would otherwise
/// open it floating; on Windows it stays on its own so hiding the pet does not also hide it.
fn create_windows(app: &tauri::App) -> tauri::Result<()> {
    let windows = &app.config().app.windows;
    let Some(mut pet_config) = windows.iter().find(|w| w.label == "pet").cloned() else { return Ok(()) };

    if docked() {
        pet_config.url = tauri::WebviewUrl::App("index.html?view=dock".into());
        let pet = tauri::WebviewWindowBuilder::from_config(app.handle(), &pet_config)?.build()?;
        dock::style(&pet);
        return Ok(());
    }

    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
    let pet = tauri::WebviewWindowBuilder::from_config(app.handle(), &pet_config)?.build()?;
    let Some(panel_config) = windows.iter().find(|w| w.label == "panel").cloned() else { return Ok(()) };
    let builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &panel_config)?;
    #[cfg(target_os = "linux")]
    let builder = builder.parent(&pet)?;
    builder.build()?;
    Ok(())
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

/// Make sure a daemon is listening: if not, start the `sushi` binary that ships next to this app
/// (or the one on the PATH). It keeps running after the app quits, like a service would.
fn ensure_daemon() {
    if request(&Request::State, Duration::from_secs(1)).is_ok() {
        return;
    }
    let exe = format!("sushi{}", std::env::consts::EXE_SUFFIX);
    let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(&exe))).filter(|p| p.exists());
    let mut cmd = std::process::Command::new(beside.unwrap_or_else(|| PathBuf::from(exe)));
    cmd.arg("daemon").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000 | 0x0000_0008); // CREATE_NO_WINDOW | DETACHED_PROCESS
    }
    let _ = cmd.spawn();
}

fn pending_keys(snapshot: &Value) -> Vec<String> {
    snapshot["pending"]
        .as_array()
        .map(|a| a.iter().map(|p| format!("{}@{}", p["id"], p["created_ms"])).collect())
        .unwrap_or_default()
}

/// What the windows were last told, and the requests already seen (a new one opens the panel).
#[derive(Default)]
struct Feed {
    known: Vec<String>,
    first: bool,
    last_sent: String,
}

impl Feed {
    fn update(&mut self, app: &AppHandle, latest: &Latest, up: bool, snapshot: Value) {
        let keys = pending_keys(&snapshot);
        let payload = json!({ "up": up, "snapshot": snapshot });
        let body = payload.to_string();
        if body != self.last_sent {
            self.last_sent = body;
            if let Ok(mut l) = latest.0.lock() {
                *l = payload.clone();
            }
            let _ = app.emit("state", payload);
        }
        if !self.first && keys.iter().any(|k| !self.known.contains(k)) && setting_on("autoOpen") {
            show_panel(app);
        }
        self.known = keys;
        self.first = false;
    }
}

/// Follow the daemon's state and pass it on to the windows. The daemon pushes every change as it
/// happens; an older one that cannot be watched is asked every second, and a missing one is looked
/// for again every second.
fn state_loop(app: AppHandle, latest: Latest) {
    let mut feed = Feed { first: true, ..Feed::default() };
    loop {
        if watch(|snapshot| feed.update(&app, &latest, true, snapshot)).is_ok() {
            // The connection dropped: the daemon restarted or quit.
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        match request(&Request::State, Duration::from_secs(2)) {
            Ok(r) => feed.update(&app, &latest, true, r.state.unwrap_or(Value::Null)),
            Err(_) => feed.update(&app, &latest, false, Value::Null),
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn pet_event(app: &AppHandle, kind: &str) {
    let _ = app.emit("petEvent", json!({ "kind": kind, "ts": now_ms() }));
}

/// Feed `path` to the pet: it eats it and the panel opens on the chat with the file attached
/// (see `fedFile` in panel.js). Shared by a real drop and `feed_clipboard` below.
fn feed_file(app: &AppHandle, path: &std::path::Path) {
    pet_event(app, "eat");
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let _ = app.emit("fedFile", json!({ "path": path.to_string_lossy(), "name": name }));
    show_panel(app);
}

/// A file dragged onto either window is fed to the pet: it waves when the file comes over it, eats
/// it when dropped, and the panel opens on the chat with the file attached.
fn on_drag_drop(app: &AppHandle, event: &DragDropEvent) {
    match event {
        DragDropEvent::Enter { .. } => pet_event(app, "hello"),
        DragDropEvent::Drop { paths, .. } => match paths.iter().find(|p| p.is_file()) {
            Some(path) => feed_file(app, path),
            None => pet_event(app, "sad"), // a folder: nothing it can eat
        },
        _ => {}
    }
}

/// Middle-click on the pet: feed it whatever file a file manager's "Copy" put on the clipboard,
/// the fallback for desktops where dragging a file onto the window is awkward (see
/// `clipboard_file.rs`). A sad shake when there is nothing to eat, same as dropping a folder.
#[tauri::command]
fn feed_clipboard(app: AppHandle) {
    match clipboard_file::path_from_clipboard() {
        Some(path) if path.is_file() => feed_file(&app, &path),
        _ => pet_event(&app, "sad"),
    }
}

/// libayatana-appindicator, which the tray uses on Linux, warns on load that it is deprecated in
/// favour of its -glib successor, which the tray crate does not support yet: drop that one warning.
#[cfg(target_os = "linux")]
fn quiet_appindicator_warning() {
    let levels = glib::LogLevels::LEVEL_WARNING;
    glib::log_set_handler(Some("libayatana-appindicator"), levels, false, false, |domain, level, message| {
        if !message.starts_with("libayatana-appindicator is deprecated") {
            glib::log_default_handler(domain, level, Some(message));
        }
    });
}

fn main() {
    #[cfg(target_os = "linux")]
    quiet_appindicator_warning();
    let latest = Latest::default();
    tauri::Builder::default()
        .manage(latest.clone())
        .invoke_handler(tauri::generate_handler![
            get_state, decide, answer, chat_send, chat_stop, chat_clear, get_settings, set_settings, toggle_panel,
            open_panel, close_panel, fit_pet, quit, feed_clipboard, care_feed, care_pet, care_nap, care_play,
            care_buy, care_equip
        ])
        .on_window_event(|window, event| match event {
            // Closing the panel only hides it; the pet window is closed from the tray.
            WindowEvent::CloseRequested { api, .. } if window.label() == "panel" => {
                api.prevent_close();
                let _ = window.hide();
            }
            // Docked: clicking outside the window collapses it, like a Control Center drop-down.
            WindowEvent::Focused(false) if window.label() == "pet" && docked() && DOCK_EXPANDED.load(Ordering::SeqCst) => {
                close_panel(window.app_handle().clone());
            }
            WindowEvent::DragDrop(drop) => on_drag_drop(window.app_handle(), drop),
            _ => {}
        })
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory); // no Dock icon, like a menu bar app

            let toggle_pet = MenuItem::with_id(app, "pet", "Show / hide the pet", true, None::<&str>)?;
            let panel = MenuItem::with_id(app, "panel", "Open the panel", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&panel, &toggle_pet, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().ok_or("no window icon")?)
                .tooltip("Sushi")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "panel" => show_panel(app),
                    "pet" => {
                        if let Some(w) = app.get_webview_window("pet") {
                            if w.is_visible().unwrap_or(true) { let _ = w.hide(); } else { let _ = w.show(); }
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            create_windows(app)?;
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(3));
                show_pet_anyway(&handle);
            });
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                ensure_daemon();
                state_loop(handle, latest);
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Sushi app");
}
