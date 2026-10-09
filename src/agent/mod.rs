//! The coding agents Sushi can watch, and the neutral shape their events are translated into.
//!
//! Every agent talks to `sushi-hook` in its own words (Claude Code and Codex through command
//! hooks, opencode and pi through the small plugins in `integrations/`). Each adapter below
//! turns that into an [`AgentEvent`] (with a normalized [`Tool`]) and turns the notch's
//! Allow / Deny back into what the agent expects. Everything after this module (sessions,
//! activity, the live viewer) is agent-agnostic.

pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod copilot;
pub mod gemini;
pub mod opencode;
pub mod pi;

use crate::protocol::Decision;
use serde::Serialize;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Agent {
    Claude,
    Codex,
    Opencode,
    Pi,
    Copilot,
    Antigravity,
    Gemini,
}

/// What an agent lets Sushi do beyond showing its sessions.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Capabilities {
    /// Allow / Deny from the notch.
    pub approve: bool,
    /// Answer the agent's questions from the notch.
    pub questions: bool,
    /// Show the plan waiting for approval.
    pub plans: bool,
    /// Context window and token usage (read from the agent's transcripts).
    pub context: bool,
    /// Plan limits (5-hour, weekly).
    pub limits: bool,
    /// The built-in chat.
    pub chat: bool,
}

impl Agent {
    pub const ALL: [Agent; 7] = [Agent::Claude, Agent::Codex, Agent::Opencode, Agent::Pi, Agent::Copilot, Agent::Antigravity, Agent::Gemini];

