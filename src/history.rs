//! Durable per-session step log, beyond what `Activity` keeps in memory: that struct resets to
//! empty on every new turn (`Activity::begin_turn`) and keeps only the last `RECENT_MAX` steps,
//! and the whole `Session` (with it) is dropped the instant `SessionEnd` arrives. This store
//! mirrors the same bounded `Event`/`Detail` data into something that survives both, backing the
//! Live tab's search/filter and export, and manual step annotations.
//!
//! Persisted like `stats.rs`/`care.rs`: `load`/`save` tolerant of a missing or corrupt file,
//! flushed from `main.rs` at most once per housekeeping tick.

use crate::activity::Event;
use crate::detail::Detail;
use crate::usage::utc_day;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::Path;

/// Steps kept per session (oldest evicted first).
pub const STEPS_MAX: usize = 500;
/// Sessions kept across the whole store (oldest-by-`last_event_ms` evicted first).
pub const SESSIONS_MAX: usize = 200;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoredStep {
    pub id: String,
    pub ts_ms: u64,
    pub tool: String,
    pub kind: String,
    pub label: String,
    pub ok: Option<bool>,
    pub added: u32,
    pub removed: u32,
    pub detail: Option<Detail>,
    #[serde(default)]
    pub flagged: bool,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionHistory {
    pub agent: String,
    pub cwd: String,
    pub name: String,
    pub last_event_ms: u64,
    pub steps: VecDeque<StoredStep>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryStore {
    pub sessions: HashMap<String, SessionHistory>,
}

fn matches_query(s: &StoredStep, q: &str) -> bool {
    if s.tool.to_lowercase().contains(q) || s.label.to_lowercase().contains(q) {
        return true;
    }
    match &s.detail {
        Some(Detail::Terminal { command, output }) => command.to_lowercase().contains(q) || output.iter().any(|l| l.to_lowercase().contains(q)),
        Some(Detail::Diff { file, lines, .. }) => file.to_lowercase().contains(q) || lines.iter().any(|l| l.text.to_lowercase().contains(q)),
        Some(Detail::File { file, lines, .. }) => file.to_lowercase().contains(q) || lines.iter().any(|l| l.to_lowercase().contains(q)),
        Some(Detail::Text { title, body }) => title.to_lowercase().contains(q) || body.to_lowercase().contains(q),
        Some(Detail::Questions { .. }) | None => false,
    }
}

/// UTC day, `HH:MM:SS UTC` for the export header (no new date dependency: reuses `usage::utc_day`).
fn fmt_ts(ms: u64) -> String {
    let secs = (ms / 1000) % 86_400;
    format!("{} {:02}:{:02}:{:02} UTC", utc_day(ms), secs / 3600, (secs / 60) % 60, secs % 60)
}

impl HistoryStore {
    pub fn load(path: &Path) -> HistoryStore {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string(self) {
            let _ = std::fs::write(path, text);
        }
    }

    /// Mirror a live step (just pushed by `pre_tool`, or just updated by `post_tool` — find it
    /// with `Activity::find` first, using the same matching `post_tool` itself used) into the
    /// durable log: a new id is pushed, a repeat id is updated in place.
    #[allow(clippy::too_many_arguments)] // one line per piece of session metadata mirrored alongside the step; a struct would just move the naming here
    pub fn record(&mut self, key: &str, agent: &str, cwd: &str, name: &str, now_ms: u64, id: &str, event: &Event) {
        let session = self.sessions.entry(key.to_string()).or_default();
        session.agent = agent.to_string();
        session.cwd = cwd.to_string();
        session.name = name.to_string();
        session.last_event_ms = now_ms;
        if let Some(existing) = session.steps.iter_mut().find(|s| s.id == id) {
            existing.ok = event.ok;
            existing.added = event.added;
            existing.removed = event.removed;
            existing.detail = event.detail.clone();
        } else {
            session.steps.push_back(StoredStep {
                id: id.to_string(),
                ts_ms: event.ts_ms,
                tool: event.tool.clone(),
                kind: event.kind.to_string(),
                label: event.label.clone(),
                ok: event.ok,
                added: event.added,
                removed: event.removed,
                detail: event.detail.clone(),
                flagged: false,
                note: None,
            });
            while session.steps.len() > STEPS_MAX {
                session.steps.pop_front();
            }
        }
        while self.sessions.len() > SESSIONS_MAX {
            let Some(oldest) = self.sessions.iter().min_by_key(|(_, s)| s.last_event_ms).map(|(k, _)| k.clone()) else { break };
            self.sessions.remove(&oldest);
        }
    }

    /// Toggle a step's "needs review" flag (and optionally its note). `false` if the session or
    /// step is not known.
    pub fn flag(&mut self, key: &str, step_id: &str, flagged: bool, note: Option<String>) -> bool {
        let Some(session) = self.sessions.get_mut(key) else { return false };
        let Some(step) = session.steps.iter_mut().find(|s| s.id == step_id) else { return false };
        step.flagged = flagged;
        if note.is_some() {
            step.note = note;
        }
        true
    }

    pub fn is_flagged(&self, key: &str, step_id: &str) -> bool {
        self.sessions.get(key).and_then(|s| s.steps.iter().find(|s| s.id == step_id)).is_some_and(|s| s.flagged)
    }

    /// Every step of `key`, or only those matching `query` (case-insensitive substring over the
    /// tool name, label, and whatever text the step's `Detail` carries).
    pub fn filtered(&self, key: &str, query: Option<&str>) -> Vec<StoredStep> {
        let Some(session) = self.sessions.get(key) else { return Vec::new() };
        match query.map(str::trim).filter(|q| !q.is_empty()) {
            None => session.steps.iter().cloned().collect(),
            Some(q) => {
                let q = q.to_lowercase();
                session.steps.iter().filter(|s| matches_query(s, &q)).cloned().collect()
            }
        }
    }
}

/// Markdown export of a session's stored steps: one `###` section per step, diffs/output as
/// fenced code blocks, in the same bounded form the Live viewer already shows (see `detail.rs`'s
/// own truncation limits) — never a complete/git-applyable patch.
pub fn export_markdown(key: &str, session: &SessionHistory, steps: &[StoredStep]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# Sushi session export — {}\n\n", session.name));
    out.push_str(&format!("- Agent: {}\n- Directory: `{}`\n- Session: `{key}`\n", session.agent, session.cwd));
    if let (Some(first), Some(last)) = (steps.first(), steps.last()) {
        out.push_str(&format!("- {} step(s), {} to {}\n", steps.len(), fmt_ts(first.ts_ms), fmt_ts(last.ts_ms)));
    }
    out.push_str("\n_Content matches what the Live viewer showed: diffs and command output are clipped the same way, never a complete/applyable patch. See Sushi's README._\n");
    for (i, step) in steps.iter().enumerate() {
        let status = match step.ok {
            Some(true) => "✓",
            Some(false) => "✗",
            None => "…",
        };
        out.push_str(&format!("\n### {}. {status} {} — {}\n\n", i + 1, step.tool, step.label));
        if step.flagged {
            out.push_str("> **Flagged: needs review**");
            if let Some(n) = &step.note {
                out.push_str(&format!(" — {n}"));
            }
            out.push_str("\n\n");
        }
        match &step.detail {
            Some(Detail::Diff { file, lines, more, .. }) => {
                out.push_str(&format!("`{file}`\n\n```diff\n"));
                for l in lines {
                    let prefix = match l.kind {
                        "add" => "+",
                        "del" => "-",
                        _ => " ",
                    };
                    out.push_str(&format!("{prefix}{}\n", l.text));
                }
                out.push_str("```\n");
                if *more > 0 {
                    out.push_str(&format!("\n_{more} more changed line(s), not shown here._\n"));
                }
            }
            Some(Detail::Terminal { command, output }) => {
                out.push_str(&format!("```\n$ {command}\n"));
                for l in output {
                    out.push_str(l);
                    out.push('\n');
                }
                out.push_str("```\n");
            }
            Some(Detail::File { file, start, lines, .. }) => {
                out.push_str(&format!("`{file}` (from line {start})\n\n```\n"));
                for l in lines {
                    out.push_str(l);
                    out.push('\n');
                }
                out.push_str("```\n");
            }
            Some(Detail::Text { title, body }) => {
                if !title.is_empty() {
                    out.push_str(&format!("**{title}**\n\n"));
                }
                if !body.is_empty() {
                    out.push_str(&format!("{body}\n"));
                }
            }
            Some(Detail::Questions { .. }) | None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detail::DiffLine;

    fn ev(ts: u64, tool: &str, ok: Option<bool>) -> Event {
        Event {
            ts_ms: ts,
            tool_use_id: String::new(),
            tool: tool.to_string(),
            kind: "run",
            label: "cargo test".into(),
            added: 0,
            removed: 0,
            ok,
            detail: None,
            policy: None,
        }
    }

    #[test]
    fn record_pushes_a_new_step_and_updates_a_repeat_id_in_place() {
        let mut h = HistoryStore::default();
        h.record("claude:a", "claude", "/p", "proj", 1, "t1", &ev(1, "Bash", None));
        assert_eq!(h.sessions["claude:a"].steps.len(), 1);
        assert_eq!(h.sessions["claude:a"].steps[0].ok, None);

        h.record("claude:a", "claude", "/p", "proj", 2, "t1", &ev(1, "Bash", Some(true)));
        assert_eq!(h.sessions["claude:a"].steps.len(), 1, "same id updates in place, not a new entry");
        assert_eq!(h.sessions["claude:a"].steps[0].ok, Some(true));
        assert_eq!(h.sessions["claude:a"].last_event_ms, 2);
    }

    #[test]
    fn steps_are_capped_per_session_and_sessions_capped_overall() {
        let mut h = HistoryStore::default();
        for i in 0..(STEPS_MAX + 10) {
            h.record("claude:a", "claude", "/p", "proj", i as u64, &format!("t{i}"), &ev(i as u64, "Read", Some(true)));
        }
        assert_eq!(h.sessions["claude:a"].steps.len(), STEPS_MAX);
        assert_eq!(h.sessions["claude:a"].steps.front().unwrap().id, "t10", "the oldest 10 were evicted");

        for i in 0..(SESSIONS_MAX + 1) {
            h.record(&format!("claude:s{i}"), "claude", "/p", "proj", i as u64, "t0", &ev(i as u64, "Read", Some(true)));
        }
        assert_eq!(h.sessions.len(), SESSIONS_MAX, "the oldest session was evicted to make room");
        assert!(!h.sessions.contains_key("claude:s0"));
    }

    #[test]
    fn flag_toggles_a_known_step_and_reports_unknown_ones() {
        let mut h = HistoryStore::default();
        h.record("claude:a", "claude", "/p", "proj", 1, "t1", &ev(1, "Bash", Some(true)));
        assert!(h.flag("claude:a", "t1", true, Some("check this".into())));
        assert!(h.is_flagged("claude:a", "t1"));
        assert_eq!(h.sessions["claude:a"].steps[0].note.as_deref(), Some("check this"));
        assert!(!h.flag("claude:a", "nope", true, None));
        assert!(!h.flag("claude:nope", "t1", true, None));
    }

    #[test]
    fn filter_matches_tool_label_and_detail_text_case_insensitively() {
        let mut h = HistoryStore::default();
        let mut e = ev(1, "Bash", Some(true));
        e.detail = Some(Detail::Terminal { command: "cargo test".into(), output: vec!["FAILED unit::foo".into()] });
        h.record("claude:a", "claude", "/p", "proj", 1, "t1", &e);
        let mut e2 = ev(2, "Edit", Some(true));
        e2.label = "src/main.rs".into();
        e2.detail = Some(Detail::Diff { file: "src/main.rs".into(), lang: "rust", lines: vec![DiffLine { n: 1, kind: "add", text: "fn main() {}".into() }], more: 0 });
        h.record("claude:a", "claude", "/p", "proj", 2, "t2", &e2);

        assert_eq!(h.filtered("claude:a", None).len(), 2);
        assert_eq!(h.filtered("claude:a", Some("FAILED")).len(), 1);
        assert_eq!(h.filtered("claude:a", Some("main.rs")).len(), 1);
        assert_eq!(h.filtered("claude:a", Some("nothing matches")).len(), 0);
        assert_eq!(h.filtered("claude:nope", None).len(), 0);
    }

    #[test]
    fn load_missing_file_is_default() {
        assert!(HistoryStore::load(Path::new("/nonexistent/history.json")).sessions.is_empty());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("sushi-history-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history.json");
        let mut h = HistoryStore::default();
        let mut e = ev(1, "Bash", Some(true));
        e.detail = Some(Detail::Terminal { command: "ls".into(), output: vec!["a.txt".into()] });
        h.record("claude:a", "claude", "/p", "proj", 1, "t1", &e);
        h.flag("claude:a", "t1", true, Some("why".into()));
        h.save(&path);
        let loaded = HistoryStore::load(&path);
        assert!(loaded.sessions["claude:a"].steps[0].flagged);
        assert_eq!(loaded.sessions["claude:a"].steps[0].note.as_deref(), Some("why"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn export_includes_header_flag_and_diff_as_a_fenced_block() {
        let mut h = HistoryStore::default();
        let mut e = ev(1_790_812_800_000, "Edit", Some(true));
        e.label = "src/a.rs".into();
        e.detail = Some(Detail::Diff { file: "src/a.rs".into(), lang: "rust", lines: vec![DiffLine { n: 5, kind: "del", text: "old".into() }, DiffLine { n: 5, kind: "add", text: "new".into() }], more: 2 });
        h.record("claude:a", "claude", "/proj", "proj", 1_790_812_800_000, "t1", &e);
        h.flag("claude:a", "t1", true, Some("double check".into()));
        let steps = h.filtered("claude:a", None);
        let md = export_markdown("claude:a", &h.sessions["claude:a"], &steps);
        assert!(md.contains("# Sushi session export — proj"));
        assert!(md.contains("Flagged: needs review") && md.contains("double check"));
        assert!(md.contains("```diff") && md.contains("-old") && md.contains("+new"));
        assert!(md.contains("2 more changed line(s)"));
    }
}
