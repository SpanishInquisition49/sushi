//! Google's official Gemini CLI (`gemini`, github.com/google-gemini/gemini-cli) — not Antigravity
//! CLI (`agy`), which has its own adapter and its own, differently-shaped hooks under
//! `~/.gemini/config/hooks.json`. Gemini CLI's hooks live in `~/.gemini/settings.json` (project or
//! user), with a schema that is already close to Claude Code's: `session_id`, `cwd`,
//! `transcript_path`, `tool_name`, `tool_input` and `tool_response` are the very same field names,
//! so only the event names need translating here (see [`canonical`]).
//!
//! Everything below is a reading of `docs/hooks/reference.md` and the tool docs in
//! google-gemini/gemini-cli, not a real install: nothing here has been checked against the real
//! CLI (the project's "untested against the real thing" tier, like Codex and opencode), so treat
//! field names and the points below as a best guess to correct once someone runs it for real.
//!
//! - There is no dedicated `PermissionRequest` hook: `BeforeTool` fires for every tool call (like
//!   `PreToolUse`) and is also the one hook whose `{"decision": "allow" | "deny"}` can gate
//!   execution. Sushi only treats it as a wait in the notch for the tools Gemini's default
//!   approval mode asks about ([`needs_confirmation`], the same heuristic `antigravity.rs` uses);
//!   every other call is just shown. Whether `"allow"` really skips Gemini's own confirmation
//!   prompt is unverified — unlike Antigravity, there is no known bug report either way, so this
//!   is presumed to work rather than disabled like `antigravity::ASK_FROM_NOTCH`.
//! - `BeforeAgent` / `AfterAgent` (once per user turn, around any number of tool calls) stand in
//!   for `UserPromptSubmit` / `Stop`. The docs don't name the field carrying the turn's final
//!   text, so a handful of likely names are tried.
//! - No tool-call id was found in the docs: like Copilot, a step is matched to its result by name.

use super::{Tool, count_lines, edits_lines, response_text, str_of};
use serde_json::{Value, json};

/// Tools Gemini's default approval mode asks about: running a shell command or writing/replacing
/// a file. A plain read or search is never worth waiting on.
pub fn needs_confirmation(name: &str) -> bool {
    matches!(name, "run_shell_command" | "write_file" | "replace")
}

/// Rewrite a Gemini payload into the keys and event names the neutral envelope reads. Field
/// names otherwise already match, so this only translates `hook_event_name`.
pub fn canonical(payload: &Value) -> Value {
    let Some(src) = payload.as_object() else { return payload.clone() };
    let mut out = src.clone();
    let native = str_of(payload, "hook_event_name").unwrap_or("");
    let tool_name = str_of(payload, "tool_name");
    let event = match native {
        "BeforeAgent" => "UserPromptSubmit",
        "AfterAgent" => "Stop",
        "BeforeTool" => {
            if tool_name.is_some_and(needs_confirmation) { "PermissionRequest" } else { "PreToolUse" }
        }
        "AfterTool" => {
            let failed = match payload.pointer("/tool_response/error") {
                Some(Value::String(e)) => !e.is_empty(),
                Some(Value::Null) | None => false,
                Some(_) => true,
            };
            if failed { "PostToolUseFailure" } else { "PostToolUse" }
        }
        other => other,
    };
    out.insert("hook_event_name".into(), json!(event));
    if native == "AfterAgent" {
        let message = str_of(payload, "last_assistant_message").or_else(|| str_of(payload, "response")).or_else(|| str_of(payload, "text"));
        if let Some(m) = message {
            out.insert("last_assistant_message".into(), json!(m));
        }
    }
    Value::Object(out)
}

