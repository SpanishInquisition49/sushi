use sushi::agent::{Agent, AgentEvent, EventKind, Role};
use sushi::care::{self, Care};
use sushi::chat::{self, Chat};
use sushi::install;
use sushi::detail::{self, Detail};
use sushi::history::HistoryStore;
use sushi::limits::{FetchError, Limits};
use sushi::paths::{claude_dir, codex_dir, config_path, copilot_dir, gemini_dir, socket_path, state_dir, state_path};
use sushi::protocol::{Decision, Reply, Request};
use sushi::sessions::{Session, Sessions, Status, session_key};
use sushi::stats::Stats;
use sushi::usage::{BudgetAlerts, Config, HookAction, UsageStore, utc_day};
use sushi::usage_history::{DaySummary, UsageHistory};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use sushi::ipc::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long the daemon keeps a permission request open (the hook gives up first).
const PERMISSION_TTL: Duration = Duration::from_secs(600);
/// `state.json` is rewritten at least this often even if nothing changed.
const HEARTBEAT: Duration = Duration::from_secs(5);
/// How often plan limits are refreshed (and after an error, unless rate limited).
const LIMITS_REFRESH: Duration = Duration::from_secs(180);
const LIMITS_RETRY: Duration = Duration::from_secs(120);
const LIMITS_BACKOFF: Duration = Duration::from_secs(600);
/// Transcripts older than this are ignored by the startup usage scan.
const SCAN_WINDOW: Duration = Duration::from_secs(7 * 24 * 3600);

/// What a pending request needs from you.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// An ordinary permission: Allow or Deny.
    Permission,
    /// The agent asks a question: answer it (the answers go back through the hook).
    Question,
    /// A plan (Claude Code's `ExitPlanMode`): approve it (maybe with auto-accepted edits) or keep planning.
    Plan,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Permission => "permission",
            Kind::Question => "question",
            Kind::Plan => "plan",
        }
    }
}

