use sushi::agent::{Agent, EventKind, Role};
use sushi::chat::{self, Chat};
use sushi::install;
use sushi::detail::{self, Detail};
use sushi::limits::{self, FetchError, Limits};
use sushi::paths::{claude_dir, config_path, socket_path, state_dir, state_path};
use sushi::protocol::{Decision, Reply, Request};
use sushi::sessions::{Sessions, Status};
use sushi::usage::{Config, UsageStore, utc_day};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use sushi::ipc::{UnixListener, UnixStream};
use std::path::PathBuf;
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
    /// A plan: the dialog can only be answered in the terminal; shown for information.
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

/// The hook's final word: a decision and, for questions, the input carrying the answers.
type Verdict = (Decision, Option<Value>);

struct Pending {
    id: u64,
    agent: Agent,
    session_id: String,
    tool: String,
    /// Raw tool name (`Bash`, `edit`, ...) and what it is about to do, for the approval card.
    tool_name: String,
    detail: Option<Detail>,
    kind: Kind,
    /// The original tool input, kept for questions (the answers are added to it).
    tool_input: Value,
    created_ms: u64,
    /// `None` for plans: nothing is waiting for an answer.
    tx: Option<Sender<Verdict>>,
}

/// Plans nobody answered in this long are dropped (they have no hook waiting to give up).
const PLAN_TTL_MS: u64 = 30 * 60 * 1000;

#[derive(Default)]
struct State {
    sessions: Sessions,
    pending: Vec<Pending>,
    next_id: u64,
    usage: UsageStore,
    last_published: String,
    last_write_ms: u64,
    config: Config,
    limits: Option<Limits>,
    limits_error: Option<String>,
    chat: Chat,
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
                v["activity"] = s.activity.to_json();
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
                    "created_ms": p.created_ms,
                })
            })
            .collect();
        let agents: serde_json::Map<String, Value> = Agent::ALL
            .into_iter()
            .map(|a| {
                let mut v = json!({ "label": a.label(), "capabilities": a.capabilities() });
                if a.capabilities().limits {
                    v["limits"] = json!({ "data": self.limits, "error": self.limits_error });
                }
                (a.id().to_string(), v)
            })
            .collect();
        json!({
            "version": 2,
            "agents": agents,
            "sessions": sessions,
            "pending": pending,
            "usage": { "claude": self.usage.summary(&utc_day(now_ms())) },
            "chat": self.chat.to_json(),
        })
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
        let mut out = snap;
        out["updated_ms"] = json!(now);
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
    if let Ok(mut line) = serde_json::to_string(reply) {
        line.push('\n');
        let _ = stream.write_all(line.as_bytes());
    }
}

fn handle_hook(mut stream: UnixStream, shared: &Shared, agent: Agent, payload: Value) {
    let Some(ev) = agent.normalize(&payload) else { return };
    let rx = {
        let mut st = lock(shared);
        let now = now_ms();
        let sid = st.sessions.apply_event(&ev, now);
        if let Some(path) = st.sessions.map.get(&sid).and_then(|s| s.transcript.clone()) {
            st.usage.refresh(&path);
        }
        match ev.kind {
            EventKind::ToolEnd { .. } | EventKind::Prompt | EventKind::Stop { .. } | EventKind::SessionEnd => st.drop_pending_for(&sid),
            EventKind::ToolStart => st.drop_plans_for(&sid),
            _ => {}
        }
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
                // A plan has nothing to decide, so nothing waits; the others wait for the notch.
                let (tx, rx) = if kind == Kind::Plan {
                    (None, None)
                } else {
                    let (tx, rx) = mpsc::channel();
                    (Some(tx), Some(rx))
                };
                let tool_input = if kind == Kind::Question { input } else { Value::Null };
                st.pending.push(Pending { id, agent, session_id: sid, tool: tool_summary, tool_name, detail, kind, tool_input, created_ms: now, tx });
                rx.map(|rx| (id, rx))
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
    st.pending.retain(|p| p.id != id);
    st.publish();
    drop(st);
    if let Some((d, updated_input)) = decision {
        write_reply(&mut stream, &Reply { decision: Some(d), updated_input, ..Reply::ok() });
    }
}

/// Allow or deny a pending permission.
fn handle_decision(shared: &Shared, id: u64, decision: Decision) -> Reply {
    resolve(shared, id, |p| match p.kind {
        Kind::Permission => Ok((decision, None)),
        Kind::Question => Err("this is a question: answer it (sushi answer) or answer it in the terminal".into()),
        Kind::Plan => Err("a plan is approved in the terminal".into()),
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
        Ok((Decision::Allow, Some(input)))
    })
}

/// Find the pending request, let `decide` produce the verdict, and wake the waiting hook with it.
fn resolve(shared: &Shared, id: u64, decide: impl FnOnce(&Pending) -> Result<Verdict, String>) -> Reply {
    let mut st = lock(shared);
    let Some(pos) = st.pending.iter().position(|p| p.id == id) else {
        return Reply::err(format!("no pending request with id {id}"));
    };
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
    }
    st.publish();
    Reply::ok()
}

