//! End-to-end tests of the permission flow: a real daemon, the real hook binary and the CLI,
//! in a throwaway directory (nothing of the user's setup is touched).
//!
//! What Claude Code does is simulated by feeding the hook the same JSON it would receive.

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Daemon {
    child: Child,
    dir: PathBuf,
}

struct Hook {
    child: Child,
    started: Instant,
}

impl Daemon {
    fn start(name: &str) -> Daemon {
        Self::start_with(name, None)
    }

    /// Start a daemon, optionally with a `config.json`.
    fn start_with(name: &str, config: Option<&str>) -> Daemon {
        let dir = std::env::temp_dir().join(format!("cc-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".config/sushi")).unwrap();
        if let Some(text) = config {
            std::fs::write(dir.join(".config/sushi/config.json"), text).unwrap();
        }
        let child = Command::new(env!("CARGO_BIN_EXE_sushi"))
            .arg("daemon")
            .envs(Self::env_for(&dir))
            .stderr(Stdio::null())
            .spawn()
            .expect("start the daemon");
        let d = Daemon { child, dir };
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(5) {
            if d.cli(&["state"]).0 {
                return d;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the daemon did not come up");
    }

    fn env_for(dir: &std::path::Path) -> Vec<(&'static str, PathBuf)> {
        vec![
            ("XDG_RUNTIME_DIR", dir.to_path_buf()),
            ("XDG_CACHE_HOME", dir.join("cache")),
            ("XDG_CONFIG_HOME", dir.join(".config")),
            ("HOME", dir.to_path_buf()), // no real ~/.claude: no sessions, no credentials
        ]
    }

    /// Run the CLI; returns (success, stdout + stderr).
    fn cli(&self, args: &[&str]) -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_sushi"))
            .args(args)
            .envs(Self::env_for(&self.dir))
            .env("SUSHI_NO_LIMITS", "1")
            .output()
            .expect("run the cli");
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        (out.status.success(), text)
    }

    fn state(&self) -> Value {
        let (ok, text) = self.cli(&["state"]);
        assert!(ok, "state failed: {text}");
        serde_json::from_str(&text).expect("state is JSON")
    }

    /// Start the hook with `payload` on stdin, as Claude Code would.
    fn hook(&self, payload: Value) -> Hook {
        self.hook_with(payload, &[])
    }

    fn hook_with(&self, payload: Value, extra_env: &[(&str, &str)]) -> Hook {
        self.hook_as(&[], payload, extra_env)
    }

    /// Start the hook for another agent (`sushi-hook --agent <id>`).
    fn hook_for(&self, agent: &str, payload: Value) -> Hook {
        self.hook_as(&["--agent", agent], payload, &[])
    }

    fn hook_as(&self, args: &[&str], payload: Value, extra_env: &[(&str, &str)]) -> Hook {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sushi-hook"))
            .args(args)
            .envs(Self::env_for(&self.dir))
            .envs(extra_env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the hook");
        child.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
        Hook { child, started: Instant::now() }
    }

    fn wait_for_pending(&self) -> Value {
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(5) {
            if let Some(p) = self.state()["pending"].as_array().and_then(|a| a.first().cloned()) {
                return p;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("no pending request appeared");
    }

    fn wait_until_no_pending(&self, within: Duration) -> bool {
        let t0 = Instant::now();
        while t0.elapsed() < within {
            if self.state()["pending"].as_array().is_some_and(|a| a.is_empty()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Hook {
    /// Wait for the hook to exit; returns (what it printed, how long it ran in total).
    fn finish(mut self) -> (String, Duration) {
        let mut out = String::new();
        self.child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
        let status = self.child.wait().unwrap();
        assert!(status.success(), "the hook must always exit 0");
        (out, self.started.elapsed())
    }
}

fn permission(tool: &str, input: Value) -> Value {
    json!({ "session_id": "s1", "cwd": "/tmp/proj", "hook_event_name": "PermissionRequest", "tool_name": tool, "tool_input": input })
}

fn event(name: &str) -> Value {
    json!({ "session_id": "s1", "cwd": "/tmp/proj", "hook_event_name": name })
}

#[test]
fn allow_makes_the_hook_print_an_allow_decision() {
    let d = Daemon::start("allow");
    let hook = d.hook(permission("Bash", json!({"command": "touch made.txt"})));
    let p = d.wait_for_pending();
    assert_eq!((p["kind"].as_str(), p["tool_name"].as_str()), (Some("permission"), Some("Bash")));
    assert_eq!(p["detail"]["type"], "terminal", "the card can show the command");
    assert_eq!(d.state()["sessions"][0]["status"], "waiting");

    let (ok, text) = d.cli(&["approve", &p["id"].to_string()]);
    assert!(ok, "{text}");
    let (out, _) = hook.finish();
    let v: Value = serde_json::from_str(out.trim()).expect("the hook prints JSON");
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PermissionRequest");
    assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "allow");
    assert!(v["hookSpecificOutput"]["decision"].get("updatedInput").is_none());
    assert!(d.wait_until_no_pending(Duration::from_secs(2)));
    assert_eq!(d.state()["sessions"][0]["status"], "working", "the session goes back to work");
}

#[test]
fn deny_makes_the_hook_print_a_deny_decision() {
    let d = Daemon::start("deny");
    let hook = d.hook(permission("Edit", json!({"file_path": "/nonexistent/a.rs", "old_string": "a", "new_string": "b"})));
    let p = d.wait_for_pending();
    assert_eq!(p["detail"]["type"], "diff");
    assert!(d.cli(&["deny", &p["id"].to_string()]).0);
    let (out, _) = hook.finish();
    let v: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "deny");
    assert!(v["hookSpecificOutput"]["decision"]["message"].is_string(), "Claude is told why");
}

#[test]
fn unknown_ids_and_double_answers_are_refused() {
    let d = Daemon::start("unknown");
    let (ok, text) = d.cli(&["approve", "99"]);
    assert!(!ok && text.contains("no pending request"), "{text}");
    let hook = d.hook(permission("Bash", json!({"command": "ls"})));
    let id = d.wait_for_pending()["id"].to_string();
    assert!(d.cli(&["deny", &id]).0);
    assert!(!d.cli(&["approve", &id]).0, "a request can only be answered once");
    let (out, _) = hook.finish();
    assert!(out.contains("deny"), "the first answer wins: {out}");
}

#[test]
fn a_question_is_answered_through_the_updated_input() {
    let d = Daemon::start("question");
    let questions = json!([{
        "header": "Colour", "question": "Which colour do you prefer?", "multiSelect": false,
        "options": [{"label": "Red", "description": "warm"}, {"label": "Blue", "description": "cool"}]
    }]);
    let hook = d.hook(permission("AskUserQuestion", json!({"questions": questions.clone()})));
    let p = d.wait_for_pending();
    assert_eq!(p["kind"], "question");
    assert_eq!(p["detail"]["type"], "questions");
    assert_eq!(p["detail"]["questions"][0]["options"][1]["label"], "Blue");
    let id = p["id"].to_string();

    // Allow / Deny make no sense for a question and must not resolve it.
    assert!(!d.cli(&["approve", &id]).0);
    assert!(!d.cli(&["deny", &id]).0);
    // Bad answers are refused and the question stays open.
    assert!(!d.cli(&["answer", &id, "[]"]).0);
    assert!(!d.cli(&["answer", &id, "{}"]).0);
    assert!(!d.cli(&["answer", &id, r#"{"Which colour do you prefer?": 3}"#]).0);
    assert_eq!(d.state()["pending"].as_array().unwrap().len(), 1);

    let (ok, text) = d.cli(&["answer", &id, r#"{"Which colour do you prefer?": "Blue"}"#]);
    assert!(ok, "{text}");
    let (out, _) = hook.finish();
    let v: Value = serde_json::from_str(out.trim()).unwrap();
    let decision = &v["hookSpecificOutput"]["decision"];
    assert_eq!(decision["behavior"], "allow");
    assert_eq!(decision["updatedInput"]["answers"]["Which colour do you prefer?"], "Blue");
    assert_eq!(decision["updatedInput"]["questions"], questions, "the original questions travel along");
}

#[test]
fn a_plan_is_shown_but_never_blocks_the_hook() {
    let d = Daemon::start("plan");
    let hook = d.hook(permission("ExitPlanMode", json!({"plan": "1. Do the thing\n2. Check it"})));
    let (out, took) = hook.finish();
    assert!(out.trim().is_empty(), "no decision: the terminal dialog stays in charge");
    assert!(took < Duration::from_secs(3), "the hook did not wait ({took:?})");

    let p = d.wait_for_pending();
    assert_eq!(p["kind"], "plan");
    assert!(p["detail"]["body"].as_str().unwrap().contains("Do the thing"));
    let id = p["id"].to_string();
    assert!(!d.cli(&["approve", &id]).0 && !d.cli(&["deny", &id]).0, "a plan is approved in the terminal");
    assert_eq!(d.state()["pending"].as_array().unwrap().len(), 1, "still shown");

    // Claude moving on (a tool starts) means the plan was dealt with.
    let mut pre = event("PreToolUse");
    pre["tool_name"] = json!("Read");
    pre["tool_input"] = json!({"file_path": "/tmp/x"});
    d.hook(pre).finish();
    assert!(d.wait_until_no_pending(Duration::from_secs(2)), "the plan went away when work resumed");
}

#[test]
fn answering_in_the_terminal_clears_the_request() {
    let d = Daemon::start("terminal");
    let mut hook = d.hook(permission("Bash", json!({"command": "ls"})));
    d.wait_for_pending();
    // Claude Code kills the hook when the user answers the terminal dialog first.
    hook.child.kill().unwrap();
    let _ = hook.child.wait();
    assert!(d.wait_until_no_pending(Duration::from_secs(3)), "the card disappears by itself");

    // The tool then runs: the events that follow must leave nothing behind either.
    let hook = d.hook(permission("Bash", json!({"command": "pwd"})));
    d.wait_for_pending();
    let mut post = event("PostToolUse");
    post["tool_name"] = json!("Bash");
    d.hook(post).finish();
    assert!(d.wait_until_no_pending(Duration::from_secs(2)));
    drop(hook);
}

#[test]
fn without_a_daemon_the_hook_exits_quietly_and_fast() {
    let dir = std::env::temp_dir().join(format!("cc-{}-nodaemon", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_sushi-hook"))
        .env("XDG_RUNTIME_DIR", &dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(permission("Bash", json!({"command": "ls"})).to_string().as_bytes()).unwrap();
    let t0 = Instant::now();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success() && out.stdout.is_empty());
    assert!(t0.elapsed() < Duration::from_secs(2), "Claude Code is never held up");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_chat_sessions_own_events_are_ignored() {
    let d = Daemon::start("chatenv");
    d.hook_with(event("SessionStart"), &[("SUSHI_CHAT", "1")]).finish();
    d.hook_with(permission("Bash", json!({"command": "ls"})), &[("SUSHI_CHAT", "1")]).finish();
    let s = d.state();
    assert!(s["sessions"].as_array().unwrap().is_empty() && s["pending"].as_array().unwrap().is_empty());
}

#[test]
fn hidden_code_never_reaches_the_state_but_requests_still_work() {
    let d = Daemon::start_with("hidden", Some(r#"{"show_code": false}"#));
    let hook = d.hook(permission("Edit", json!({"file_path": "/x", "old_string": "SECRET", "new_string": "OTHER"})));
    let p = d.wait_for_pending();
    assert_eq!(p["detail"]["type"], "text");
    assert!(!p.to_string().contains("SECRET"), "no code in the state: {p}");
    assert!(d.cli(&["deny", &p["id"].to_string()]).0);
    hook.finish();

    // A question is then only a permission as far as the state is concerned: no questions leak.
    let hook = d.hook(permission("AskUserQuestion", json!({"questions": [{"question": "Secret question?", "options": [{"label": "a"}]}]})));
    let p = d.wait_for_pending();
    assert_eq!(p["kind"], "permission");
    assert!(!p.to_string().contains("Secret question"), "{p}");
    assert!(d.cli(&["deny", &p["id"].to_string()]).0);
    hook.finish();
}

#[test]
fn codex_speaks_the_same_hook_language_and_gets_a_codex_session() {
    let d = Daemon::start("codex");
    let patch = "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-old\n+new\n*** End Patch";
    let hook = d.hook_for("codex", permission("apply_patch", json!({"command": patch})));
    let p = d.wait_for_pending();
    assert_eq!((p["agent"].as_str(), p["kind"].as_str()), (Some("codex"), Some("permission")));
    assert_eq!(p["detail"]["type"], "diff", "the patch is shown as a diff");
    let s = &d.state()["sessions"][0];
    assert_eq!((s["agent"].as_str(), s["id"].as_str()), (Some("codex"), Some("codex:s1")));
    assert!(s.get("context").is_none(), "no transcript parsing for Codex");

    assert!(d.cli(&["approve", &p["id"].to_string()]).0);
    let (out, _) = hook.finish();
    let v: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "allow");
}

#[test]
fn pi_and_opencode_get_their_plugins_answer_format() {
    let d = Daemon::start("plugins");
    for (agent, tool, input) in [
        ("pi", "bash", json!({"command": "ls"})),
        ("opencode", "edit", json!({"filePath": "/nonexistent/a.ts", "oldString": "a", "newString": "b"})),
    ] {
        let hook = d.hook_for(agent, permission(tool, input));
        let p = d.wait_for_pending();
        assert_eq!(p["agent"], agent);
        assert!(d.cli(&["approve", &p["id"].to_string()]).0);
        let (out, _) = hook.finish();
        assert_eq!(serde_json::from_str::<Value>(out.trim()).unwrap(), json!({"decision": "allow"}), "{agent}");

        let hook = d.hook_for(agent, permission(tool, json!({"command": "ls"})));
        let p = d.wait_for_pending();
        assert!(d.cli(&["deny", &p["id"].to_string()]).0);
        let (out, _) = hook.finish();
        let v: Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(v["decision"], "deny", "{agent}");
        assert!(v["message"].is_string());
    }
    // Both agents' sessions are listed, side by side.
    let agents: Vec<String> = d.state()["sessions"].as_array().unwrap().iter().map(|s| s["agent"].as_str().unwrap().to_string()).collect();
    assert!(agents.contains(&"pi".to_string()) && agents.contains(&"opencode".to_string()), "{agents:?}");
}

#[test]
fn a_pi_session_is_checked_by_its_pid() {
    let d = Daemon::start("pid");
    let mut ev = event("SessionStart");
    ev["pid"] = json!(4_000_000); // no such process
    d.hook_for("pi", ev).finish();
    // housekeeping runs every 2 s and drops sessions whose process is gone
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(6) {
        if d.state()["sessions"].as_array().unwrap().is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("the session of a dead process stayed");
}

#[test]
fn the_state_declares_what_each_agent_can_do() {
    let d = Daemon::start("caps");
    let s = d.state();
    assert_eq!(s["version"], 2);
    assert_eq!(s["agents"]["claude"]["capabilities"]["chat"], true);
    assert_eq!(s["agents"]["pi"]["capabilities"]["chat"], true);
    assert_eq!(s["agents"]["opencode"]["capabilities"]["chat"], false);
    assert!(s["agents"]["claude"].get("limits").is_some() && s["agents"]["codex"].get("limits").is_none());
}

#[test]
fn an_unknown_agent_is_ignored_quietly() {
    let d = Daemon::start("unknownagent");
    let (out, took) = d.hook_for("nope", permission("Bash", json!({"command": "ls"}))).finish();
    assert!(out.is_empty() && took < Duration::from_secs(2));
    assert!(d.state()["pending"].as_array().unwrap().is_empty());
}

/// Payloads recorded from the real GitHub Copilot CLI (1.0.89).
fn copilot_event(name: &str, extra: Value) -> Value {
    let mut v = json!({"hook_event_name": name, "session_id": "cp1", "cwd": "/tmp/proj"});
    for (k, x) in extra.as_object().unwrap() {
        v[k] = x.clone();
    }
    v
}

#[test]
fn copilot_permission_requests_come_in_camel_case_and_answer_with_a_behavior() {
    let d = Daemon::start("copilot");
    let request = json!({"hookName": "permissionRequest", "sessionId": "cp1", "timestamp": 1, "cwd": "/tmp/proj",
        "toolName": "bash", "toolInput": {"command": "echo hi"}, "permissionSuggestions": []});
    let hook = d.hook_for("copilot", request.clone());
    let p = d.wait_for_pending();
    assert_eq!((p["agent"].as_str(), p["tool_name"].as_str(), p["detail"]["type"].as_str()), (Some("copilot"), Some("bash"), Some("terminal")));
    assert!(d.cli(&["approve", &p["id"].to_string()]).0);
    let (out, _) = hook.finish();
    assert_eq!(serde_json::from_str::<Value>(out.trim()).unwrap(), json!({"behavior": "allow"}));

    let hook = d.hook_for("copilot", request);
    let p = d.wait_for_pending();
    assert!(d.cli(&["deny", &p["id"].to_string()]).0);
    let (out, _) = hook.finish();
    let v: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!((v["behavior"].as_str(), v["message"].is_string()), (Some("deny"), true));
}

#[test]
fn copilot_steps_are_followed_without_call_ids_and_edits_show_as_diffs() {
    let d = Daemon::start("copilot-steps");
    let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-one\n-two\n+one\n+three\n*** End Patch\n";
    d.hook_for("copilot", copilot_event("UserPromptSubmit", json!({"prompt": "go"}))).finish();
    d.hook_for("copilot", copilot_event("PreToolUse", json!({"tool_name": "Edit", "tool_input": patch}))).finish();
    let steps = d.state()["sessions"][0]["activity"]["recent"].clone();
    assert_eq!((steps[0]["ok"].is_null(), steps[0]["detail"]["type"].as_str()), (true, Some("diff")));
    d.hook_for("copilot", copilot_event("PostToolUse", json!({"tool_name": "Edit", "tool_input": patch,
        "tool_result": {"result_type": "success", "text_result_for_llm": "Modified 1 file(s)"}}))).finish();
    d.hook_for("copilot", copilot_event("PreToolUse", json!({"tool_name": "Bash", "tool_input": {"command": "false"}}))).finish();
    d.hook_for("copilot", copilot_event("PostToolUse", json!({"tool_name": "Bash", "tool_input": {"command": "false"},
        "tool_result": {"result_type": "failure", "text_result_for_llm": "exit 1"}}))).finish();
    let a = d.state()["sessions"][0]["activity"].clone();
    assert_eq!((a["tool_calls"].as_u64(), a["files_changed"].as_u64(), a["failures"].as_u64()), (Some(2), Some(1), Some(1)));
    assert_eq!((a["recent"][0]["ok"].as_bool(), a["recent"][1]["ok"].as_bool()), (Some(true), Some(false)), "matched by tool name");
    d.hook_for("copilot", copilot_event("Stop", json!({"stop_reason": "end_turn"}))).finish();
    assert_eq!(d.state()["sessions"][0]["status"], "idle");
}

#[cfg(unix)]
/// A fake agent executable that prints `lines` and records its arguments in `args.txt`.
fn fake_agent(dir: &std::path::Path, name: &str, lines: &[&str]) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    let echoes: String = lines.iter().map(|l| format!("echo '{l}'\n")).collect();
    let body = format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > {}/{name}-args.txt\n{echoes}", dir.display());
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)] // the fake agent is a /bin/sh script
#[test]
fn the_chat_can_talk_to_another_agent() {
    let tmp = std::env::temp_dir().join(format!("cc-{}-chatagent-bin", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let fake = fake_agent(&tmp, "copilot", &[
        r#"{"type":"assistant.message_delta","data":{"deltaContent":"Hel"}}"#,
        r#"{"type":"assistant.message","data":{"content":"Hello from Copilot"}}"#,
        r#"{"type":"result","exitCode":0}"#,
    ]);
    let config = format!(r#"{{"agent_paths": {{"copilot": "{}"}}}}"#, fake.display());
    let d = Daemon::start_with("chatagent", Some(&config));
    let (ok, text) = d.cli(&["chat", "hi", "there", "--agent", "copilot"]);
    assert!(ok, "{text}");
    let t0 = Instant::now();
    let chat = loop {
        let c = d.state()["chat"].clone();
        if c["busy"] == false && !c["messages"].as_array().unwrap().is_empty() && c["messages"][1]["text"] != "" {
            break c;
        }
        assert!(t0.elapsed() < Duration::from_secs(5), "no answer: {c}");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!((chat["agent"].as_str(), chat["messages"][1]["text"].as_str(), chat["error"].is_null()), (Some("copilot"), Some("Hello from Copilot"), true));
    let args = std::fs::read_to_string(tmp.join("copilot-args.txt")).unwrap();
    assert!(args.contains("--available-tools") && args.contains("--prompt=") && args.contains("hi there"), "{args}");

    // An agent with no tool-free headless mode is refused; an unknown one too.
    let (ok, text) = d.cli(&["chat", "hi", "--agent", "opencode"]);
    assert!(!ok && text.contains("cannot run as the chat"), "{text}");
    assert!(!d.cli(&["chat", "hi", "--agent", "nope"]).0);
    std::fs::remove_dir_all(&tmp).ok();
}