/// The hook's final word: a decision, the input to use instead (the answers of a question, the
/// approved plan) and the permission mode to switch to.
type Verdict = (Decision, Option<Value>, Option<&'static str>);

struct Pending {
    id: u64,
    agent: Agent,
    session_id: String,
    tool: String,
    /// Raw tool name (`Bash`, `edit`, ...) and what it is about to do, for the approval card.
    tool_name: String,
    detail: Option<Detail>,
    kind: Kind,
    /// The original tool input, kept for questions (the answers are added to it) and plans.
    tool_input: Value,
    created_ms: u64,
    /// `None` once nothing waits for an answer: a plan whose hook gave up stays shown until it is
    /// dealt with in the terminal.
    tx: Option<Sender<Verdict>>,
}

/// Plans left to the terminal are dropped after this long (no hook is waiting to give up).
const PLAN_TTL_MS: u64 = 30 * 60 * 1000;

/// One-shot events the pet should notice once (not a continuous state like `status`):
/// `budget_alert`, `milestone`, `grew_up`. Capped so `state.json` cannot grow without bound.
const EVENTS_MAX: usize = 20;

#[derive(Default)]
struct State {
    sessions: Sessions,
    pending: Vec<Pending>,
    next_id: u64,
    usage: UsageStore,
    last_published: String,
    last_write_ms: u64,
    config: Config,
    /// Plan usage per agent (see `limits::`): only the agents with a known endpoint
    /// (`Agent::capabilities().limits`) ever get an entry.
    limits: HashMap<Agent, Limits>,
    limits_error: HashMap<Agent, String>,
    chat: Chat,
    /// Connections that asked to watch the state: each change is written to them at once.
    watchers: Vec<UnixStream>,
    stats: Stats,
    /// Set after a change to `stats`, so it is saved to disk at most once per housekeeping tick.
    stats_dirty: bool,
    care: Care,
    /// Set after a change to `care` (a care action, or decay/currency in `housekeeping`), so it
    /// is saved to disk at most once per housekeeping tick.
    care_dirty: bool,
    /// Durable step log beyond what `Activity` keeps live (see `sushi::history`): backs the Live
    /// tab's search/filter and export, and manual step annotations.
    history: HistoryStore,
    history_dirty: bool,
    /// Day-by-day token/cost ledger (see `sushi::usage_history`), unlike `usage` which is
    /// rebuilt from transcripts and never persisted.
    usage_history: UsageHistory,
    usage_history_dirty: bool,
    /// Recent one-shot events (see `EVENTS_MAX`), newest last.
    events: Vec<Value>,
    next_event_id: u64,
    /// Budget thresholds already raised, so each one fires only once (see `check_budget_alerts`).
    alerted: HashSet<String>,
}

type Shared = Arc<Mutex<State>>;

fn lock(s: &Shared) -> MutexGuard<'_, State> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl State {
    fn snapshot(&self) -> Value {
        let sessions: Vec<Value> = self
            .sessions
            .map
            .values()
            .map(|s| {
                let mut v = serde_json::to_value(s).unwrap_or(Value::Null);
                // Token usage and context come from transcripts, which only some agents have.
                if let Some(path) = s.transcript.as_deref().filter(|_| s.agent.capabilities().context) {
                    v["tokens"] = serde_json::to_value(self.usage.file_total(path)).unwrap_or(Value::Null);
                    if let Some(c) = self.usage.context(path) {
                        let window = self.config.window_for(&c.model, c.tokens);
                        let pct = |t: u64| (t as f64 / window as f64 * 100.0).min(100.0);
                        let history: Vec<f64> = self
                            .usage
                            .context_history(path, 40)
                            .into_iter()
                            .map(|t| (pct(t) * 10.0).round() / 10.0)
                            .collect();
                        v["context"] = json!({
                            "tokens": c.tokens, "window": window, "model": c.model,
                            "percent": pct(c.tokens), "history": history,
                        });
                    }
                }
                let mut activity = s.activity.to_json();
                // The durable flag (see `sushi::history`) shows up on the live rail immediately,
                // not only inside a fetched/filtered `History` request.
                if let Some(recent) = activity.get_mut("recent").and_then(Value::as_array_mut) {
                    for step in recent.iter_mut() {
                        let id = step.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                        step["flagged"] = json!(self.history.is_flagged(&s.id, &id));
                    }
                }
                v["activity"] = activity;
                v
            })
            .collect();
        let pending: Vec<Value> = self
            .pending
            .iter()
            .map(|p| {
                let name = self.sessions.map.get(&p.session_id).map(|s| s.name.as_str()).unwrap_or("");
                json!({
                    "id": p.id, "agent": p.agent, "session_id": p.session_id, "session_name": name,
                    "tool": p.tool, "tool_name": p.tool_name, "detail": p.detail, "kind": p.kind.as_str(),
                    "created_ms": p.created_ms, "answerable": p.tx.is_some(),
                })
            })
            .collect();
        let agents: serde_json::Map<String, Value> = Agent::ALL
            .into_iter()
            .map(|a| {
                let mut v = json!({ "label": a.label(), "capabilities": a.capabilities() });
                // Only published once there is something to say: data, or a real error. An agent
                // that was simply never logged into (`FetchError::NotConfigured`, dropped below
                // in `limits_loop`) never gets a `limits` key at all, so it is never shown as
                // broken to someone who doesn't even use it.
                if self.limits.contains_key(&a) || self.limits_error.contains_key(&a) {
                    v["limits"] = json!({ "data": self.limits.get(&a), "error": self.limits_error.get(&a) });
                }
                (a.id().to_string(), v)
            })
            .collect();
        // Token usage, comparative across agents: only Claude's transcripts are ever read today (see
        // `agent::Capabilities::context`), so every other agent is honestly "not available yet"
        // rather than fabricated.
        let today = utc_day(now_ms());
        let claude_summary = self.usage.summary(&today);
        let usage: serde_json::Map<String, Value> = Agent::ALL
            .into_iter()
            .map(|a| {
                let v = if a.capabilities().context {
                    let mut sv = serde_json::to_value(&claude_summary).unwrap_or(Value::Null);
                    sv["estimated_cost_usd"] = json!(claude_summary.estimated_cost(&self.config.model_prices));
                    sv["available"] = json!(true);
                    sv
                } else {
                    json!({ "available": false })
                };
                (a.id().to_string(), v)
            })
            .collect();
        // The last 30 days only: the full retained history (`usage_history::RETENTION_DAYS`)
        // stays on disk, a publish never carries all of it.
        let recent_history: std::collections::BTreeMap<&String, &DaySummary> = self.usage_history.by_day.iter().rev().take(30).collect();
        let cfg = &self.config.budget_alerts;
        let global_cost_today = self.usage_history.by_day.get(&today).map(|d| d.estimated_cost_usd).unwrap_or(0.0);
        let budgets_by_cwd: serde_json::Map<String, Value> = self
            .config
            .budget_alerts_by_cwd
            .iter()
            .map(|(cwd, rules)| {
                let tokens_today = self.usage.summary_for_cwd(cwd, &today).today.total();
                let cost_today = self.usage.day_cost(&today, Some(cwd), &self.config.model_prices);
                (
                    cwd.clone(),
                    json!({
                        "daily_tokens": rules.daily_tokens, "daily_cost_usd": rules.daily_cost_usd,
                        "tokens_today": tokens_today, "cost_today_usd": cost_today,
                    }),
                )
            })
            .collect();
        json!({
            "version": 2,
            "agents": agents,
            "sessions": sessions,
            "pending": pending,
            "usage": usage,
            "usage_history": { "by_day": recent_history },
            "budgets": {
                "global": {
                    "daily_tokens": cfg.daily_tokens, "daily_cost_usd": cfg.daily_cost_usd,
                    "tokens_today": claude_summary.today.total(), "cost_today_usd": global_cost_today,
                },
                "by_cwd": budgets_by_cwd,
            },
            "stats": {
                "streak_days": self.stats.streak(&today),
                "total_sessions": self.stats.total_sessions,
                "total_tool_calls": self.stats.total_tool_calls,
                // The highest milestone ever reached under each track: an earned badge (see
                // `Stats::highest_unlocked`), not a live gauge — it survives e.g. a broken streak.
                "streak_badge": self.stats.highest_unlocked("streak:"),
                "steps_badge": self.stats.highest_unlocked("tool_calls:"),
            },
            "care": {
                "hunger": self.care.hunger, "energy": self.care.energy, "affection": self.care.affection,
                "growth_tier": self.care.growth_tier(), "xp": self.care.xp, "currency": self.care.currency,
                "owned": self.care.owned, "equipped": self.care.equipped,
                // When each action next becomes available again (0 = available now), so the UI can
                // grey out a button and count down instead of letting a spammed click do nothing
                // silently (see `care::CARE_COOLDOWN_MS` / `PLAY_COOLDOWN_MS`).
                "next_feed_ms": self.care.last_feed_ms.saturating_add(care::CARE_COOLDOWN_MS),
                "next_pet_ms": self.care.last_pet_ms.saturating_add(care::CARE_COOLDOWN_MS),
                "next_nap_ms": self.care.last_nap_ms.saturating_add(care::CARE_COOLDOWN_MS),
                "next_play_ms": self.care.last_play_ms.saturating_add(care::PLAY_COOLDOWN_MS),
            },
            "events": self.events,
            "chat": self.chat.to_json(),
        })
    }

    /// Append a one-shot event (see `EVENTS_MAX`): `kind` plus whatever `extra` carries, stamped
    /// with a fresh id and `now_ms`.
    fn push_event(&mut self, kind: &str, now_ms: u64, mut extra: Value) {
        self.next_event_id += 1;
        if let Some(obj) = extra.as_object_mut() {
            obj.insert("id".into(), json!(self.next_event_id));
            obj.insert("kind".into(), json!(kind));
            obj.insert("ts_ms".into(), json!(now_ms));
        }
        self.events.push(extra);
        if self.events.len() > EVENTS_MAX {
            self.events.remove(0);
        }
    }

    /// Raise a `budget_alert` event for every configured threshold crossed since the last check
    /// (plan limits, the global daily token/cost budget, and any per-project one — see
    /// `Config::budget_alerts_by_cwd`), each at most once per period/scope.
    fn check_budget_alerts(&mut self, now_ms: u64) {
        let mut to_fire: Vec<(String, Value)> = Vec::new();
        let cfg = &self.config.budget_alerts;
        for (agent, lim) in &self.limits {
            for (label, w) in [("five_hour", lim.five_hour.as_ref()), ("seven_day", lim.seven_day.as_ref())] {
                if let Some(w) = w {
                    for &t in &cfg.plan_percent {
                        if w.percent >= t {
                            let key = format!("{}:{label}:{}:{}", agent.id(), t as i64, w.resets_at_ms.unwrap_or(0));
                            if !self.alerted.contains(&key) {
                                to_fire.push((key, json!({ "agent": agent, "scope": label, "percent": w.percent, "threshold": t })));
                            }
                        }
                    }
                }
            }
        }
        let today = utc_day(now_ms);
        let global_tokens = self.usage.summary(&today).today.total();
        // Real per-day cost, not a blended all-time rate: `usage_history`'s housekeeping tick
        // already computes it the same way `day_cost` does, so this just reads it back.
        let global_cost = self.usage_history.by_day.get(&today).map(|d| d.estimated_cost_usd).unwrap_or(0.0);
        check_daily_thresholds(&mut self.alerted, &mut to_fire, "global", &today, global_tokens, global_cost, cfg);
        for (cwd, rules) in &self.config.budget_alerts_by_cwd {
            let tokens = self.usage.summary_for_cwd(cwd, &today).today.total();
            let cost = self.usage.day_cost(&today, Some(cwd), &self.config.model_prices);
            check_daily_thresholds(&mut self.alerted, &mut to_fire, cwd, &today, tokens, cost, rules);
        }
        for (key, extra) in to_fire {
            self.alerted.insert(key);
            self.push_event("budget_alert", now_ms, extra);
        }
    }

    /// Stats, external hooks and milestones triggered by `ev`. `prev` is the session as it was
    /// just before `ev` was applied (so `SessionEnd`, which removes it, and the waiting-edge
    /// check below still have something to read). Hooks are fire-and-forget: see `fire_hook`.
    fn handle_side_effects(&mut self, agent: Agent, ev: &AgentEvent, prev: Option<&Session>, sid: &str, now_ms: u64) {
        let today = utc_day(now_ms);
        let mut milestones = Vec::new();
        match &ev.kind {
            EventKind::SessionStart => {
                self.stats.total_sessions += 1;
                self.stats.mark_day(&today);
                self.stats_dirty = true;
                milestones = self.stats.check_milestones(&today);
                let hook = self.config.hooks.on_session_start.clone();
                fire_hook(&hook, "session_start", agent, self.sessions.map.get(sid));
            }
            EventKind::SessionEnd => {
                let hook = self.config.hooks.on_session_end.clone();
                fire_hook(&hook, "session_end", agent, prev);
            }
            EventKind::ToolEnd { ok: true } => {
                self.stats.total_tool_calls += 1;
                self.stats_dirty = true;
                milestones = self.stats.check_milestones(&today);
            }
            EventKind::Stop { .. } => {
                let hook = self.config.hooks.on_turn_end.clone();
                fire_hook(&hook, "turn_end", agent, self.sessions.map.get(sid));
            }
            _ => {}
        }
        let now_waiting = self.sessions.map.get(sid).map(|s| s.status) == Some(Status::Waiting);
        let was_waiting = prev.map(|s| s.status) == Some(Status::Waiting);
        if now_waiting && !was_waiting {
            let hook = self.config.hooks.on_waiting.clone();
            fire_hook(&hook, "waiting", agent, self.sessions.map.get(sid));
        }
        for (kind, value) in milestones {
            self.push_event("milestone", now_ms, json!({ "type": kind, "value": value }));
        }
    }

    /// Write `state.json` atomically when something changed, and at least every
    /// `HEARTBEAT` so readers can tell a stopped daemon from a quiet one.
    fn publish(&mut self) {
        let snap = self.snapshot();
        let body = snap.to_string();
        let now = now_ms();
        if body == self.last_published && now.saturating_sub(self.last_write_ms) < HEARTBEAT.as_millis() as u64 {
            return;
        }
        let changed = body != self.last_published;
        let mut out = snap;
        out["updated_ms"] = json!(now);
        if changed && !self.watchers.is_empty() {
            let reply = Reply { state: Some(out.clone()), ..Reply::ok() };
            // A watcher that is gone, or too slow to keep up, is dropped (it reconnects).
            self.watchers.retain_mut(|w| write_line(w, &reply).is_ok());
        }
        let dir = state_dir();
        let _ = std::fs::create_dir_all(&dir);
        let tmp = dir.join("state.json.tmp");
        if write_private(&tmp, out.to_string().as_bytes()).is_ok() && std::fs::rename(&tmp, state_path()).is_ok() {
            self.last_published = body;
            self.last_write_ms = now;
        }
    }

    fn drop_pending_for(&mut self, session_id: &str) {
        // Dropping the sender wakes the waiting hook thread.
        self.pending.retain(|p| p.session_id != session_id);
    }

    /// A tool started: any plan that was waiting for the terminal has been dealt with.
    fn drop_plans_for(&mut self, session_id: &str) {
        self.pending.retain(|p| !(p.kind == Kind::Plan && p.session_id == session_id));
    }
}

