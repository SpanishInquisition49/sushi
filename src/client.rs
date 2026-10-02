//! One request to the running daemon over its local socket (used by the CLI and the desktop app).

use crate::ipc::UnixStream;
use crate::paths::socket_path;
use crate::protocol::{Reply, Request};
use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

/// Follow the daemon's state: `on_state` gets it at once and then on every change, until the
/// connection drops (`Ok`) or the daemon refuses to be watched (an older one: `Err`).
pub fn watch(mut on_state: impl FnMut(serde_json::Value)) -> Result<(), String> {
    let mut stream = UnixStream::connect(socket_path()).map_err(|e| format!("daemon not running ({e})"))?;
    let mut line = serde_json::to_string(&Request::Watch).map_err(|e| e.to_string())?;
    line.push('\n');
    stream.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        let reply: Reply = serde_json::from_str(&line).map_err(|e| format!("bad reply: {e}"))?;
        match reply.state {
            Some(state) => on_state(state),
            None => return Err(reply.error.unwrap_or_else(|| "the daemon cannot be watched".into())),
        }
    }
    Ok(())
}

/// Send `req` and wait up to `timeout` for the reply line.
pub fn request(req: &Request, timeout: Duration) -> Result<Reply, String> {
    let mut stream = UnixStream::connect(socket_path()).map_err(|e| format!("daemon not running ({e})"))?;
    stream.set_read_timeout(Some(timeout)).ok();
    let mut line = serde_json::to_string(req).map_err(|e| e.to_string())?;
    line.push('\n');
    stream.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).map_err(|e| e.to_string())?;
    serde_json::from_str(&reply).map_err(|e| format!("bad reply: {e}"))
}
