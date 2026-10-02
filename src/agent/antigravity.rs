//! Google Antigravity CLI (`agy`). Its hooks live in `~/.gemini/config/hooks.json`, as named groups
//! with the events `PreToolUse`, `PostToolUse`, `PreInvocation`, `PostInvocation` and `Stop`.
//!
//! Payloads were recorded from `agy` 1.2.14 and differ from the other agents in three ways:
//!
//! - There is no event name: the event is told from the fields (`toolCall` without `error` is a
//!   `PreToolUse`, with `error` a `PostToolUse`, `invocationNum` a `PreInvocation`, `executionNum` a
//!   `Stop`).
//! - Everything is `camelCase` (`conversationId`, `workspacePaths`, `toolCall: {name, args}`) and
//!   the arguments of a tool are `PascalCase` (`CommandLine`, `TargetFile`).
//! - There is no permission-request hook, and a `PreToolUse` hook cannot approve: in `agy` 1.2.14 its
//!   `{"decision": "allow"}` (with or without `permissionOverrides`) does not skip Antigravity's own
//!   confirmation (seen interactively, and in a headless run; reported upstream as
//!   google-antigravity/antigravity-cli#1053). Only `deny` takes effect. So Sushi only watches
//!   Antigravity: no request waits in the notch. Set [`ASK_FROM_NOTCH`] once `allow` is honoured.
//!   What it does is flag the session as waiting while a tool that usually needs a confirmation
//!   runs (see [`needs_confirmation`]), so the pet calls you to the terminal.
//!
//! Not recorded yet, and taken from the documentation: the argument names of `view_file`,
//! `replace_file_content`, `multi_replace_file_content` and the search tools.

use super::{Tool, count_lines, edits_lines, str_of};
use serde_json::{Map, Value, json};

/// Put the tools that run something, write a file or fetch a URL to the notch (see the module doc).
pub const ASK_FROM_NOTCH: bool = false;

/// Tools Antigravity asks the user to confirm by default (`request-review`): they run something,
/// write a file or fetch a URL. Whether it asks for a given call depends on the user's rules, so
/// this is only a hint that the session may be waiting.
pub fn needs_confirmation(name: &str) -> bool {
    matches!(name, "run_command" | "write_to_file" | "replace_file_content" | "multi_replace_file_content" | "read_url_content")
}

fn asks(name: &str) -> bool {
    ASK_FROM_NOTCH && needs_confirmation(name)
}

/// Rewrite an Antigravity payload into the keys (and event names) the neutral envelope reads.
pub fn canonical(payload: &Value) -> Value {
    let Some(src) = payload.as_object() else { return payload.clone() };
    let mut out = Map::new();
    if let Some(id) = str_of(payload, "conversationId") {
        out.insert("session_id".into(), json!(id));
    }
    if let Some(cwd) = payload.pointer("/workspacePaths/0").and_then(Value::as_str) {
        out.insert("cwd".into(), json!(cwd));
    }
    // Added by `sushi-hook`: the `agy` process, so the session goes away when it quits.
    if let Some(pid) = src.get("pid") {
        out.insert("pid".into(), pid.clone());
    }
    let event = if let Some(call) = src.get("toolCall") {
        let name = str_of(call, "name").unwrap_or("");
        out.insert("tool_name".into(), json!(name));
        out.insert("tool_input".into(), call.get("args").cloned().unwrap_or(Value::Null));
        if let Some(step) = src.get("stepIdx").and_then(Value::as_u64) {
            out.insert("tool_use_id".into(), json!(step.to_string()));
        }
        match src.get("error") {
            None => if asks(name) { "PermissionRequest" } else { "PreToolUse" },
            Some(Value::String(e)) if !e.is_empty() => {
                out.insert("tool_response".into(), json!(e));
                "PostToolUseFailure"
            }
            Some(_) => "PostToolUse",
        }
    } else if src.contains_key("executionNum") {
        "Stop"
    } else if src.get("invocationNum").and_then(Value::as_u64) == Some(0) {
        // The first model call of a turn: the user just sent a prompt.
        "UserPromptSubmit"
    } else if src.contains_key("invocationNum") {
        // A later model call of the turn: whatever was waiting got an answer (a refused tool ends
        // with no `PostToolUse`), so the agent is working again.
        "PostToolUse"
    } else {
        ""
    };
    out.insert("hook_event_name".into(), json!(event));
    Value::Object(out)
}