/// Daily token/cost thresholds for one scope (`"global"` or a configured cwd), folding `scope`
/// into the one-shot key so each project's thresholds fire independently of the global ones and
/// of each other.
fn check_daily_thresholds(
    alerted: &mut HashSet<String>,
    to_fire: &mut Vec<(String, Value)>,
    scope: &str,
    today: &str,
    today_tokens: u64,
    today_cost: f64,
    cfg: &BudgetAlerts,
) {
    if cfg.daily_tokens > 0 && today_tokens > 0 {
        let pct = today_tokens as f64 / cfg.daily_tokens as f64 * 100.0;
        for &t in &cfg.daily_percent {
            if pct >= t {
                let key = format!("daily_tokens:{scope}:{today}:{}", t as i64);
                if !alerted.contains(&key) {
                    to_fire.push((key, json!({ "scope": "daily_tokens", "project": scope, "percent": pct, "threshold": t })));
                }
            }
        }
    }
    if cfg.daily_cost_usd > 0.0 && today_cost > 0.0 {
        let pct = today_cost / cfg.daily_cost_usd * 100.0;
        for &t in &cfg.daily_percent {
            if pct >= t {
                let key = format!("daily_cost:{scope}:{today}:{}", t as i64);
                if !alerted.contains(&key) {
                    to_fire.push((key, json!({ "scope": "daily_cost", "project": scope, "percent": pct, "threshold": t })));
                }
            }
        }
    }
}

/// Write a file only the current user can read (it may hold code and command output).
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    // On Windows the state lives under %LOCALAPPDATA%, which only the user can read.
    let mut f = opts.open(path)?;
    f.write_all(bytes)
}

/// A shell invocation of `cmd`, portable the same way `install::hook_command` already needs to be.
fn shell_command(cmd: &str) -> Command {
    if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", cmd]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        c
    }
}

/// Run an external hook (see `usage::Hooks`): `action.cmd` with `SUSHI_*` env vars, and/or
/// `action.url` posted a small JSON body via `curl`. Both fire-and-forget in their own thread —
/// this never blocks the daemon, and a slow or failing script/endpoint is simply ignored.
fn fire_hook(action: &HookAction, event: &'static str, agent: Agent, session: Option<&Session>) {
    if action.cmd.is_empty() && action.url.is_empty() {
        return;
    }
    let (sid, name, cwd, status) = session
        .map(|s| (s.native_id.clone(), s.name.clone(), s.cwd.clone(), format!("{:?}", s.status).to_lowercase()))
        .unwrap_or_default();
    let agent_id = agent.id().to_string();
    let cmd = action.cmd.clone();
    let url = action.url.clone();
    thread::spawn(move || {
        if !cmd.is_empty() {
            let mut c = shell_command(&cmd);
            c.env("SUSHI_EVENT", event)
                .env("SUSHI_AGENT", &agent_id)
                .env("SUSHI_SESSION_ID", &sid)
                .env("SUSHI_SESSION_NAME", &name)
                .env("SUSHI_CWD", &cwd)
                .env("SUSHI_STATUS", &status)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let _ = c.spawn();
        }
        if !url.is_empty() {
            let body = json!({ "event": event, "agent": agent_id, "session_id": sid, "session_name": name, "cwd": cwd, "status": status }).to_string();
            if let Ok(mut child) = Command::new("curl")
                .args(["-sS", "--max-time", "5", "-X", "POST", "-H", "Content-Type: application/json", "--data-binary", "@-", url.as_str()])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(body.as_bytes());
                }
                let _ = child.wait();
            }
        }
    });
}

