//! opencode: events come from `integrations/opencode/sushi.ts` (a plugin), which forwards its
//! hooks as Claude-style events carrying opencode's own tool names and arguments.
//! The decision goes back as `{"decision": "allow" | "deny"}`.

use super::{Tool, count_lines, edits_lines, response_text, str_of};
use serde_json::Value;

pub fn tool(name: &str, input: &Value, response: Option<&Value>) -> Tool {
    let s = |k: &str| str_of(input, k).map(str::to_string);
    let kind = match name {
        "read" | "grep" | "glob" | "list" => "read",
        "edit" | "write" | "patch" | "multiedit" => "write",
        "bash" => "run",
        "webfetch" | "websearch" | "codesearch" => "search",
        "task" | "todowrite" | "todoread" => "think",
        _ => "other",
    };
    let mut t = Tool::new(name, kind, input);
    let path = || s("filePath").or_else(|| s("file_path")).or_else(|| s("path"));
    match name {
        "read" => {
            t.path = path();
            t.offset = input.get("offset").and_then(Value::as_u64).map(|o| o as u32 + 1);
        }
        "edit" => {
            t.path = path();
            t.edits = vec![(s("oldString").unwrap_or_default(), s("newString").unwrap_or_default())];
            (t.added, t.removed) = edits_lines(&t.edits);
        }
        "multiedit" => {
            t.path = path();
            t.edits = input
                .get("edits")
                .and_then(Value::as_array)
                .map(|list| list.iter().map(|e| (str_of(e, "oldString").unwrap_or("").to_string(), str_of(e, "newString").unwrap_or("").to_string())).collect())
                .unwrap_or_default();
            (t.added, t.removed) = edits_lines(&t.edits);
        }
        "write" => {
            t.path = path();
            t.content = s("content");
            t.added = t.content.as_deref().map(count_lines).unwrap_or(0);
        }
        "grep" | "glob" => {
            t.pattern = s("pattern");
            t.search_path = s("path");
        }
        "bash" => {
            t.command = s("command");
            t.output = response.map(response_text);
        }
        "webfetch" => t.url = s("url"),
        "websearch" | "codesearch" => t.pattern = s("query"),
        "task" => {
            t.description = s("description");
            t.prompt = s("prompt");
        }
        _ => {}
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn camel_case_arguments_are_understood() {
        let t = tool("edit", &json!({"filePath": "/p/a.ts", "oldString": "a", "newString": "b\nc"}), None);
        assert_eq!((t.kind, t.path.as_deref(), t.added, t.removed), ("write", Some("/p/a.ts"), 2, 1));
        let t = tool("bash", &json!({"command": "ls"}), Some(&json!("a\nb")));
        assert_eq!((t.kind, t.output.as_deref()), ("run", Some("a\nb")));
        let t = tool("read", &json!({"filePath": "/p/x", "offset": 4}), None);
        assert_eq!((t.path.as_deref(), t.offset), (Some("/p/x"), Some(5)));
    }
}
