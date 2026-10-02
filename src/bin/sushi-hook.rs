//! The hook every agent calls: forwards its event JSON to the daemon.
//!
//! `sushi-hook [--agent claude|codex|opencode|pi]` (Claude Code if omitted).
//!
//! Never blocks the agent: if the daemon is not running (or anything fails) it exits 0
//! without output. Only `PermissionRequest` waits for an answer, up to
//! `SUSHI_PERMISSION_TIMEOUT_SECS` (default 30); on timeout it prints nothing, so the agent
//! falls back to its normal permission flow.

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
    let payload: Value = serde_json::from_str(&input).ok()?;
    // Only a permission can be answered by the notch. A plan is approved in the terminal, so
    // there is nothing to wait for: the daemon just shows it.
    let wants_answer = agent.wants_answer(&payload);

    let mut stream = UnixStream::connect(socket_path()).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut line = serde_json::to_string(&Request::Hook { agent: agent.id().to_string(), payload }).ok()?;
    line.push('\n');
    stream.write_all(line.as_bytes()).ok()?;

    if wants_answer {
        let secs = std::env::var("SUSHI_PERMISSION_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(30);
        stream.set_read_timeout(Some(Duration::from_secs(secs))).ok()?;
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).ok()?;
        let reply = serde_json::from_str::<Reply>(&reply).ok()?;
        println!("{}", agent.encode_decision(reply.decision?, reply.updated_input.as_ref()));
    }
    Some(())
}

fn main() {
    let _ = run();
}