    pub fn id(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Opencode => "opencode",
            Agent::Pi => "pi",
            Agent::Copilot => "copilot",
            Agent::Antigravity => "antigravity",
            Agent::Gemini => "gemini",
        }
    }

    pub fn from_id(id: &str) -> Option<Agent> {
        Agent::ALL.into_iter().find(|a| a.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Opencode => "opencode",
            Agent::Pi => "pi",
            Agent::Copilot => "GitHub Copilot",
            Agent::Antigravity => "Antigravity",
            Agent::Gemini => "Gemini CLI",
        }
    }

    pub fn capabilities(self) -> Capabilities {
        let basic = Capabilities { approve: true, questions: false, plans: false, context: false, limits: false, chat: false };
        match self {
            Agent::Claude => Capabilities { approve: true, questions: true, plans: true, context: true, limits: true, chat: true },
            // The chat needs a way to run the agent headless without tools (see `chat.rs`).
            // Codex and Copilot also get plan usage (see `limits::codex` / `limits::copilot`);
            // pi has no known usage endpoint.
            Agent::Codex | Agent::Copilot => Capabilities { chat: true, limits: true, ..basic },
            Agent::Pi => Capabilities { chat: true, ..basic },
            // Antigravity is watched only: a hook cannot approve (see `antigravity.rs`), and its
            // headless mode has no tool-free option for the chat. Plan usage (`limits::antigravity`)
            // is the one thing it does get, reusing Google's Cloud Code Assist backend.
            Agent::Opencode => basic,
            Agent::Antigravity => Capabilities { approve: antigravity::ASK_FROM_NOTCH, limits: true, ..basic },
            // Gemini CLI: see `gemini.rs` for why `approve` is a documentation-only reading (no
            // real install was tested, unlike Antigravity's confirmed bug) and `context`/`chat`
            // stay off (no confirmed transcript schema, no tool-free headless mode). No known
            // usage endpoint either.
            Agent::Gemini => basic,
        }
    }

    /// Translate what the agent sent into a neutral event. `None` for events Sushi ignores.
    pub fn normalize(self, payload: &Value) -> Option<AgentEvent> {
        let payload = &self.canonical(payload);
        let mut ev = envelope(self, payload)?;
        if let Some(raw) = payload.get("tool_name").and_then(Value::as_str) {
            let input = payload.get("tool_input").cloned().unwrap_or(Value::Null);
            let response = payload.get("tool_response");
            let mut tool = match self {
                Agent::Claude => claude::tool(raw, &input, response),
                Agent::Codex => codex::tool(raw, &input, response),
                Agent::Opencode => opencode::tool(raw, &input, response),
                Agent::Pi => pi::tool(raw, &input, response),
                Agent::Copilot => copilot::tool(raw, &input, response),
                Agent::Antigravity => antigravity::tool(raw, &input, response),
                Agent::Gemini => gemini::tool(raw, &input, response),
            };
            tool.id = payload.get("tool_use_id").and_then(Value::as_str).unwrap_or("").to_string();
            ev.tool = Some(tool);
        }
        Some(ev)
    }

    /// Does this event wait for an answer from the notch?
    pub fn wants_answer(self, payload: &Value) -> bool {
        let payload = &self.canonical(payload);
        let is_permission = payload.get("hook_event_name").and_then(Value::as_str) == Some("PermissionRequest");
        is_permission && self.capabilities().approve
    }

    /// Is this Claude Code's "Ready to code?" dialog (`ExitPlanMode`)? A plan takes longer to read
    /// than a permission, so the hook waits longer for it (the terminal shows the dialog meanwhile).
    pub fn is_plan(self, payload: &Value) -> bool {
        self == Agent::Claude && payload.get("tool_name").and_then(Value::as_str) == Some("ExitPlanMode")
    }

    /// What the hook prints on stdout for a decision. `updated_input` replaces the tool's input
    /// (how the answers to a question reach the agent, and how a plan is approved: Claude Code
    /// ignores a bare `allow` for the tools that need the user, like `ExitPlanMode`). `mode` is the
    /// permission mode to switch to (`acceptEdits`).
    pub fn encode_decision(self, decision: Decision, updated_input: Option<&Value>, mode: Option<&str>) -> Value {
        match self {
            Agent::Claude | Agent::Codex => hook_specific_output(decision, updated_input, mode),
            // Copilot's `PermissionRequest` hook answers with a bare behavior.
            Agent::Copilot => match decision {
                Decision::Allow => json!({ "behavior": "allow" }),
                Decision::Deny => json!({ "behavior": "deny", "message": "Denied from Sushi" }),
            },
            // Antigravity's `PreToolUse` hook and Gemini's `BeforeTool` hook both answer with a
            // decision and a reason.
            Agent::Antigravity | Agent::Gemini => match decision {
                Decision::Allow => json!({ "decision": "allow" }),
                Decision::Deny => json!({ "decision": "deny", "reason": "Denied from Sushi" }),
            },
            // The opencode and pi plugins read this and map it onto their own API.
            Agent::Opencode | Agent::Pi => match decision {
                Decision::Allow => json!({ "decision": "allow" }),
                Decision::Deny => json!({ "decision": "deny", "message": "Denied from Sushi" }),
            },
        }
    }
}

impl Agent {
    /// The payload with the keys the neutral envelope reads (Copilot mixes naming styles, Antigravity has its own event names).
    fn canonical(self, payload: &Value) -> Value {
        match self {
            Agent::Codex => codex::canonical(payload),
            Agent::Copilot => copilot::canonical(payload),
            Agent::Antigravity => antigravity::canonical(payload),
            Agent::Gemini => gemini::canonical(payload),
            _ => payload.clone(),
        }
    }
}

/// The `PermissionRequest` output of Claude Code and Codex.
fn hook_specific_output(decision: Decision, updated_input: Option<&Value>, mode: Option<&str>) -> Value {
    let decision = match decision {
        Decision::Allow => {
            let mut d = json!({ "behavior": "allow" });
            if let Some(input) = updated_input {
                d["updatedInput"] = input.clone();
            }
            if let Some(mode) = mode {
                d["updatedPermissions"] = json!([{ "type": "setMode", "mode": mode, "destination": "session" }]);
            }
            d
        }
        Decision::Deny => json!({ "behavior": "deny", "message": "Denied from Sushi" }),
    };
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    SessionStart,
    SessionEnd,
    /// The user sent a prompt: a turn begins.
    Prompt,
    ToolStart,
    ToolEnd { ok: bool },
    PermissionRequest,
    /// The agent is blocked waiting for the user.
    Waiting,
    /// The agent is waiting for the next prompt.
    Idle,
    /// The turn is over; `message` is the agent's final answer.
    Stop { message: Option<String> },
}

