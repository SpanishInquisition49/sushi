//! Line-delimited JSON protocol between hook, CLI and daemon.
//!
//! The hook sends one `Request` line. Only `PermissionRequest` hooks wait for a
//! reply line (`Reply`); every other event is fire-and-forget.

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn claude() -> String {
    "claude".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    /// An event from an agent, in that agent's own format (`agent` is its id, `claude` if absent).
    Hook {
        #[serde(default = "claude")]
        agent: String,
        payload: Value,
    },
    /// `accept_edits`: for a plan, also let the agent make its edits without asking (Claude Code's
    /// "Yes, auto-accept edits").
    Approve {
        id: u64,
        #[serde(default)]
        accept_edits: bool,
    },
    Deny { id: u64 },
    State,
    /// Keep the connection open: the daemon writes a `Reply` with the state now and again every
    /// time it changes, until the connection closes.
    Watch,
    /// Send a message to the built-in chat (`model` and `agent` override the configured ones).
    ChatSend {
        text: String,
        model: Option<String>,
        #[serde(default)]
        agent: Option<String>,
        /// The absolute path of a file fed with the message: its content goes into the prompt.
        #[serde(default)]
        file: Option<String>,
    },
    ChatStop,
    ChatClear,
    /// Answer a question: `answers` maps each question text to the chosen label.
    Answer { id: u64, answers: Value },
    /// Tamagotchi care actions (see `sushi::care::Care`): feed raises hunger, pet raises
    /// affection, nap is a one-shot "sent for a nap" energy boost.
    CareFeed,
    CarePet,
    CareNap,
    /// A finished mini-game round; `score` is 0..=100.
    CarePlay { score: u32 },
    CareBuy { id: String },
    /// Equip `id`, or `""` to go bare.
    CareEquip { id: String },
    /// The durable step history of one session (see `sushi::history`), beyond the last-8-steps
    /// `Activity` publishes live — optionally filtered by a case-insensitive substring `query`.
    History {
        session_id: String,
        #[serde(default)]
        query: Option<String>,
    },
    /// Toggle a step's "needs review" flag (and optionally set/replace its note).
    FlagStep {
        session_id: String,
        step_id: String,
        flagged: bool,
        #[serde(default)]
        note: Option<String>,
    },
    /// Write a markdown export of a session's stored steps (optionally filtered by `query`) to
    /// disk; the reply's `state` is the resulting file path as a JSON string.
    Export {
        session_id: String,
        #[serde(default)]
        query: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<Decision>,
    /// The tool input to use instead of the original (how answers reach `AskUserQuestion`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_input: Option<Value>,
    /// The permission mode the agent switches to (`acceptEdits` when a plan is approved that way).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
}

impl Reply {
    pub fn ok() -> Self {
        Reply { ok: true, decision: None, updated_input: None, mode: None, error: None, state: None }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Reply { ok: false, decision: None, updated_input: None, mode: None, error: Some(msg.into()), state: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let line = r#"{"kind":"approve","id":7}"#;
        match serde_json::from_str::<Request>(line).unwrap() {
            Request::Approve { id, accept_edits } => assert_eq!((id, accept_edits), (7, false)),
            other => panic!("unexpected {other:?}"),
        }
        match serde_json::from_str::<Request>(r#"{"kind":"approve","id":8,"accept_edits":true}"#).unwrap() {
            Request::Approve { id, accept_edits } => assert_eq!((id, accept_edits), (8, true)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn chat_messages_may_carry_a_file() {
        match serde_json::from_str::<Request>(r#"{"kind":"chat_send","text":"hi","model":null}"#).unwrap() {
            Request::ChatSend { file, .. } => assert_eq!(file, None, "older clients send no file"),
            other => panic!("unexpected {other:?}"),
        }
        match serde_json::from_str::<Request>(r#"{"kind":"chat_send","text":"","model":null,"file":"/tmp/a.rs"}"#).unwrap() {
            Request::ChatSend { file, .. } => assert_eq!(file.as_deref(), Some("/tmp/a.rs")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn history_flag_and_export_requests_roundtrip() {
        match serde_json::from_str::<Request>(r#"{"kind":"history","session_id":"claude:a"}"#).unwrap() {
            Request::History { session_id, query } => assert_eq!((session_id.as_str(), query), ("claude:a", None)),
            other => panic!("unexpected {other:?}"),
        }
        match serde_json::from_str::<Request>(r#"{"kind":"flag_step","session_id":"claude:a","step_id":"t1","flagged":true}"#).unwrap() {
            Request::FlagStep { session_id, step_id, flagged, note } => assert_eq!((session_id.as_str(), step_id.as_str(), flagged, note), ("claude:a", "t1", true, None)),
            other => panic!("unexpected {other:?}"),
        }
        match serde_json::from_str::<Request>(r#"{"kind":"export","session_id":"claude:a","query":"cargo"}"#).unwrap() {
            Request::Export { session_id, query } => assert_eq!((session_id.as_str(), query.as_deref()), ("claude:a", Some("cargo"))),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn hooks_default_to_claude() {
        match serde_json::from_str::<Request>(r#"{"kind":"hook","payload":{}}"#).unwrap() {
            Request::Hook { agent, .. } => assert_eq!(agent, "claude"),
            other => panic!("unexpected {other:?}"),
        }
        match serde_json::from_str::<Request>(r#"{"kind":"hook","agent":"pi","payload":{}}"#).unwrap() {
            Request::Hook { agent, .. } => assert_eq!(agent, "pi"),
            other => panic!("unexpected {other:?}"),
        }
    }
}
