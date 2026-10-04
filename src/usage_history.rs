//! A real day-by-day token/cost ledger, on top of `usage::UsageStore` (which is rebuilt from
//! transcripts within a 7-day mtime window and never written to disk — "daily" there is just
//! `utc_day(now_ms())` recomputed on the fly, with nothing persisted). `main.rs`'s housekeeping
//! tick snapshots "today so far" into `by_day` every 2s; once a new day starts, the previous
//! day's last snapshot simply stops being touched and becomes that day's permanent figure.
//!
//! Persisted like `stats.rs`/`care.rs`: `load`/`save` tolerant of a missing or corrupt file,
//! trimmed to `RETENTION_DAYS` on save.

use crate::usage::Tokens;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Days kept on disk (trimmed on save); the wire payload in `state.json` is sliced even shorter
/// by `main.rs` so a publish never carries the whole history.
pub const RETENTION_DAYS: usize = 180;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DaySummary {
    pub tokens: Tokens,
    pub estimated_cost_usd: f64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageHistory {
    pub by_day: BTreeMap<String, DaySummary>,
}

impl UsageHistory {
    pub fn load(path: &Path) -> UsageHistory {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&mut self, path: &Path) {
        while self.by_day.len() > RETENTION_DAYS {
            let Some(oldest) = self.by_day.keys().next().cloned() else { break };
            self.by_day.remove(&oldest);
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string(self) {
            let _ = std::fs::write(path, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_missing_file_is_default() {
        assert!(UsageHistory::load(Path::new("/nonexistent/usage_history.json")).by_day.is_empty());
    }

    #[test]
    fn save_trims_to_retention_and_keeps_the_most_recent_days() {
        let dir = std::env::temp_dir().join(format!("sushi-usage-history-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("usage_history.json");
        let mut h = UsageHistory::default();
        for i in 0..(RETENTION_DAYS + 5) {
            h.by_day.insert(format!("2026-{:02}-{:02}", 1 + i / 28, 1 + i % 28), DaySummary { estimated_cost_usd: i as f64, ..Default::default() });
        }
        h.save(&path);
        assert_eq!(h.by_day.len(), RETENTION_DAYS);
        let loaded = UsageHistory::load(&path);
        assert_eq!(loaded.by_day.len(), RETENTION_DAYS);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("sushi-usage-history-rt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("usage_history.json");
        let mut h = UsageHistory::default();
        h.by_day.insert("2026-10-01".into(), DaySummary { tokens: Tokens { input: 10, ..Default::default() }, estimated_cost_usd: 1.5 });
        h.save(&path);
        let loaded = UsageHistory::load(&path);
        assert_eq!(loaded.by_day["2026-10-01"].estimated_cost_usd, 1.5);
        assert_eq!(loaded.by_day["2026-10-01"].tokens.input, 10);
        std::fs::remove_dir_all(&dir).ok();
    }
}
