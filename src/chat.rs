//! The built-in chat ("Ask <agent> anything").
//!
//! Each message runs one of the agents headless, streaming, with no tools so it can only
//! answer: `claude -p`, `copilot -p`, `pi -p` or `codex exec`. It uses that agent's own login
//! (no API key) and counts against its plan like any other session. Follow-up messages resume the
//! same session, so the conversation keeps its context.
//!
//! What was checked against the real CLI (Claude Code, GitHub Copilot) and what only follows
//! the documentation (pi, Codex) is in the README. The spawned process gets `SUSHI_CHAT=1`, which
//! makes `sushi-hook` ignore it, and runs in its own folder, which the session list skips.

use crate::agent::Agent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

pub const MAX_MESSAGES: usize = 50;
pub const MAX_INPUT_CHARS: usize = 8000;

const SYSTEM_PROMPT: &str = "You are the assistant inside a small desktop widget: a pet that lives in the \
user's notch. Reply briefly and plainly, in the language of the question, without headings. You have no \
tools, no files and no internet: never claim to have run or opened anything.";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    /// `user` or `assistant`.
    pub role: String,
    pub text: String,
}

/// What is kept on disk between runs.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Saved {
    /// The agent this conversation is with (`claude` when saved by an older version).
    #[serde(default = "claude_id")]
    pub agent: String,
    pub session_id: String,
    /// Whether the session exists on the agent's side (so the next turn must resume it).
    pub started: bool,
    pub messages: Vec<Message>,
}

#[derive(Debug, Default)]
pub struct Chat {
    pub saved: Saved,
    pub busy: bool,
    pub error: Option<String>,
    /// The running `claude` process, so it can be stopped.
    pub child: Arc<Mutex<Option<Child>>>,
    /// Bumped by `clear`: a turn that started before it must not write into the new conversation.
    pub generation: u64,
}

fn claude_id() -> String {
    "claude".to_string()
}

pub fn new_session_id() -> String {
    // 16 random bytes from the OS, formatted as a version-4 uuid.
    let mut b = [0u8; 16];
    let ok = std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b)).is_ok();
    if !ok {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        b.copy_from_slice(&t.to_le_bytes());
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

impl Chat {
    pub fn load(path: &Path) -> Chat {
        let mut saved: Saved = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        if saved.agent.is_empty() {
            saved.agent = claude_id();
        }
        if saved.session_id.is_empty() {
            saved.session_id = new_session_id();
            saved.started = false;
        }
        Chat { saved, ..Chat::default() }
    }

    pub fn save(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string(&self.saved) {
            let _ = std::fs::write(path, text);
        }
    }

    /// Start over: a new conversation (the old session is left alone on the agent's side).
    pub fn clear(&mut self) {
        self.saved = Saved { agent: self.saved.agent.clone(), session_id: new_session_id(), started: false, messages: Vec::new() };
        self.error = None;
        self.busy = false;
        self.generation += 1;
    }

    /// Add a user message and an empty assistant message that the answer streams into.
    pub fn begin_turn(&mut self, text: &str) {
        self.saved.messages.push(Message { role: "user".into(), text: text.to_string() });
        self.saved.messages.push(Message { role: "assistant".into(), text: String::new() });
        let excess = self.saved.messages.len().saturating_sub(MAX_MESSAGES);
        self.saved.messages.drain(..excess);
        self.busy = true;
        self.error = None;
    }

    pub fn set_answer(&mut self, text: &str) {
        if let Some(last) = self.saved.messages.last_mut().filter(|m| m.role == "assistant") {
            last.text = text.to_string();
        }
    }

    /// Talk to `agent` from now on. A conversation is with one agent: switching starts a new one.
    pub fn use_agent(&mut self, agent: Agent) {
        if self.saved.agent != agent.id() {
            self.clear();
            self.saved.agent = agent.id().to_string();
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "agent": self.saved.agent, "busy": self.busy, "error": self.error, "messages": self.saved.messages })
    }
}

