//! What a session has been doing during its current turn: tools used, files
//! changed (with line counts), commands run and the last result the agent gave.
//!
//! Fed by the agent's events (a prompt starts a turn, tool starts and ends follow the tools,
//! the stop carries the final message). Line counts come from the tool inputs, so they are
//! the size of the edit as requested, not a real diff.

use crate::agent::{Tool, truncate};
use crate::detail::{self, Detail};
use crate::policy::PolicyTag;
use serde_json::{Value, json};
use std::collections::VecDeque;

const RECENT_MAX: usize = 8;

/// A step's stable identity for annotations/history (see `history.rs`): the agent's own call id
/// when it has one, else a fallback from when it was first seen (stable across `pre_tool`'s push
/// and `post_tool`'s later update of the same event, since both read the same `ts_ms`).
pub fn step_id(e: &Event) -> String {
    if e.tool_use_id.is_empty() { format!("t{}", e.ts_ms) } else { e.tool_use_id.clone() }
}

const FILES_MAX: usize = 50;
const RESULT_MAX_CHARS: usize = 320;

#[derive(Debug, Clone)]
pub struct Event {
    pub ts_ms: u64,
    pub tool_use_id: String,
    pub tool: String,
    /// `read`, `write`, `run`, `search`, `think` or `other`.
    pub kind: &'static str,
    pub label: String,
    pub added: u32,
    pub removed: u32,
    /// `None` while the tool is still running.
    pub ok: Option<bool>,
    /// What the live viewer shows for this step.
    pub detail: Option<Detail>,
    /// Set when the call matches a configured policy rule (see `policy.rs`): flagged in the
    /// viewer even though the agent may have auto-approved it.
    pub policy: Option<PolicyTag>,
}

#[derive(Debug, Clone, Default)]
pub struct Activity {
    pub turn_started_ms: Option<u64>,
    pub turn_ms: Option<u64>,
    pub finished_ms: Option<u64>,
    pub tool_calls: u32,
    pub files: Vec<String>,
    pub added: u32,
    pub removed: u32,
    pub commands: u32,
    pub failures: u32,
    pub recent: VecDeque<Event>,
    pub last_result: Option<String>,
}

impl Activity {
    pub fn begin_turn(&mut self, now_ms: u64) {
        let result = self.last_result.take(); // keep showing the previous result
        *self = Activity { turn_started_ms: Some(now_ms), last_result: result, ..Activity::default() };
    }

    pub fn pre_tool(&mut self, tool: &Tool, now_ms: u64, cwd: &str, show_code: bool) {
        self.tool_calls += 1;
        if tool.kind == "run" {
            self.commands += 1;
        }
        self.recent.push_back(Event {
            ts_ms: now_ms,
            tool_use_id: tool.id.clone(),
            tool: tool.name.clone(),
            kind: tool.kind,
            label: tool.label(cwd),
            added: 0,
            removed: 0,
            ok: None,
            detail: detail::for_tool(tool, cwd, show_code),
            policy: None,
        });
        while self.recent.len() > RECENT_MAX {
            self.recent.pop_front();
        }
    }

    /// A tool finished (`ok`) or failed.
    pub fn post_tool(&mut self, tool: &Tool, ok: bool, cwd: &str, show_code: bool) {
        if !ok {
            self.failures += 1;
        }
        let (added, removed) = if ok { (tool.added, tool.removed) } else { (0, 0) };
        if ok && tool.kind == "write" {
            self.added += added;
            self.removed += removed;
            let file = tool.label(cwd);
            if !file.is_empty() && !self.files.contains(&file) && self.files.len() < FILES_MAX {
                self.files.push(file);
            }
        }
        // Agents that give no call id (GitHub Copilot) are matched by tool name: the latest
        // step of that tool that is still running.
        let id = tool.id.as_str();
        let found = if id.is_empty() {
            self.recent.iter_mut().rev().find(|e| e.ok.is_none() && e.tool_use_id.is_empty() && e.tool == tool.name)
        } else {
            self.recent.iter_mut().rev().find(|e| e.tool_use_id == id)
        };
        if let Some(e) = found {
            e.ok = Some(ok);
            e.added = added;
            e.removed = removed;
            e.detail = detail::after_tool(tool, e.detail.take(), cwd, show_code);
        }
    }

    /// The turn ended; `message` is the agent's final answer.
    pub fn stop(&mut self, message: Option<&str>, now_ms: u64) {
        self.finished_ms = Some(now_ms);
        self.turn_ms = self.turn_started_ms.map(|s| now_ms.saturating_sub(s));
        if let Some(m) = message {
            let m = m.trim();
            if !m.is_empty() {
                self.last_result = Some(truncate(&m.split_whitespace().collect::<Vec<_>>().join(" "), RESULT_MAX_CHARS));
            }
        }
        // Anything still "running" at the end of the turn did not report back.
        for e in self.recent.iter_mut().filter(|e| e.ok.is_none()) {
            e.ok = Some(true);
        }
    }

    /// Find the event a tool call refers to: by its id, or (for agents that give no call id,
    /// e.g. Copilot) the most recent one with the same tool name — the same matching
    /// `post_tool` already uses right before mutating it, so a caller mirroring that step into
    /// durable storage (see `history.rs`) sees the exact entry that was just touched, whether it
    /// was just pushed by `pre_tool` or just updated by `post_tool`.
    pub fn find(&self, tool: &Tool) -> Option<&Event> {
        let id = tool.id.as_str();
        if id.is_empty() {
            self.recent.iter().rev().find(|e| e.tool_use_id.is_empty() && e.tool == tool.name)
        } else {
            self.recent.iter().rev().find(|e| e.tool_use_id == id)
        }
    }

