//! What the live viewer shows for a step: a diff with line numbers for an edit,
//! a terminal for a command, an excerpt for a file that was read.
//!
//! Everything is bounded (a few lines, clipped columns) because it is published
//! in `state.json`. With `show_code = false` only a title is kept, never code.

use crate::agent::{Role, Tool};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_COLS: usize = 160;
const MAX_CHANGED_DEL: usize = 12;
const MAX_CHANGED_ADD: usize = 24;
const MAX_FILE_LINES: usize = 20;
const OUTPUT_LINES: usize = 20;
const CONTEXT: usize = 3;
const MAX_FILE_BYTES: u64 = 2_000_000;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DiffLine {
    /// 1-based line number, 0 when unknown.
    pub n: u32,
    /// `ctx`, `del` or `add`.
    pub kind: &'static str,
    pub text: String,
}

/// Hand-written for the same reason as `Detail`'s own impl just above: a derived `Deserialize`
/// for the `&'static str` field would force an impossible `'de: 'static` bound.
impl<'de> Deserialize<'de> for DiffLine {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            n: u32,
            kind: String,
            text: String,
        }
        let raw = Raw::deserialize(deserializer)?;
        let kind = match raw.kind.as_str() {
            "del" => "del",
            "add" => "add",
            _ => "ctx",
        };
        Ok(DiffLine { n: raw.n, kind, text: raw.text })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

/// One question Claude asks (the `AskUserQuestion` tool).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Question {
    pub header: String,
    pub question: String,
    pub multi: bool,
    pub options: Vec<QuestionOption>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Detail {
    Questions { questions: Vec<Question> },
    Diff { file: String, lang: &'static str, lines: Vec<DiffLine>, more: u32 },
    Terminal { command: String, output: Vec<String> },
    File { file: String, lang: &'static str, start: u32, lines: Vec<String> },
    Text { title: String, body: String },
}

/// `lang_of` only ever runs forward (path → `&'static str`); matching the owned string back
/// against the same fixed set of keys (an unrecognized one — old data, a typo — safely degrades
/// to `"other"`, since this is only ever a display hint, never stored data of its own).
fn static_lang(s: &str) -> &'static str {
    match s {
        "rust" => "rust",
        "ts" => "ts",
        "js" => "js",
        "python" => "python",
        "lua" => "lua",
        "shell" => "shell",
        "json" => "json",
        "toml" => "toml",
        "markdown" => "markdown",
        "c" => "c",
        "cpp" => "cpp",
        "go" => "go",
        "java" => "java",
        "html" => "html",
        "css" => "css",
        "yaml" => "yaml",
        "nix" => "nix",
        _ => "other",
    }
}

/// Hand-written, not derived: a derived `Deserialize` for a `&'static str` field forces an
/// impossible `'de: 'static` bound on the whole enum. Pivoting through `serde_json::Value`
/// mirrors the exact `#[serde(tag = "type", ...)]` shape `Serialize` produces above, and is the
/// only place `Detail` is ever deserialized from (`history.rs`'s own JSON file).
impl<'de> Deserialize<'de> for Detail {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(deserializer)?;
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string).unwrap_or_default();
        let from = |k: &str| v.get(k).cloned().unwrap_or(Value::Null);
        let detail = match v.get("type").and_then(Value::as_str) {
            Some("questions") => Detail::Questions { questions: serde_json::from_value(from("questions")).unwrap_or_default() },
            Some("diff") => Detail::Diff {
                file: s("file"),
                lang: static_lang(&s("lang")),
                lines: serde_json::from_value(from("lines")).unwrap_or_default(),
                more: v.get("more").and_then(Value::as_u64).unwrap_or(0) as u32,
            },
            Some("terminal") => Detail::Terminal { command: s("command"), output: serde_json::from_value(from("output")).unwrap_or_default() },
            Some("file") => Detail::File {
                file: s("file"),
                lang: static_lang(&s("lang")),
                start: v.get("start").and_then(Value::as_u64).unwrap_or(1) as u32,
                lines: serde_json::from_value(from("lines")).unwrap_or_default(),
            },
            Some("text") => Detail::Text { title: s("title"), body: s("body") },
            other => return Err(serde::de::Error::custom(format!("unknown Detail type {other:?}"))),
        };
        Ok(detail)
    }
}

