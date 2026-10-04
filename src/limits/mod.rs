//! Plan usage limits: the percentages each agent's own `/usage` (or equivalent) shows, read
//! straight from the same account the agent itself is logged into.
//!
//! Every agent here talks to a different, undocumented endpoint with its own credentials file,
//! so each gets its own submodule. What they share: the [`Limits`]/[`Window`]/[`ModelWindow`]
//! shape the rest of Sushi reads, a [`FetchError`] that never panics, and the `curl`-on-stdin
//! HTTP helpers below (the token is written to curl's stdin, never argv, so it never shows up in
//! `ps`, and it is never logged or written to `state.json`).
//!
//! Confidence differs a lot between agents — see each submodule's doc comment. An error here just
//! means "no data" to the rest of Sushi: a wrong header or field name degrades to "Plan limits
//! unavailable" in the UI, never a fabricated number.

pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod copilot;

use serde::Serialize;
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Window {
    pub percent: f64,
    pub resets_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelWindow {
    pub kind: String,
    pub model: String,
    pub percent: f64,
    pub resets_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Limits {
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    pub models: Vec<ModelWindow>,
    pub fetched_ms: u64,
}

#[derive(Debug)]
pub enum FetchError {
    /// HTTP status and a short body excerpt.
    Http(u16, String),
    Other(String),
    /// No credentials were found anywhere this module looks: the agent simply isn't set up on
    /// this machine (never installed, never logged in). Unlike every other variant this is never
    /// shown as an error — see `main.rs`'s `limits_loop`, which drops it instead of publishing it
    /// — because most people only use one or two of the four agents this module knows about, and
    /// "Codex: cannot read ~/.codex/auth.json" is just noise to someone who has never run Codex.
    NotConfigured,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Http(401 | 403, _) => {
                write!(f, "token rejected (HTTP 401/403): log in with the agent's CLI once to refresh it")
            }
            FetchError::Http(c, b) => write!(f, "HTTP {c}: {b}"),
            FetchError::Other(m) => f.write_str(m),
            FetchError::NotConfigured => f.write_str("not logged in"),
        }
    }
}

/// Unix seconds (as a JSON number or numeric string) to milliseconds. `None` for anything else,
/// including a clearly-bogus negative value.
pub(crate) fn epoch_s_to_ms(v: &Value) -> Option<u64> {
    let s = v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))?;
    (s >= 0).then(|| s as u64 * 1000)
}

/// Run `curl` with `auth_header` (`"Bearer …"` or `"token …"`) on stdin as the `Authorization`
/// header, and the given `args` (which must include the URL and end with no body/auth already
/// set). Returns the parsed JSON body of a 2xx response.
fn curl(args: &[String], auth_header: &str) -> Result<Value, FetchError> {
    let mut child = Command::new("curl")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| FetchError::Other(format!("cannot run curl: {e}")))?;
    child
        .stdin
        .take()
        .ok_or_else(|| FetchError::Other("curl stdin unavailable".into()))?
        .write_all(format!("Authorization: {auth_header}\n").as_bytes())
        .map_err(|e| FetchError::Other(format!("curl stdin: {e}")))?;
    let out = child.wait_with_output().map_err(|e| FetchError::Other(e.to_string()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(FetchError::Other(format!("curl failed: {}", err.lines().next().unwrap_or(""))));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", "0"));
    let code: u16 = code.trim().parse().unwrap_or(0);
    if !(200..300).contains(&code) {
        return Err(FetchError::Http(code, body.chars().take(160).collect()));
    }
    serde_json::from_str(body).map_err(|_| FetchError::Other("response is not JSON".into()))
}

/// `GET url` with `Authorization: <auth_header>` plus `extra_headers`.
pub(crate) fn http_get(url: &str, auth_header: &str, extra_headers: &[(&str, String)]) -> Result<Value, FetchError> {
    let mut args = vec![
        "-sS".to_string(), "--max-time".to_string(), "8".to_string(), "-w".to_string(), "\n%{http_code}".to_string(),
        "-H".to_string(), "Accept: application/json".to_string(),
        "-H".to_string(), "User-Agent: sushi/0.1".to_string(),
    ];
    for (k, v) in extra_headers {
        args.push("-H".into());
        args.push(format!("{k}: {v}"));
    }
    args.push("-H".into());
    args.push("@-".into()); // the Authorization header is read from stdin
    args.push(url.to_string());
    curl(&args, auth_header)
}

/// `POST url` with a JSON `body`, `Authorization: <auth_header>` plus `extra_headers`.
pub(crate) fn http_post(url: &str, auth_header: &str, extra_headers: &[(&str, String)], body: &str) -> Result<Value, FetchError> {
    let mut args = vec![
        "-sS".to_string(), "--max-time".to_string(), "8".to_string(), "-w".to_string(), "\n%{http_code}".to_string(),
        "-X".to_string(), "POST".to_string(),
        "-H".to_string(), "Accept: application/json".to_string(),
        "-H".to_string(), "Content-Type: application/json".to_string(),
        "-H".to_string(), "User-Agent: sushi/0.1".to_string(),
    ];
    for (k, v) in extra_headers {
        args.push("-H".into());
        args.push(format!("{k}: {v}"));
    }
    args.push("-H".into());
    args.push("@-".into());
    args.push("-d".into());
    args.push(body.to_string());
    args.push(url.to_string());
    curl(&args, auth_header)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_seconds_parse_as_number_or_string_and_reject_negative() {
        assert_eq!(epoch_s_to_ms(&Value::from(10)), Some(10_000));
        assert_eq!(epoch_s_to_ms(&Value::from("10")), Some(10_000));
        assert_eq!(epoch_s_to_ms(&Value::from(-1)), None);
        assert_eq!(epoch_s_to_ms(&Value::from("nope")), None);
    }
}
