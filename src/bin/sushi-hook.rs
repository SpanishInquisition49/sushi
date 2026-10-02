//! The hook every agent calls: forwards its event JSON to the daemon.
//!
//! `sushi-hook [--agent claude|codex|opencode|pi|copilot|antigravity]` (Claude Code if omitted).
//!
//! Never blocks the agent: if the daemon is not running (or anything fails) it exits 0
//! without output. Only `PermissionRequest` waits for an answer, up to
//! `SUSHI_PERMISSION_TIMEOUT_SECS` (default 30), or `SUSHI_PLAN_TIMEOUT_SECS` (default 300) for a
//! Claude Code plan; on timeout it prints nothing, so the agent falls back to its normal
//! permission flow.

use sushi::agent::Agent;
use sushi::paths::socket_path;
use sushi::protocol::{Reply, Request};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use sushi::ipc::UnixStream;
use std::time::Duration;

/// The agent named by `--agent <id>` (or `--agent=<id>`), Claude Code by default.
fn agent_from_args() -> Option<Agent> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let id = args.iter().enumerate().find_map(|(i, a)| match a.strip_prefix("--agent=") {
        Some(id) => Some(id.to_string()),
        None if a == "--agent" => args.get(i + 1).cloned(),
        None => None,
    });
    match id {
        Some(id) => Agent::from_id(&id),
        None => Some(Agent::Claude),
    }
}

fn run() -> Option<()> {
    let agent = agent_from_args()?;
    // The built-in chat runs `claude` with this set: its events are not sessions to watch.
    if std::env::var_os("SUSHI_CHAT").is_some() {
        return Some(());
    }
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).ok()?;
    let mut payload: Value = serde_json::from_str(&input).ok()?;
    // Antigravity names no process: the agent's own lets the daemon drop the session once `agy` quits
    // (it could be left "waiting" for hours otherwise).
    if agent == Agent::Antigravity
        && payload.get("pid").is_none()
        && let (Some(obj), Some(pid)) = (payload.as_object_mut(), agent_pid())
    {
        obj.insert("pid".into(), pid.into());
    }
    // Only a permission (or a Claude Code plan) can be answered by the notch.
    let wants_answer = agent.wants_answer(&payload);
    let is_plan = agent.is_plan(&payload);

    let mut stream = UnixStream::connect(socket_path()).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut line = serde_json::to_string(&Request::Hook { agent: agent.id().to_string(), payload }).ok()?;
    line.push('\n');
    stream.write_all(line.as_bytes()).ok()?;

    if wants_answer {
        // Claude Code shows the plan dialog in the terminal while this runs, so a longer wait
        // blocks nothing.
        let (var, default) = if is_plan { ("SUSHI_PLAN_TIMEOUT_SECS", 300) } else { ("SUSHI_PERMISSION_TIMEOUT_SECS", 30) };
        let secs = std::env::var(var).ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(default);
        stream.set_read_timeout(Some(Duration::from_secs(secs))).ok()?;
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).ok()?;
        let reply = serde_json::from_str::<Reply>(&reply).ok()?;
        println!("{}", agent.encode_decision(reply.decision?, reply.updated_input.as_ref(), reply.mode.as_deref()));
    }
    Some(())
}

/// The agent that started this hook: the first ancestor that is not a shell (agents may run hooks
/// through `sh -c`).
#[cfg(target_os = "linux")]
fn agent_pid() -> Option<u32> {
    const WRAPPERS: [&str; 9] = ["sh", "bash", "zsh", "dash", "fish", "ash", "busybox", "env", "timeout"];
    let mut pid = std::os::unix::process::parent_id();
    for _ in 0..8 {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // `pid (comm) state ppid ...`: comm may hold spaces and parentheses.
        let (head, rest) = stat.rsplit_once(')')?;
        let comm = head.split_once('(')?.1;
        if !WRAPPERS.contains(&comm) {
            return Some(pid);
        }
        pid = rest.split_whitespace().nth(1)?.parse().ok()?;
        if pid <= 1 {
            return None;
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn agent_pid() -> Option<u32> {
    None
}

fn main() {
    let _ = run();
}
