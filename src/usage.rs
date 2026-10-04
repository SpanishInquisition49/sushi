//! Token usage aggregated from Claude Code transcripts (`~/.claude/projects/*/<session>.jsonl`).
//!
//! Files are read incrementally (by byte offset). A streamed response is written
//! as several lines sharing one `requestId`, so usage is stored per request and
//! the last line wins instead of being summed.

use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Tokens {
    pub fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
    }
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

#[derive(Debug, Clone)]
struct Entry {
    /// UTC date, `YYYY-MM-DD`.
    day: String,
    model: String,
    tokens: Tokens,
}

/// Size of the prompt of the latest main-thread request (what fills the context window).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub tokens: u64,
    pub model: String,
}

#[derive(Debug, Default)]
struct FileUsage {
    offset: u64,
    requests: HashMap<String, Entry>,
    last_context: Option<Context>,
    /// Prompt size of each main-thread request, oldest first (one point per request).
    ctx_history: Vec<(String, u64)>,
}

const CTX_HISTORY_MAX: usize = 400;

#[derive(Debug, Default)]
pub struct UsageStore {
    files: HashMap<PathBuf, FileUsage>,
}

#[derive(Debug, Default, Serialize)]
pub struct UsageSummary {
    pub today: Tokens,
    pub total: Tokens,
    pub by_day: BTreeMap<String, Tokens>,
    pub by_model: BTreeMap<String, Tokens>,
}

struct Parsed {
    key: String,
    entry: Entry,
    sidechain: bool,
}

fn parse_line(line: &str) -> Option<Parsed> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let msg = v.get("message")?;
    let usage = msg.get("usage")?;
    let n = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
    let key = v
        .get("requestId")
        .or_else(|| v.get("uuid"))?
        .as_str()?
        .to_string();
    let day = v.get("timestamp")?.as_str()?.get(..10)?.to_string();
    Some(Parsed {
        key,
        sidechain: v.get("isSidechain").and_then(Value::as_bool).unwrap_or(false),
        entry: Entry {
            day,
            model: msg.get("model").and_then(Value::as_str).unwrap_or("unknown").to_string(),
            tokens: Tokens {
                input: n("input_tokens"),
                output: n("output_tokens"),
                cache_read: n("cache_read_input_tokens"),
                cache_write: n("cache_creation_input_tokens"),
            },
        },
    })
}

impl UsageStore {
    /// Read whatever was appended to `path` since the last call.
    pub fn refresh(&mut self, path: &Path) {
        let fu = self.files.entry(path.to_path_buf()).or_default();
        let Ok(mut f) = File::open(path) else { return };
        let Ok(len) = f.metadata().map(|m| m.len()) else { return };
        if len < fu.offset {
            // Truncated or replaced: start over.
            *fu = FileUsage::default();
        }
        if len == fu.offset || f.seek(SeekFrom::Start(fu.offset)).is_err() {
            return;
        }
        let mut buf = Vec::new();
        if f.read_to_end(&mut buf).is_err() {
            return;
        }
        // Only consume complete lines; a partial tail is re-read next time.
        let Some(end) = buf.iter().rposition(|&b| b == b'\n') else { return };
        for line in buf[..end].split(|&b| b == b'\n') {
            if let Some(p) = std::str::from_utf8(line).ok().and_then(parse_line) {
                if !p.sidechain {
                    let t = &p.entry.tokens;
                    let tokens = t.input + t.cache_read + t.cache_write;
                    fu.last_context = Some(Context { tokens, model: p.entry.model.clone() });
                    // A streamed request is several lines: update its point instead of adding more.
                    match fu.ctx_history.last_mut() {
                        Some((k, v)) if *k == p.key => *v = tokens,
                        _ => {
                            fu.ctx_history.push((p.key.clone(), tokens));
                            if fu.ctx_history.len() > CTX_HISTORY_MAX {
                                fu.ctx_history.remove(0);
                            }
                        }
                    }
                }
                fu.requests.insert(p.key, p.entry);
            }
        }
        fu.offset += end as u64 + 1;
    }

    /// Totals for a single transcript file.
    pub fn file_total(&self, path: &Path) -> Tokens {
        let mut t = Tokens::default();
        if let Some(fu) = self.files.get(path) {
            fu.requests.values().for_each(|e| t.add(&e.tokens));
        }
        t
    }

    /// Context size of the latest main-thread request in this transcript.
    pub fn context(&self, path: &Path) -> Option<&Context> {
        self.files.get(path)?.last_context.as_ref()
    }

