//! Codex's plan usage when logged in with a ChatGPT account (not an API key): the same 5-hour /
//! weekly windows the `codex` CLI's own status line shows.
//!
//! Source: verified against the open-source `codex-rs` client — `backend-client/src/client.rs`
//! (`token_usage_profile_url`/`PathStyle::ChatGptApi` picks `{base}/wham/usage`),
//! `backend-client/src/client/rate_limit_resets.rs` (`GET {base}/wham/usage`) and the
//! `codex-backend-openapi-models` crate's `RateLimitStatusPayload` / `RateLimitWindowSnapshot`
//! for the field names below. Credentials: `$CODEX_HOME/auth.json` (`~/.codex/auth.json` by
//! default), `tokens.access_token` / `tokens.account_id`.
//!
//! An API-key login (`OPENAI_API_KEY` set, no `tokens` in `auth.json`) has no plan window at all —
//! that is reported the same as "not logged in", never guessed.

use super::{epoch_s_to_ms, http_get, FetchError, Limits, ModelWindow, Window};
use serde_json::Value;
use std::path::Path;

const URL: &str = "https://chatgpt.com/backend-api/wham/usage";

#[derive(Debug)]
struct Token {
    access: String,
    account_id: Option<String>,
}

fn read_token(auth: &Path) -> Result<Token, FetchError> {
    let text = match std::fs::read_to_string(auth) {
        Ok(t) => t,
        // No `auth.json` at all: Codex was never installed or never logged in, not a real error.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(FetchError::NotConfigured),
        Err(e) => return Err(FetchError::Other(format!("cannot read {}: {e}", auth.display()))),
    };
    let v: Value = serde_json::from_str(&text).map_err(|_| FetchError::Other("auth.json is not valid JSON".into()))?;
    let access = v
        .pointer("/tokens/access_token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        // An API-key-only login has an `OPENAI_API_KEY` but no `tokens`: no plan window at all.
        .ok_or(FetchError::NotConfigured)?
        .to_string();
    let account_id = v.pointer("/tokens/account_id").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string);
    Ok(Token { access, account_id })
}

pub fn fetch(codex_home: &Path) -> Result<Value, FetchError> {
    let token = read_token(&codex_home.join("auth.json"))?;
    let mut headers: Vec<(&str, String)> = Vec::new();
    if let Some(id) = token.account_id {
        headers.push(("ChatGPT-Account-Id", id));
    }
    http_get(URL, &format!("Bearer {}", token.access), &headers)
}

/// A `primary_window` / `secondary_window` object: `used_percent`, and a reset given either as
/// `reset_at` (Unix seconds) or, failing that, `reset_after_seconds` from now.
fn window(v: &Value, now_ms: u64) -> Option<Window> {
    let percent = v.get("used_percent")?.as_f64()?;
    let resets_at_ms = v
        .get("reset_at")
        .and_then(epoch_s_to_ms)
        .or_else(|| v.get("reset_after_seconds").and_then(Value::as_i64).filter(|s| *s >= 0).map(|s| now_ms + s as u64 * 1000));
    Some(Window { percent, resets_at_ms })
}

pub fn parse(v: &Value, now_ms: u64) -> Limits {
    let rl = v.get("rate_limit");
    let models = v
        .get("additional_rate_limits")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|l| {
                    let name = l.get("limit_name").and_then(Value::as_str)?;
                    let w = l.pointer("/rate_limit/primary_window").and_then(|w| window(w, now_ms))?;
                    Some(ModelWindow { kind: "limit".to_string(), model: name.to_string(), percent: w.percent, resets_at_ms: w.resets_at_ms })
                })
                .collect()
        })
        .unwrap_or_default();
    Limits {
        five_hour: rl.and_then(|r| r.get("primary_window")).and_then(|w| window(w, now_ms)),
        seven_day: rl.and_then(|r| r.get("secondary_window")).and_then(|w| window(w, now_ms)),
        models,
        fetched_ms: now_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_primary_and_secondary_windows_and_additional_limits() {
        let v = json!({
            "plan_type": "plus",
            "rate_limit": {
                "allowed": true, "limit_reached": false,
                "primary_window": {"used_percent": 23, "limit_window_seconds": 18000, "reset_after_seconds": 5400, "reset_at": 1_900_000_000},
                "secondary_window": {"used_percent": 61, "limit_window_seconds": 604800, "reset_after_seconds": 200000, "reset_at": 1_900_500_000}
            },
            "additional_rate_limits": [
                {"limit_name": "gpt-5.3-codex-spark", "metered_feature": "spark",
                 "rate_limit": {"allowed": true, "limit_reached": false, "primary_window": {"used_percent": 9, "limit_window_seconds": 300, "reset_after_seconds": 120, "reset_at": 1_900_000_100}}}
            ]
        });
        let l = parse(&v, 1_000);
        assert_eq!(l.five_hour.as_ref().unwrap().percent, 23.0);
        assert_eq!(l.five_hour.as_ref().unwrap().resets_at_ms, Some(1_900_000_000_000));
        assert_eq!(l.seven_day.as_ref().unwrap().percent, 61.0);
        assert_eq!(l.models.len(), 1);
        assert_eq!(l.models[0].model, "gpt-5.3-codex-spark");
        assert_eq!(l.models[0].percent, 9.0);
    }

    #[test]
    fn missing_rate_limit_means_no_data_not_a_panic() {
        let l = parse(&json!({"plan_type": "free"}), 1);
        assert!(l.five_hour.is_none() && l.seven_day.is_none() && l.models.is_empty());
    }

    #[test]
    fn falls_back_to_reset_after_seconds_when_reset_at_is_absent() {
        let w = window(&json!({"used_percent": 5, "reset_after_seconds": 10}), 1_000).unwrap();
        assert_eq!(w.resets_at_ms, Some(11_000));
    }

    #[test]
    fn missing_auth_file_is_not_configured_not_a_panic() {
        let e = read_token(Path::new("/nonexistent/auth.json")).unwrap_err();
        assert!(matches!(e, FetchError::NotConfigured));
    }

    #[test]
    fn an_api_key_only_login_has_no_oauth_token() {
        let dir = std::env::temp_dir().join(format!("sushi-codex-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let auth = dir.join("auth.json");
        std::fs::write(&auth, r#"{"OPENAI_API_KEY": "sk-..."}"#).unwrap();
        let e = read_token(&auth).unwrap_err();
        assert!(matches!(e, FetchError::NotConfigured));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
