//! GitHub Copilot CLI. Its hooks are registered in `~/.copilot/hooks/sushi.json` with
//! Claude-style event names (`SessionStart`, `PreToolUse`...), which makes it send
//! `snake_case` payloads. Two quirks, both seen on the real CLI (1.0.89):
//!
//! - `PermissionRequest` arrives in `camelCase` (`hookName`, `sessionId`, `toolName`,
//!   `toolInput`), and its answer is a bare `{"behavior": "allow" | "deny", "message"}`.
//! - There is no tool call id, the result is `tool_result: {result_type, text_result_for_llm}`
//!   and file changes (`Edit`) travel as an `apply_patch` patch in a plain string.

use super::{Tool, codex, count_lines, edits_lines, response_text, str_of};
use serde_json::{Map, Value, json};

/// Rewrite a Copilot payload into the keys the neutral envelope reads.
pub fn canonical(payload: &Value) -> Value {
    let Some(src) = payload.as_object() else { return payload.clone() };
    let mut out = Map::new();
    for (k, v) in src {
        let key = match k.as_str() {
            "sessionId" => "session_id",
            "toolName" => "tool_name",
            "toolInput" | "toolArgs" => "tool_input",
            "transcriptPath" => "transcript_path",
            "notificationType" => "notification_type",
            other => other,
        };
        out.entry(key.to_string()).or_insert_with(|| v.clone());
    }
    // `permissionRequest` has a hook name instead of an event name.
    if !out.contains_key("hook_event_name") {
        let name = src.get("hookName").or_else(|| src.get("hookEventName")).and_then(Value::as_str).unwrap_or("");
        let event = match name {
            "userPromptSubmitted" => "UserPromptSubmit".to_string(),
            "agentStop" => "Stop".to_string(),
            "" => String::new(),
            n => n[..1].to_ascii_uppercase() + &n[1..],
        };
        if !event.is_empty() {
            out.insert("hook_event_name".into(), json!(event));
        }
    }
    // The result: its text becomes the tool response, a failure becomes a failed tool end.
    if let Some(result) = src.get("tool_result").or_else(|| src.get("toolResult")) {
        let text = str_of(result, "text_result_for_llm").or_else(|| str_of(result, "textResultForLlm")).unwrap_or("");
        out.insert("tool_response".into(), json!(text));
        let kind = str_of(result, "result_type").or_else(|| str_of(result, "resultType")).unwrap_or("success");
        if kind != "success" && out.get("hook_event_name").and_then(Value::as_str) == Some("PostToolUse") {
            out.insert("hook_event_name".into(), json!("PostToolUseFailure"));
        }
    }
    Value::Object(out)
}

/// `(old, new)` text of each hunk of a unified diff.
pub fn parse_unified_diff(diff: &str) -> Vec<(String, String)> {
    let mut hunks = Vec::new();
    let (mut old, mut new) = (Vec::<&str>::new(), Vec::<&str>::new());
    let mut in_hunk = false;
    fn flush(hunks: &mut Vec<(String, String)>, old: &mut Vec<&str>, new: &mut Vec<&str>) {
        if !old.is_empty() || !new.is_empty() {
            hunks.push((old.join("\n"), new.join("\n")));
        }
        old.clear();
        new.clear();
    }
    for line in diff.lines() {
        if line.starts_with("@@") {
            flush(&mut hunks, &mut old, &mut new);
            in_hunk = true;
        } else if line.starts_with("diff ") {
            flush(&mut hunks, &mut old, &mut new);
            in_hunk = false;
        } else if !in_hunk {
            continue; // headers: index, ---, +++, new file mode...
        } else if let Some(l) = line.strip_prefix('+') {
            new.push(l);
        } else if let Some(l) = line.strip_prefix('-') {
            old.push(l);
        } else if let Some(l) = line.strip_prefix(' ') {
            old.push(l);
            new.push(l);
        }
    }
    flush(&mut hunks, &mut old, &mut new);
    hunks
}