fn peer_closed(stream: &mut UnixStream) -> bool {
    let _ = stream.set_nonblocking(true);
    let mut b = [0u8; 1];
    let closed = match stream.read(&mut b) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => e.kind() != std::io::ErrorKind::WouldBlock,
    };
    let _ = stream.set_nonblocking(false);
    closed
}

fn write_reply(stream: &mut UnixStream, reply: &Reply) {
    let _ = write_line(stream, reply);
}

fn write_line(stream: &mut UnixStream, reply: &Reply) -> std::io::Result<()> {
    let mut line = serde_json::to_string(reply).map_err(std::io::Error::other)?;
    line.push('\n');
    stream.write_all(line.as_bytes())
}

fn handle_hook(mut stream: UnixStream, shared: &Shared, agent: Agent, payload: Value) {
    let Some(ev) = agent.normalize(&payload) else { return };
    let rx = {
        let mut st = lock(shared);
        let now = now_ms();
        let prev = st.sessions.map.get(&session_key(agent, &ev.session_id)).cloned();
        let policies = st.config.policies.clone();
        let sid = st.sessions.apply_event(&ev, now, &policies);
        if let Some(path) = st.sessions.map.get(&sid).and_then(|s| s.transcript.clone()) {
            st.usage.refresh(&path);
        }
        // Mirror the step `apply_event` just pushed or updated into the durable history (see
        // `sushi::history`): `Activity::find` uses the same id-or-name matching `post_tool`
        // itself used, so this sees the exact entry that was just touched either way.
        if matches!(ev.kind, EventKind::ToolStart | EventKind::ToolEnd { .. } | EventKind::PermissionRequest)
            && let (Some(session), Some(tool)) = (st.sessions.map.get(&sid), ev.tool.as_ref())
            && let Some(event) = session.activity.find(tool).cloned()
        {
            let id = sushi::activity::step_id(&event);
            let (agent_id, cwd, name) = (session.agent.id().to_string(), session.cwd.clone(), session.name.clone());
            st.history.record(&sid, &agent_id, &cwd, &name, now, &id, &event);
            st.history_dirty = true;
        }
        match ev.kind {
            EventKind::ToolEnd { .. } | EventKind::Prompt | EventKind::Stop { .. } | EventKind::SessionEnd => st.drop_pending_for(&sid),
            EventKind::ToolStart => st.drop_plans_for(&sid),
            _ => {}
        }
        st.handle_side_effects(agent, &ev, prev.as_ref(), &sid, now);
        let rx = match (&ev.kind, &ev.tool) {
            (EventKind::PermissionRequest, tool) if st.sessions.map.contains_key(&sid) => {
                st.next_id += 1;
                let id = st.next_id;
                let tool_summary = st.sessions.map.get(&sid).and_then(|s| s.last_tool.as_ref()).map(|t| t.text.clone()).unwrap_or_default();
                let cwd = st.sessions.map.get(&sid).map(|s| s.cwd.clone()).unwrap_or_default();
                let hide = st.sessions.hide_code;
                let (tool_name, detail, role, input) = match tool {
                    Some(t) => (t.name.clone(), detail::for_tool(t, &cwd, !hide), t.role, t.input.clone()),
                    None => (String::new(), None, Role::Normal, Value::Null),
                };
                let caps = agent.capabilities();
                let kind = match role {
                    Role::Question if !hide && caps.questions => Kind::Question,
                    Role::Plan if caps.plans => Kind::Plan,
                    _ => Kind::Permission,
                };
                let (tx, rx) = mpsc::channel();
                let tool_input = if matches!(kind, Kind::Question | Kind::Plan) { input } else { Value::Null };
                st.pending.push(Pending { id, agent, session_id: sid, tool: tool_summary, tool_name, detail, kind, tool_input, created_ms: now, tx: Some(tx) });
                Some((id, rx))
            }
            _ => None,
        };
        st.publish();
        rx
    };
    let Some((id, rx)) = rx else { return };

    let started = std::time::Instant::now();
    let decision = loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(d) => break Some(d),
            Err(RecvTimeoutError::Disconnected) => break None,
            Err(RecvTimeoutError::Timeout) => {
                // The hook is killed when the user answers in the terminal.
                if peer_closed(&mut stream) || started.elapsed() > PERMISSION_TTL {
                    break None;
                }
            }
        }
    };
    let mut st = lock(shared);
    match st.pending.iter_mut().find(|p| p.id == id) {
        // The plan is still open in the terminal: keep showing it until Claude moves on.
        Some(p) if p.kind == Kind::Plan && decision.is_none() => p.tx = None,
        _ => st.pending.retain(|p| p.id != id),
    }
    st.publish();
    drop(st);
    if let Some((d, updated_input, mode)) = decision {
        write_reply(&mut stream, &Reply { decision: Some(d), updated_input, mode: mode.map(str::to_string), ..Reply::ok() });
    }
}

/// Allow or deny a pending permission, or approve a plan (`accept_edits`: and let Claude edit
/// without asking) or send it back to keep planning.
fn handle_decision(shared: &Shared, id: u64, decision: Decision, accept_edits: bool) -> Reply {
    resolve(shared, id, |p| match (p.kind, decision) {
        (Kind::Permission, _) => Ok((decision, None, None)),
        (Kind::Question, _) => Err("this is a question: answer it (sushi answer) or answer it in the terminal".into()),
        // Claude Code only takes the approval of a plan along with its input.
        (Kind::Plan, Decision::Allow) => Ok((decision, Some(p.tool_input.clone()), accept_edits.then_some("acceptEdits"))),
        (Kind::Plan, Decision::Deny) => Ok((decision, None, None)),
    })
}

/// Answer an `AskUserQuestion`: `answers` maps each question text to the chosen label (labels of a
/// multi-select question joined with ", "). Claude Code takes them as the tool's `answers` input.
fn handle_answer(shared: &Shared, id: u64, answers: Value) -> Reply {
    resolve(shared, id, |p| {
        if p.kind != Kind::Question {
            return Err("this request is not a question".into());
        }
        let ok = answers.as_object().is_some_and(|m| !m.is_empty() && m.values().all(Value::is_string));
        if !ok {
            return Err("answers must be an object of question text → chosen label".into());
        }
        let mut input = p.tool_input.clone();
        match input.as_object_mut() {
            Some(obj) => obj.insert("answers".into(), answers.clone()),
            None => return Err("the question has no input to answer".into()),
        };
        Ok((Decision::Allow, Some(input), None))
    })
}