#[derive(Debug, Clone)]
pub struct AgentEvent {
    pub agent: Agent,
    pub session_id: String,
    pub cwd: String,
    pub pid: Option<u32>,
    /// Transcript to read token usage from (only for agents that support it).
    pub transcript: Option<PathBuf>,
    pub kind: EventKind,
    pub tool: Option<Tool>,
}

/// The event names every adapter understands: Claude Code's, which Codex shares and which the
/// opencode and pi plugins use too.
fn envelope(agent: Agent, v: &Value) -> Option<AgentEvent> {
    let s = |k: &str| v.get(k).and_then(Value::as_str);
    let kind = match s("hook_event_name")? {
        "SessionStart" => EventKind::SessionStart,
        "SessionEnd" => EventKind::SessionEnd,
        "UserPromptSubmit" => EventKind::Prompt,
        "PreToolUse" => EventKind::ToolStart,
        "PostToolUse" => EventKind::ToolEnd { ok: true },
        "PostToolUseFailure" => EventKind::ToolEnd { ok: false },
        "PermissionRequest" => EventKind::PermissionRequest,
        "Stop" => EventKind::Stop { message: s("last_assistant_message").map(str::to_string) },
        "Notification" => match s("notification_type") {
            Some("permission_prompt") => EventKind::Waiting,
            Some("idle_prompt" | "agent_idle") => EventKind::Idle,
            _ => return None,
        },
        _ => return None,
    };
    let transcript = if agent.capabilities().context { s("transcript_path").map(PathBuf::from) } else { None };
    Some(AgentEvent {
        agent,
        session_id: s("session_id")?.to_string(),
        cwd: s("cwd").unwrap_or("").to_string(),
        pid: v.get("pid").and_then(Value::as_u64).map(|p| p as u32),
        transcript,
        kind,
        tool: None,
    })
}

/// What a tool call needs the notch to do beyond showing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    #[default]
    Normal,
    /// The agent asks the user a question; the answer goes back as the tool's input.
    Question,
    /// A plan waiting for approval in the terminal.
    Plan,
}

/// A tool call, whichever agent made it.
#[derive(Debug, Clone, Default)]
pub struct Tool {
    /// The agent's own name for the tool (`Bash`, `edit`, ...).
    pub name: String,
    /// `read`, `write`, `run`, `search`, `think` or `other`.
    pub kind: &'static str,
    /// Correlates the start and the end of one call.
    pub id: String,
    pub role: Role,
    /// File being read or written.
    pub path: Option<String>,
    pub command: Option<String>,
    /// Search pattern (grep, glob) or web query.
    pub pattern: Option<String>,
    /// Folder a search runs in.
    pub search_path: Option<String>,
    pub url: Option<String>,
    /// Short description of a subagent task.
    pub description: Option<String>,
    pub prompt: Option<String>,
    /// Replacements `(old, new)` a write-type tool makes in `path`.
    pub edits: Vec<(String, String)>,
    /// Whole content a write-type tool puts in `path`.
    pub content: Option<String>,
    /// First line read (1-based).
    pub offset: Option<u32>,
    pub plan: Option<String>,
    /// Lines added / removed by a write-type tool, as requested.
    pub added: u32,
    pub removed: u32,
    /// The original input, kept for questions (the answers are added to it).
    pub input: Value,
    /// What a command printed, once it ran.
    pub output: Option<String>,
    /// File content a read returned, with the number of its first line.
    pub read: Option<(String, u32)>,
}

impl Tool {
    pub fn new(name: &str, kind: &'static str, input: &Value) -> Tool {
        Tool { name: name.to_string(), kind, input: input.clone(), ..Tool::default() }
    }

    /// What the step is about, short: a path relative to `cwd`, a command, a pattern...
    pub fn label(&self, cwd: &str) -> String {
        if let Some(p) = &self.path {
            short_path(p, cwd)
        } else if let Some(c) = &self.command {
            truncate(first_line(c), 70)
        } else if let Some(p) = &self.pattern {
            truncate(p, 60)
        } else if let Some(u) = &self.url {
            truncate(u, 70)
        } else if let Some(d) = self.description.as_ref().or(self.prompt.as_ref()) {
            truncate(first_line(d), 60)
        } else {
            self.name.clone()
        }
    }