fn chat_dir() -> std::path::PathBuf {
    chat::cache_dir().join("chat")
}

fn chat_file() -> std::path::PathBuf {
    chat::cache_dir().join("chat.json")
}

/// Start a chat turn in the background; the answer streams into the shared state. `agent` and
/// `model` override the configured ones.
fn chat_send(shared: &Shared, text: &str, model: Option<String>, agent: Option<String>) -> Reply {
    let text: String = text.trim().chars().take(chat::MAX_INPUT_CHARS).collect();
    if text.is_empty() {
        return Reply::err("empty message");
    }
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
        let args = chat::build_args(agent, &st.chat.saved.session_id, st.chat.saved.started, model.as_deref(), &text);
        let was_started = st.chat.saved.started;
        st.chat.begin_turn(&text);
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
        Ok(Request::Approve { id }) => write_reply(&mut stream, &handle_decision(&shared, id, Decision::Allow)),
        Ok(Request::Deny { id }) => write_reply(&mut stream, &handle_decision(&shared, id, Decision::Deny)),
        Ok(Request::State) => {
            let snap = lock(&shared).snapshot();
            write_reply(&mut stream, &Reply { state: Some(snap), ..Reply::ok() });
        }
        Ok(Request::Answer { id, answers }) => write_reply(&mut stream, &handle_answer(&shared, id, answers)),
        Ok(Request::ChatSend { text, model, agent }) => write_reply(&mut stream, &chat_send(&shared, &text, model, agent)),
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
            st.publish();
        }
        thread::sleep(Duration::from_secs(2));
    }
}

/// Keep plan limits (5h / weekly) fresh. Disabled with `SUSHI_NO_LIMITS=1`.
fn limits_loop(shared: Shared) {
    let creds = claude_dir().join(".credentials.json");
    loop {
        let result = limits::fetch(&creds);
        let wait = {
            let mut st = lock(&shared);
            let wait = match result {
                Ok(v) => {
                    st.limits = Some(limits::parse(&v, now_ms()));
                    st.limits_error = None;
                    LIMITS_REFRESH
                }
                Err(e) => {
                    st.limits_error = Some(e.to_string());
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
    let mut state = State { config, chat: Chat::load(&chat_file()), ..State::default() };
    state.sessions.hide_code = !state.config.show_code;
    state.sessions.ignore_cwd = Some(chat_dir().to_string_lossy().into_owned());
    let shared: Shared = Arc::new(Mutex::new(state));
    if std::env::var_os("SUSHI_NO_LIMITS").is_none() {
        let s = shared.clone();
        thread::spawn(move || limits_loop(s));
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
    }
    Ok(())
}

fn usage() -> ! {
    eprintln!("usage: sushi <daemon | state | approve ID | deny ID | answer ID JSON | chat TEXT [--model M] [--agent A] | chat-stop | chat-clear | install [--agent claude|codex|opencode|pi|all] [--write]>");
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
            // sushi chat <text...> [--model <alias>] [--agent <id>]
            let mut model = None;
            let mut agent = None;
            let mut words = Vec::new();
            let mut it = rest.iter();
            while let Some(w) = it.next() {
                if *w == "--model" {
                    model = it.next().map(|m| m.to_string());
                } else if *w == "--agent" {
                    agent = it.next().map(|m| m.to_string());
                } else {
                    words.push(*w);
                }
            }
            client(&Request::ChatSend { text: words.join(" "), model, agent })
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
        [cmd @ ("approve" | "deny"), id] => match id.parse::<u64>() {
            Ok(id) => {
                let req = if *cmd == "approve" { Request::Approve { id } } else { Request::Deny { id } };
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
