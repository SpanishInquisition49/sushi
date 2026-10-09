//! `sushi install --agent <id> [--write]`: connect an agent to Sushi without touching anything
//! else of its setup.
//!
//! - Claude Code, Codex and Gemini CLI: add Sushi's hooks to their hooks file (existing hooks,
//!   other keys and key order stay as they are). Gemini's file is `~/.gemini/settings.json`, not
//!   to be confused with Antigravity's `~/.gemini/config/hooks.json` below.
//! - opencode and pi: write the plugin from `integrations/` into the agent's plugin folder.
//! - Antigravity CLI: add a `sushi` group to its user-wide `~/.gemini/config/hooks.json` (other
//!   groups stay as they are).
//! - GitHub Copilot CLI: write its own file, `~/.copilot/hooks/sushi.json` (nothing to merge).

use crate::agent::Agent;
use crate::paths;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const OPENCODE_PLUGIN: &str = include_str!("../integrations/opencode/sushi.ts");
const PI_EXTENSION: &str = include_str!("../integrations/pi/sushi.ts");
/// Replaced by the path of `sushi-hook` when a plugin is written.
const HOOK_PLACEHOLDER: &str = "__SUSHI_HOOK__";

/// The command an agent runs for each event: `sushi-hook`, plus `--agent <id>` for all but Claude.
fn hook_command(hook: &Path, agent: Agent) -> String {
    let path = hook.to_string_lossy();
    // Forward slashes work in cmd, PowerShell and Git Bash alike; backslashes would be eaten by bash.
    // Double quotes are the only quoting all of them share.
    let path = if cfg!(windows) { path.replace('\\', "/") } else { path.into_owned() };
    let path = if !path.contains(' ') {
        path
    } else if cfg!(windows) {
        format!("\"{path}\"")
    } else {
        format!("'{path}'")
    };
    match agent {
        Agent::Claude => path,
        other => format!("{path} --agent {}", other.id()),
    }
}

/// Seconds Claude Code and Codex give the `PermissionRequest` hook.
const PERMISSION_HOOK_TIMEOUT: u32 = 310;

/// The hooks Sushi needs, all running `hook` (the `sushi-hook` binary).
pub fn hook_snippet(hook: &Path, agent: Agent) -> Value {
    if agent == Agent::Gemini {
        return gemini_hooks(hook);
    }
    let command = hook_command(hook, agent);
    let entry = |timeout: u32| json!([{ "matcher": "*", "hooks": [{ "type": "command", "command": command, "timeout": timeout }] }]);
    let mut hooks = serde_json::Map::new();
    for ev in [
        "SessionStart", "SessionEnd", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PostToolUseFailure",
        "Notification", "Stop",
    ] {
        hooks.insert(ev.into(), entry(5));
    }
    // Long enough for a Claude Code plan (the hook waits up to 300 s for one); a permission waits 30 s.
    hooks.insert("PermissionRequest".into(), entry(PERMISSION_HOOK_TIMEOUT));
    if agent == Agent::Codex {
        hooks.insert("Interrupt".into(), entry(3));
    }
    json!({ "hooks": hooks })
}

/// Sushi's hooks for Gemini CLI's `~/.gemini/settings.json`: the same group shape as Claude's,
/// under Gemini's own event names (see `gemini.rs`), and each handler carries a `name` (the
/// format's own key, read by nothing of Sushi's). There is no separate permission event: `BeforeTool`
/// fires for every tool call and may turn into a wait (see `gemini::needs_confirmation`), so it
/// gets the longer timeout every other event here does not need.
fn gemini_hooks(hook: &Path) -> Value {
    let command = hook_command(hook, Agent::Gemini);
    let entry = |timeout: u32| json!([{ "matcher": "*", "hooks": [{ "name": "sushi", "type": "command", "command": command, "timeout": timeout }] }]);
    let mut hooks = serde_json::Map::new();
    for ev in ["SessionStart", "SessionEnd", "BeforeAgent", "AfterAgent", "AfterTool", "Notification"] {
        hooks.insert(ev.into(), entry(5));
    }
    hooks.insert("BeforeTool".into(), entry(40));
    json!({ "hooks": hooks })
}

