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
    Approve { id: u64 },
    Deny { id: u64 },
    State,
    /// Send a message to the built-in chat (`model` and `agent` override the configured ones).
    ChatSend {
        text: String,
        model: Option<String>,
        #[serde(default)]
        agent: Option<String>,
    },
    ChatStop,
    ChatClear,
    /// Answer a question: `answers` maps each question text to the chosen label.
    Answer { id: u64, answers: Value },
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
}

impl Reply {
    pub fn ok() -> Self {
        Reply { ok: true, decision: None, updated_input: None, error: None, state: None }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Reply { ok: false, decision: None, updated_input: None, error: Some(msg.into()), state: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let line = r#"{"kind":"approve","id":7}"#;
        match serde_json::from_str::<Request>(line).unwrap() {
            Request::Approve { id } => assert_eq!(id, 7),
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