/// The model to ask for when nothing was chosen: only Claude Code has one of its own here
/// (`sonnet`); the others use their configured default.
pub fn default_model(agent: Agent, claude_model: &str) -> Option<String> {
    (agent == Agent::Claude).then(|| claude_model.to_string())
}

/// Is this one of Claude Code's model aliases (meaningless to the other agents)?
pub fn is_claude_alias(model: &str) -> bool {
    matches!(model.trim(), "sonnet" | "haiku" | "opus" | "default")
}

/// The command line of one turn for `agent` (the prompt goes last, so it may start with a dash).
/// `model` of `None` means the agent's own default.
pub fn build_args(agent: Agent, session_id: &str, started: bool, model: Option<&str>, text: &str) -> Vec<String> {
    let strings = |list: &[&str]| -> Vec<String> { list.iter().map(|s| s.to_string()).collect() };
    // Agents with no system prompt flag get it in front of the first message.
    let first_prompt = || if started { text.to_string() } else { format!("{SYSTEM_PROMPT}\n\n{text}") };
    let mut a: Vec<String>;
    match agent {
        Agent::Claude => {
            a = strings(&["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages"]);
            a.extend(strings(&["--model", model.unwrap_or("sonnet"), "--tools", "", "--strict-mcp-config", "--disable-slash-commands"]));
            a.extend(strings(&["--append-system-prompt", SYSTEM_PROMPT]));
            a.push(if started { "--resume".into() } else { "--session-id".into() });
            a.push(session_id.to_string());
            a.extend(strings(&["--", text]));
        }
        Agent::Copilot => {
            // Non-interactive mode wants `--allow-all-tools`; with no tools available that grants nothing.
            a = strings(&["--output-format", "json", "--stream", "on", "--allow-all-tools", "--available-tools", ""]);
            a.extend(strings(&["--disable-builtin-mcps", "--session-id", session_id]));
            if let Some(m) = model {
                a.extend(strings(&["--model", m]));
            }
            a.push(format!("--prompt={}", first_prompt()));
        }
        Agent::Pi => {
            a = strings(&["-p", "--mode", "json", "--no-tools", "--no-extensions", "--no-skills", "--no-context-files"]);
            a.extend(strings(&["--no-prompt-templates", "--append-system-prompt", SYSTEM_PROMPT, "--session-id", session_id]));
            if let Some(m) = model {
                a.extend(strings(&["--model", m]));
            }
            a.extend(strings(&["--", text]));
        }
        Agent::Codex => {
            // Codex cannot run without tools: a read-only sandbox is the closest it offers. It names
            // its own sessions (`thread.started`), so later turns resume the id it gave.
            a = if started { strings(&["exec", "resume", session_id]) } else { strings(&["exec"]) };
            a.extend(strings(&["--json", "--skip-git-repo-check"]));
            if !started {
                a.extend(strings(&["--sandbox", "read-only"]));
            }
            if let Some(m) = model {
                a.extend(strings(&["--model", m]));
            }
            a.extend(strings(&["--", &first_prompt()]));
        }
        Agent::Opencode => a = Vec::new(), // no way to run it without tools: not offered
    }
    a
}

/// What has been read from the stream of one turn.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Turn {
    pub text: String,
    pub done: bool,
    pub error: Option<String>,
    /// A session id the agent chose for itself (Codex), to resume next time.
    pub session: Option<String>,
}

/// Feed one line of the agent's JSON stream; returns true when the text changed.
pub fn parse_line(agent: Agent, turn: &mut Turn, line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(line) else { return false };
    match agent {
        Agent::Claude => claude_line(turn, &v),
        Agent::Copilot => copilot_line(turn, &v),
        Agent::Pi => pi_line(turn, &v),
        Agent::Codex => codex_line(turn, &v),
        Agent::Opencode => false,
    }
}

/// Make `full` the text of the turn if it differs (the complete message is authoritative, so a
/// missed delta cannot leave a gap).
fn settle(turn: &mut Turn, full: String) -> bool {
    if !full.is_empty() && full != turn.text {
        turn.text = full;
        return true;
    }
    false
}