/// Add each event's hook unless a hook with the same command is already there (whose timeout is
/// brought up to date). Returns the events that were added or changed.
pub fn merge_hooks(settings: &mut Value, snippet: &Value) -> Vec<String> {
    let mut added = Vec::new();
    let Some(wanted) = snippet.get("hooks").and_then(Value::as_object) else { return added };
    if !settings.is_object() {
        *settings = json!({});
    }
    let hooks = settings
        .as_object_mut()
        .expect("object")
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let Some(hooks) = hooks.as_object_mut() else { return added };
    for (event, groups) in wanted {
        let command = groups.pointer("/0/hooks/0/command").and_then(Value::as_str).unwrap_or("");
        let current = hooks.entry(event.clone()).or_insert_with(|| json!([]));
        let Some(list) = current.as_array_mut() else { continue };
        let timeout = groups.pointer("/0/hooks/0/timeout");
        let mut present = false;
        let mut changed = false;
        for h in list.iter_mut().filter_map(|g| g.get_mut("hooks").and_then(Value::as_array_mut)).flatten() {
            if h.get("command").and_then(Value::as_str) == Some(command) {
                present = true;
                if let (Some(t), Some(obj)) = (timeout, h.as_object_mut())
                    && obj.get("timeout") != Some(t)
                {
                    obj.insert("timeout".into(), t.clone());
                    changed = true;
                }
            }
        }
        if changed {
            added.push(event.clone());
        }
        if !present {
            list.extend(groups.as_array().cloned().unwrap_or_default());
            added.push(event.clone());
        }
    }
    added
}

/// Merge Sushi-managed `permissions.ask` / `permissions.deny` patterns (Claude Code's own syntax,
/// e.g. `Bash(rm -rf*)`) into `settings`, keeping every existing entry (nothing is ever removed).
/// Returns which of "permissions.ask" / "permissions.deny" changed. This is the only way Sushi can
/// make Claude Code genuinely ask or refuse a tool call it would otherwise auto-approve — see
/// `policy.rs` for why the other agents only get a visibility flag, not real enforcement.
pub fn merge_permissions(settings: &mut Value, perms: &crate::usage::ClaudePermissions) -> Vec<String> {
    let mut changed = Vec::new();
    if !settings.is_object() {
        *settings = json!({});
    }
    let permissions = settings.as_object_mut().expect("object").entry("permissions").or_insert_with(|| json!({}));
    let Some(permissions) = permissions.as_object_mut() else { return changed };
    for (key, patterns) in [("ask", &perms.ask), ("deny", &perms.deny)] {
        if patterns.is_empty() {
            continue;
        }
        let list = permissions.entry(key).or_insert_with(|| json!([]));
        let Some(list) = list.as_array_mut() else { continue };
        for p in patterns {
            if !list.iter().any(|v| v.as_str() == Some(p.as_str())) {
                list.push(json!(p));
                let name = format!("permissions.{key}");
                if !changed.contains(&name) {
                    changed.push(name);
                }
            }
        }
    }
    changed
}

/// Merge `perms` into the Claude Code settings file at `path` (created if missing). A different
/// previous file is backed up, same as `install`. A no-op when `perms` is empty.
pub fn install_permissions(path: &Path, perms: &crate::usage::ClaudePermissions, unix_time: u64) -> Result<(Vec<String>, Option<PathBuf>), String> {
    if perms.ask.is_empty() && perms.deny.is_empty() {
        return Ok((Vec::new(), None));
    }
    let existing = std::fs::read_to_string(path).ok();
    let mut settings: Value = match &existing {
        Some(text) => serde_json::from_str(text).map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?,
        None => json!({}),
    };
    let changed = merge_permissions(&mut settings, perms);
    if changed.is_empty() {
        return Ok((changed, None));
    }
    let backup = match existing {
        Some(text) => {
            let b = backup_name(path, unix_time);
            std::fs::write(&b, text).map_err(|e| format!("cannot write the backup {}: {e}", b.display()))?;
            Some(b)
        }
        None => None,
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())? + "\n";
    std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, path)).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok((changed, backup))
}

