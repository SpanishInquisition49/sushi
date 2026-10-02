//! Codex CLI: its hooks speak Claude Code's language (same events, same `PermissionRequest`
//! output), so only the tools differ. Shell commands are `Bash`; file changes arrive as an
//! `apply_patch` call carrying the patch text.

use super::{Tool, edits_lines, response_text, str_of};
use serde_json::Value;

/// The command of a shell tool: a string, or an argv array (`["bash", "-lc", "ls"]`).
fn command_of(input: &Value) -> Option<String> {
    match input.get("command").or_else(|| input.get("cmd"))? {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => {
            let words: Vec<&str> = a.iter().filter_map(Value::as_str).collect();
            match words.as_slice() {
                [_, "-lc" | "-c", script] => Some(script.to_string()),
                [] => None,
                w => Some(w.join(" ")),
            }
        }
        _ => None,
    }
}

/// One file of an `apply_patch` patch: its path and the `(old, new)` text of each hunk.
#[derive(Debug, PartialEq, Eq)]
pub struct PatchFile {
    pub path: String,
    pub hunks: Vec<(String, String)>,
    pub is_new: bool,
}

/// Read the files and hunks of an `apply_patch` patch (`*** Begin Patch` ... `*** End Patch`).
pub fn parse_patch(text: &str) -> Vec<PatchFile> {
    let mut files: Vec<PatchFile> = Vec::new();
    let (mut old, mut new) = (Vec::<&str>::new(), Vec::<&str>::new());
    fn flush(files: &mut [PatchFile], old: &mut Vec<&str>, new: &mut Vec<&str>) {
        if (!old.is_empty() || !new.is_empty()) && let Some(f) = files.last_mut() {
            f.hunks.push((old.join("\n"), new.join("\n")));
        }
        old.clear();
        new.clear();
    }
    for line in text.lines() {
        if let Some(path) = line
            .strip_prefix("*** Update File: ")
            .or_else(|| line.strip_prefix("*** Add File: "))
            .or_else(|| line.strip_prefix("*** Delete File: "))
        {
            flush(&mut files, &mut old, &mut new);
            files.push(PatchFile { path: path.trim().to_string(), hunks: Vec::new(), is_new: line.starts_with("*** Add") });
        } else if line.starts_with("@@") || line.starts_with("*** ") {
            flush(&mut files, &mut old, &mut new);
        } else if let Some(l) = line.strip_prefix('+') {
            new.push(l);
        } else if let Some(l) = line.strip_prefix('-') {
            old.push(l);
        } else if let Some(l) = line.strip_prefix(' ') {
            old.push(l);
            new.push(l);
        }
    }
    flush(&mut files, &mut old, &mut new);
    files
}

/// The patch text of a tool input: the input itself when it is a string, or one of its fields.
pub fn patch_text(input: &Value) -> Option<&str> {
    let text = match input {
        Value::String(t) => Some(t.as_str()),
        _ => ["input", "patch", "command"].iter().find_map(|k| str_of(input, k)),
    };
    text.filter(|t| t.contains("*** Begin Patch") || t.contains("*** Update File") || t.contains("*** Add File"))
}

/// A write-type tool made from an `apply_patch` patch. The viewer shows the first file; the
/// counts cover them all.
pub fn patch_tool(name: &str, input: &Value, text: &str) -> Tool {
    let files = parse_patch(text);
    let mut t = Tool::new(name, "write", input);
    for f in &files {
        let (a, r) = edits_lines(&f.hunks);
        t.added += a;
        t.removed += r;
    }
    if let Some(first) = files.first() {
        t.path = Some(first.path.clone());
        if first.is_new {
            t.content = Some(first.hunks.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>().join("\n"));
        } else {
            t.edits = first.hunks.clone();
        }
    }
    t
}

pub fn tool(name: &str, input: &Value, response: Option<&Value>) -> Tool {
    let s = |k: &str| str_of(input, k).map(str::to_string);
    if let Some(text) = patch_text(input) {
        return patch_tool(name, input, text);
    }
    let kind = match name {
        "Bash" | "shell" | "local_shell" | "exec_command" | "container.exec" => "run",
        "Edit" | "Write" | "apply_patch" => "write",
        "update_plan" => "think",
        "web_search" => "search",
        _ => "other",
    };
    let mut t = Tool::new(name, kind, input);
    match kind {
        "run" => {
            t.command = command_of(input);
            t.output = response.map(response_text);
        }
        "write" => {
            t.path = s("file_path").or_else(|| s("path"));
            if name == "Edit" {
                t.edits = vec![(s("old_string").unwrap_or_default(), s("new_string").unwrap_or_default())];
            } else {
                t.content = s("content");
            }
            (t.added, t.removed) = edits_lines(&t.edits);
            if let Some(c) = &t.content {
                t.added = super::count_lines(c);
            }
        }
        "search" => t.pattern = s("query"),
        _ => {}
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PATCH: &str = "*** Begin Patch\n*** Update File: src/a.rs\n@@ fn main\n ctx\n-old one\n-old two\n+new one\n*** Add File: b.txt\n+hello\n+world\n*** End Patch";

    #[test]
    fn patches_become_files_and_hunks() {
        let files = parse_patch(PATCH);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "src/a.rs");
        assert_eq!(files[0].hunks, vec![("ctx\nold one\nold two".to_string(), "ctx\nnew one".to_string())]);
        assert!(files[1].is_new && files[1].hunks[0].1 == "hello\nworld");
    }

    #[test]
    fn apply_patch_is_a_write_with_counts_over_all_files() {
        let t = tool("apply_patch", &json!({"command": PATCH}), None);
        assert_eq!((t.kind, t.path.as_deref()), ("write", Some("src/a.rs")));
        assert_eq!((t.added, t.removed), (2 + 2, 3));
        assert_eq!(t.edits.len(), 1);
    }

    #[test]
    fn shell_commands_unwrap_the_argv() {
        let t = tool("Bash", &json!({"command": "cargo test"}), Some(&json!({"stdout": "ok"})));
        assert_eq!((t.kind, t.command.as_deref(), t.output.as_deref()), ("run", Some("cargo test"), Some("ok")));
        let t = tool("shell", &json!({"command": ["bash", "-lc", "ls -la"]}), None);
        assert_eq!(t.command.as_deref(), Some("ls -la"));
    }
}
