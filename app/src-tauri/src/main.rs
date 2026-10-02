// Sushi desktop app: a small always-on-top pet window and a panel window, both web views of
// `app/ui`. The daemon (`sushi daemon`) stays the single source of truth; this process polls its
// state over the local socket, forwards it to the windows and turns Allow / Deny clicks back
// into requests. It works the same on macOS, Linux and Windows.

#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use sushi::client::request;
use sushi::paths::config_home;
use sushi::protocol::Request;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};

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
    send(if action == "approve" { Request::Approve { id } } else { Request::Deny { id } })
}

#[tauri::command]
async fn answer(id: u64, answers: Value) -> Result<(), String> {
    send(Request::Answer { id, answers })
}

#[tauri::command]
async fn chat_send(text: String, model: Option<String>, agent: Option<String>) -> Result<(), String> {
    send(Request::ChatSend { text, model, agent })
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

fn show_panel(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("panel") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[tauri::command]
fn toggle_panel(app: AppHandle) {
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
    if let Some(w) = app.get_webview_window("panel") {
        let _ = w.hide();
    }
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

/// Poll the daemon, tell the windows when something changed and open the panel for a new request.
fn poll_loop(app: AppHandle, latest: Latest) {
    let mut known: Vec<String> = Vec::new();
    let mut first = true;
    let mut last_sent = String::new();
    loop {
        let (up, snapshot) = match request(&Request::State, Duration::from_secs(2)) {
            Ok(r) => (true, r.state.unwrap_or(Value::Null)),
            Err(_) => (false, Value::Null),
        };
        let busy = snapshot["chat"]["busy"].as_bool().unwrap_or(false);
        let payload = json!({ "up": up, "snapshot": snapshot });
        let body = payload.to_string();
        if body != last_sent {
            last_sent = body;
            if let Ok(mut l) = latest.0.lock() {
                *l = payload.clone();
            }
            let _ = app.emit("state", payload);
        }
        let keys = pending_keys(&snapshot);
        if !first && keys.iter().any(|k| !known.contains(k)) && setting_on("autoOpen") {
            show_panel(&app);
        }
        known = keys;
        first = false;
        // Look more often while the chat streams an answer, so the text appears smoothly.
        std::thread::sleep(Duration::from_millis(if busy { 200 } else { 1000 }));
    }
}

fn main() {
    let latest = Latest::default();
    tauri::Builder::default()
        .manage(latest.clone())
        .invoke_handler(tauri::generate_handler![
            get_state, decide, answer, chat_send, chat_stop, chat_clear, get_settings, set_settings, toggle_panel,
            open_panel, close_panel, quit
        ])
        .on_window_event(|window, event| {
            // Closing the panel only hides it; the pet window is closed from the tray.
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "panel"
            {
                api.prevent_close();
                let _ = window.hide();
            }
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

            let handle = app.handle().clone();
            std::thread::spawn(move || {
                ensure_daemon();
                poll_loop(handle, latest);
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Sushi app");
}