    pub fn to_json(&self) -> Value {
        let recent: Vec<Value> = self
            .recent
            .iter()
            .map(|e| {
                json!({
                    "id": step_id(e), "ts_ms": e.ts_ms, "tool": e.tool, "kind": e.kind, "label": e.label,
                    "added": e.added, "removed": e.removed, "ok": e.ok, "detail": e.detail,
                    "policy": e.policy,
                })
            })
            .collect();
        let files: Vec<&String> = self.files.iter().rev().take(6).collect();
        json!({
            "turn_started_ms": self.turn_started_ms,
            "turn_ms": self.turn_ms,
            "finished_ms": self.finished_ms,
            "tool_calls": self.tool_calls,
            "files_changed": self.files.len(),
            "files": files,
            "lines_added": self.added,
            "lines_removed": self.removed,
            "commands": self.commands,
            "failures": self.failures,
            "recent": recent,
            "last_result": self.last_result,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::claude;

    fn pre(tool: &str, id: &str, input: Value) -> Tool {
        let mut t = claude::tool(tool, &input, None);
        t.id = id.to_string();
        t
    }

    #[test]
    fn counts_edits_commands_and_failures() {
        let mut a = Activity::default();
        a.begin_turn(1000);

        let edit = pre("Edit", "t1", json!({"file_path": "/p/src/a.rs", "old_string": "x\ny", "new_string": "x\ny\nz\nw"}));
        a.pre_tool(&edit, 1100, "/p", true);
        assert_eq!(a.recent.back().unwrap().ok, None, "running until PostToolUse");
        a.post_tool(&edit, true, "/p", true);

        let write = pre("Write", "t2", json!({"file_path": "/p/src/b.rs", "content": "1\n2\n3"}));
        a.pre_tool(&write, 1200, "/p", true);
        a.post_tool(&write, true, "/p", true);
        // Editing the same file again must not count it twice.
        let edit2 = pre("Edit", "t3", json!({"file_path": "/p/src/a.rs", "old_string": "a", "new_string": "b"}));
        a.pre_tool(&edit2, 1300, "/p", true);
        a.post_tool(&edit2, true, "/p", true);

        let bash = pre("Bash", "t4", json!({"command": "cargo test\nsecond line"}));
        a.pre_tool(&bash, 1400, "/p", true);
        a.post_tool(&bash, false, "/p", true);

        assert_eq!(a.tool_calls, 4);
        assert_eq!(a.files, vec!["src/a.rs", "src/b.rs"]);
        assert_eq!((a.added, a.removed), (4 + 3 + 1, 2 + 1)); // edit + write + edit; the write removes nothing
        assert_eq!((a.commands, a.failures), (1, 1));
        let last = a.recent.back().unwrap();
        assert_eq!((last.kind, last.label.as_str(), last.ok), ("run", "cargo test", Some(false)));
    }

    #[test]
    fn multiedit_and_recent_cap() {
        let mut a = Activity::default();
        let me = pre("MultiEdit", "m", json!({"file_path": "f", "edits": [
            {"old_string": "a", "new_string": "a\nb"}, {"old_string": "c\nd", "new_string": "e"}]}));
        a.pre_tool(&me, 1, "", true);
        a.post_tool(&me, true, "", true);
        assert_eq!((a.added, a.removed), (3, 3));
        for i in 0..20 {
            a.pre_tool(&pre("Read", &format!("r{i}"), json!({"file_path": "x"})), i, "", true);
        }
        assert_eq!(a.recent.len(), RECENT_MAX);
    }

    #[test]
    fn calls_without_an_id_are_matched_by_tool_name() {
        let mut a = Activity::default();
        a.pre_tool(&pre("Bash", "", json!({"command": "one"})), 1, "", true);
        a.pre_tool(&pre("Read", "", json!({"file_path": "x"})), 2, "", true);
        a.post_tool(&pre("Bash", "", json!({"command": "one"})), true, "", true);
        assert_eq!((a.recent[0].ok, a.recent[1].ok), (Some(true), None));
        a.post_tool(&pre("Read", "", json!({"file_path": "x"})), false, "", true);
        assert_eq!(a.recent[1].ok, Some(false));
    }

    #[test]
    fn stop_records_result_and_duration() {
        let mut a = Activity::default();
        a.begin_turn(1_000);
        a.pre_tool(&pre("Read", "r", json!({"file_path": "x"})), 1_100, "", true);
        a.stop(Some("  All done.\n\nI changed   three files. "), 4_500);
        assert_eq!(a.turn_ms, Some(3_500));
        assert_eq!(a.last_result.as_deref(), Some("All done. I changed three files."));
        assert_eq!(a.recent.back().unwrap().ok, Some(true), "unfinished tools are closed at the end of the turn");
        // The previous result stays visible while the next turn runs.
        a.begin_turn(9_000);
        assert_eq!(a.last_result.as_deref(), Some("All done. I changed three files."));
        assert_eq!(a.tool_calls, 0);
    }

    #[test]
    fn long_results_are_truncated_on_char_boundaries() {
        let mut a = Activity::default();
        a.stop(Some(&"è".repeat(1000)), 1);
        let r = a.last_result.unwrap();
        assert_eq!(r.chars().count(), RESULT_MAX_CHARS + 1);
        assert!(r.ends_with('…'));
    }
}