    /// The detail shown next to the tool name in summaries (`cargo test` in `Bash: cargo test`).
    pub fn summary_detail(&self) -> Option<String> {
        let d = self
            .command
            .as_deref()
            .or(self.path.as_deref())
            .or(self.pattern.as_deref())
            .or(self.url.as_deref())
            .or(self.description.as_deref())?;
        Some(d.lines().next().unwrap_or("").chars().take(80).collect())
    }

    pub fn summary(&self) -> String {
        match self.summary_detail() {
            Some(d) => format!("{}: {d}", self.name),
            None => self.name.clone(),
        }
    }
}

pub fn truncate(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

pub fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or("")
}

/// `/home/me/proj/src/main.rs` → `src/main.rs` when inside the session's cwd.
pub fn short_path(path: &str, cwd: &str) -> String {
    match path.strip_prefix(cwd).map(|r| r.trim_start_matches('/')) {
        Some(r) if !r.is_empty() && !cwd.is_empty() => r.to_string(),
        _ => path.to_string(),
    }
}

pub fn count_lines(s: &str) -> u32 {
    s.lines().count() as u32
}

/// `(added, removed)` lines of a list of replacements.
pub fn edits_lines(edits: &[(String, String)]) -> (u32, u32) {
    edits.iter().fold((0, 0), |(a, r), (old, new)| (a + count_lines(new), r + count_lines(old)))
}

/// String field of a JSON object.
pub fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