pub fn settings_path() -> PathBuf {
    paths::claude_dir().join("settings.json")
}

/// Where each agent is connected: its hooks file or its plugin file.
pub fn target_path(agent: Agent) -> PathBuf {
    match agent {
        Agent::Claude => settings_path(),
        Agent::Codex => paths::codex_dir().join("hooks.json"),
        Agent::Opencode => paths::opencode_dir().join("plugins").join("sushi.ts"),
        Agent::Pi => paths::pi_dir().join("extensions").join("sushi.ts"),
        Agent::Copilot => paths::copilot_dir().join("hooks").join("sushi.json"),
        Agent::Antigravity => paths::antigravity_config_dir().join("hooks.json"),
        Agent::Gemini => paths::gemini_dir().join("settings.json"),
    }
}

/// Sushi's hook file for Copilot: its own format (`bash` command, `timeoutSec`), no matcher groups.
fn copilot_hooks(hook: &Path) -> Value {
    let command = hook_command(hook, Agent::Copilot);
    let entry = |timeout: u32| {
        let mut e = json!({ "type": "command", "bash": command, "timeoutSec": timeout });
        if cfg!(windows) {
            e["powershell"] = json!(command); // Copilot runs `powershell` on Windows
        }
        json!([e])
    };
    let mut hooks = serde_json::Map::new();
    for ev in [
        "SessionStart", "SessionEnd", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PostToolUseFailure",
        "Notification", "Stop",
    ] {
        hooks.insert(ev.into(), entry(5));
    }
    hooks.insert("PermissionRequest".into(), entry(40));
    json!({ "version": 1, "hooks": hooks })
}

/// Name of Sushi's group in Antigravity's `hooks.json`.
const ANTIGRAVITY_GROUP: &str = "sushi";

/// Sushi's group for Antigravity: named, with its own event names and timeouts in seconds. Nothing
/// waits for the notch (a hook cannot approve there), so the timeouts are short. Events without a tool take handlers directly.
fn antigravity_group(hook: &Path) -> Value {
    let command = hook_command(hook, Agent::Antigravity);
    let handler = |timeout: u32| json!({ "type": "command", "command": command, "timeout": timeout });
    json!({ ANTIGRAVITY_GROUP: {
        "enabled": true,
        "PreToolUse": [{ "matcher": "*", "hooks": [handler(5)] }],
        "PostToolUse": [{ "matcher": "*", "hooks": [handler(5)] }],
        "PreInvocation": [handler(5)],
        "Stop": [handler(5)],
    } })
}

/// The file written for an agent that gets a file of its own (a plugin for opencode and pi, the
/// hook file for Copilot), with the hook's path filled in.
fn plugin_source(agent: Agent, hook: &Path) -> Option<String> {
    let template = match agent {
        Agent::Opencode => OPENCODE_PLUGIN,
        Agent::Pi => PI_EXTENSION,
        Agent::Copilot => return Some(serde_json::to_string_pretty(&copilot_hooks(hook)).unwrap_or_default() + "\n"),
        Agent::Claude | Agent::Codex | Agent::Antigravity | Agent::Gemini => return None,
    };
    Some(template.replace(HOOK_PLACEHOLDER, &hook.to_string_lossy()))
}