/// Find the pending request, let `decide` produce the verdict, and wake the waiting hook with it.
fn resolve(shared: &Shared, id: u64, decide: impl FnOnce(&Pending) -> Result<Verdict, String>) -> Reply {
    let mut st = lock(shared);
    let Some(pos) = st.pending.iter().position(|p| p.id == id) else {
        return Reply::err(format!("no pending request with id {id}"));
    };
    if st.pending[pos].tx.is_none() {
        return Reply::err("nothing is waiting for this answer any more: answer it in the terminal");
    }
    let verdict = match decide(&st.pending[pos]) {
        Ok(v) => v,
        Err(e) => return Reply::err(e),
    };
    let p = st.pending.remove(pos);
    if let Some(tx) = &p.tx {
        let _ = tx.send(verdict);
    }
    if let Some(s) = st.sessions.map.get_mut(&p.session_id) {
        s.status = Status::Working;
        s.attention = None;
    }
    st.publish();
    Reply::ok()
}

/// Run a care action that bumps a need + xp, saving the result and celebrating a growth tier
/// crossing (if any) the same way a milestone is celebrated. `act` itself enforces the
/// per-action cooldown (see `Care::CARE_COOLDOWN_MS`), so a too-soon click is refused here too.
fn care_action(shared: &Shared, act: impl FnOnce(&mut Care, u64) -> Result<Option<u64>, &'static str>) -> Reply {
    let mut st = lock(shared);
    let now = now_ms();
    st.care.decay(now);
    match act(&mut st.care, now) {
        Ok(tier) => {
            st.care_dirty = true;
            if let Some(t) = tier {
                st.push_event("grew_up", now, json!({ "tier": t }));
            }
            st.publish();
            Reply::ok()
        }
        Err(e) => Reply::err(e),
    }
}

fn handle_care_play(shared: &Shared, score: u32) -> Reply {
    let mut st = lock(shared);
    let decayed = st.care.decay(now_ms());
    if decayed {
        st.care_dirty = true;
    }
    match st.care.play(score, now_ms()) {
        Ok(tier) => {
            st.care_dirty = true;
            if let Some(t) = tier {
                st.push_event("grew_up", now_ms(), json!({ "tier": t }));
            }
            st.publish();
            Reply::ok()
        }
        Err(e) => Reply::err(e),
    }
}

fn handle_care_buy(shared: &Shared, id: &str) -> Reply {
    let mut st = lock(shared);
    match st.care.buy(id) {
        Ok(()) => {
            st.care_dirty = true;
            st.publish();
            Reply::ok()
        }
        Err(e) => Reply::err(e),
    }
}

fn handle_care_equip(shared: &Shared, id: &str) -> Reply {
    let mut st = lock(shared);
    match st.care.equip(id) {
        Ok(()) => {
            st.care_dirty = true;
            st.publish();
            Reply::ok()
        }
        Err(e) => Reply::err(e),
    }
}

fn chat_dir() -> std::path::PathBuf {
    chat::cache_dir().join("chat")
}

fn chat_file() -> std::path::PathBuf {
    chat::cache_dir().join("chat.json")
}

fn stats_file() -> std::path::PathBuf {
    chat::cache_dir().join("stats.json")
}

fn care_file() -> std::path::PathBuf {
    chat::cache_dir().join("care.json")
}

fn history_file() -> std::path::PathBuf {
    chat::cache_dir().join("history.json")
}

fn usage_history_file() -> std::path::PathBuf {
    chat::cache_dir().join("usage_history.json")
}

/// Where `sushi export` writes markdown files — a fixed, daemon-chosen location (not a path the
/// user picks): the app has no file-save dialog wired up and the Noctalia plugin cannot write
/// files from Lua at all, so both just show the path this returns.
fn exports_dir() -> std::path::PathBuf {
    chat::cache_dir().join("exports")
}

fn handle_flag_step(shared: &Shared, session_id: &str, step_id: &str, flagged: bool, note: Option<String>) -> Reply {
    let mut st = lock(shared);
    if !st.history.flag(session_id, step_id, flagged, note) {
        return Reply::err(format!("no step {step_id} in session {session_id}"));
    }
    st.history_dirty = true;
    st.publish();
    Reply::ok()
}

fn handle_export(shared: &Shared, session_id: &str, query: Option<&str>) -> Reply {
    let st = lock(shared);
    let Some(session) = st.history.sessions.get(session_id) else {
        return Reply::err(format!("no stored history for session {session_id}"));
    };
    let steps = st.history.filtered(session_id, query);
    let markdown = sushi::history::export_markdown(session_id, session, &steps);
    let dir = exports_dir();
    drop(st);
    if std::fs::create_dir_all(&dir).is_err() {
        return Reply::err(format!("cannot create {}", dir.display()));
    }
    let short = session_id.replace([':', '/', '\\'], "-");
    let path = dir.join(format!("{short}-{}.md", now_ms()));
    match std::fs::write(&path, markdown) {
        Ok(()) => Reply { state: Some(json!(path.to_string_lossy())), ..Reply::ok() },
        Err(e) => Reply::err(format!("cannot write {}: {e}", path.display())),
    }
}

