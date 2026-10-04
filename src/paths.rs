use std::env;
use std::path::PathBuf;

/// Where the socket and published state live: `$XDG_RUNTIME_DIR` when set (Linux
/// sessions), otherwise a private per-user directory (macOS and Windows have no
/// runtime dir; a bare `/tmp` would be shared between users).
fn runtime_dir() -> PathBuf {
    if let Some(d) = env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(d);
    }
    #[cfg(target_os = "macos")]
    {
        // SAFETY: getuid has no preconditions and cannot fail.
        let uid = unsafe { libc::getuid() };
        let dir = env::temp_dir().join(format!("sushi-{uid}"));
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));
        return dir;
    }
    #[cfg(windows)]
    {
        // %LOCALAPPDATA% is only readable by the user (and admins).
        let dir = env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(env::temp_dir).join("sushi").join("run");
        let _ = std::fs::create_dir_all(&dir);
        return dir;
    }
    #[allow(unreachable_code)]
    PathBuf::from("/tmp")
}

/// Whether a process with this pid exists (portable replacement for `/proc/<pid>`).
#[cfg(unix)]
pub fn pid_alive(pid: u32) -> bool {
    // Signal 0 only checks existence/permission. EPERM means it exists but is not ours.
    // SAFETY: kill with signal 0 sends nothing.
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(windows)]
pub fn pid_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    // SAFETY: plain handle calls; the handle is closed before returning.
    unsafe {
        let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return false;
        }
        // An exited process whose handle is still held by someone is signalled.
        let alive = WaitForSingleObject(h, 0) == WAIT_TIMEOUT;
        CloseHandle(h);
        alive
    }
}

/// The user's home: `$HOME`, or `%USERPROFILE%` on Windows.
pub fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").or_else(|| if cfg!(windows) { env::var_os("USERPROFILE") } else { None }).map(PathBuf::from)
}

/// Base for per-user config: `$XDG_CONFIG_HOME`, `%APPDATA%` on Windows, else `~/.config`.
pub fn config_home() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .or_else(|| if cfg!(windows) { env::var_os("APPDATA") } else { None })
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
}

/// Base for per-user caches: `$XDG_CACHE_HOME`, `%LOCALAPPDATA%` on Windows, else `~/.cache`.
pub fn cache_home() -> PathBuf {
    env::var_os("XDG_CACHE_HOME")
        .or_else(|| if cfg!(windows) { env::var_os("LOCALAPPDATA") } else { None })
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".cache")))
        .unwrap_or_else(|| PathBuf::from(".cache"))
}

/// Unix socket the daemon listens on (hooks and the CLI connect here).
pub fn socket_path() -> PathBuf {
    runtime_dir().join("sushi.sock")
}

/// Directory holding the published state for the Noctalia plugin.
pub fn state_dir() -> PathBuf {
    runtime_dir().join("sushi")
}

pub fn state_path() -> PathBuf {
    state_dir().join("state.json")
}

fn home_join(rel: &str) -> PathBuf {
    home_dir().map(|h| h.join(rel)).unwrap_or_else(|| PathBuf::from(rel))
}

pub fn claude_dir() -> PathBuf {
    home_join(".claude")
}

/// Codex's home (`$CODEX_HOME`, `~/.codex` by default).
pub fn codex_dir() -> PathBuf {
    env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home_join(".codex"))
}

/// opencode's config folder (`$XDG_CONFIG_HOME/opencode`).
pub fn opencode_dir() -> PathBuf {
    // opencode uses `~/.config` on every OS (including Windows).
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_join(".config"))
        .join("opencode")
}

/// GitHub Copilot CLI's home (`$COPILOT_HOME`, `~/.copilot` by default).
pub fn copilot_dir() -> PathBuf {
    env::var_os("COPILOT_HOME").map(PathBuf::from).unwrap_or_else(|| home_join(".copilot"))
}

/// Where Antigravity CLI reads its user-wide hooks (`~/.gemini/config`).
pub fn antigravity_config_dir() -> PathBuf {
    home_join(".gemini/config")
}

/// Gemini CLI's home (`~/.gemini`) — its own `settings.json`, not Antigravity's `config/hooks.json`.
pub fn gemini_dir() -> PathBuf {
    home_join(".gemini")
}

/// pi's agent folder (`~/.pi/agent`).
pub fn pi_dir() -> PathBuf {
    home_join(".pi/agent")
}

/// Optional user config (`~/.config/sushi/config.json`).
pub fn config_path() -> PathBuf {
    config_home()
        .join("sushi")
        .join("config.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_alive_sees_self_and_not_a_bogus_pid() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(i32::MAX as u32));
    }
}
