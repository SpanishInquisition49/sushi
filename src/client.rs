//! One request to the running daemon over its local socket (used by the CLI and the desktop app).

use crate::ipc::UnixStream;
use crate::paths::socket_path;
use crate::protocol::{Reply, Request};
use std::io::{BufRead, BufReader, Write};
use std::time::Duration;

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