/// What `install` would write, for a dry run.
pub fn preview(agent: Agent, hook: &Path) -> String {
    if agent == Agent::Antigravity {
        return serde_json::to_string_pretty(&antigravity_group(hook)).unwrap_or_default();
    }
    match plugin_source(agent, hook) {
        Some(src) => src,
        None => serde_json::to_string_pretty(&hook_snippet(hook, agent)).unwrap_or_default(),
    }
}

/// What an install did.
pub struct Report {
    pub path: PathBuf,
    /// Events whose hook was added, or `["plugin"]` when a plugin file was written.
    pub added: Vec<String>,
    pub backup: Option<PathBuf>,
}

/// Connect `agent` to Sushi. Idempotent; the previous file is backed up when it changes.
pub fn install_agent(agent: Agent, hook: &Path, unix_time: u64) -> Result<Report, String> {
    let path = target_path(agent);
    if agent == Agent::Antigravity {
        let (added, backup) = install_group(&path, &antigravity_group(hook), unix_time)?;
        return Ok(Report { path, added, backup });
    }
    let (added, backup) = match plugin_source(agent, hook) {
        Some(src) => install_plugin(&path, &src, unix_time)?,
        None => install(&path, hook, agent, unix_time)?,
    };
    Ok(Report { path, added, backup })
}

fn backup_name(path: &Path, unix_time: u64) -> PathBuf {
    let name = format!("{}.bak-sushi-{unix_time}", path.file_name().and_then(|n| n.to_str()).unwrap_or("file"));
    path.with_file_name(name)
}

/// Write a plugin file (created with its folder if missing). A different previous version is
/// kept next to it as a backup.
pub fn install_plugin(path: &Path, source: &str, unix_time: u64) -> Result<(Vec<String>, Option<PathBuf>), String> {
    let existing = std::fs::read_to_string(path).ok();
    if existing.as_deref() == Some(source) {
        return Ok((Vec::new(), None));
    }
    let backup = match existing {
        Some(text) => {
            let b = backup_name(path, unix_time);
            std::fs::write(&b, text).map_err(|e| format!("cannot write the backup {}: {e}", b.display()))?;
            Some(b)
        }
        None => None,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(path, source).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok((vec!["plugin".to_string()], backup))
}

/// Set each named group of `groups` in the JSON object at `path` (created if missing), keeping
/// the other groups. A different previous file is backed up. Returns (events set, backup path).
pub fn install_group(path: &Path, groups: &Value, unix_time: u64) -> Result<(Vec<String>, Option<PathBuf>), String> {
    let existing = std::fs::read_to_string(path).ok();
    let mut file: Value = match &existing {
        Some(text) => serde_json::from_str(text).map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?,
        None => json!({}),
    };
    let Some(obj) = file.as_object_mut() else { return Err(format!("{} is not a JSON object", path.display())) };
    let mut added = Vec::new();
    for (name, group) in groups.as_object().into_iter().flatten() {
        if obj.get(name) != Some(group) {
            obj.insert(name.clone(), group.clone());
            added.extend(group.as_object().into_iter().flatten().filter(|(_, v)| v.is_array()).map(|(k, _)| k.clone()));
        }
    }
    if added.is_empty() {
        return Ok((added, None));
    }
    let backup = match existing {
        Some(text) => {
            let b = backup_name(path, unix_time);
            std::fs::write(&b, text).map_err(|e| format!("cannot write the backup {}: {e}", b.display()))?;
            Some(b)
        }
        None => None,
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())? + "\n";
    std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, path)).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok((added, backup))
}