pub fn tool(name: &str, input: &Value, response: Option<&Value>) -> Tool {
    let lname = name.to_ascii_lowercase();
    let s = |k: &str| str_of(input, k).map(str::to_string);
    let kind = match lname.as_str() {
        "bash" | "shell" | "powershell" => "run",
        "edit" | "create" | "write" | "str_replace_editor" | "apply_patch" | "multiedit" => "write",
        "read" | "view" | "grep" | "glob" | "search" | "ls" => "read",
        "web_fetch" | "webfetch" | "web_search" | "websearch" => "search",
        "task" | "agent" | "delegate" => "think",
        _ => "other",
    };
    // File changes made as a patch (what `Edit` sends in its `PreToolUse`).
    if kind == "write" && let Some(text) = codex::patch_text(input) {
        return codex::patch_tool(name, input, text);
    }
    let mut t = Tool::new(name, kind, input);
    match (kind, lname.as_str()) {
        ("write", _) => {
            t.path = s("file_path").or_else(|| s("path"));
            if let Some(diff) = s("diff") {
                t.edits = parse_unified_diff(&diff);
            } else if let (Some(old), Some(new)) = (s("old_str").or_else(|| s("old_string")), s("new_str").or_else(|| s("new_string"))) {
                t.edits = vec![(old, new)];
            } else {
                t.content = s("file_text").or_else(|| s("content"));
            }
            (t.added, t.removed) = edits_lines(&t.edits);
            if let Some(c) = &t.content {
                t.added = count_lines(c);
            }
        }
        ("read", "read" | "view") => {
            t.path = s("path").or_else(|| s("file_path"));
            t.offset = input.pointer("/view_range/0").and_then(Value::as_u64).map(|o| o as u32);
        }
        ("read", _) => {
            t.pattern = s("pattern").or_else(|| s("query"));
            t.search_path = s("paths").or_else(|| s("path"));
        }
        ("run", _) => {
            t.command = s("command");
            t.output = response.map(response_text);
        }
        ("search", "web_fetch" | "webfetch") => t.url = s("url"),
        ("search", _) => t.pattern = s("query"),
        ("think", _) => {
            t.description = s("description").or_else(|| s("name"));
            t.prompt = s("prompt");
        }
        _ => {}
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    // Payloads recorded from the real CLI.
    const PERMISSION: &str = r#"{"hookName":"permissionRequest","sessionId":"s1","timestamp":1790876501106,"cwd":"/p","toolName":"bash","toolInput":{"command":"echo hi"},"permissionSuggestions":[]}"#;
    const POST: &str = r#"{"hook_event_name":"PostToolUse","session_id":"s1","cwd":"/p","tool_name":"Bash","tool_input":{"command":"echo hi","description":"x"},"tool_result":{"result_type":"success","text_result_for_llm":"hi\n<shellId: 0 completed with exit code 0>"}}"#;

    #[test]
    fn the_camel_case_permission_request_is_canonicalized() {
        let v = canonical(&serde_json::from_str(PERMISSION).unwrap());
        assert_eq!(v["hook_event_name"], "PermissionRequest");
        assert_eq!((v["session_id"].as_str(), v["tool_name"].as_str()), (Some("s1"), Some("bash")));
        assert_eq!(v["tool_input"]["command"], "echo hi");
    }

    #[test]
    fn results_become_responses_and_failures_become_failed_ends() {
        let v = canonical(&serde_json::from_str(POST).unwrap());
        assert!(v["tool_response"].as_str().unwrap().starts_with("hi\n"));
        assert_eq!(v["hook_event_name"], "PostToolUse");
        let mut failed: Value = serde_json::from_str(POST).unwrap();
        failed["tool_result"]["result_type"] = json!("failure");
        assert_eq!(canonical(&failed)["hook_event_name"], "PostToolUseFailure");
    }

    #[test]
    fn edit_patches_and_permission_diffs_are_both_understood() {
        let patch = json!("*** Begin Patch\n*** Update File: a.txt\n@@\n-one\n-two\n+one\n+three\n*** End Patch\n");
        let t = tool("Edit", &patch, None);
        assert_eq!((t.kind, t.path.as_deref(), t.added, t.removed), ("write", Some("a.txt"), 2, 2));
        let diff = "\ndiff --git a/x/a.txt b/x/a.txt\nnew file mode 100644\nindex 0000000..1\n--- /dev/null\n+++ b/x/a.txt\n@@ -0,0 +1,2 @@\n+one\n+two\n";
        let t = tool("edit", &json!({"file_path": "/x/a.txt", "diff": diff}), None);
        assert_eq!((t.path.as_deref(), t.edits.clone()), (Some("/x/a.txt"), vec![(String::new(), "one\ntwo".to_string())]));
        assert_eq!((t.added, t.removed), (2, 0));
    }

    #[test]
    fn the_other_tools() {
        let t = tool("Read", &json!({"path": "/p/a.txt"}), None);
        assert_eq!((t.kind, t.path.as_deref()), ("read", Some("/p/a.txt")));
        let t = tool("Grep", &json!({"pattern": "three", "paths": ".", "output_mode": "content"}), None);
        assert_eq!((t.pattern.as_deref(), t.search_path.as_deref(), t.path.clone()), (Some("three"), Some("."), None));
        let t = tool("Bash", &json!({"command": "ls"}), Some(&json!("out")));
        assert_eq!((t.kind, t.command.as_deref(), t.output.as_deref()), ("run", Some("ls"), Some("out")));
    }
}