    /// The context size over the life of the session, at most `max_points` evenly spaced points.
    pub fn context_history(&self, path: &Path, max_points: usize) -> Vec<u64> {
        let Some(fu) = self.files.get(path) else { return Vec::new() };
        let all: Vec<u64> = fu.ctx_history.iter().map(|(_, t)| *t).collect();
        if all.len() <= max_points || max_points < 2 {
            return all;
        }
        (0..max_points).map(|i| all[i * (all.len() - 1) / (max_points - 1)]).collect()
    }

    pub fn summary(&self, today: &str) -> UsageSummary {
        let mut s = UsageSummary::default();
        for e in self.files.values().flat_map(|fu| fu.requests.values()) {
            s.total.add(&e.tokens);
            s.by_day.entry(e.day.clone()).or_default().add(&e.tokens);
            s.by_model.entry(e.model.clone()).or_default().add(&e.tokens);
            if e.day == today {
                s.today.add(&e.tokens);
            }
        }
        s
    }
}

/// USD per million tokens, for a rough cost estimate — always labeled "estimated" in the UI, never
/// a billed amount.
#[derive(Debug, Clone, Copy, Default, Serialize, serde::Deserialize)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

impl ModelPrice {
    fn cost(&self, t: &Tokens) -> f64 {
        (t.input as f64 * self.input + t.output as f64 * self.output + t.cache_read as f64 * self.cache_read + t.cache_write as f64 * self.cache_write)
            / 1_000_000.0
    }
}

/// USD / 1M tokens, Anthropic's published list prices at the time of writing. Override a model in
/// `~/.config/sushi/config.json`'s `model_prices` if these drift.
fn default_model_prices() -> HashMap<String, ModelPrice> {
    [
        ("claude-opus", ModelPrice { input: 15.0, output: 75.0, cache_read: 1.5, cache_write: 18.75 }),
        ("claude-sonnet", ModelPrice { input: 3.0, output: 15.0, cache_read: 0.3, cache_write: 3.75 }),
        ("claude-haiku", ModelPrice { input: 0.8, output: 4.0, cache_read: 0.08, cache_write: 1.0 }),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

impl UsageSummary {
    /// Rough, clearly-labeled cost estimate (not a billed amount): each model in `by_model` is
    /// matched against `prices` by prefix (`"claude-sonnet-5"` matches a `"claude-sonnet"` entry so
    /// dated/versioned ids still work), 0 for a model with no matching price.
    pub fn estimated_cost(&self, prices: &HashMap<String, ModelPrice>) -> f64 {
        self.by_model
            .iter()
            .map(|(model, tokens)| prices.iter().find(|(prefix, _)| model.starts_with(prefix.as_str())).map(|(_, p)| p.cost(tokens)).unwrap_or(0.0))
            .sum()
    }
}

/// What a tool call runs, when an external hook fires (see `Hooks` below): `cmd` with `SUSHI_*` env
/// vars, and/or `url` posted a small JSON body — both fire-and-forget, never blocking the daemon.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct HookAction {
    pub cmd: String,
    pub url: String,
}

/// External hooks run on session/turn transitions (see `main.rs`'s `fire_hook`), e.g. to trigger a
/// build, a staging deploy, or update an internal dashboard.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct Hooks {
    pub on_session_start: HookAction,
    pub on_session_end: HookAction,
    pub on_waiting: HookAction,
    pub on_turn_end: HookAction,
}

/// Thresholds (percent) that make the pet raise a one-shot `budget_alert` event. `plan_percent`
/// applies to Claude's 5-hour and weekly plan windows; `daily_tokens` / `daily_cost_usd` (0 =
/// disabled) are a budget across every tracked agent's usage today, checked against `daily_percent`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct BudgetAlerts {
    pub plan_percent: Vec<f64>,
    pub daily_tokens: u64,
    pub daily_cost_usd: f64,
    pub daily_percent: Vec<f64>,
}

impl Default for BudgetAlerts {
    fn default() -> Self {
        BudgetAlerts { plan_percent: vec![80.0, 95.0], daily_tokens: 0, daily_cost_usd: 0.0, daily_percent: vec![80.0, 95.0] }
    }
}

/// Patterns merged into Claude Code's own `permissions.ask` / `permissions.deny` (its native
/// syntax, e.g. `Bash(rm -rf*)`) by `sushi install --agent claude --write` (see
/// `install::merge_permissions`). This is the only agent Sushi can make genuinely ask or refuse a
/// tool call the agent would otherwise have auto-approved; see `policy.rs` for why.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct ClaudePermissions {
    pub ask: Vec<String>,
    pub deny: Vec<String>,
}

