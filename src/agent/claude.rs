//! Claude Code: hook JSON on stdin, `hookSpecificOutput` on stdout.

use super::{Role, Tool, count_lines, edits_lines, response_text, str_of};
use serde_json::Value;

pub fn tool(name: &str, input: &Value, response: Option<&Value>) -> Tool {
    let s = |k: &str| str_of(input, k).map(str::to_string);
    let kind = match name {
        "Read" | "Grep" | "Glob" | "LS" | "NotebookRead" => "read",
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => "write",
        "Bash" | "BashOutput" | "KillShell" => "run",
        "WebFetch" | "WebSearch" => "search",
        "Task" | "Agent" | "TodoWrite" => "think",
        _ => "other",
    };
    let mut t = Tool::new(name, kind, input);
    match name {
        "Read" | "NotebookRead" => {
            t.path = s("file_path").or_else(|| s("notebook_path"));
            t.offset = input.get("offset").and_then(Value::as_u64).map(|o| o as u32);
        }
        "Edit" => {
            t.path = s("file_path");
            t.edits = vec![(s("old_string").unwrap_or_default(), s("new_string").unwrap_or_default())];
            (t.added, t.removed) = edits_lines(&t.edits);
        }
        "MultiEdit" => {
            t.path = s("file_path");
            t.edits = input
                .get("edits")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .map(|e| (str_of(e, "old_string").unwrap_or("").to_string(), str_of(e, "new_string").unwrap_or("").to_string()))
                        .collect()
                })
                .unwrap_or_default();
            (t.added, t.removed) = edits_lines(&t.edits);
        }
        "Write" => {
            t.path = s("file_path");
            t.content = s("content");
            t.added = t.content.as_deref().map(count_lines).unwrap_or(0);
        }
        "NotebookEdit" => {
            t.path = s("notebook_path").or_else(|| s("file_path"));
            t.added = str_of(input, "new_source").map(count_lines).unwrap_or(0);
        }
        "Grep" | "Glob" => {
            t.pattern = s("pattern");
            t.search_path = s("path");
        }
        "Bash" => t.command = s("command"),
        "WebFetch" => t.url = s("url"),
        "WebSearch" => t.pattern = s("query"),
        "Task" | "Agent" => {
            t.description = s("description");
            t.prompt = s("prompt");
        }
        "AskUserQuestion" => t.role = Role::Question,
        "ExitPlanMode" => {
            t.role = Role::Plan;
            t.plan = s("plan");
        }
        _ => {}
    }
    if let Some(r) = response {
        match name {
            "Bash" => t.output = Some(response_text(r)),
            "Read" => {
                if let Some(content) = r.pointer("/file/content").and_then(Value::as_str) {
                    let start = r.pointer("/file/startLine").and_then(Value::as_u64).unwrap_or(1) as u32;
                    t.read = Some((content.to_string(), start));
                }
            }
            _ => {}
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn edits_count_lines_like_the_requests() {
        let t = tool("Edit", &json!({"file_path": "/p/a.rs", "old_string": "x\ny", "new_string": "x\ny\nz\nw"}), None);
        assert_eq!((t.kind, t.added, t.removed, t.path.as_deref()), ("write", 4, 2, Some("/p/a.rs")));
        let t = tool("MultiEdit", &json!({"file_path": "f", "edits": [
            {"old_string": "a", "new_string": "a\nb"}, {"old_string": "c\nd", "new_string": "e"}]}), None);
        assert_eq!((t.added, t.removed, t.edits.len()), (3, 3, 2));
        let t = tool("Write", &json!({"file_path": "f", "content": "1\n2\n3"}), None);
        assert_eq!((t.added, t.removed), (3, 0));
    }

    #[test]
    fn questions_and_plans_have_a_role() {
        assert_eq!(tool("AskUserQuestion", &json!({}), None).role, Role::Question);
        let t = tool("ExitPlanMode", &json!({"plan": "1. go"}), None);
        assert_eq!((t.role, t.plan.as_deref()), (Role::Plan, Some("1. go")));
        assert_eq!(tool("Bash", &json!({}), None).role, Role::Normal);
    }

    #[test]
    fn responses_become_output_and_read_content() {
        let t = tool("Bash", &json!({"command": "ls"}), Some(&json!({"stdout": "a", "stderr": "b"})));
        assert_eq!(t.output.as_deref(), Some("a\nb"));
        let t = tool("Read", &json!({"file_path": "x"}), Some(&json!({"file": {"content": "a\nb", "startLine": 10}})));
        assert_eq!(t.read, Some(("a\nb".to_string(), 10)));
    }
}
