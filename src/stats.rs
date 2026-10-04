//! Light, local-only stats about the pet's long-term use: a day streak, total sessions and total
//! tool calls watched, and the milestones already celebrated once. Persisted next to `chat.json`
//! (see `chat::cache_dir`) so the app and the Noctalia plugin see the same numbers. Days are UTC,
//! the same boundary `usage.rs` already uses for daily totals.

use crate::usage::{parse_iso_ms, utc_day};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

/// Streak milestones (consecutive days with at least one session) and tool-call milestones,
/// each celebrated once.
pub const STREAK_MILESTONES: [u64; 5] = [3, 7, 14, 30, 100];
pub const TOOL_CALL_MILESTONES: [u64; 5] = [100, 500, 1_000, 5_000, 10_000];

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    /// UTC days (`YYYY-MM-DD`) on which at least one session was active.
    pub days: BTreeSet<String>,
    pub total_sessions: u64,
    pub total_tool_calls: u64,
    /// Milestones already celebrated, so they fire once (`streak:7`, `tool_calls:500`).
    pub milestones_unlocked: BTreeSet<String>,
}

/// `YYYY-MM-DD` minus one day.
fn prev_day(day: &str) -> Option<String> {
    let ms = parse_iso_ms(&format!("{day}T00:00:00Z"))?;
    Some(utc_day(ms.checked_sub(86_400_000)?))
}

impl Stats {
    pub fn load(path: &Path) -> Stats {
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

    pub fn mark_day(&mut self, day: &str) {
        self.days.insert(day.to_string());
    }

    /// Consecutive UTC days up to today, or ending yesterday if nothing happened yet today (so the
    /// streak does not drop to zero right at midnight before the first event of the new day).
    pub fn streak(&self, today: &str) -> u64 {
        let mut day = if self.days.contains(today) { today.to_string() } else { prev_day(today).unwrap_or_default() };
        if day.is_empty() || !self.days.contains(&day) {
            return 0;
        }
        let mut n = 0u64;
        loop {
            if !self.days.contains(&day) {
                break;
            }
            n += 1;
            match prev_day(&day) {
                Some(p) => day = p,
                None => break,
            }
        }
        n
    }

    /// The highest milestone ever unlocked under `prefix` (`"streak:"` or `"tool_calls:"`), 0 if
    /// none — an earned badge, so it stays even if e.g. the streak itself later resets to 0.
    pub fn highest_unlocked(&self, prefix: &str) -> u64 {
        self.milestones_unlocked.iter().filter_map(|k| k.strip_prefix(prefix).and_then(|n| n.parse::<u64>().ok())).max().unwrap_or(0)
    }

    /// Newly crossed milestones (`("streak", 7)`, `("tool_calls", 500)`), marking them unlocked so
    /// they are not returned again.
    pub fn check_milestones(&mut self, today: &str) -> Vec<(&'static str, u64)> {
        let mut out = Vec::new();
        let streak = self.streak(today);
        for &m in &STREAK_MILESTONES {
            if streak >= m && self.milestones_unlocked.insert(format!("streak:{m}")) {
                out.push(("streak", m));
            }
        }
        for &m in &TOOL_CALL_MILESTONES {
            if self.total_tool_calls >= m && self.milestones_unlocked.insert(format!("tool_calls:{m}")) {
                out.push(("tool_calls", m));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streak_counts_consecutive_days_and_tolerates_todays_gap() {
        let mut s = Stats::default();
        for d in ["2026-09-28", "2026-09-29", "2026-09-30", "2026-10-01"] {
            s.mark_day(d);
        }
        assert_eq!(s.streak("2026-10-01"), 4);
        // Nothing happened yet today: still counts through yesterday.
        assert_eq!(s.streak("2026-10-02"), 4);
        // A gap breaks it.
        assert_eq!(s.streak("2026-10-03"), 0);
    }

    #[test]
    fn empty_stats_have_no_streak() {
        assert_eq!(Stats::default().streak("2026-10-01"), 0);
    }

    #[test]
    fn milestones_fire_once_each() {
        let mut s = Stats::default();
        for d in ["2026-09-25", "2026-09-26", "2026-09-27"] {
            s.mark_day(d);
        }
        s.total_tool_calls = 100;
        let first = s.check_milestones("2026-09-27");
        assert!(first.contains(&("streak", 3)));
        assert!(first.contains(&("tool_calls", 100)));
        assert!(s.check_milestones("2026-09-27").is_empty(), "already unlocked");
    }

    #[test]
    fn highest_unlocked_is_a_badge_that_survives_a_broken_streak() {
        let mut s = Stats::default();
        for d in ["2026-09-25", "2026-09-26", "2026-09-27"] {
            s.mark_day(d);
        }
        s.total_tool_calls = 500;
        s.check_milestones("2026-09-27");
        assert_eq!((s.highest_unlocked("streak:"), s.highest_unlocked("tool_calls:")), (3, 500));
        // The streak resets (a day was missed), but the badge already earned stays.
        assert_eq!(s.streak("2026-10-05"), 0);
        assert_eq!(s.highest_unlocked("streak:"), 3);
        assert_eq!(Stats::default().highest_unlocked("streak:"), 0);
    }

    #[test]
    fn load_missing_file_is_default() {
        let s = Stats::load(Path::new("/nonexistent/stats.json"));
        assert_eq!(s.total_sessions, 0);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("sushi-stats-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stats.json");
        let mut s = Stats::default();
        s.mark_day("2026-10-01");
        s.total_sessions = 5;
        s.save(&path);
        let loaded = Stats::load(&path);
        assert_eq!((loaded.total_sessions, loaded.days.len()), (5, 1));
        std::fs::remove_dir_all(&dir).ok();
    }
}