/// Start a chat turn in the background; the answer streams into the shared state. `agent` and
/// `model` override the configured ones; `file` is a file fed with the message, whose content goes
/// into the prompt (the log only keeps its name).
fn chat_send(shared: &Shared, text: &str, model: Option<String>, agent: Option<String>, file: Option<String>) -> Reply {
    let text: String = text.trim().chars().take(chat::MAX_INPUT_CHARS).collect();
    let (transcribe_model, whisper) = {
        let st = lock(shared);
        (st.config.transcribe_model_path.clone(), st.config.whisper_path.clone())
    };
    let attachment = match file.filter(|f| !f.trim().is_empty()) {
        Some(f) => match chat::read_attachment(std::path::Path::new(&f), &transcribe_model, &whisper) {
            Ok(a) => Some(a),
            Err(e) => return Reply::err(e),
        },
        None => None,
    };
    if text.is_empty() && attachment.is_none() {
        return Reply::err("empty message");
    }
    let prompt = match &attachment {
        Some((name, content)) => chat::prompt_with_file(name, content, &text),
        None => text.clone(),
    };
    let (agent, program, args, slot, generation, was_started) = {
        let mut st = lock(shared);
        if st.chat.busy {
            return Reply::err("The assistant is still answering");
        }
        let id = agent.filter(|a| !a.trim().is_empty()).unwrap_or_else(|| st.config.chat_agent.clone());
        let agent = match Agent::from_id(&id) {
            Some(a) if a.capabilities().chat => a,
            Some(a) => return Reply::err(format!("{} cannot run as the chat: it has no tool-free headless mode", a.label())),
            None => return Reply::err(format!("unknown chat agent {id}")),
        };
        st.chat.use_agent(agent);
        let model = model
            .filter(|m| !m.trim().is_empty())
            .filter(|m| agent == Agent::Claude || !chat::is_claude_alias(m))
            .or_else(|| st.config.chat_models.get(agent.id()).cloned())
            .or_else(|| chat::default_model(agent, &st.config.chat_model));
        let args = chat::build_args(agent, &st.chat.saved.session_id, st.chat.saved.started, model.as_deref(), &prompt);
        let was_started = st.chat.saved.started;
        st.chat.begin_turn(&text, attachment.as_ref().map(|(name, _)| name.as_str()));
        st.publish();
        (agent, st.config.program_for(agent), args, st.chat.child.clone(), st.chat.generation, was_started)
    };
    let shared = shared.clone();
    thread::spawn(move || {
        let mut last_publish = std::time::Instant::now() - Duration::from_secs(1);
        let outcome = chat::run_turn(agent, &program, &args, &chat_dir(), &slot, |partial| {
            let mut st = lock(&shared);
            if st.chat.generation != generation {
                return;
            }
            st.chat.set_answer(partial);
            if last_publish.elapsed() >= Duration::from_millis(80) {
                st.publish();
                last_publish = std::time::Instant::now();
            }
        });
        let mut st = lock(&shared);
        if st.chat.generation != generation {
            return; // the conversation was cleared meanwhile
        }
        let answered = !outcome.turn.text.is_empty();
        if answered {
            st.chat.set_answer(&outcome.turn.text);
        } else if let Some(last) = st.chat.saved.messages.last_mut() {
            // Nothing came back: say why instead of leaving an empty bubble.
            last.text = if outcome.stopped { "(stopped)".into() } else { "(no answer)".into() };
        }
        if outcome.stopped && answered {
            let t = format!("{} (stopped)", outcome.turn.text);
            st.chat.set_answer(&t);
        }
        st.chat.error = outcome.turn.error.clone();
        if let Some(id) = &outcome.turn.session {
            st.chat.saved.session_id = id.clone(); // the agent names its own sessions (Codex)
        }
        match &outcome.turn.error {
            None if answered => st.chat.saved.started = true, // the session exists now
            Some(e) if was_started && (e.contains("No conversation found") || e.to_lowercase().contains("session not found")) => {
                // The remembered session is gone on the agent's side: begin a new one next time.
                st.chat.saved.session_id = chat::new_session_id();
                st.chat.saved.started = false;
            }
            _ => {}
        }
        st.chat.busy = false;
        st.chat.save(&chat_file());
        st.publish();
    });
    Reply::ok()
}

fn handle_conn(mut stream: UnixStream, shared: Shared) {
    let mut line = String::new();
    let Ok(clone) = stream.try_clone() else { return };
    if BufReader::new(clone).read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    match serde_json::from_str::<Request>(&line) {
        Ok(Request::Hook { agent, payload }) => match Agent::from_id(&agent) {
            Some(agent) => handle_hook(stream, &shared, agent, payload),
            None => write_reply(&mut stream, &Reply::err(format!("unknown agent {agent}"))),
        },
        Ok(Request::Approve { id, accept_edits }) => write_reply(&mut stream, &handle_decision(&shared, id, Decision::Allow, accept_edits)),
        Ok(Request::Deny { id }) => write_reply(&mut stream, &handle_decision(&shared, id, Decision::Deny, false)),
        Ok(Request::State) => {
            let snap = lock(&shared).snapshot();
            write_reply(&mut stream, &Reply { state: Some(snap), ..Reply::ok() });
        }
        Ok(Request::Watch) => {
            let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
            let mut st = lock(&shared);
            let mut snap = st.snapshot();
            snap["updated_ms"] = json!(now_ms());
            if write_line(&mut stream, &Reply { state: Some(snap), ..Reply::ok() }).is_ok() {
                st.watchers.push(stream);
            }
        }
        Ok(Request::Answer { id, answers }) => write_reply(&mut stream, &handle_answer(&shared, id, answers)),
        Ok(Request::ChatSend { text, model, agent, file }) => {
            write_reply(&mut stream, &chat_send(&shared, &text, model, agent, file))
        }
        Ok(Request::ChatStop) => {
            let slot = lock(&shared).chat.child.clone();
            chat::stop(&slot);
            write_reply(&mut stream, &Reply::ok());
        }
        Ok(Request::ChatClear) => {
            let slot = {
                let mut st = lock(&shared);
                st.chat.clear();
                st.chat.save(&chat_file());
                st.publish();
                st.chat.child.clone()
            };
            chat::stop(&slot); // the old turn, if any, ends on its own and is ignored
            write_reply(&mut stream, &Reply::ok());
        }
        Ok(Request::CareFeed) => write_reply(&mut stream, &care_action(&shared, Care::feed)),
        Ok(Request::CarePet) => write_reply(&mut stream, &care_action(&shared, Care::pet)),
        Ok(Request::CareNap) => write_reply(&mut stream, &care_action(&shared, Care::nap_boost)),
        Ok(Request::CarePlay { score }) => write_reply(&mut stream, &handle_care_play(&shared, score)),
        Ok(Request::CareBuy { id }) => write_reply(&mut stream, &handle_care_buy(&shared, &id)),
        Ok(Request::CareEquip { id }) => write_reply(&mut stream, &handle_care_equip(&shared, &id)),
        Ok(Request::History { session_id, query }) => {
            let steps = lock(&shared).history.filtered(&session_id, query.as_deref());
            write_reply(&mut stream, &Reply { state: Some(json!(steps)), ..Reply::ok() });
        }
        Ok(Request::FlagStep { session_id, step_id, flagged, note }) => {
            write_reply(&mut stream, &handle_flag_step(&shared, &session_id, &step_id, flagged, note));
        }
        Ok(Request::Export { session_id, query }) => write_reply(&mut stream, &handle_export(&shared, &session_id, query.as_deref())),
        Err(e) => write_reply(&mut stream, &Reply::err(format!("bad request: {e}"))),
    }
}

fn find_transcript(session_id: &str) -> Option<PathBuf> {
    let projects = claude_dir().join("projects");
    std::fs::read_dir(projects).ok()?.flatten().find_map(|d| {
        let p = d.path().join(format!("{session_id}.jsonl"));
        p.exists().then_some(p)
    })
}