pub fn tool(name: &str, input: &Value, response: Option<&Value>) -> Tool {
    let s = |k: &str| str_of(input, k).map(str::to_string);
    let kind = match name {
        "read_file" | "list_directory" | "glob" | "grep_search" => "read",
        "write_file" | "replace" => "write",
        "run_shell_command" => "run",
        "google_web_search" => "search",
        _ => "other",
    };
    let mut t = Tool::new(name, kind, input);
    match name {
        "read_file" => {
            t.path = s("file_path");
            t.offset = input.get("offset").and_then(Value::as_u64).map(|o| o as u32);
        }
        "write_file" => {
            t.path = s("file_path");
            t.content = s("content");
            t.added = t.content.as_deref().map(count_lines).unwrap_or(0);
        }
        "replace" => {
            t.path = s("file_path");
            t.edits = vec![(s("old_string").unwrap_or_default(), s("new_string").unwrap_or_default())];
            (t.added, t.removed) = edits_lines(&t.edits);
        }
        "list_directory" => t.path = s("dir_path"),
        "glob" => {
            t.pattern = s("pattern");
            t.search_path = s("path");
        }
        "grep_search" => {
            t.pattern = s("pattern");
            t.search_path = s("path");
        }
        "run_shell_command" => {
            t.command = s("command");
            t.description = s("description");
            // `AfterTool`'s response is `{llmContent, returnDisplay, error}`, not the plain
            // string or `{stdout, stderr}` shape `response_text` otherwise reads.
            t.output = response.map(|r| {
                str_of(r, "llmContent").or_else(|| str_of(r, "returnDisplay")).or_else(|| str_of(r, "error")).map(str::to_string).unwrap_or_else(|| response_text(r))
            });
        }
        "google_web_search" => t.pattern = s("query"),
        _ => {}
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(raw: Value) -> Value {
        canonical(&raw)
    }

    #[test]
    fn event_names_are_translated() {
        let v = c(json!({"hook_event_name": "BeforeAgent", "session_id": "s1"}));
        assert_eq!(v["hook_event_name"], "UserPromptSubmit");
        let v = c(json!({"hook_event_name": "AfterAgent", "session_id": "s1", "response": "All done"}));
        assert_eq!((v["hook_event_name"].as_str(), v["last_assistant_message"].as_str()), (Some("Stop"), Some("All done")));
        assert_eq!(c(json!({"hook_event_name": "SessionStart"}))["hook_event_name"], "SessionStart");
    }

    #[test]
    fn only_tools_that_ask_wait_in_the_notch() {
        let read = c(json!({"hook_event_name": "BeforeTool", "tool_name": "read_file"}));
        assert_eq!(read["hook_event_name"], "PreToolUse");
        let run = c(json!({"hook_event_name": "BeforeTool", "tool_name": "run_shell_command"}));
        assert_eq!(run["hook_event_name"], "PermissionRequest");
    }

    #[test]
    fn after_tool_failure_is_told_from_the_response() {
        let ok = c(json!({"hook_event_name": "AfterTool", "tool_name": "run_shell_command", "tool_response": {"llmContent": "ok"}}));
        assert_eq!(ok["hook_event_name"], "PostToolUse");
        let failed = c(json!({"hook_event_name": "AfterTool", "tool_name": "run_shell_command", "tool_response": {"error": "exit 1"}}));
        assert_eq!(failed["hook_event_name"], "PostToolUseFailure");
    }

    #[test]
    fn the_tools() {
        let t = tool("run_shell_command", &json!({"command": "echo hi"}), Some(&json!({"llmContent": "hi"})));
        assert_eq!((t.kind, t.command.as_deref(), t.output.as_deref()), ("run", Some("echo hi"), Some("hi")));
        let t = tool("write_file", &json!({"file_path": "/p/a.txt", "content": "hi\nthere\n"}), None);
        assert_eq!((t.kind, t.path.as_deref(), t.added), ("write", Some("/p/a.txt"), 2));
        let t = tool("replace", &json!({"file_path": "/p/a.txt", "old_string": "a\nb", "new_string": "c"}), None);
        assert_eq!((t.added, t.removed), (1, 2));
        let t = tool("grep_search", &json!({"pattern": "foo", "path": "/p"}), None);
        assert_eq!((t.pattern.as_deref(), t.search_path.as_deref()), (Some("foo"), Some("/p")));
    }
}
