//! Cross-agent policy flagging: configurable rules that mark a tool call as needing extra
//! attention ("ask" or "deny") in the live viewer, even when the agent itself auto-approved it.
//!
//! This is visibility only, for every agent. Real enforcement (actually forcing a confirmation the
//! agent cannot skip) only exists for Claude Code's own `permissions.ask` / `permissions.deny`
//! (see `install::merge_permissions` and `Config::claude_permissions`): Sushi writes rules there
//! instead of racing the agent's own hook, which would add latency to every tool call and still not
//! be reliable for agents like Antigravity that do not actually honor a hook's decision (see the
//! README).

use crate::agent::Tool;
use serde::{Deserialize, Serialize};

/// One rule: `tool` is an exact tool name or `*` for any; `contains` is matched (case-insensitive)
/// against the command, path, pattern or url of the call, when set.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PolicyRule {
    pub tool: String,
    pub contains: String,
    /// `ask` or `deny` (display only outside Claude Code's native enforcement).
    pub level: String,
    /// Shown in the UI next to the flagged step.
    pub label: String,
}

/// What a matched step carries for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyTag {
    pub level: String,
    pub label: String,
}

fn haystack(tool: &Tool) -> Option<&str> {
    tool.command.as_deref().or(tool.path.as_deref()).or(tool.pattern.as_deref()).or(tool.url.as_deref())
}

/// The first rule that matches `tool`, if any.
pub fn evaluate(rules: &[PolicyRule], tool: &Tool) -> Option<PolicyTag> {
    rules.iter().find_map(|r| {
        let tool_ok = r.tool.is_empty() || r.tool == "*" || r.tool.eq_ignore_ascii_case(&tool.name);
        if !tool_ok {
            return None;
        }
        let text_ok = r.contains.is_empty()
            || haystack(tool).is_some_and(|h| h.to_lowercase().contains(&r.contains.to_lowercase()));
        if !text_ok {
            return None;
        }
        Some(PolicyTag { level: r.level.clone(), label: r.label.clone() })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bash(cmd: &str) -> Tool {
        crate::agent::claude::tool("Bash", &json!({ "command": cmd }), None)
    }

    #[test]
    fn matches_on_tool_and_substring() {
        let rules = vec![PolicyRule { tool: "Bash".into(), contains: "rm -rf".into(), level: "ask".into(), label: "destructive delete".into() }];
        let tag = evaluate(&rules, &bash("rm -rf /tmp/x")).unwrap();
        assert_eq!((tag.level.as_str(), tag.label.as_str()), ("ask", "destructive delete"));
        assert!(evaluate(&rules, &bash("ls -la")).is_none());
        let other = crate::agent::claude::tool("Write", &json!({"file_path": "f"}), None);
        assert!(evaluate(&rules, &other).is_none(), "wrong tool");
    }

    #[test]
    fn wildcard_tool_and_case_insensitive() {
        let rules = vec![PolicyRule { tool: "*".into(), contains: "SUDO".into(), level: "deny".into(), label: "no sudo".into() }];
        assert!(evaluate(&rules, &bash("sudo rm -rf /")).is_some());
    }

    #[test]
    fn empty_contains_matches_any_use_of_the_tool() {
        let rules = vec![PolicyRule { tool: "Bash".into(), contains: String::new(), level: "ask".into(), label: "any shell command".into() }];
        assert!(evaluate(&rules, &bash("echo hi")).is_some());
    }

    #[test]
    fn first_match_wins() {
        let rules = vec![
            PolicyRule { tool: "Bash".into(), contains: "rm".into(), level: "ask".into(), label: "first".into() },
            PolicyRule { tool: "Bash".into(), contains: "rm -rf".into(), level: "deny".into(), label: "second".into() },
        ];
        assert_eq!(evaluate(&rules, &bash("rm -rf x")).unwrap().label, "first");
    }
}