/// Periodically: reconcile sessions, locate transcripts, refresh usage.
fn housekeeping(shared: Shared) {
    loop {
        {
            let mut st = lock(&shared);
            st.sessions.reconcile(&claude_dir().join("sessions"), now_ms());
            let ids: Vec<String> =
                st.sessions.map.values().filter(|s| s.agent.capabilities().context).map(|s| s.id.clone()).collect();
            for id in ids {
                let Some(s) = st.sessions.map.get(&id) else { continue };
                let path = match s.transcript.clone() {
                    Some(p) => Some(p),
                    None => find_transcript(&s.native_id),
                };
                if let Some(p) = path {
                    st.usage.refresh(&p);
                    if let Some(s) = st.sessions.map.get_mut(&id) {
                        s.transcript = Some(p);
                    }
                }
            }
            let live: Vec<String> = st.sessions.map.keys().cloned().collect();
            let now = now_ms();
            st.pending.retain(|p| {
                live.contains(&p.session_id) && !(p.kind == Kind::Plan && now.saturating_sub(p.created_ms) > PLAN_TTL_MS)
            });
            // Snapshot "today so far" into the durable ledger before `check_budget_alerts` reads
            // it back for the real (non-blended) daily cost — see `usage_history.rs`.
            let today = utc_day(now);
            let today_tokens = st.usage.summary(&today).today;
            let today_cost = st.usage.day_cost(&today, None, &st.config.model_prices);
            let candidate = DaySummary { tokens: today_tokens, estimated_cost_usd: today_cost };
            if st.usage_history.by_day.get(&today) != Some(&candidate) {
                st.usage_history.by_day.insert(today, candidate);
                st.usage_history_dirty = true;
            }
            st.check_budget_alerts(now);
            if st.stats_dirty {
                st.stats.save(&stats_file());
                st.stats_dirty = false;
            }
            if st.history_dirty {
                st.history.save(&history_file());
                st.history_dirty = false;
            }
            if st.usage_history_dirty {
                st.usage_history.save(&usage_history_file());
                st.usage_history_dirty = false;
            }
            let decayed = st.care.decay(now);
            let total_tool_calls = st.stats.total_tool_calls;
            let coins = st.care.accrue_currency(total_tool_calls);
            if decayed || coins > 0 {
                st.care_dirty = true;
            }
            if st.care_dirty {
                st.care.save(&care_file());
                st.care_dirty = false;
            }
            st.publish();
        }
        thread::sleep(Duration::from_secs(2));
    }
}

/// Keep one agent's plan usage fresh, independently of every other agent (so a slow or
/// rate-limited one never delays the others). Disabled with `SUSHI_NO_LIMITS=1`.
fn limits_loop(shared: Shared, agent: Agent, fetch: impl Fn() -> Result<Value, FetchError>, parse: fn(&Value, u64) -> Limits) {
    loop {
        let result = fetch();
        let wait = {
            let mut st = lock(&shared);
            let wait = match result {
                Ok(v) => {
                    st.limits.insert(agent, parse(&v, now_ms()));
                    st.limits_error.remove(&agent);
                    LIMITS_REFRESH
                }
                Err(FetchError::NotConfigured) => {
                    // Never logged into this agent: say nothing rather than publish an error.
                    st.limits_error.remove(&agent);
                    LIMITS_RETRY
                }
                Err(e) => {
                    st.limits_error.insert(agent, e.to_string());
                    if matches!(e, FetchError::Http(429, _)) { LIMITS_BACKOFF } else { LIMITS_RETRY }
                }
            };
            st.publish();
            wait
        };
        thread::sleep(wait);
    }
}

/// One-off scan of recent transcripts so daily totals are available at startup.
fn initial_usage_scan(shared: Shared) {
    let Ok(rd) = std::fs::read_dir(claude_dir().join("projects")) else { return };
    let cutoff = SystemTime::now() - SCAN_WINDOW;
    for dir in rd.flatten() {
        let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
        for f in files.flatten() {
            let p = f.path();
            let recent = f.metadata().and_then(|m| m.modified()).is_ok_and(|t| t >= cutoff);
            if recent && p.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                lock(&shared).usage.refresh(&p);
            }
        }
    }
    lock(&shared).publish();
}

fn run_daemon() -> Result<(), String> {
    let sock = socket_path();
    if UnixStream::connect(&sock).is_ok() {
        return Err(format!("daemon already running on {}", sock.display()));
    }
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock).map_err(|e| format!("bind {}: {e}", sock.display()))?;
    #[cfg(unix)]
    std::fs::set_permissions(&sock, std::os::unix::fs::PermissionsExt::from_mode(0o600)).map_err(|e| e.to_string())?;

    let config = Config::load(&config_path());
    let mut state = State {
        config,
        chat: Chat::load(&chat_file()),
        stats: Stats::load(&stats_file()),
        care: Care::load(&care_file()),
        history: HistoryStore::load(&history_file()),
        usage_history: UsageHistory::load(&usage_history_file()),
        ..State::default()
    };
    state.sessions.hide_code = !state.config.show_code;
    state.sessions.ignore_cwd = Some(chat_dir().to_string_lossy().into_owned());
    let shared: Shared = Arc::new(Mutex::new(state));
    if std::env::var_os("SUSHI_NO_LIMITS").is_none() {
        let s = shared.clone();
        thread::spawn(move || limits_loop(s, Agent::Claude, move || sushi::limits::claude::fetch(&claude_dir().join(".credentials.json")), sushi::limits::claude::parse));
        let s = shared.clone();
        thread::spawn(move || limits_loop(s, Agent::Codex, move || sushi::limits::codex::fetch(&codex_dir()), sushi::limits::codex::parse));
        let s = shared.clone();
        thread::spawn(move || limits_loop(s, Agent::Copilot, move || sushi::limits::copilot::fetch(&copilot_dir()), sushi::limits::copilot::parse));
        let s = shared.clone();
        thread::spawn(move || limits_loop(s, Agent::Antigravity, move || sushi::limits::antigravity::fetch(&gemini_dir()), sushi::limits::antigravity::parse));
    }
    {
        let s = shared.clone();
        thread::spawn(move || housekeeping(s));
        let s = shared.clone();
        thread::spawn(move || initial_usage_scan(s));
    }
    eprintln!("sushi: listening on {}", sock.display());
    for conn in listener.incoming().flatten() {
        let s = shared.clone();
        thread::spawn(move || handle_conn(conn, s));
    }
    Ok(())
}

fn client(req: &Request) -> Result<Reply, String> {
    sushi::client::request(req, Duration::from_secs(5))
}

fn hook_binary() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(format!("sushi-hook{}", std::env::consts::EXE_SUFFIX))))
        .unwrap_or_else(|| PathBuf::from("sushi-hook"))
}