/// Language key (used for the chip and the syntax colors) from a file name.
pub fn lang_of(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "rs" => "rust",
        "ts" | "tsx" => "ts",
        "js" | "jsx" | "mjs" | "cjs" => "js",
        "py" => "python",
        "lua" | "luau" => "lua",
        "sh" | "bash" | "zsh" => "shell",
        "json" => "json",
        "toml" => "toml",
        "md" | "markdown" => "markdown",
        "c" | "h" => "c",
        "cc" | "cpp" | "hpp" | "cxx" => "cpp",
        "go" => "go",
        "java" => "java",
        "html" | "htm" => "html",
        "css" => "css",
        "yml" | "yaml" => "yaml",
        "nix" => "nix",
        _ => "other",
    }
}

fn clip(s: &str) -> String {
    let mut out: String = s.chars().take(MAX_COLS).collect();
    if s.chars().count() > MAX_COLS {
        out.push('…');
    }
    out
}

fn read_text(path: &str) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Drop terminal escape sequences (colors, cursor moves) from command output.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    out
}

fn tail_lines(s: &str, n: usize) -> Vec<String> {
    let clean = strip_ansi(s);
    let lines: Vec<&str> = clean.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|l| clip(l)).collect()
}

/// A diff for replacing `old` with `new` in `path`.
///
/// The file is read to find where the text sits, so lines get real numbers and the
/// change is shown as whole lines with context (like an editor would). Before the
/// edit runs the file still holds `old`; afterwards it holds `new`
/// (`file_has_new`). If the file cannot be read, the changed lines are shown
/// without numbers.
pub fn diff_snippet(path: &str, display: &str, old: &str, new: &str, file_has_new: bool) -> Detail {
    let text = read_text(path);
    let needle = if file_has_new { new } else { old };
    let located = text.as_deref().and_then(|t| {
        if needle.is_empty() { None } else { t.find(needle).map(|pos| (t, pos)) }
    });

    let mut lines: Vec<DiffLine> = Vec::new();
    // The changed text as whole lines, the line number where it starts (0 = unknown) and the
    // untouched lines around it.
    struct Located {
        old_block: String,
        new_block: String,
        first: u32,
        before: Vec<String>,
        after: Vec<String>,
    }
    let loc = match located {
        Some((t, pos)) => {
            let line_start = t[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let end_pos = pos + needle.len();
            let line_end = t[end_pos..].find('\n').map(|i| end_pos + i).unwrap_or(t.len());
            let (prefix, suffix) = (&t[line_start..pos], &t[end_pos..line_end]);
            let old_block = format!("{prefix}{old}{suffix}");
            let new_block = format!("{prefix}{new}{suffix}");
            let start_idx = t[..line_start].matches('\n').count();
            let file_lines: Vec<&str> = t.lines().collect();
            // How many lines the changed block occupies in the file as it is now.
            let in_file = if file_has_new { new_block.lines().count() } else { old_block.lines().count() };
            let to_owned = |r: &[&str]| r.iter().map(|l| l.to_string()).collect::<Vec<_>>();
            let after_from = (start_idx + in_file).min(file_lines.len());
            Located {
                before: to_owned(&file_lines[start_idx.saturating_sub(CONTEXT)..start_idx.min(file_lines.len())]),
                after: to_owned(&file_lines[after_from..(after_from + CONTEXT).min(file_lines.len())]),
                first: start_idx as u32 + 1,
                old_block,
                new_block,
            }
        }
        None => Located { old_block: old.to_string(), new_block: new.to_string(), first: 0, before: vec![], after: vec![] },
    };
    let (old_block, new_block, first) = (loc.old_block, loc.new_block, loc.first);
    let (ctx_before, ctx_after_src) = (loc.before, loc.after);

    let old_lines: Vec<&str> = old_block.lines().collect();
    let new_lines: Vec<&str> = new_block.lines().collect();
    let num = |i: usize| if first > 0 { first + i as u32 } else { 0 };

    let before_first = first.saturating_sub(ctx_before.len() as u32);
    for (i, l) in ctx_before.iter().enumerate() {
        lines.push(DiffLine { n: before_first + i as u32, kind: "ctx", text: clip(l) });
    }
    let del_n = old_lines.len().min(MAX_CHANGED_DEL);
    let add_n = new_lines.len().min(MAX_CHANGED_ADD);
    for (i, l) in old_lines.iter().take(del_n).enumerate() {
        lines.push(DiffLine { n: num(i), kind: "del", text: clip(l) });
    }
    for (i, l) in new_lines.iter().take(add_n).enumerate() {
        lines.push(DiffLine { n: num(i), kind: "add", text: clip(l) });
    }
    // Lines after the change are numbered as in the new file.
    for (j, l) in ctx_after_src.iter().enumerate() {
        let n = if first > 0 { first + new_lines.len() as u32 + j as u32 } else { 0 };
        lines.push(DiffLine { n, kind: "ctx", text: clip(l) });
    }
    let more = (old_lines.len() - del_n + new_lines.len() - add_n) as u32;
    Detail::Diff { file: display.to_string(), lang: lang_of(path), lines, more }
}

/// A new file (or a full rewrite): its first lines, all added.
pub fn write_snippet(path: &str, display: &str, content: &str) -> Detail {
    let all: Vec<&str> = content.lines().collect();
    let lines = all
        .iter()
        .take(MAX_FILE_LINES)
        .enumerate()
        .map(|(i, l)| DiffLine { n: i as u32 + 1, kind: "add", text: clip(l) })
        .collect();
    Detail::Diff { file: display.to_string(), lang: lang_of(path), lines, more: all.len().saturating_sub(MAX_FILE_LINES) as u32 }
}

/// An excerpt of a file. `content` is either the whole file (`skip` = lines to drop to
/// reach `start`) or already starts at line `start` (`skip` = 0).
pub fn file_excerpt(path: &str, display: &str, content: &str, start: u32, skip: usize) -> Detail {
    let lines = content.lines().skip(skip).take(MAX_FILE_LINES).map(clip).collect();
    Detail::File { file: display.to_string(), lang: lang_of(path), start: start.max(1), lines }
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

pub use crate::agent::short_path;

/// The questions of an `AskUserQuestion` call (a few of them, a few options each).
pub fn questions(input: &Value) -> Option<Detail> {
    let list = input.get("questions")?.as_array()?;
    let questions: Vec<Question> = list
        .iter()
        .take(4)
        .filter_map(|q| {
            let options: Vec<QuestionOption> = q
                .get("options")?
                .as_array()?
                .iter()
                .take(6)
                .filter_map(|o| {
                    Some(QuestionOption { label: clip(s(o, "label")?), description: clip(s(o, "description").unwrap_or("")) })
                })
                .collect();
            if options.is_empty() {
                return None;
            }
            Some(Question {
                header: clip(s(q, "header").unwrap_or("")),
                question: s(q, "question")?.to_string(),
                multi: q.get("multiSelect").and_then(Value::as_bool).unwrap_or(false),
                options,
            })
        })
        .collect();
    if questions.is_empty() { None } else { Some(Detail::Questions { questions }) }
}

/// The detail to show for a tool call that is about to run (or is waiting for approval).
pub fn for_tool(tool: &Tool, cwd: &str, show_code: bool) -> Option<Detail> {
    if !show_code {
        return Some(Detail::Text { title: tool.name.clone(), body: String::new() });
    }
    match tool.role {
        Role::Question => return questions(&tool.input),
        Role::Plan => {
            let plan: String = tool.plan.as_deref().unwrap_or("").lines().take(24).map(clip).collect::<Vec<_>>().join("\n");
            return Some(Detail::Text { title: "Plan".into(), body: plan });
        }
        Role::Normal => {}
    }
    let path = tool.path.as_deref().unwrap_or("");
    let display = short_path(path, cwd);
    match tool.kind {
        "write" => match (tool.edits.first(), &tool.content) {
            (Some((old, new)), _) => Some(diff_snippet(path, &display, old, new, false)),
            (None, Some(content)) => Some(write_snippet(path, &display, content)),
            _ => None,
        },
        "read" if tool.path.is_some() => {
            let content = read_text(path)?;
            let start = tool.offset.unwrap_or(1).max(1);
            Some(file_excerpt(path, &display, &content, start, start as usize - 1))
        }
        "read" => {
            let mut body = clip(tool.pattern.as_deref().unwrap_or(""));
            if let Some(p) = &tool.search_path {
                body.push_str(&format!("  in {}", short_path(p, cwd)));
            }
            Some(Detail::Text { title: tool.name.clone(), body })
        }
        "run" => Some(Detail::Terminal { command: clip(tool.command.as_deref().unwrap_or("")), output: Vec::new() }),
        "search" if tool.url.is_some() => Some(Detail::Text { title: "Web fetch".into(), body: clip(tool.url.as_deref().unwrap_or("")) }),
        "search" => Some(Detail::Text { title: "Web search".into(), body: clip(tool.pattern.as_deref().unwrap_or("")) }),
        "think" if tool.description.is_some() || tool.prompt.is_some() => {
            let title = clip(tool.description.as_deref().unwrap_or("Subagent"));
            let body: String = tool.prompt.as_deref().unwrap_or("").chars().take(240).collect();
            Some(Detail::Text { title, body })
        }
        _ => None,
    }
}

/// The detail once the tool has finished: command output, the content that was read,
/// or (if we missed the start) the diff computed from the already edited file.
pub fn after_tool(tool: &Tool, existing: Option<Detail>, cwd: &str, show_code: bool) -> Option<Detail> {
    if !show_code {
        return existing;
    }
    let path = tool.path.as_deref().unwrap_or("");
    let display = short_path(path, cwd);
    match tool.kind {
        "run" if tool.role == Role::Normal => {
            let out = tool.output.as_deref()?;
            Some(Detail::Terminal { command: clip(tool.command.as_deref().unwrap_or("")), output: tail_lines(out, OUTPUT_LINES) })
        }
        "read" => match &tool.read {
            Some((content, start)) => Some(file_excerpt(path, &display, content, *start, 0)),
            None => existing,
        },
        "write" if existing.is_none() => {
            let (old, new) = tool.edits.first()?;
            Some(diff_snippet(path, &display, old, new, true))
        }
        _ => existing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::claude;
    use serde_json::json;
    use std::io::Write;

    fn ct(name: &str, input: &Value) -> Tool {
        claude::tool(name, input, None)
    }

    fn ctr(name: &str, input: &Value, response: &Value) -> Tool {
        claude::tool(name, input, Some(response))
    }

    fn temp_file(name: &str, body: &str) -> String {
        let dir = std::env::temp_dir().join(format!("sushi-detail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::File::create(&p).unwrap().write_all(body.as_bytes()).unwrap();
        p.to_string_lossy().into_owned()
    }

    const SRC: &str = "l1\nl2\nl3\nl4\nconst TVA = 0.196\nl6\nl7\nl8\nl9\n";

    fn kinds(d: &Detail) -> Vec<(u32, &'static str, String)> {
        match d {
            Detail::Diff { lines, .. } => lines.iter().map(|l| (l.n, l.kind, l.text.clone())).collect(),
            other => panic!("not a diff: {other:?}"),
        }
    }

    #[test]
    fn edit_is_shown_as_whole_lines_with_numbers_and_context() {
        let path = temp_file("a.ts", SRC);
        // Only a fragment of the line changes; the diff still shows full lines.
        let d = diff_snippet(&path, "src/a.ts", "0.196", "0.2", false);
        assert_eq!(
            kinds(&d),
            vec![
                (2, "ctx", "l2".into()), (3, "ctx", "l3".into()), (4, "ctx", "l4".into()),
                (5, "del", "const TVA = 0.196".into()),
                (5, "add", "const TVA = 0.2".into()),
                (6, "ctx", "l6".into()), (7, "ctx", "l7".into()), (8, "ctx", "l8".into()),
            ]
        );
        assert!(matches!(d, Detail::Diff { lang: "ts", .. }));
    }

    #[test]
    fn multi_line_replacement_renumbers_the_lines_after_it() {
        let path = temp_file("b.rs", SRC);
        let d = diff_snippet(&path, "b.rs", "l6\nl7", "x\ny\nz", false);
        let k = kinds(&d);
        // old block = 2 lines at 6..7, new = 3 lines; the context after starts at 6 + 3.
        assert!(k.contains(&(6, "del", "l6".into())) && k.contains(&(7, "del", "l7".into())));
        assert!(k.contains(&(8, "add", "z".into())));
        assert!(k.contains(&(9, "ctx", "l8".into())), "{k:?}");
    }

    #[test]
    fn after_the_edit_the_file_holds_the_new_text() {
        let after = SRC.replace("0.196", "0.2");
        let path = temp_file("c.ts", &after);
        let d = diff_snippet(&path, "c.ts", "0.196", "0.2", true);
        let k = kinds(&d);
        assert!(k.contains(&(5, "del", "const TVA = 0.196".into())), "{k:?}");
        assert!(k.contains(&(5, "add", "const TVA = 0.2".into())));
    }

    #[test]
    fn unreadable_file_or_missing_text_falls_back_to_unnumbered_lines() {
        let d = diff_snippet("/nonexistent/x.rs", "x.rs", "a\nb", "c", false);
        assert_eq!(kinds(&d), vec![(0, "del", "a".into()), (0, "del", "b".into()), (0, "add", "c".into())]);
        let path = temp_file("d.rs", SRC);
        let d = diff_snippet(&path, "d.rs", "not in the file", "z", false);
        assert!(kinds(&d).iter().all(|(n, _, _)| *n == 0));
    }

    #[test]
    fn long_changes_and_columns_are_truncated() {
        let n = 40;
        let old = (0..n).map(|i| format!("old{i}")).collect::<Vec<_>>().join("\n");
        let new = (0..n).map(|i| format!("new{i}")).collect::<Vec<_>>().join("\n");
        match diff_snippet("/nonexistent", "f", &old, &new, false) {
            Detail::Diff { lines, more, .. } => {
                assert_eq!(lines.iter().filter(|l| l.kind == "del").count(), MAX_CHANGED_DEL);
                assert_eq!(lines.iter().filter(|l| l.kind == "add").count(), MAX_CHANGED_ADD);
                assert_eq!(more as usize, (n - MAX_CHANGED_DEL) + (n - MAX_CHANGED_ADD));
            }
            other => panic!("{other:?}"),
        }
        let long = "è".repeat(500);
        assert_eq!(clip(&long).chars().count(), MAX_COLS + 1);
    }

    #[test]
    fn write_shows_the_first_lines_as_added() {
        let content = (1..=30).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        match write_snippet("notes.md", "notes.md", &content) {
            Detail::Diff { lines, more, lang, .. } => {
                assert_eq!((lines.len(), more, lang), (MAX_FILE_LINES, 30 - MAX_FILE_LINES as u32, "markdown"));
                assert_eq!((lines[0].n, lines[0].kind), (1, "add"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn read_from_disk_respects_the_offset() {
        let path = temp_file("e.py", SRC);
        match for_tool(&ct("Read", &json!({"file_path": path, "offset": 5})), "/", true).unwrap() {
            Detail::File { start, lines, .. } => {
                assert_eq!(start, 5);
                assert_eq!(lines[0], "const TVA = 0.196");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn terminal_output_keeps_the_tail_without_escape_codes() {
        let out: String = (1..=30).map(|i| format!("\u{1b}[32mline {i}\u{1b}[0m\n")).collect();
        let resp = json!({"stdout": out, "stderr": "boom\n"});
        let d = after_tool(&ctr("Bash", &json!({"command": "cargo test"}), &resp), None, "/p", true).unwrap();
        match d {
            Detail::Terminal { command, output } => {
                assert_eq!(command, "cargo test");
                assert_eq!(output.len(), OUTPUT_LINES);
                assert_eq!(output.last().unwrap(), "boom");
                assert!(output.iter().all(|l| !l.contains('\u{1b}')));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn read_uses_the_content_in_the_response() {
        let resp = json!({"file": {"content": "a\nb\nc", "startLine": 10}});
        match after_tool(&ctr("Read", &json!({"file_path": "/p/x.py"}), &resp), None, "/p", true).unwrap() {
            Detail::File { file, start, lines, lang } => {
                assert_eq!((file.as_str(), start, lines.len(), lang), ("x.py", 10, 3, "python"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn tool_details_before_running() {
        let bash = for_tool(&ct("Bash", &json!({"command": "ls -la"})), "/p", true).unwrap();
        assert_eq!(bash, Detail::Terminal { command: "ls -la".into(), output: vec![] });
        assert!(for_tool(&ct("Weird", &json!({})), "/p", true).is_none());
        let g = for_tool(&ct("Grep", &json!({"pattern": "foo", "path": "/p/src"})), "/p", true).unwrap();
        assert_eq!(g, Detail::Text { title: "Grep".into(), body: "foo  in src".into() });
    }

    #[test]
    fn questions_are_parsed_and_bounded() {
        let input = json!({"questions": [
            {"header": "Colour", "question": "Which colour?", "multiSelect": false,
             "options": [{"label": "Red", "description": "warm"}, {"label": "Blue", "description": "cool"}]},
            {"header": "Extras", "question": "Which extras?", "multiSelect": true,
             "options": (0..9).map(|i| json!({"label": format!("o{i}"), "description": ""})).collect::<Vec<_>>()},
            {"question": "No options at all", "options": []},
            {"header": "bad"},
        ]});
        match for_tool(&ct("AskUserQuestion", &input), "/p", true).unwrap() {
            Detail::Questions { questions } => {
                assert_eq!(questions.len(), 2, "questions without options are dropped");
                assert_eq!((questions[0].question.as_str(), questions[0].multi, questions[0].options.len()), ("Which colour?", false, 2));
                assert_eq!(questions[0].options[1], QuestionOption { label: "Blue".into(), description: "cool".into() });
                assert!(questions[1].multi && questions[1].options.len() == 6, "options are capped");
            }
            other => panic!("{other:?}"),
        }
        assert!(for_tool(&ct("AskUserQuestion", &json!({})), "/p", true).is_none());
    }

    #[test]
    fn a_plan_is_shown_as_text() {
        let plan = (1..=40).map(|i| format!("step {i}")).collect::<Vec<_>>().join("\n");
        match for_tool(&ct("ExitPlanMode", &json!({"plan": plan})), "/p", true).unwrap() {
            Detail::Text { title, body } => {
                assert_eq!(title, "Plan");
                assert_eq!(body.lines().count(), 24);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn privacy_mode_never_includes_code() {
        let path = temp_file("secret.rs", SRC);
        let d = for_tool(&ct("Edit", &json!({"file_path": path, "old_string": "l2", "new_string": "SECRET"})), "/", false).unwrap();
        assert_eq!(d, Detail::Text { title: "Edit".into(), body: String::new() });
        let resp = json!({"stdout": "password=hunter2"});
        assert!(after_tool(&ctr("Bash", &json!({"command": "env"}), &resp), None, "/", false).is_none());
    }
}
