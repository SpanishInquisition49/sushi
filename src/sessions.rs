//! Live view of the agents' sessions, fed by their events. Claude Code sessions are also
//! reconciled with `~/.claude/sessions/<pid>.json`.

use crate::activity::Activity;
use crate::agent::{Agent, AgentEvent, EventKind, Tool};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Sessions of agents that cannot be checked by process id disappear after this long without
/// any event (their end event may have been lost).
const SILENT_EXPIRY_MS: u64 = 3 * 3600 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Idle,
    Working,
    Waiting,
}

/// The last tool a session used, as the widget and the pet show it.
#[derive(Debug, Clone, Serialize)]
pub struct LastTool {
    pub name: String,
    pub kind: &'static str,
    pub detail: Option<String>,
    /// `Bash: cargo test`
    pub text: String,
}

impl LastTool {
    fn of(tool: &Tool) -> LastTool {
        LastTool { name: tool.name.clone(), kind: tool.kind, detail: tool.summary_detail(), text: tool.summary() }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Session {
    /// Unique across agents: `<agent>:<the agent's own session id>`.
    pub id: String,
    pub agent: Agent,
    pub name: String,
    pub cwd: String,
    pub status: Status,
    pub last_tool: Option<LastTool>,
    pub pid: Option<u32>,
    pub last_event_ms: u64,
    #[serde(skip)]
    pub native_id: String,
    #[serde(skip)]
    pub transcript: Option<PathBuf>,
    /// What the session did during its current turn (published separately).
    #[serde(skip)]
    pub activity: Activity,
}

#[derive(Debug, Default)]
pub struct Sessions {
    pub map: BTreeMap<String, Session>,
    /// Privacy: do not keep code or command output in the published steps.
    pub hide_code: bool,
    /// Sessions running in this folder are not listed (the built-in chat runs there).
    pub ignore_cwd: Option<String>,
}

fn basename(cwd: &str) -> String {
    Path::new(cwd)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.to_string())
}

pub fn session_key(agent: Agent, native_id: &str) -> String {
    format!("{}:{native_id}", agent.id())
}

impl Session {
    fn new(agent: Agent, native_id: &str, cwd: &str, now_ms: u64) -> Session {
        Session {
            id: session_key(agent, native_id),
            agent,
            name: basename(cwd),
            cwd: cwd.to_string(),
            status: Status::Idle,
            last_tool: None,
            pid: None,
            last_event_ms: now_ms,
            native_id: native_id.to_string(),
            transcript: None,
            activity: Activity::default(),
        }
    }
}

impl Sessions {
    fn entry(&mut self, ev: &AgentEvent, now_ms: u64) -> &mut Session {
        let key = session_key(ev.agent, &ev.session_id);
        let s = self.map.entry(key).or_insert_with(|| Session::new(ev.agent, &ev.session_id, &ev.cwd, now_ms));
        if s.cwd.is_empty() && !ev.cwd.is_empty() {
            s.cwd = ev.cwd.clone();
        }
        if ev.transcript.is_some() {
            s.transcript = ev.transcript.clone();
        }
        if ev.pid.is_some() {
            s.pid = ev.pid;
        }
        s.last_event_ms = now_ms;
        s
    }

    /// Apply an event. Returns the key of the session it concerned.
    pub fn apply_event(&mut self, ev: &AgentEvent, now_ms: u64) -> String {
        if ev.kind == EventKind::SessionEnd {
            let key = session_key(ev.agent, &ev.session_id);
            self.map.remove(&key);
            return key;
        }
        let show_code = !self.hide_code;
        let s = self.entry(ev, now_ms);
        let cwd = s.cwd.clone();
        match (&ev.kind, &ev.tool) {
            (EventKind::SessionStart, _) => {
                s.status = Status::Idle;
                s.activity = Activity::default();
            }
            (EventKind::Stop { message }, _) => {
                s.status = Status::Idle;
                s.activity.stop(message.as_deref(), now_ms);
            }
            (EventKind::Prompt, _) => {
                s.status = Status::Working;
                s.activity.begin_turn(now_ms);
            }
            (EventKind::ToolEnd { ok }, Some(tool)) => {
                s.status = Status::Working;
                s.activity.post_tool(tool, *ok, &cwd, show_code);
            }
            (EventKind::ToolEnd { .. }, None) => s.status = Status::Working,
            (EventKind::ToolStart, tool) => {
                s.status = Status::Working;
                if let Some(tool) = tool {
                    s.last_tool = Some(LastTool::of(tool));
                    s.activity.pre_tool(tool, now_ms, &cwd, show_code);
                }
            }
            (EventKind::PermissionRequest, tool) => {
                s.status = Status::Waiting;
                if let Some(tool) = tool {
                    s.last_tool = Some(LastTool::of(tool));
                }
            }
            (EventKind::Waiting, _) => s.status = Status::Waiting,
            (EventKind::Idle, _) => s.status = Status::Idle,
            (EventKind::SessionEnd, _) => {}
        }
        s.id.clone()
    }