/// `claude -p --output-format stream-json --include-partial-messages`.
fn claude_line(turn: &mut Turn, v: &Value) -> bool {
    match v.get("type").and_then(Value::as_str) {
        Some("stream_event") => {
            let delta = v.pointer("/event/delta");
            let is_text = v.pointer("/event/type").and_then(Value::as_str) == Some("content_block_delta")
                && delta.and_then(|d| d.get("type")).and_then(Value::as_str) == Some("text_delta");
            if let (true, Some(t)) = (is_text, delta.and_then(|d| d.get("text")).and_then(Value::as_str)) {
                turn.text.push_str(t);
                return true;
            }
            false
        }
        Some("assistant") => {
            let full: String = v
                .pointer("/message/content")
                .and_then(Value::as_array)
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                        .filter_map(|b| b.get("text").and_then(Value::as_str))
                        .collect()
                })
                .unwrap_or_default();
            settle(turn, full)
        }
        Some("result") => {
            turn.done = true;
            let result = v.get("result").and_then(Value::as_str).unwrap_or("");
            if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
                turn.error = Some(if result.is_empty() { "Claude reported an error".into() } else { result.to_string() });
            } else if turn.text.is_empty() && !result.is_empty() {
                turn.text = result.to_string();
                return true;
            }
            false
        }
        _ => false,
    }
}

/// `copilot -p --output-format json`: `assistant.message_delta` while it streams,
/// `assistant.message` with the whole text, `result` at the end.
fn copilot_line(turn: &mut Turn, v: &Value) -> bool {
    match v.get("type").and_then(Value::as_str) {
        Some("assistant.message_delta") => match v.pointer("/data/deltaContent").and_then(Value::as_str) {
            Some(t) if !t.is_empty() => {
                turn.text.push_str(t);
                true
            }
            _ => false,
        },
        Some("assistant.message") => settle(turn, v.pointer("/data/content").and_then(Value::as_str).unwrap_or("").to_string()),
        Some("result") => {
            turn.done = true;
            if v.get("exitCode").and_then(Value::as_i64).is_some_and(|c| c != 0) {
                turn.error = Some("Copilot reported an error".into());
            }
            false
        }
        Some("session.error") => {
            turn.error = Some(v.pointer("/data/message").and_then(Value::as_str).unwrap_or("Copilot reported an error").to_string());
            false
        }
        _ => false,
    }
}

/// `pi -p --mode json`: `message_update` carries text deltas, `message_end` the whole message.
fn pi_line(turn: &mut Turn, v: &Value) -> bool {
    match v.get("type").and_then(Value::as_str) {
        Some("message_update") => {
            let ev = v.get("assistantMessageEvent");
            let is_text = ev.and_then(|e| e.get("type")).and_then(Value::as_str) == Some("text_delta");
            match (is_text, ev.and_then(|e| e.get("delta")).and_then(Value::as_str)) {
                (true, Some(t)) => {
                    turn.text.push_str(t);
                    true
                }
                _ => false,
            }
        }
        Some("message_end") if v.pointer("/message/role").and_then(Value::as_str) == Some("assistant") => {
            if v.pointer("/message/stopReason").and_then(Value::as_str) == Some("error") {
                let msg = v.pointer("/message/errorMessage").and_then(Value::as_str).unwrap_or("pi reported an error");
                turn.error = Some(msg.to_string());
            }
            let full: String = v
                .pointer("/message/content")
                .and_then(Value::as_array)
                .map(|parts| {
                    parts
                        .iter()
                        .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                        .filter_map(|p| p.get("text").and_then(Value::as_str))
                        .collect()
                })
                .unwrap_or_default();
            settle(turn, full)
        }
        Some("agent_end") => {
            turn.done = true;
            false
        }
        _ => false,
    }
}