/// Optional `~/.config/sushi/config.json`:
/// `{"context_window": 200000, "context_windows": {"<model id>": 1000000}, "show_code": true,
///   "chat_agent": "claude", "chat_models": {"copilot": "auto"}, "agent_paths": {"pi": "/opt/pi"},
///   "transcribe_model_path": "/opt/whisper/ggml-base.en.bin", "whisper_path": "whisper-cli"}`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct Config {
    pub context_window: u64,
    pub context_windows: HashMap<String, u64>,
    /// Publish code (diffs, command output, file excerpts) for the live viewer.
    pub show_code: bool,
    /// Model alias for the built-in chat with Claude Code, and the `claude` executable to run.
    pub chat_model: String,
    pub claude_path: String,
    /// The agent the built-in chat talks to (`claude`, `copilot`, `pi`, `codex`).
    pub chat_agent: String,
    /// Chat model per agent id, for the agents other than Claude Code.
    pub chat_models: HashMap<String, String>,
    /// Executable per agent id when it is not on the `PATH` under its own name.
    pub agent_paths: HashMap<String, String>,
    /// Path to a whisper.cpp GGML/GGUF model (e.g. `ggml-base.en.bin`). Empty disables
    /// transcription: a fed audio file is then refused like any other binary file.
    pub transcribe_model_path: String,
    /// The `whisper-cli` executable, when it is not on the `PATH` under its own name.
    pub whisper_path: String,
    /// USD / 1M tokens per model, for the estimated cost shown in the Usage tab.
    pub model_prices: HashMap<String, ModelPrice>,
    pub budget_alerts: BudgetAlerts,
    pub hooks: Hooks,
    pub claude_permissions: ClaudePermissions,
    /// Cross-agent visibility rules (see `policy.rs`): flag a matching tool call in the live
    /// viewer, independent of whether it can actually be enforced for that agent.
    pub policies: Vec<crate::policy::PolicyRule>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            context_window: 200_000,
            context_windows: HashMap::new(),
            show_code: true,
            chat_model: "sonnet".into(),
            claude_path: "claude".into(),
            chat_agent: "claude".into(),
            chat_models: HashMap::new(),
            agent_paths: HashMap::new(),
            transcribe_model_path: String::new(),
            whisper_path: "whisper-cli".into(),
            model_prices: default_model_prices(),
            budget_alerts: BudgetAlerts::default(),
            hooks: Hooks::default(),
            claude_permissions: ClaudePermissions::default(),
            policies: Vec::new(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Config {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// The executable that runs `agent` headless for the chat.
    pub fn program_for(&self, agent: crate::agent::Agent) -> String {
        match agent {
            crate::agent::Agent::Claude => self.claude_path.clone(),
            other => self.agent_paths.get(other.id()).cloned().unwrap_or_else(|| other.id().to_string()),
        }
    }

    /// Transcripts do not record the window size, so it is an estimate: a model
    /// override wins, then the default, and a prompt larger than the window
    /// means the session is on the 1M window.
    pub fn window_for(&self, model: &str, tokens: u64) -> u64 {
        let w = self.context_windows.get(model).copied().unwrap_or(self.context_window);
        if tokens > w { w.max(1_000_000) } else { w }
    }
}

/// Parse `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)` into unix milliseconds.
pub fn parse_iso_ms(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    let num = |a: usize, n: usize| -> Option<i64> { s.get(a..a + n)?.parse().ok() };
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    let (y, mo, d, h, mi, sec) = (num(0, 4)?, num(5, 2)?, num(8, 2)?, num(11, 2)?, num(14, 2)?, num(17, 2)?);
    let mut i = 19;
    let mut millis = 0i64;
    if b.get(i) == Some(&b'.') {
        let digits: String = s[i + 1..].chars().take_while(|c| c.is_ascii_digit()).collect();
        millis = format!("{:0<3}", &digits[..digits.len().min(3)]).parse().ok()?;
        i += 1 + digits.len();
    }
    let offset_min = match b.get(i) {
        None | Some(b'Z') | Some(b'z') => 0,
        Some(&sign @ (b'+' | b'-')) => {
            let v = num(i + 1, 2)? * 60 + num(i + 4, 2)?;
            if sign == b'+' { v } else { -v }
        }
        _ => return None,
    };
    // days-from-civil (Howard Hinnant)
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2.rem_euclid(400);
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec - offset_min * 60;
    u64::try_from(secs * 1000 + millis).ok()
}

/// UTC date (`YYYY-MM-DD`) for a unix timestamp in milliseconds.
pub fn utc_day(ms: u64) -> String {
    // Civil-from-days (Howard Hinnant).
    let z = (ms / 86_400_000) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn line(req: &str, out: u64, ts: &str) -> String {
        format!(
            r#"{{"type":"assistant","requestId":"{req}","timestamp":"{ts}","message":{{"model":"m1","usage":{{"input_tokens":10,"output_tokens":{out},"cache_read_input_tokens":5,"cache_creation_input_tokens":2}}}}}}"#
        )
    }

    #[test]
    fn dedupes_by_request_and_reads_incrementally() {
        let dir = std::env::temp_dir().join(format!("sushi-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.jsonl");
        let mut f = File::create(&path).unwrap();
        // Same request streamed twice: last line wins.
        writeln!(f, "{}", line("r1", 1, "2026-10-01T10:00:00Z")).unwrap();
        writeln!(f, "{}", line("r1", 50, "2026-10-01T10:00:01Z")).unwrap();
        writeln!(f, r#"{{"type":"user","message":{{}}}}"#).unwrap();
        // Partial trailing line must not be consumed yet.
        write!(f, "{}", line("r2", 7, "2026-10-02T10:00:00Z")).unwrap();
        drop(f);

        let mut store = UsageStore::default();
        store.refresh(&path);
        assert_eq!(store.file_total(&path).output, 50);

        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f).unwrap();
        drop(f);
        store.refresh(&path);
        let t = store.file_total(&path);
        assert_eq!(t.output, 57);
        assert_eq!(t.input, 20);

        let s = store.summary("2026-10-02");
        assert_eq!(s.today.output, 7);
        assert_eq!(s.by_day.len(), 2);
        assert_eq!(s.by_model["m1"].output, 57);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tracks_context_of_main_thread_only() {
        let dir = std::env::temp_dir().join(format!("sushi-ctx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.jsonl");
        let mut f = File::create(&path).unwrap();
        writeln!(f, "{}", line("r1", 1, "2026-10-01T10:00:00Z")).unwrap(); // 10 + 5 + 2 = 17
        // A subagent request must not change the context.
        writeln!(
            f,
            r#"{{"type":"assistant","isSidechain":true,"requestId":"s1","timestamp":"2026-10-01T10:00:01Z","message":{{"model":"m2","usage":{{"input_tokens":999}}}}}}"#
        )
        .unwrap();
        drop(f);
        let mut store = UsageStore::default();
        store.refresh(&path);
        let c = store.context(&path).unwrap();
        assert_eq!((c.tokens, c.model.as_str()), (17, "m1"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn context_history_is_one_point_per_request_and_downsampled() {
        let dir = std::env::temp_dir().join(format!("sushi-hist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("h.jsonl");
        let mut f = File::create(&path).unwrap();
        for i in 0..100u64 {
            // Each request appears twice (streamed); the second line is the final one.
            for out in [1, 2] {
                writeln!(f, r#"{{"type":"assistant","requestId":"r{i}","timestamp":"2026-10-01T10:00:00Z","message":{{"model":"m","usage":{{"input_tokens":{},"output_tokens":{out}}}}}}}"#, i * 10).unwrap();
            }
        }
        drop(f);
        let mut store = UsageStore::default();
        store.refresh(&path);
        let all = store.context_history(&path, 1000);
        assert_eq!(all.len(), 100);
        assert_eq!((all[0], all[99]), (0, 990));
        let small = store.context_history(&path, 10);
        assert_eq!(small.len(), 10);
        assert_eq!((small[0], small[9]), (0, 990));
        assert!(small.windows(2).all(|w| w[0] <= w[1]));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn estimated_cost_matches_models_by_prefix() {
        let mut s = UsageSummary::default();
        s.by_model.insert("claude-sonnet-5".into(), Tokens { input: 1_000_000, output: 1_000_000, cache_read: 0, cache_write: 0 });
        s.by_model.insert("some-other-model".into(), Tokens { input: 1_000_000, ..Default::default() });
        let prices = default_model_prices();
        // 1M input @ $3/1M + 1M output @ $15/1M = $18; the unknown model contributes 0.
        assert!((s.estimated_cost(&prices) - 18.0).abs() < 1e-9);
    }

    #[test]
    fn context_window_estimate() {
        let mut cfg = Config::default();
        assert_eq!(cfg.window_for("any", 50_000), 200_000);
        assert_eq!(cfg.window_for("any", 250_000), 1_000_000); // must be the 1M window
        cfg.context_windows.insert("big".into(), 1_000_000);
        assert_eq!(cfg.window_for("big", 10), 1_000_000);
    }

    #[test]
    fn parses_iso_timestamps() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_ms("2026-10-01T00:00:00.500Z"), Some(1_790_812_800_500));
        assert_eq!(parse_iso_ms("2026-10-01T02:00:00+02:00"), Some(1_790_812_800_000));
        assert_eq!(parse_iso_ms("2026-10-01T00:00:00.123456+00:00"), Some(1_790_812_800_123));
        assert_eq!(parse_iso_ms("nope"), None);
    }

    #[test]
    fn utc_day_known_dates() {
        assert_eq!(utc_day(0), "1970-01-01");
        assert_eq!(utc_day(1_790_834_684_600), "2026-10-01");
        assert_eq!(utc_day(951_782_400_000), "2000-02-29");
    }
}
