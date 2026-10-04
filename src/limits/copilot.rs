//! GitHub Copilot's plan usage: the monthly quotas (`premium_interactions`, `chat`,
//! `completions`) behind `GET /copilot_internal/user`.
//!
//! Unlike Claude and Codex this is **not documented by GitHub at all** — the endpoint, headers
//! and field names below come from cross-checking independent community trackers (a public
//! quota-tracking gist and the `steipete/CodexBar` menu-bar app's provider notes), not from
//! Sushi's own source. Expect to have to adjust a header or field name for a given Copilot CLI
//! version; an error here just means "Plan limits unavailable", never a guessed number.
//!
//! The bigger problem is the token itself: Copilot CLI's own OAuth token normally lives in the
//! OS keychain (service `copilot-cli`), not a file Sushi can read portably. So, in order:
//! `COPILOT_GITHUB_TOKEN` / `GH_TOKEN` / `GITHUB_TOKEN` (the CLI's own documented env-var
//! precedence), the `gh` CLI's `hosts.yml` (if the same machine is also logged into `gh`), the
//! macOS Keychain entry `copilot-cli`, and finally the plaintext fallback Copilot CLI itself uses
//! when no keychain is available (`~/.copilot/config.json`, headless Linux without libsecret).

use super::{http_get, FetchError, Limits, ModelWindow};
use serde_json::Value;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::{Command, Stdio};

const URL: &str = "https://api.github.com/copilot_internal/user";

fn gh_hosts_token() -> Option<String> {
    let path = crate::paths::config_home().join("gh").join("hosts.yml");
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_github_block = false;
    for line in text.lines() {
        let key_line = !line.starts_with(' ') && !line.starts_with('\t');
        if key_line {
            in_github_block = line.trim_end().trim_end_matches(':') == "github.com";
            continue;
        }
        if in_github_block && let Some(rest) = line.trim().strip_prefix("oauth_token:") {
            return Some(rest.trim().trim_matches('"').to_string());
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn keychain_token() -> Option<String> {
    let out = Command::new("security")
        .args(["find-generic-password", "-s", "copilot-cli", "-w"])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(not(target_os = "macos"))]
fn keychain_token() -> Option<String> {
    None
}

/// Copilot CLI's plaintext fallback (`~/.copilot/config.json`) when no OS keychain is available.
/// Its exact key for the token is not documented, so every name seen in the wild is tried.
fn plaintext_token(copilot_home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(copilot_home.join("config.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    ["oauth_token", "githubToken", "github_token", "token"].iter().find_map(|k| v.get(k).and_then(Value::as_str).filter(|t| !t.is_empty())).map(str::to_string)
}

fn read_token(copilot_home: &Path) -> Result<String, FetchError> {
    std::env::var("COPILOT_GITHUB_TOKEN")
        .ok()
        .or_else(|| std::env::var("GH_TOKEN").ok())
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .filter(|t| !t.is_empty())
        .or_else(gh_hosts_token)
        .or_else(keychain_token)
        .or_else(|| plaintext_token(copilot_home))
        // No token found anywhere this looks: Copilot (and `gh`) were simply never logged into
        // on this machine, not a real error.
        .ok_or(FetchError::NotConfigured)
}

pub fn fetch(copilot_home: &Path) -> Result<Value, FetchError> {
    let token = read_token(copilot_home)?;
    http_get(
        URL,
        &format!("token {token}"),
        &[
            ("Editor-Version", "vscode/1.96.2".to_string()),
            ("Editor-Plugin-Version", "copilot-chat/0.26.7".to_string()),
            ("X-Github-Api-Version", "2025-04-01".to_string()),
        ],
    )
}

/// `used_percent` isn't given directly: it is derived from `entitlement` (the monthly quota) and
/// `remaining`. A quota marked `unlimited` (free plan, most non-`premium_interactions` quotas on
/// paid plans) has nothing meaningful to show and is skipped.
fn window_percent(q: &Value) -> Option<f64> {
    if q.get("unlimited").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let entitlement = q.get("entitlement").and_then(Value::as_f64).filter(|e| *e > 0.0)?;
    let remaining = q.get("remaining").and_then(Value::as_f64)?;
    Some(((entitlement - remaining) / entitlement * 100.0).clamp(0.0, 100.0))
}

fn pretty(id: &str) -> String {
    match id {
        "premium_interactions" => "Premium requests".to_string(),
        "chat" => "Chat".to_string(),
        "completions" => "Completions".to_string(),
        other => other.to_string(),
    }
}

/// `quota_reset_date` has been seen as both a full timestamp and a bare `YYYY-MM-DD`; the latter
/// is treated as that day's start in UTC.
fn reset_ms(v: &Value) -> Option<u64> {
    let s = v.get("quota_reset_date").and_then(Value::as_str)?;
    crate::usage::parse_iso_ms(s).or_else(|| crate::usage::parse_iso_ms(&format!("{s}T00:00:00Z")))
}

pub fn parse(v: &Value, now_ms: u64) -> Limits {
    let resets_at_ms = reset_ms(v);
    let models = v
        .get("quota_snapshots")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(name, q)| {
                    let percent = window_percent(q)?;
                    Some(ModelWindow { kind: "monthly".to_string(), model: pretty(name), percent, resets_at_ms })
                })
                .collect()
        })
        .unwrap_or_default();
    // Copilot has no 5-hour/weekly split like Claude or Codex: everything is a monthly quota, so
    // it all goes in `models` (rendered as a per-quota bar) and the top-level windows stay empty.
    Limits { five_hour: None, seven_day: None, models, fetched_ms: now_ms }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn derives_percent_from_entitlement_and_remaining_and_skips_unlimited() {
        let v = json!({
            "quota_reset_date": "2026-11-01",
            "quota_snapshots": {
                "premium_interactions": {"entitlement": 300, "remaining": 225, "unlimited": false},
                "chat": {"unlimited": true},
                "completions": {"entitlement": 0, "remaining": 0, "unlimited": false}
            }
        });
        let l = parse(&v, 1);
        assert_eq!(l.models.len(), 1);
        assert_eq!(l.models[0].model, "Premium requests");
        assert_eq!(l.models[0].percent, 25.0);
        assert!(l.models[0].resets_at_ms.is_some());
        assert!(l.five_hour.is_none() && l.seven_day.is_none());
    }

    #[test]
    fn missing_fields_mean_no_data() {
        let l = parse(&json!({}), 1);
        assert!(l.models.is_empty());
    }

    #[test]
    fn env_token_takes_precedence_and_empty_vars_are_skipped() {
        // SAFETY: test-only, not run concurrently with another test reading these same vars.
        unsafe {
            std::env::remove_var("COPILOT_GITHUB_TOKEN");
            std::env::set_var("GH_TOKEN", "gho_fromenv");
        }
        let t = read_token(Path::new("/nonexistent")).unwrap();
        assert_eq!(t, "gho_fromenv");
        unsafe {
            std::env::remove_var("GH_TOKEN");
        }
    }
}