/// `codex exec --json`: whole messages (`item.completed`), no deltas.
fn codex_line(turn: &mut Turn, v: &Value) -> bool {
    match v.get("type").and_then(Value::as_str) {
        Some("thread.started") => {
            turn.session = v.get("thread_id").and_then(Value::as_str).map(str::to_string);
            false
        }
        Some("item.completed") if v.pointer("/item/type").and_then(Value::as_str) == Some("agent_message") => {
            let Some(t) = v.pointer("/item/text").and_then(Value::as_str) else { return false };
            if !turn.text.is_empty() {
                turn.text.push_str("\n\n");
            }
            turn.text.push_str(t);
            true
        }
        Some("turn.completed") => {
            turn.done = true;
            false
        }
        Some("turn.failed") | Some("error") => {
            let msg = v.pointer("/error/message").or_else(|| v.get("message")).and_then(Value::as_str);
            turn.error = Some(msg.unwrap_or("Codex reported an error").to_string());
            false
        }
        _ => false,
    }
}

/// Outcome of running the command.
#[derive(Debug)]
pub struct Outcome {
    pub turn: Turn,
    /// The process was killed by `stop`.
    pub stopped: bool,
}

/// Run `program args` in `cwd`, calling `on_text(partial_answer)` as the text grows. The
/// child is parked in `slot` while it runs so another thread can kill it.
pub fn run_turn(
    agent: Agent,
    program: &str,
    args: &[String],
    cwd: &Path,
    slot: &Arc<Mutex<Option<Child>>>,
    mut on_text: impl FnMut(&str),
) -> Outcome {
    let mut turn = Turn::default();
    let _ = std::fs::create_dir_all(cwd);
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(cwd).env("SUSHI_CHAT", "1").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    cmd.process_group(0); // its own group, so `stop` can take its children down too
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashing up
    }
    let spawned = cmd.spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            turn.error = Some(format!("cannot run `{program}`: {e}"));
            return Outcome { turn, stopped: false };
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);

    // stderr is drained on its own thread so a chatty child cannot block on a full pipe.
    let err_reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut e) = stderr {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    if let Some(out) = stdout {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if parse_line(agent, &mut turn, &line) {
                on_text(&turn.text);
            }
        }
    }
    let stderr_text = err_reader.join().unwrap_or_default();
    let status = slot.lock().unwrap_or_else(|e| e.into_inner()).take().and_then(|mut c| c.wait().ok());
    let stopped = status.as_ref().is_some_and(|s| !s.success() && s.code().is_none()); // killed by a signal
    if !stopped && turn.error.is_none() && !status.as_ref().is_some_and(|s| s.success()) {
        let first = stderr_text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        turn.error = Some(if first.is_empty() { format!("{program} exited with an error") } else { first.to_string() });
    }
    Outcome { turn, stopped }
}

/// Kill the running chat process and everything it started, if any. Returns whether
/// there was one.
pub fn stop(slot: &Arc<Mutex<Option<Child>>>) -> bool {
    match slot.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        Some(child) => {
            // The child leads its own process group (pgid = pid): signal the whole group.
            #[cfg(unix)]
            // SAFETY: killpg only sends a signal to the group we created.
            unsafe {
                libc::killpg(child.id() as libc::pid_t, libc::SIGKILL)
            };
            #[cfg(windows)]
            {
                // /T takes the whole process tree down.
                let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &child.id().to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status();
            }
            let _ = child.kill();
            true
        }
        None => false,
    }
}

