//! Antigravity's model quotas: the same numbers its own `/usage` (alias `/quota`) panel shows.
//!
//! This one is the least certain of the four. Antigravity reuses Google's internal "Cloud Code
//! Assist" backend, not a documented Antigravity API: the endpoint, request and field names below
//! come entirely from `steipete/CodexBar`'s reverse-engineered provider notes (a real, maintained
//! menu-bar app that tracks this across several agents) — not from anything in Sushi's own
//! testing, and that project's own issue tracker shows this has broken across `agy` versions
//! before (a required client-identifying header changed, a local CSRF token got enforced). Expect
//! this to need a fix after an `agy` update; until then a failure here is just "Plan limits
//! unavailable", same as everywhere else in this module.
//!
//! Credentials: `agy`'s own OAuth file, tried in the order real installs have been seen to use
//! it — `~/.gemini/antigravity-cli/antigravity-oauth-token` (its native, SSH/headless-friendly
//! location) and, failing that, `~/.gemini/oauth_creds.json` (the format Antigravity shares with
//! the official Gemini CLI, since both are built on the same Google OAuth stack).

use super::{http_post, FetchError, Limits, ModelWindow};
use crate::usage::parse_iso_ms;
use serde_json::Value;
use std::path::Path;

const URL: &str = "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary";

fn read_token(gemini_home: &Path) -> Result<String, FetchError> {
    for rel in ["antigravity-cli/antigravity-oauth-token", "oauth_creds.json"] {
        let Ok(text) = std::fs::read_to_string(gemini_home.join(rel)) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        if let Some(t) = ["access_token", "token"].iter().find_map(|k| v.get(k).and_then(Value::as_str).filter(|t| !t.is_empty())) {
            return Ok(t.to_string());
        }
    }
    // Neither candidate file is readable with a usable token in it. Given the low confidence in
    // these exact paths/fields to begin with, this is treated as "not set up" rather than "set
    // up but broken" either way — see `FetchError::NotConfigured`.
    Err(FetchError::NotConfigured)
}

pub fn fetch(gemini_home: &Path) -> Result<Value, FetchError> {
    let token = read_token(gemini_home)?;
    http_post(URL, &format!("Bearer {token}"), &[], "{}")
}

/// Each `groups[].buckets[]` is one window (5-hour, weekly, ...) for one model family;
/// `remainingFraction` is how much quota is *left*, so the percent *used* is its complement.
pub fn parse(v: &Value, now_ms: u64) -> Limits {
    let models: Vec<ModelWindow> = v
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|g| {
            let group = g.get("name").or_else(|| g.get("displayName")).and_then(Value::as_str).unwrap_or("").to_string();
            let buckets = g.get("buckets").and_then(Value::as_array).cloned().unwrap_or_default();
            buckets.into_iter().filter_map(move |b| {
                let frac = b.get("remainingFraction").and_then(Value::as_f64)?;
                let percent = ((1.0 - frac) * 100.0).clamp(0.0, 100.0);
                let resets_at_ms = b.get("resetTime").and_then(Value::as_str).and_then(parse_iso_ms);
                let kind = b.get("description").and_then(Value::as_str).unwrap_or("").chars().take(24).collect();
                Some(ModelWindow { kind, model: group.clone(), percent, resets_at_ms })
            })
        })
        .collect();
    Limits { five_hour: None, seven_day: None, models, fetched_ms: now_ms }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn remaining_fraction_is_inverted_into_a_used_percent() {
        let v = json!({
            "groups": [
                {"name": "Gemini 3 Pro", "buckets": [
                    {"remainingFraction": 0.75, "resetTime": "2026-10-05T00:00:00Z", "description": "5-hour window"},
                    {"remainingFraction": 0.4, "resetTime": null, "description": "Weekly window"}
                ]}
            ]
        });
        let l = parse(&v, 1);
        assert_eq!(l.models.len(), 2);
        assert_eq!(l.models[0].model, "Gemini 3 Pro");
        assert_eq!(l.models[0].percent, 25.0);
        assert!(l.models[0].resets_at_ms.is_some());
        assert_eq!(l.models[1].percent, 60.0);
        assert!(l.five_hour.is_none() && l.seven_day.is_none());
    }

    #[test]
    fn missing_groups_means_no_data() {
        let l = parse(&json!({}), 1);
        assert!(l.models.is_empty());
    }

    #[test]
    fn missing_credentials_is_not_configured_not_a_panic() {
        let e = read_token(Path::new("/nonexistent")).unwrap_err();
        assert!(matches!(e, FetchError::NotConfigured));
    }
}