/// `sushi install`: show what connecting each agent would write, or write it.
fn install_command(agents: &[Agent], write: bool) -> Result<(), String> {
    let hook = hook_binary();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    for &agent in agents {
        if !write {
            println!("# {}: would be written to {}\n{}", agent.label(), install::target_path(agent).display(), install::preview(agent, &hook));
            eprintln!("Run it with --write to do it (the previous file is backed up).");
            continue;
        }
        let report = install::install_agent(agent, &hook, now)?;
        if report.added.is_empty() {
            println!("{}: already connected, nothing changed.", agent.label());
        } else {
            println!("{}: connected through {} ({})", agent.label(), report.path.display(), report.added.join(", "));
            if let Some(b) = report.backup {
                println!("  previous file saved as {}", b.display());
            }
            println!("  Sessions that are already open pick it up after a restart.");
        }
        // Claude Code is the only agent whose own permissions can really force a confirmation or a
        // refusal (see policy.rs): merge `claude_permissions` from config.json alongside its hooks.
        if agent == Agent::Claude {
            let perms = Config::load(&config_path()).claude_permissions;
            match install::install_permissions(&install::settings_path(), &perms, now) {
                Ok((changed, _)) if !changed.is_empty() => {
                    println!("  policy: merged {} into {}", changed.join(", "), install::settings_path().display());
                }
                Ok(_) => {}
                Err(e) => eprintln!("  policy: {e}"),
            }
        }
    }
    Ok(())
}

fn usage() -> ! {
    eprintln!(
        "usage: sushi <daemon | state | approve ID [--accept-edits] | deny ID | answer ID JSON | chat [TEXT] [--file PATH] [--model M] [--agent A] | chat-stop | chat-clear | care feed|pet|nap | care play SCORE | care buy|equip ID | history SESSION_ID [--query TEXT] | flag SESSION_ID STEP_ID on|off [--note TEXT] | export SESSION_ID [--query TEXT] | install [--agent claude|codex|opencode|pi|copilot|antigravity|gemini|all] [--write]>"
    );
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["daemon"] => run_daemon(),
        ["state"] => client(&Request::State).map(|r| {
            println!("{}", serde_json::to_string_pretty(&r.state).unwrap_or_default());
        }),
        ["chat", rest @ ..] if !rest.is_empty() => {
            // sushi chat [text...] [--file <path>] [--model <alias>] [--agent <id>]
            let mut model = None;
            let mut agent = None;
            let mut file = None;
            let mut words = Vec::new();
            let mut it = rest.iter();
            while let Some(w) = it.next() {
                if *w == "--model" {
                    model = it.next().map(|m| m.to_string());
                } else if *w == "--agent" {
                    agent = it.next().map(|m| m.to_string());
                } else if *w == "--file" {
                    // The daemon runs elsewhere: hand it an absolute path.
                    let Some(p) = it.next() else { usage() };
                    file = Some(std::fs::canonicalize(p).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| p.to_string()));
                } else {
                    words.push(*w);
                }
            }
            client(&Request::ChatSend { text: words.join(" "), model, agent, file })
                .and_then(|r| if r.ok { Ok(()) } else { Err(r.error.unwrap_or_default()) })
        }
        ["answer", id, json] => match (id.parse::<u64>(), serde_json::from_str::<Value>(json)) {
            (Ok(id), Ok(answers)) => {
                client(&Request::Answer { id, answers }).and_then(|r| if r.ok { Ok(()) } else { Err(r.error.unwrap_or_default()) })
            }
            _ => usage(),
        },
        ["chat-stop"] => client(&Request::ChatStop).map(|_| ()),
        ["chat-clear"] => client(&Request::ChatClear).map(|_| ()),
        ["care", cmd @ ("feed" | "pet" | "nap")] => {
            let req = match *cmd {
                "feed" => Request::CareFeed,
                "pet" => Request::CarePet,
                _ => Request::CareNap,
            };
            client(&req).and_then(|r| if r.ok { Ok(()) } else { Err(r.error.unwrap_or_default()) })
        }
        ["care", "play", score] => match score.parse::<u32>() {
            Ok(score) => client(&Request::CarePlay { score })
                .and_then(|r| if r.ok { Ok(()) } else { Err(r.error.unwrap_or_default()) }),
            Err(_) => usage(),
        },
        ["care", "buy", id] => {
            client(&Request::CareBuy { id: id.to_string() }).and_then(|r| if r.ok { Ok(()) } else { Err(r.error.unwrap_or_default()) })
        }
        ["care", "equip", id] => {
            client(&Request::CareEquip { id: id.to_string() }).and_then(|r| if r.ok { Ok(()) } else { Err(r.error.unwrap_or_default()) })
        }
        ["history", session_id, rest @ ..] => {
            let query = match rest {
                ["--query", q] => Some(q.to_string()),
                [] => None,
                _ => usage(),
            };
            client(&Request::History { session_id: session_id.to_string(), query }).map(|r| {
                println!("{}", serde_json::to_string_pretty(&r.state).unwrap_or_default());
            })
        }
        ["flag", session_id, step_id, state @ ("on" | "off"), rest @ ..] => {
            let note = match rest {
                ["--note", n] => Some(n.to_string()),
                [] => None,
                _ => usage(),
            };
            client(&Request::FlagStep { session_id: session_id.to_string(), step_id: step_id.to_string(), flagged: *state == "on", note })
                .and_then(|r| if r.ok { Ok(()) } else { Err(r.error.unwrap_or_default()) })
        }
        ["export", session_id, rest @ ..] => {
            let query = match rest {
                ["--query", q] => Some(q.to_string()),
                [] => None,
                _ => usage(),
            };
            client(&Request::Export { session_id: session_id.to_string(), query }).and_then(|r| {
                if r.ok {
                    println!("{}", r.state.and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default());
                    Ok(())
                } else {
                    Err(r.error.unwrap_or_default())
                }
            })
        }
        [cmd @ ("approve" | "deny"), id, flags @ ..] if flags.is_empty() || (*cmd == "approve" && flags == ["--accept-edits"]) => match id.parse::<u64>() {
            Ok(id) => {
                let req = if *cmd == "approve" { Request::Approve { id, accept_edits: !flags.is_empty() } } else { Request::Deny { id } };
                client(&req).and_then(|r| if r.ok { Ok(()) } else { Err(r.error.unwrap_or_default()) })
            }
            Err(_) => usage(),
        },
        ["install-hooks"] => install_command(&[Agent::Claude], false),
        ["install-hooks", "--write"] => install_command(&[Agent::Claude], true),
        ["install", rest @ ..] => {
            let mut agents: Vec<Agent> = Vec::new();
            let mut write = false;
            let mut it = rest.iter();
            while let Some(a) = it.next() {
                match *a {
                    "--write" => write = true,
                    "--agent" => match it.next() {
                        Some(&"all") => agents = Agent::ALL.to_vec(),
                        Some(id) => match Agent::from_id(id) {
                            Some(a) => agents.push(a),
                            None => usage(),
                        },
                        None => usage(),
                    },
                    _ => usage(),
                }
            }
            if agents.is_empty() {
                agents.push(Agent::Claude);
            }
            install_command(&agents, write)
        }
        _ => usage(),
    };
    if let Err(e) = result {
        eprintln!("sushi: {e}");
        std::process::exit(1);
    }
}