/// `~/.cache/sushi` (or `$XDG_CACHE_HOME/sushi`).
pub fn cache_dir() -> PathBuf {
    crate::paths::cache_home().join("sushi")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    // Lines shaped like the real `claude -p --output-format stream-json --include-partial-messages` output.
    const DELTA: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Hey "}}}"#;
    const DELTA2: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"there!"}}}"#;
    const THINK: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}}"#;
    const ASSISTANT: &str = r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":""},{"type":"text","text":"Hey there!"}]}}"#;
    const RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"Hey there!"}"#;

    #[test]
    fn parses_the_stream() {
        let mut t = Turn::default();
        assert!(!parse_line(Agent::Claude, &mut t, r#"{"type":"system","subtype":"init"}"#));
        assert!(!parse_line(Agent::Claude, &mut t, THINK), "thinking is not shown");
        assert!(!parse_line(Agent::Claude, &mut t, "not json"));
        assert!(parse_line(Agent::Claude, &mut t, DELTA));
        assert!(parse_line(Agent::Claude, &mut t, DELTA2));
        assert_eq!(t.text, "Hey there!");
        assert!(!parse_line(Agent::Claude, &mut t, ASSISTANT), "the full message matches what was streamed");
        assert!(!t.done);
        parse_line(Agent::Claude, &mut t, RESULT);
        assert!(t.done && t.error.is_none() && t.text == "Hey there!");
    }

    #[test]
    fn the_full_message_repairs_a_missed_delta() {
        let mut t = Turn::default();
        parse_line(Agent::Claude, &mut t, DELTA2); // "Hey " was lost
        assert_eq!(t.text, "there!");
        assert!(parse_line(Agent::Claude, &mut t, ASSISTANT));
        assert_eq!(t.text, "Hey there!");
    }

    #[test]
    fn errors_and_result_only_answers() {
        let mut t = Turn::default();
        parse_line(Agent::Claude, &mut t, r#"{"type":"result","subtype":"error","is_error":true,"result":"Not logged in"}"#);
        assert_eq!(t.error.as_deref(), Some("Not logged in"));
        let mut t = Turn::default();
        assert!(parse_line(Agent::Claude, &mut t, RESULT), "no stream at all: the result is the answer");
        assert_eq!(t.text, "Hey there!");
    }

    #[test]
    fn first_turn_creates_the_session_later_turns_resume_it() {
        let first = build_args(Agent::Claude, "sid", false, Some("sonnet"), "- hi");
        assert!(first.windows(2).any(|w| w == ["--session-id", "sid"]));
        assert!(!first.contains(&"--resume".to_string()));
        let later = build_args(Agent::Claude, "sid", true, Some("haiku"), "again");
        assert!(later.windows(2).any(|w| w == ["--resume", "sid"]));
        assert!(later.windows(2).any(|w| w == ["--model", "haiku"]));
        // no tools, no MCP, and the prompt comes last, after `--`, even if it starts with a dash
        assert!(first.windows(2).any(|w| w == ["--tools", ""]));
        assert!(first.contains(&"--strict-mcp-config".to_string()));
        assert_eq!(&first[first.len() - 2..], ["--", "- hi"]);
    }

    #[test]
    fn history_is_capped_cleared_and_saved() {
        let mut c = Chat::default();
        for i in 0..40 {
            c.begin_turn(&format!("q{i}"));
            c.set_answer(&format!("a{i}"));
        }
        assert_eq!(c.saved.messages.len(), MAX_MESSAGES);
        assert_eq!(c.saved.messages.last().unwrap().text, "a39");
        assert!(c.busy);

        let dir = std::env::temp_dir().join(format!("sushi-chat-{}", std::process::id()));
        let path = dir.join("chat.json");
        c.saved.session_id = "abc".into();
        c.save(&path);
        let loaded = Chat::load(&path);
        assert_eq!((loaded.saved.session_id.as_str(), loaded.saved.messages.len()), ("abc", MAX_MESSAGES));
        assert!(!loaded.busy);

        let mut c = loaded;
        c.clear();
        assert!(c.saved.messages.is_empty() && !c.saved.started && c.saved.session_id != "abc");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    fn script(name: &str, body: &str) -> String {
        let dir = std::env::temp_dir().join(format!("sushi-fake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::File::create(&p).unwrap().write_all(format!("#!/bin/sh\n{body}\n").as_bytes()).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.to_string_lossy().into_owned()
    }

    #[cfg(unix)] // runs a /bin/sh script
    #[test]
    fn streams_a_whole_turn_from_a_fake_claude() {
        let body = format!(
            "echo '{DELTA}'; sleep 0.05; echo '{DELTA2}'; echo '{ASSISTANT}'; echo '{RESULT}'; \
             test \"$SUSHI_CHAT\" = 1 || exit 3"
        );
        let prog = script("claude-ok", &body);
        let slot = Arc::new(Mutex::new(None));
        let mut seen = Vec::new();
        let out = run_turn(Agent::Claude, &prog, &[], &std::env::temp_dir(), &slot, |t| seen.push(t.to_string()));
        assert_eq!(seen, vec!["Hey ", "Hey there!"], "one update per change, not per line");
        assert!(out.turn.done && out.turn.error.is_none() && !out.stopped);
        assert_eq!(out.turn.text, "Hey there!");
        assert!(slot.lock().unwrap().is_none(), "the child is released afterwards");
    }

    #[cfg(unix)] // runs a /bin/sh script
    #[test]
    fn a_failing_claude_reports_its_first_stderr_line() {
        let prog = script("claude-fail", "echo 'Not logged in. Run claude to log in.' >&2; exit 1");
        let slot = Arc::new(Mutex::new(None));
        let out = run_turn(Agent::Claude, &prog, &[], &std::env::temp_dir(), &slot, |_| {});
        assert_eq!(out.turn.error.as_deref(), Some("Not logged in. Run claude to log in."));
        let missing = run_turn(Agent::Claude, "/nonexistent/claude", &[], &std::env::temp_dir(), &slot, |_| {});
        assert!(missing.turn.error.unwrap().contains("cannot run"));
    }

    #[cfg(unix)] // runs a /bin/sh script
    #[test]
    fn stop_kills_the_process_promptly() {
        let prog = script("claude-slow", &format!("echo '{DELTA}'; sleep 30"));
        let slot = Arc::new(Mutex::new(None));
        let killer = {
            let slot = slot.clone();
            std::thread::spawn(move || {
                for _ in 0..100 {
                    std::thread::sleep(std::time::Duration::from_millis(30));
                    if stop(&slot) {
                        return true;
                    }
                }
                false
            })
        };
        let started = std::time::Instant::now();
        let out = run_turn(Agent::Claude, &prog, &[], &std::env::temp_dir(), &slot, |_| {});
        assert!(killer.join().unwrap(), "there was a process to stop");
        assert!(started.elapsed().as_secs() < 10, "did not wait for the 30 s sleep");
        assert!(out.stopped && out.turn.error.is_none(), "{out:?}");
        assert!(!stop(&slot), "nothing left to stop");
    }

    // Lines recorded from the real `copilot -p --output-format json --stream on`.
    const COPILOT_LINES: [&str; 5] = [
        r#"{"type":"assistant.message_start","data":{"messageId":"m","phase":"final_answer"},"ephemeral":true}"#,
        r#"{"type":"assistant.message_delta","data":{"messageId":"m","deltaContent":"Under"},"ephemeral":true}"#,
        r#"{"type":"assistant.message_delta","data":{"messageId":"m","deltaContent":"stood."},"ephemeral":true}"#,
        r#"{"type":"assistant.message","data":{"messageId":"m","content":"Understood.","toolRequests":[]}}"#,
        r#"{"type":"result","sessionId":"s","exitCode":0,"usage":{"premiumRequests":1}}"#,
    ];

    #[test]
    fn copilot_streams_deltas_and_ends_with_a_result() {
        let mut t = Turn::default();
        let changed: Vec<bool> = COPILOT_LINES.iter().map(|l| parse_line(Agent::Copilot, &mut t, l)).collect();
        assert_eq!(changed, [false, true, true, false, false], "the whole message matches what streamed");
        assert!(t.done && t.error.is_none() && t.text == "Understood.");
        let mut failed = Turn::default();
        parse_line(Agent::Copilot, &mut failed, r#"{"type":"result","exitCode":1}"#);
        assert!(failed.error.is_some());
    }

    #[test]
    fn pi_streams_text_deltas_and_reports_errors() {
        let mut t = Turn::default();
        assert!(!parse_line(Agent::Pi, &mut t, r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","delta":"hm"}}"#));
        assert!(parse_line(Agent::Pi, &mut t, r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Hel"}}"#));
        assert!(parse_line(Agent::Pi, &mut t, r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"lo"}}"#));
        assert!(!parse_line(Agent::Pi, &mut t, r#"{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"x"}]}}"#));
        assert!(!parse_line(Agent::Pi, &mut t, r#"{"type":"message_end","message":{"role":"assistant","content":[{"type":"text","text":"Hello"}]}}"#));
        parse_line(Agent::Pi, &mut t, r#"{"type":"agent_end","messages":[]}"#);
        assert!(t.done && t.error.is_none() && t.text == "Hello");
        let mut e = Turn::default();
        parse_line(Agent::Pi, &mut e, r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"connection refused"}}"#);
        assert_eq!(e.error.as_deref(), Some("connection refused"));
    }

    #[test]
    fn codex_gives_its_own_session_and_whole_messages() {
        let mut t = Turn::default();
        parse_line(Agent::Codex, &mut t, r#"{"type":"thread.started","thread_id":"th-1"}"#);
        assert!(parse_line(Agent::Codex, &mut t, r#"{"type":"item.completed","item":{"id":"i","type":"agent_message","text":"Hi!"}}"#));
        assert!(!parse_line(Agent::Codex, &mut t, r#"{"type":"item.completed","item":{"type":"command_execution","command":"ls"}}"#));
        parse_line(Agent::Codex, &mut t, r#"{"type":"turn.completed","usage":{}}"#);
        assert_eq!((t.session.as_deref(), t.text.as_str(), t.done), (Some("th-1"), "Hi!", true));
        let mut f = Turn::default();
        parse_line(Agent::Codex, &mut f, r#"{"type":"turn.failed","error":{"message":"not logged in"}}"#);
        assert_eq!(f.error.as_deref(), Some("not logged in"));
    }

    #[test]
    fn every_agent_runs_without_tools_and_the_prompt_comes_last() {
        let has = |a: &[String], w: &[&str]| a.windows(w.len()).any(|x| x == w);
        let c = build_args(Agent::Copilot, "sid", false, Some("auto"), "- hi");
        assert!(has(&c, &["--available-tools", ""]) && has(&c, &["--session-id", "sid"]) && has(&c, &["--model", "auto"]));
        assert!(c.last().unwrap().starts_with("--prompt=") && c.last().unwrap().ends_with("- hi"), "{c:?}");
        assert!(c.last().unwrap().contains("no tools"), "the first turn carries the instructions");
        let later = build_args(Agent::Copilot, "sid", true, None, "again");
        assert_eq!(later.last().unwrap(), "--prompt=again");
        assert!(!has(&later, &["--model", "auto"]));

        let p = build_args(Agent::Pi, "sid", true, None, "- hi");
        assert!(p.contains(&"--no-tools".to_string()) && has(&p, &["--session-id", "sid"]));
        assert_eq!(&p[p.len() - 2..], ["--", "- hi"]);

        let first = build_args(Agent::Codex, "ignored", false, None, "hi");
        assert!(has(&first, &["--sandbox", "read-only"]) && first[0] == "exec" && first[1] != "resume");
        let again = build_args(Agent::Codex, "th-1", true, None, "hi");
        assert_eq!(&again[..3], ["exec", "resume", "th-1"]);
    }

    #[test]
    fn a_conversation_is_with_one_agent() {
        let mut c = Chat::default();
        c.saved.agent = "claude".into();
        c.begin_turn("hi");
        let old = c.saved.session_id.clone();
        c.use_agent(Agent::Claude);
        assert_eq!(c.saved.messages.len(), 2, "same agent: the conversation goes on");
        c.use_agent(Agent::Copilot);
        assert!(c.saved.messages.is_empty() && c.saved.session_id != old && c.saved.agent == "copilot");
        assert_eq!(c.to_json()["agent"], "copilot");
    }

    #[test]
    fn claude_aliases_are_not_models_of_the_others() {
        assert!(is_claude_alias("sonnet") && !is_claude_alias("gpt-5"));
        assert_eq!(default_model(Agent::Claude, "haiku").as_deref(), Some("haiku"));
        assert_eq!(default_model(Agent::Pi, "haiku"), None);
    }
}

#[cfg(test)]
mod uuid_tests {
    #[test]
    fn session_ids_are_v4_uuids_and_distinct() {
        let (a, b) = (super::new_session_id(), super::new_session_id());
        assert_eq!(a.len(), 36);
        assert_eq!(a.as_bytes()[14], b'4');
        assert_ne!(a, b);
    }
}