pub fn tool(name: &str, input: &Value, response: Option<&Value>) -> Tool {
    let kind = match name {
        "run_command" => "run",
        "write_to_file" | "replace_file_content" | "multi_replace_file_content" => "write",
        "view_file" | "list_dir" | "find_by_name" | "grep_search" => "read",
        "search_web" | "read_url_content" => "search",
        "invoke_subagent" | "define_subagent" => "think",
        _ => "other",
    };
    let mut t = Tool::new(name, kind, input);
    let s = |k: &str| str_of(input, k).map(str::to_string);
    match name {
        "run_command" => {
            t.command = s("CommandLine");
            t.description = s("toolSummary");
            t.output = response.and_then(Value::as_str).map(str::to_string);
        }
        "write_to_file" => {
            t.path = s("TargetFile");
            t.content = s("CodeContent");
            t.added = t.content.as_deref().map(count_lines).unwrap_or(0);
        }
        "replace_file_content" => {
            t.path = s("TargetFile");
            if let (Some(old), Some(new)) = (s("TargetContent"), s("ReplacementContent")) {
                t.edits = vec![(old, new)];
            }
            (t.added, t.removed) = edits_lines(&t.edits);
        }
        "multi_replace_file_content" => {
            t.path = s("TargetFile");
            t.edits = input
                .get("ReplacementChunks")
                .and_then(Value::as_array)
                .map(|chunks| {
                    chunks
                        .iter()
                        .filter_map(|c| Some((str_of(c, "TargetContent")?.to_string(), str_of(c, "ReplacementContent")?.to_string())))
                        .collect()
                })
                .unwrap_or_default();
            (t.added, t.removed) = edits_lines(&t.edits);
        }
        "view_file" => {
            t.path = s("AbsolutePath");
            t.offset = input.get("StartLine").and_then(Value::as_u64).map(|l| l as u32);
        }
        "list_dir" => t.path = s("DirectoryPath"),
        "find_by_name" => {
            t.pattern = s("Pattern");
            t.search_path = s("SearchDirectory");
        }
        "grep_search" => {
            t.pattern = s("Query");
            t.search_path = s("SearchPath");
        }
        "search_web" => t.pattern = s("query"),
        "read_url_content" => t.url = s("Url"),
        _ => {}
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    // Payloads recorded from `agy` 1.2.14 (paths shortened).
    const WRITE: &str = r#"{"artifactDirectoryPath":"/g/brain/c1","conversationId":"c1","modelName":"gemini-3.8-flash-high","stepIdx":2,"toolCall":{"args":{"CodeContent":"hi\n","Description":"Create a.txt with hi","Overwrite":true,"TargetFile":"/p/a.txt","toolAction":"Creating a.txt","toolSummary":"Create a.txt"},"name":"write_to_file"},"transcriptPath":"/g/t.jsonl","workspacePaths":["/p"]}"#;
    const WRITE_DONE: &str = r#"{"conversationId":"c1","error":"","stepIdx":2,"toolCall":{"args":{"CodeContent":"hi\n","TargetFile":"/p/a.txt"},"name":"write_to_file"},"workspacePaths":["/p"]}"#;
    const RUN: &str = r#"{"conversationId":"c1","stepIdx":4,"toolCall":{"args":{"CommandLine":"echo hello","Cwd":"/p","WaitMsBeforeAsync":5000,"toolSummary":"Run echo command"},"name":"run_command"},"workspacePaths":["/p"]}"#;
    const INVOCATION: &str = r#"{"conversationId":"c1","initialNumSteps":1,"invocationNum":0,"modelName":"m","workspacePaths":["/p"]}"#;

    fn c(raw: &str) -> Value {
        canonical(&serde_json::from_str(raw).unwrap())
    }

    #[test]
    fn the_event_is_told_from_the_fields() {
        let v = c(WRITE);
        assert_eq!(v["hook_event_name"], "PreToolUse");
        assert_eq!((v["session_id"].as_str(), v["cwd"].as_str(), v["tool_use_id"].as_str()), (Some("c1"), Some("/p"), Some("2")));
        assert_eq!(v["tool_name"], "write_to_file");
        assert_eq!(c(WRITE_DONE)["hook_event_name"], "PostToolUse");
        assert_eq!(c(WRITE_DONE)["tool_use_id"], "2", "start and end share the step index");
        assert_eq!(c(INVOCATION)["hook_event_name"], "UserPromptSubmit");
        let later = json!({"conversationId": "c1", "invocationNum": 3});
        let v = canonical(&later);
        assert_eq!((v["hook_event_name"].as_str(), v.get("tool_name")), (Some("PostToolUse"), None), "back to working, no step");
        assert_eq!(canonical(&json!({"conversationId": "c1"}))["hook_event_name"], "");
        assert_eq!(canonical(&json!({"conversationId": "c1", "executionNum": 1, "fullyIdle": true}))["hook_event_name"], "Stop");
        assert_eq!(canonical(&json!({"conversationId": "c1", "invocationNum": 0, "pid": 42}))["pid"], 42, "the hook's pid is kept");
    }

    #[test]
    fn nothing_waits_while_a_hook_cannot_approve() {
        assert_eq!(c(RUN)["hook_event_name"], "PreToolUse");
        let read = json!({"conversationId": "c1", "stepIdx": 1, "toolCall": {"name": "view_file", "args": {"AbsolutePath": "/p/a"}}});
        assert_eq!(canonical(&read)["hook_event_name"], "PreToolUse");
        let failed = json!({"conversationId": "c1", "stepIdx": 4, "error": "exit status 1", "toolCall": {"name": "run_command", "args": {}}});
        let v = canonical(&failed);
        assert_eq!((v["hook_event_name"].as_str(), v["tool_response"].as_str()), (Some("PostToolUseFailure"), Some("exit status 1")));
    }

    #[test]
    fn the_tools() {
        let v = c(RUN);
        let t = tool("run_command", &v["tool_input"], None);
        assert_eq!((t.kind, t.command.as_deref()), ("run", Some("echo hello")));
        let t = tool("write_to_file", &c(WRITE)["tool_input"], None);
        assert_eq!((t.kind, t.path.as_deref(), t.added), ("write", Some("/p/a.txt"), 1));
        let t = tool("replace_file_content", &json!({"TargetFile": "/p/a", "TargetContent": "a\nb", "ReplacementContent": "c"}), None);
        assert_eq!((t.path.as_deref(), t.added, t.removed), (Some("/p/a"), 1, 2));
        let chunks = json!({"TargetFile": "/p/a", "ReplacementChunks": [{"TargetContent": "x", "ReplacementContent": "y\nz"}]});
        let t = tool("multi_replace_file_content", &chunks, None);
        assert_eq!((t.edits.len(), t.added, t.removed), (1, 2, 1));
        let t = tool("view_file", &json!({"AbsolutePath": "/p/a", "StartLine": 5}), None);
        assert_eq!((t.kind, t.path.as_deref(), t.offset), ("read", Some("/p/a"), Some(5)));
        let t = tool("grep_search", &json!({"Query": "foo", "SearchPath": "/p"}), None);
        assert_eq!((t.pattern.as_deref(), t.search_path.as_deref()), (Some("foo"), Some("/p")));
    }
}