    /// Merge `~/.claude/sessions/*.json` and drop sessions whose process died. Sessions of other
    /// agents are dropped when their process (reported by their plugin) is gone, or when they
    /// have been silent for hours and have no process to check.
    pub fn reconcile(&mut self, dir: &Path, now_ms: u64) {
        let mut alive_pids = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("json") {
                    continue;
                }
                let Some(v) = std::fs::read_to_string(&p)
                    .ok()
                    .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                else {
                    continue;
                };
                let (Some(id), Some(pid)) = (
                    v.get("sessionId").and_then(Value::as_str),
                    v.get("pid").and_then(Value::as_u64),
                ) else {
                    continue;
                };
                if !crate::paths::pid_alive(pid as u32) {
                    continue;
                }
                alive_pids.push(pid as u32);
                let cwd = v.get("cwd").and_then(Value::as_str).unwrap_or("");
                if self.ignore_cwd.as_deref() == Some(cwd) {
                    continue;
                }
                let s = self.map.entry(session_key(Agent::Claude, id)).or_insert_with(|| {
                    let mut s = Session::new(Agent::Claude, id, cwd, 0);
                    s.last_event_ms = 0;
                    s
                });
                s.pid = Some(pid as u32);
                if let Some(n) = v.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()) {
                    s.name = n.to_string();
                }
                // Without any hook event yet, trust the file's own status.
                if s.last_event_ms == 0 {
                    s.last_event_ms = now_ms;
                    if v.get("status").and_then(Value::as_str) != Some("idle") {
                        s.status = Status::Working;
                        // A turn already in progress: it started when the status last changed.
                        if s.activity.turn_started_ms.is_none() {
                            s.activity.turn_started_ms = v.get("statusUpdatedAt").and_then(Value::as_u64);
                        }
                    }
                }
            }
        }
        self.map.retain(|_, s| match (s.agent, s.pid) {
            (Agent::Claude, pid) => pid.is_none_or(|p| alive_pids.contains(&p)),
            (_, Some(pid)) => crate::paths::pid_alive(pid),
            (_, None) => now_ms.saturating_sub(s.last_event_ms) < SILENT_EXPIRY_MS,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Feed a Claude-style payload, as the hook would.
    fn feed(s: &mut Sessions, agent: Agent, v: &Value, now: u64) -> Option<String> {
        agent.normalize(v).map(|ev| s.apply_event(&ev, now))
    }

    #[test]
    fn lifecycle() {
        let mut s = Sessions::default();
        let base = |ev: &str| json!({"session_id":"a","cwd":"/x/proj","hook_event_name":ev});
        feed(&mut s, Agent::Claude, &base("SessionStart"), 1);
        assert_eq!(s.map["claude:a"].name, "proj");
        assert_eq!(s.map["claude:a"].status, Status::Idle);

        let mut pre = base("PreToolUse");
        pre["tool_name"] = json!("Bash");
        pre["tool_input"] = json!({"command":"cargo test\nmore"});
        feed(&mut s, Agent::Claude, &pre, 2);
        assert_eq!(s.map["claude:a"].status, Status::Working);
        let last = s.map["claude:a"].last_tool.as_ref().unwrap();
        assert_eq!((last.text.as_str(), last.kind, last.name.as_str()), ("Bash: cargo test", "run", "Bash"));

        feed(&mut s, Agent::Claude, &base("PermissionRequest"), 3);
        assert_eq!(s.map["claude:a"].status, Status::Waiting);
        feed(&mut s, Agent::Claude, &base("Stop"), 4);
        assert_eq!(s.map["claude:a"].status, Status::Idle);
        feed(&mut s, Agent::Claude, &base("SessionEnd"), 5);
        assert!(s.map.is_empty());
    }

    #[test]
    fn hooks_feed_the_activity_of_the_turn() {
        let mut s = Sessions::default();
        let ev = |name: &str, extra: Value| {
            let mut v = json!({"session_id": "a", "cwd": "/p", "hook_event_name": name});
            for (k, x) in extra.as_object().unwrap() {
                v[k] = x.clone();
            }
            v
        };
        let c = Agent::Claude;
        feed(&mut s, c, &ev("SessionStart", json!({})), 1);
        feed(&mut s, c, &ev("UserPromptSubmit", json!({})), 1_000);
        let edit = json!({"tool_name": "Edit", "tool_use_id": "t1",
            "tool_input": {"file_path": "/p/a.rs", "old_string": "a", "new_string": "b\nc"}});
        feed(&mut s, c, &ev("PreToolUse", edit.clone()), 1_100);
        feed(&mut s, c, &ev("PostToolUse", edit), 1_200);
        let a = &s.map["claude:a"].activity;
        assert_eq!((a.tool_calls, a.files.len(), a.added, a.removed), (1, 1, 2, 1));
        assert_eq!(a.turn_started_ms, Some(1_000));
        feed(&mut s, c, &ev("Stop", json!({"last_assistant_message": "Done."})), 5_000);
        let a = &s.map["claude:a"].activity;
        assert_eq!((a.turn_ms, a.last_result.as_deref()), (Some(4_000), Some("Done.")));
        // A new session start wipes everything.
        feed(&mut s, c, &ev("SessionStart", json!({})), 6_000);
        assert_eq!(s.map["claude:a"].activity.tool_calls, 0);
    }

    #[test]
    fn notification_types() {
        let mut s = Sessions::default();
        let mut n = json!({"session_id":"a","hook_event_name":"Notification","notification_type":"permission_prompt"});
        feed(&mut s, Agent::Claude, &n, 1);
        assert_eq!(s.map["claude:a"].status, Status::Waiting);
        n["notification_type"] = json!("idle_prompt");
        feed(&mut s, Agent::Claude, &n, 2);
        assert_eq!(s.map["claude:a"].status, Status::Idle);
    }

    #[test]
    fn agents_do_not_share_sessions_even_with_the_same_id() {
        let mut s = Sessions::default();
        let v = json!({"session_id": "same", "cwd": "/p", "hook_event_name": "UserPromptSubmit"});
        feed(&mut s, Agent::Claude, &v, 1);
        feed(&mut s, Agent::Pi, &v, 1);
        assert_eq!(s.map.len(), 2);
        assert_eq!(s.map["pi:same"].agent, Agent::Pi);
    }

    #[test]
    fn other_agents_are_dropped_when_their_process_is_gone_or_silent() {
        let mut s = Sessions::default();
        let dir = std::env::temp_dir().join(format!("sushi-recon-{}", std::process::id()));
        let alive = std::process::id();
        feed(&mut s, Agent::Pi, &json!({"session_id": "live", "pid": alive, "hook_event_name": "SessionStart"}), 1);
        feed(&mut s, Agent::Pi, &json!({"session_id": "dead", "pid": 4_000_000, "hook_event_name": "SessionStart"}), 1);
        feed(&mut s, Agent::Opencode, &json!({"session_id": "quiet", "hook_event_name": "SessionStart"}), 1);
        s.reconcile(&dir, 1_000);
        assert!(s.map.contains_key("pi:live") && s.map.contains_key("opencode:quiet") && !s.map.contains_key("pi:dead"));
        s.reconcile(&dir, SILENT_EXPIRY_MS + 10);
        assert!(!s.map.contains_key("opencode:quiet"), "silent for hours, nothing to check");
        assert!(s.map.contains_key("pi:live"), "its process still runs");
    }
}