/// Merge the hooks into `path` (created if missing). The previous file is kept next to it as
/// `<name>.bak-sushi-<unix time>`. Returns (events added, backup path if any).
pub fn install(path: &Path, hook: &Path, agent: Agent, unix_time: u64) -> Result<(Vec<String>, Option<PathBuf>), String> {
    let existing = std::fs::read_to_string(path).ok();
    let mut settings: Value = match &existing {
        Some(text) => serde_json::from_str(text).map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?,
        None => json!({}),
    };
    let added = merge_hooks(&mut settings, &hook_snippet(hook, agent));
    if added.is_empty() {
        return Ok((added, None)); // nothing to change, nothing to back up
    }
    let backup = match existing {
        Some(text) => {
            let b = backup_name(path, unix_time);
            std::fs::write(&b, text).map_err(|e| format!("cannot write the backup {}: {e}", b.display()))?;
            Some(b)
        }
        None => None,
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())? + "\n";
    std::fs::write(&tmp, body).and_then(|_| std::fs::rename(&tmp, path)).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok((added, backup))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook() -> PathBuf {
        PathBuf::from("/home/me/.cargo/bin/sushi-hook")
    }

    #[test]
    fn adds_every_event_to_an_empty_settings_object() {
        let mut s = json!({});
        let added = merge_hooks(&mut s, &hook_snippet(&hook(), Agent::Claude));
        assert_eq!(added.len(), 9);
        assert!(s["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"] == 310);
    }

    #[test]
    fn an_older_timeout_is_brought_up_to_date() {
        let mut s = json!({});
        merge_hooks(&mut s, &hook_snippet(&hook(), Agent::Claude));
        s["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"] = json!(40);
        let changed = merge_hooks(&mut s, &hook_snippet(&hook(), Agent::Claude));
        assert_eq!(changed, ["PermissionRequest"]);
        assert_eq!(s["hooks"]["PermissionRequest"].as_array().unwrap().len(), 1, "not added twice");
        assert_eq!(s["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"], 310);
    }

    #[test]
    fn keeps_other_hooks_and_keys_and_is_idempotent() {
        let mut s = json!({
            "model": "sonnet",
            "hooks": { "SessionStart": [{ "matcher": "*", "hooks": [{ "type": "command", "command": "bash herdr.sh" }] }] },
            "theme": "auto"
        });
        let before = s.clone();
        let added = merge_hooks(&mut s, &hook_snippet(&hook(), Agent::Claude));
        assert!(added.contains(&"SessionStart".to_string()));
        // the existing SessionStart group is still first, ours follows
        assert_eq!(s["hooks"]["SessionStart"][0], before["hooks"]["SessionStart"][0]);
        assert_eq!(s["hooks"]["SessionStart"].as_array().unwrap().len(), 2);
        assert_eq!(s["model"], "sonnet");
        assert_eq!(s["theme"], "auto");
        // the key order of the user's file is unchanged
        let keys: Vec<_> = s.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["model", "hooks", "theme"]);
        // running it again changes nothing
        let again = merge_hooks(&mut s, &hook_snippet(&hook(), Agent::Claude));
        assert!(again.is_empty());
        assert_eq!(s["hooks"]["SessionStart"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn writes_a_backup_only_when_something_changes() {
        let dir = std::env::temp_dir().join(format!("sushi-install-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, "{\n  \"model\": \"sonnet\"\n}\n").unwrap();

        let (added, backup) = install(&path, &hook(), Agent::Claude, 111).unwrap();
        assert_eq!(added.len(), 9);
        let backup = backup.expect("backup of the old file");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "{\n  \"model\": \"sonnet\"\n}\n");
        assert!(backup.to_string_lossy().ends_with("settings.json.bak-sushi-111"));
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["model"], "sonnet");

        let (added, backup) = install(&path, &hook(), Agent::Claude, 222).unwrap();
        assert!(added.is_empty() && backup.is_none(), "second run: nothing to do");
        assert!(!dir.join("settings.json.bak-sushi-222").exists());

        // a file that is not JSON is never overwritten
        std::fs::write(&path, "not json").unwrap();
        assert!(install(&path, &hook(), Agent::Claude, 333).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_file_is_created_without_a_backup() {
        let dir = std::env::temp_dir().join(format!("sushi-install2-{}", std::process::id()));
        let path = dir.join("settings.json");
        let (added, backup) = install(&path, &hook(), Agent::Claude, 1).unwrap();
        assert_eq!(added.len(), 9);
        assert!(backup.is_none() && path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn other_agents_run_the_hook_with_their_id() {
        let snippet = hook_snippet(&hook(), Agent::Codex);
        assert_eq!(snippet["hooks"]["Interrupt"][0]["hooks"][0]["timeout"], 3);
        assert_eq!(snippet["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "/home/me/.cargo/bin/sushi-hook --agent codex");
        let claude = hook_snippet(&hook(), Agent::Claude);
        assert!(claude["hooks"].get("Interrupt").is_none());
        assert_eq!(claude["hooks"]["PreToolUse"][0]["hooks"][0]["command"], "/home/me/.cargo/bin/sushi-hook");
        let spaced = hook_snippet(Path::new("/my bin/sushi-hook"), Agent::Codex);
        let expected = if cfg!(windows) {
            "\"/my bin/sushi-hook\" --agent codex"
        } else {
            "'/my bin/sushi-hook' --agent codex"
        };
        assert_eq!(spaced["hooks"]["Stop"][0]["hooks"][0]["command"], expected);
    }

    #[test]
    fn antigravity_gets_a_named_group_in_its_hooks_file() {
        let g = antigravity_group(&hook());
        let sushi = &g["sushi"];
        assert_eq!(sushi["PreToolUse"][0]["hooks"][0]["command"], "/home/me/.cargo/bin/sushi-hook --agent antigravity");
        assert_eq!((sushi["PreToolUse"][0]["hooks"][0]["timeout"].as_u64(), sushi["Stop"][0]["timeout"].as_u64()), (Some(5), Some(5)));
        assert_eq!(sushi["enabled"], true);
        assert!(plugin_source(Agent::Antigravity, &hook()).is_none());

        let dir = std::env::temp_dir().join(format!("sushi-agy-{}", std::process::id()));
        let path = dir.join("hooks.json");
        std::fs::create_dir_all(&dir).unwrap();
        let mine = json!({ "lint": { "PreToolUse": [{ "matcher": "run_command", "hooks": [{ "type": "command", "command": "mine" }] }] } });
        std::fs::write(&path, serde_json::to_string(&mine).unwrap()).unwrap();
        let (added, backup) = install_group(&path, &g, 7).unwrap();
        assert_eq!(added.len(), 4);
        assert!(backup.unwrap().to_string_lossy().ends_with("hooks.json.bak-sushi-7"));
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((&v["lint"], &v["sushi"]), (&mine["lint"], &g["sushi"]));
        let (added, backup) = install_group(&path, &g, 8).unwrap();
        assert!(added.is_empty() && backup.is_none(), "second run: nothing to do");
        std::fs::write(&path, "nope").unwrap();
        assert!(install_group(&path, &g, 9).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn gemini_gets_its_own_event_names_merged_into_settings_json() {
        assert!(plugin_source(Agent::Gemini, &hook()).is_none());
        assert!(target_path(Agent::Gemini).ends_with(".gemini/settings.json"));
        let snippet = hook_snippet(&hook(), Agent::Gemini);
        assert_eq!(snippet["hooks"]["BeforeTool"][0]["hooks"][0]["command"], "/home/me/.cargo/bin/sushi-hook --agent gemini");
        assert_eq!(snippet["hooks"]["BeforeTool"][0]["hooks"][0]["name"], "sushi");
        assert_eq!(snippet["hooks"]["BeforeTool"][0]["hooks"][0]["timeout"], 40);
        assert_eq!(snippet["hooks"]["SessionStart"][0]["hooks"][0]["timeout"], 5);
        assert!(snippet["hooks"].get("PermissionRequest").is_none(), "no such event for Gemini");

        let dir = std::env::temp_dir().join(format!("sushi-gemini-{}", std::process::id()));
        let path = dir.join("settings.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"theme": "dark"}"#).unwrap();
        let (added, backup) = install(&path, &hook(), Agent::Gemini, 1).unwrap();
        assert_eq!(added.len(), 7);
        assert!(backup.is_some());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["hooks"]["BeforeTool"][0]["hooks"][0]["command"], "/home/me/.cargo/bin/sushi-hook --agent gemini");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn copilot_gets_a_hook_file_of_its_own() {
        let src = plugin_source(Agent::Copilot, &hook()).unwrap();
        let v: Value = serde_json::from_str(&src).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["hooks"]["PreToolUse"][0]["bash"], "/home/me/.cargo/bin/sushi-hook --agent copilot");
        assert_eq!(v["hooks"]["PermissionRequest"][0]["timeoutSec"], 40);
        assert_eq!(v["hooks"].as_object().unwrap().len(), 9);
    }

    #[test]
    fn merges_permission_patterns_without_touching_existing_ones() {
        use crate::usage::ClaudePermissions;
        let mut s = json!({ "permissions": { "ask": ["Bash(git push*)"] } });
        let perms = ClaudePermissions { ask: vec!["Bash(git push*)".into(), "Bash(rm -rf*)".into()], deny: vec!["Bash(sudo*)".into()] };
        let changed = merge_permissions(&mut s, &perms);
        assert_eq!(changed, vec!["permissions.ask", "permissions.deny"]);
        assert_eq!(s["permissions"]["ask"], json!(["Bash(git push*)", "Bash(rm -rf*)"]), "kept the existing pattern, added the new one");
        assert_eq!(s["permissions"]["deny"], json!(["Bash(sudo*)"]));
        // Running it again changes nothing.
        assert!(merge_permissions(&mut s, &perms).is_empty());
    }

    #[test]
    fn install_permissions_is_a_noop_when_nothing_is_configured() {
        use crate::usage::ClaudePermissions;
        let dir = std::env::temp_dir().join(format!("sushi-perms-{}", std::process::id()));
        let path = dir.join("settings.json");
        let (changed, backup) = install_permissions(&path, &ClaudePermissions::default(), 1).unwrap();
        assert!(changed.is_empty() && backup.is_none() && !path.exists());
    }

    #[test]
    fn install_permissions_writes_and_backs_up() {
        use crate::usage::ClaudePermissions;
        let dir = std::env::temp_dir().join(format!("sushi-perms2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"model": "sonnet"}"#).unwrap();
        let perms = ClaudePermissions { ask: vec!["Bash(rm -rf*)".into()], deny: vec![] };
        let (changed, backup) = install_permissions(&path, &perms, 5).unwrap();
        assert_eq!(changed, vec!["permissions.ask"]);
        assert!(backup.is_some());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!((v["model"].as_str(), &v["permissions"]["ask"]), (Some("sonnet"), &json!(["Bash(rm -rf*)"])));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn plugins_get_the_hook_path_and_are_backed_up_when_they_change() {
        for agent in [Agent::Opencode, Agent::Pi] {
            let src = plugin_source(agent, &hook()).unwrap();
            assert!(src.contains("/home/me/.cargo/bin/sushi-hook") && !src.contains(HOOK_PLACEHOLDER), "{agent:?}");
        }
        assert!(plugin_source(Agent::Claude, &hook()).is_none());

        let dir = std::env::temp_dir().join(format!("sushi-plugin-{}", std::process::id()));
        let path = dir.join("plugins").join("sushi.ts");
        let (added, backup) = install_plugin(&path, "v1", 1).unwrap();
        assert_eq!((added, backup), (vec!["plugin".to_string()], None));
        assert!(install_plugin(&path, "v1", 2).unwrap().0.is_empty(), "unchanged: nothing to do");
        let (_, backup) = install_plugin(&path, "v2", 3).unwrap();
        assert_eq!(std::fs::read_to_string(backup.unwrap()).unwrap(), "v1");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "v2");
        std::fs::remove_dir_all(&dir).ok();
    }
}