/// A tool response as text: the string itself, or `stdout` + `stderr`.
pub fn response_text(response: &Value) -> String {
    match response {
        Value::String(t) => t.clone(),
        Value::Null => String::new(),
        other => {
            let out = str_of(other, "stdout").or_else(|| str_of(other, "output")).unwrap_or("");
            let err = str_of(other, "stderr").unwrap_or("");
            if err.is_empty() { out.to_string() } else { format!("{out}\n{err}") }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_roundtrip() {
        for a in Agent::ALL {
            assert_eq!(Agent::from_id(a.id()), Some(a));
        }
        assert_eq!(Agent::from_id("nope"), None);
    }

    #[test]
    fn only_claude_has_questions_and_plans() {
        assert!(Agent::Claude.capabilities().chat && Agent::Claude.capabilities().limits);
        assert!(!Agent::Antigravity.capabilities().approve, "watched only");
        for a in [Agent::Codex, Agent::Opencode, Agent::Pi, Agent::Copilot, Agent::Gemini] {
            let c = a.capabilities();
            assert!(c.approve && !c.context && !c.questions && !c.plans);
        }
        assert!(!Agent::Opencode.capabilities().chat && Agent::Copilot.capabilities().chat);
        assert!(!Agent::Antigravity.capabilities().chat && !Agent::Gemini.capabilities().chat);
    }

    #[test]
    fn limits_are_reported_only_for_agents_with_a_known_usage_endpoint() {
        for a in [Agent::Claude, Agent::Codex, Agent::Copilot, Agent::Antigravity] {
            assert!(a.capabilities().limits, "{a:?} should report plan usage");
        }
        for a in [Agent::Opencode, Agent::Pi, Agent::Gemini] {
            assert!(!a.capabilities().limits, "{a:?} has no known usage endpoint");
        }
    }

    #[test]
    fn decisions_are_encoded_per_agent() {
        let v = Agent::Codex.encode_decision(Decision::Deny, None, None);
        assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert_eq!(Agent::Copilot.encode_decision(Decision::Allow, None, None), json!({"behavior": "allow"}));
        let v = Agent::Copilot.encode_decision(Decision::Deny, None, None);
        assert_eq!((v["behavior"].as_str(), v["message"].is_string()), (Some("deny"), true));
        let v = Agent::Antigravity.encode_decision(Decision::Deny, None, None);
        assert_eq!((v["decision"].as_str(), v["reason"].is_string()), (Some("deny"), true));
        assert_eq!(Agent::Antigravity.encode_decision(Decision::Allow, None, None), json!({"decision": "allow"}));
        let v = Agent::Pi.encode_decision(Decision::Allow, None, None);
        assert_eq!(v, json!({"decision": "allow"}));
        let v = Agent::Opencode.encode_decision(Decision::Deny, None, None);
        assert_eq!(v["decision"], "deny");
        assert!(v["message"].is_string());
    }

    #[test]
    fn allow_can_carry_an_updated_input_but_deny_never_does() {
        let input = json!({"questions": [], "answers": {"Which colour?": "Blue"}});
        let v = Agent::Claude.encode_decision(Decision::Allow, Some(&input), None);
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PermissionRequest");
        assert_eq!(v["hookSpecificOutput"]["decision"]["updatedInput"]["answers"]["Which colour?"], "Blue");
        let v = Agent::Claude.encode_decision(Decision::Deny, Some(&input), None);
        assert!(v["hookSpecificOutput"]["decision"].get("updatedInput").is_none());
        assert!(v["hookSpecificOutput"]["decision"]["message"].is_string());
    }

    #[test]
    fn a_plan_is_approved_with_its_input_and_maybe_a_new_mode() {
        // Recorded from Claude Code 2.1.287: a bare allow leaves the "Ready to code?" dialog open.
        let input = json!({"plan": "# Plan", "planFilePath": "/home/u/.claude/plans/p.md"});
        let v = Agent::Claude.encode_decision(Decision::Allow, Some(&input), Some("acceptEdits"));
        let d = &v["hookSpecificOutput"]["decision"];
        assert_eq!((d["behavior"].as_str(), &d["updatedInput"]), (Some("allow"), &input));
        assert_eq!(d["updatedPermissions"], json!([{"type": "setMode", "mode": "acceptEdits", "destination": "session"}]));
        let v = Agent::Claude.encode_decision(Decision::Allow, Some(&input), None);
        assert!(v["hookSpecificOutput"]["decision"].get("updatedPermissions").is_none());
        let v = Agent::Claude.encode_decision(Decision::Deny, Some(&input), Some("acceptEdits"));
        assert!(v["hookSpecificOutput"]["decision"].get("updatedPermissions").is_none());
    }

    #[test]
    fn only_permissions_wait() {
        let p = |tool: &str| json!({"hook_event_name": "PermissionRequest", "tool_name": tool});
        assert!(Agent::Claude.wants_answer(&p("Bash")));
        assert!(Agent::Claude.wants_answer(&p("ExitPlanMode")), "a plan can be approved from the notch");
        assert!(Agent::Claude.is_plan(&p("ExitPlanMode")) && !Agent::Codex.is_plan(&p("ExitPlanMode")));
        assert!(Agent::Codex.wants_answer(&p("ExitPlanMode")));
        assert!(!Agent::Claude.wants_answer(&json!({"hook_event_name": "PreToolUse"})));
        let g = |tool: &str| json!({"conversationId": "c", "stepIdx": 1, "toolCall": {"name": tool, "args": {}}});
        assert!(!Agent::Antigravity.wants_answer(&g("run_command")), "nothing waits for Antigravity");
    }

    #[test]
    fn unknown_events_are_ignored_and_pid_is_read() {
        assert!(Agent::Pi.normalize(&json!({"session_id": "a", "hook_event_name": "Whatever"})).is_none());
        assert!(Agent::Pi.normalize(&json!({"hook_event_name": "Stop"})).is_none(), "no session id");
        let ev = Agent::Pi.normalize(&json!({"session_id": "a", "cwd": "/p", "pid": 42, "hook_event_name": "UserPromptSubmit"})).unwrap();
        assert_eq!((ev.pid, ev.kind), (Some(42), EventKind::Prompt));
    }

    #[test]
    fn transcripts_are_only_kept_for_agents_that_can_be_read() {
        let v = json!({"session_id": "a", "hook_event_name": "Stop", "transcript_path": "/t.jsonl"});
        assert!(Agent::Claude.normalize(&v).unwrap().transcript.is_some());
        assert!(Agent::Codex.normalize(&v).unwrap().transcript.is_none());
    }
}
