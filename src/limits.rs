//! Plan usage limits (the percentages `/usage` shows: 5-hour and weekly windows).
//!
//! Source: Anthropic's OAuth usage endpoint, called with the access token Claude
//! Code stores in `~/.claude/.credentials.json`. This is an undocumented API
//! (the same one Claude Code and community plugins use), so every field is
//! parsed defensively and failures just mean "no data".
//!
//! The token is handed to `curl` on stdin (never in argv, so it is not visible in
//! `ps`) and is never logged or written to `state.json`. It is not refreshed
//! here: when it has expired, Claude Code refreshes it on its next use.

use crate::usage::parse_iso_ms;
use serde::Serialize;
use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const URL: &str = "https://api.anthropic.com/api/oauth/usage";

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
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Http(401 | 403, _) => {
                write!(f, "token rejected (HTTP 401/403): use Claude Code once to refresh the login")
            }
            FetchError::Http(c, b) => write!(f, "HTTP {c}: {b}"),
            FetchError::Other(m) => f.write_str(m),
        }
    }
}

fn window(v: &Value) -> Option<Window> {
    let percent = v.get("utilization")?.as_f64()?;
    let resets_at_ms = v.get("resets_at").and_then(Value::as_str).and_then(parse_iso_ms);
    Some(Window { percent, resets_at_ms })
}

pub fn parse(v: &Value, now_ms: u64) -> Limits {
    let models = v
        .get("limits")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|l| {
                    let percent = l.get("percent")?.as_f64()?;
                    let model = l.pointer("/scope/model/display_name").and_then(Value::as_str).unwrap_or("");
                    if model.is_empty() {
                        return None; // only model-scoped windows; the rest duplicates five_hour/seven_day
                    }
                    Some(ModelWindow {
                        kind: l.get("kind").and_then(Value::as_str).unwrap_or("").to_string(),
                        model: model.to_string(),
                        percent,
                        resets_at_ms: l.get("resets_at").and_then(Value::as_str).and_then(parse_iso_ms),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Limits {
        five_hour: v.get("five_hour").and_then(window),
        seven_day: v.get("seven_day").and_then(window),
        models,
        fetched_ms: now_ms,
    }
}

#[cfg(target_os = "macos")]
fn keychain_credentials() -> Option<String> {
    let out = Command::new("security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(not(target_os = "macos"))]
fn keychain_credentials() -> Option<String> {
    None
}

fn read_token(creds: &Path) -> Result<String, FetchError> {
    let text = match std::fs::read_to_string(creds) {
        Ok(t) => t,
        // On macOS Claude Code keeps its credentials in the Keychain, not in a file.
        Err(e) => keychain_credentials()
            .ok_or_else(|| FetchError::Other(format!("cannot read {}: {e}", creds.display())))?,
    };
    let v: Value = serde_json::from_str(&text).map_err(|_| FetchError::Other("credentials file is not valid JSON".into()))?;
    v.pointer("/claudeAiOauth/accessToken")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .ok_or_else(|| FetchError::Other("no OAuth token (not logged in with a Claude account)".into()))
}

pub fn fetch(creds: &Path) -> Result<Value, FetchError> {
    let token = read_token(creds)?;
    let mut child = Command::new("curl")
        .args([
            "-sS", "--max-time", "8", "-w", "\n%{http_code}",
            "-H", "Accept: application/json",
            "-H", "anthropic-beta: oauth-2025-04-20",
            "-H", "User-Agent: sushi/0.1",
            "-H", "@-", // the Authorization header is read from stdin
            URL,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| FetchError::Other(format!("cannot run curl: {e}")))?;
    child
        .stdin
        .take()
        .ok_or_else(|| FetchError::Other("curl stdin unavailable".into()))?
        .write_all(format!("Authorization: Bearer {token}\n").as_bytes())
        .map_err(|e| FetchError::Other(format!("curl stdin: {e}")))?;
    let out = child.wait_with_output().map_err(|e| FetchError::Other(e.to_string()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(FetchError::Other(format!("curl failed: {}", err.lines().next().unwrap_or(""))));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or(("", "0"));
    let code: u16 = code.trim().parse().unwrap_or(0);
    if code != 200 {
        return Err(FetchError::Http(code, body.chars().take(160).collect()));
    }
    serde_json::from_str(body).map_err(|_| FetchError::Other("usage response is not JSON".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_windows_and_model_scopes() {
        let v = json!({
            "five_hour": {"utilization": 37.5, "resets_at": "2026-10-01T18:00:00.000000+00:00"},
            "seven_day": {"utilization": 12.0, "resets_at": null},
            "limits": [
                {"kind": "weekly", "scope": {"model": {"display_name": "Opus"}}, "percent": 61, "resets_at": "2026-10-05T00:00:00Z"},
                {"kind": "session", "percent": 37}
            ]
        });
        let l = parse(&v, 5);
        assert_eq!(l.five_hour.as_ref().unwrap().percent, 37.5);
        assert!(l.five_hour.as_ref().unwrap().resets_at_ms.is_some());
        assert_eq!(l.seven_day.as_ref().unwrap().resets_at_ms, None);
        assert_eq!(l.models.len(), 1);
        assert_eq!(l.models[0].model, "Opus");
        assert_eq!(l.fetched_ms, 5);
    }

    #[test]
    fn missing_fields_mean_no_data() {
        let l = parse(&json!({"unexpected": true}), 1);
        assert!(l.five_hour.is_none() && l.seven_day.is_none() && l.models.is_empty());
    }

    #[test]
    fn missing_credentials_is_an_error_not_a_panic() {
        let e = read_token(Path::new("/nonexistent/creds.json")).unwrap_err();
        assert!(e.to_string().contains("cannot read"));
    }
}
