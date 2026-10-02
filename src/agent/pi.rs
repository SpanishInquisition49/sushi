//! pi: events come from `integrations/pi/sushi.ts` (an extension), which forwards pi's events as
//! Claude-style ones carrying pi's tool names (`read`, `edit`, `write`, `bash`) and arguments.
//! The decision goes back as `{"decision": "allow" | "deny"}`.

use super::{Tool, count_lines, edits_lines, response_text, str_of};
use serde_json::Value;

pub fn tool(name: &str, input: &Value, response: Option<&Value>) -> Tool {
    let s = |k: &str| str_of(input, k).map(str::to_string);
    let kind = match name {
        "read" | "grep" | "find" | "ls" => "read",
        "edit" | "write" => "write",
        "bash" => "run",
        _ => "other",
    };
    let mut t = Tool::new(name, kind, input);
    match name {
        "read" => {
            t.path = s("path");
            t.offset = input.get("offset").and_then(Value::as_u64).map(|o| o as u32);
        }
        "edit" => {
            t.path = s("path");
            // Current pi sends `edits: [{oldText, newText}]`; older sessions a single pair.
            t.edits = match input.get("edits").and_then(Value::as_array) {
                Some(list) => list
                    .iter()
                    .map(|e| (str_of(e, "oldText").unwrap_or("").to_string(), str_of(e, "newText").unwrap_or("").to_string()))
                    .collect(),
                None => vec![(s("oldText").unwrap_or_default(), s("newText").unwrap_or_default())],
            };
            (t.added, t.removed) = edits_lines(&t.edits);
        }
        "write" => {
            t.path = s("path");
            t.content = s("content");
            t.added = t.content.as_deref().map(count_lines).unwrap_or(0);
        }
        "grep" | "find" => {
            t.pattern = s("pattern");
            t.search_path = s("path");
        }
        "bash" => {
            t.command = s("command");
            t.output = response.map(response_text);
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
    fn edits_array_and_single_pair() {
        let t = tool("edit", &json!({"path": "/p/a.ts", "edits": [{"oldText": "a", "newText": "b\nc"}, {"oldText": "d", "newText": "e"}]}), None);
        assert_eq!((t.kind, t.edits.len(), t.added, t.removed), ("write", 2, 3, 2));
        let t = tool("edit", &json!({"path": "/p/a.ts", "oldText": "a", "newText": "b"}), None);
        assert_eq!(t.edits, vec![("a".to_string(), "b".to_string())]);
    }

    #[test]
    fn read_and_bash() {
        let t = tool("read", &json!({"path": "/p/x", "offset": 3}), None);
        assert_eq!((t.path.as_deref(), t.offset), (Some("/p/x"), Some(3)));
        let t = tool("bash", &json!({"command": "ls"}), Some(&json!("out")));
        assert_eq!((t.command.as_deref(), t.output.as_deref()), (Some("ls"), Some("out")));
    }
}
