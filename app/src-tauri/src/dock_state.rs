//! Presentation policy independent of AppKit, so transitions can be tested without a GUI.
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Collapsed,
    Preview,
    Attention,
    Full,
}
impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Collapsed => "collapsed",
            Self::Preview => "preview",
            Self::Attention => "attention",
            Self::Full => "full",
        }
    }
    pub fn size(self) -> Option<(f64, f64)> {
        match self {
            Self::Collapsed => None,
            Self::Preview => Some((420.0, 220.0)),
            Self::Attention => Some((560.0, 380.0)),
            Self::Full => Some((900.0, 600.0)),
        }
    }
}

#[derive(Default)]
pub struct DockState {
    pub mode: Mode,
    pub enabled: bool,
    pub hovered: bool,
    geometry_ready: bool,
    pending_open: bool,
    known: Vec<String>,
}
impl DockState {
    pub const fn new() -> Self {
        Self {
            mode: Mode::Collapsed,
            enabled: false,
            hovered: false,
            geometry_ready: false,
            pending_open: false,
            known: Vec::new(),
        }
    }
    pub fn hover(&mut self, hovered: bool) {
        self.hovered = hovered;
        if self.mode == Mode::Full {
            return;
        }
        if hovered && (self.enabled || self.mode == Mode::Attention) {
            self.mode = if self.known.is_empty() {
                Mode::Preview
            } else {
                Mode::Attention
            };
        } else if self.mode == Mode::Preview {
            self.mode = Mode::Collapsed;
        }
    }
    pub fn close(&mut self) {
        self.pending_open = false;
        self.mode = Mode::Collapsed;
        self.hovered = false;
    }
    pub fn update(&mut self, keys: Vec<String>, auto_open: bool) {
        let new_wait = keys.iter().any(|key| !self.known.contains(key));
        self.known = keys;
        if self.known.is_empty() || self.mode == Mode::Full {
            self.pending_open = false;
        } else if new_wait && auto_open {
            self.pending_open = true;
        }
        if self.mode == Mode::Full {
            return;
        }
        if self.known.is_empty() && self.mode == Mode::Attention {
            self.mode = if self.hovered && self.enabled {
                Mode::Preview
            } else {
                Mode::Collapsed
            };
        } else if !self.known.is_empty()
            && self.enabled
            && ((self.pending_open && auto_open) || self.mode == Mode::Preview)
        {
            self.mode = Mode::Attention;
            self.pending_open = false;
        }
    }
    pub fn display_changed(&mut self, enabled: bool, auto_open: bool) {
        self.enabled = enabled;
        self.hovered = false;
        if self.mode == Mode::Preview {
            self.mode = Mode::Collapsed;
        }
        if self.pending_open
            && enabled
            && auto_open
            && !self.known.is_empty()
            && self.mode != Mode::Full
        {
            self.mode = Mode::Attention;
            self.pending_open = false;
        }
        self.geometry_ready = true;
    }
    pub fn has_waits(&self) -> bool {
        !self.known.is_empty()
    }
    pub fn awaiting_geometry(&self) -> bool {
        !self.geometry_ready
    }
}

pub fn attention_keys(snapshot: &Value) -> Vec<String> {
    let pending = snapshot["pending"].as_array().cloned().unwrap_or_default();
    let sessions = snapshot["sessions"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut keys: Vec<_> = pending
        .iter()
        .map(|p| {
            // A timed-out hook leaves the same operation waiting in Codex. Keep its
            // identity so dismissing it is not undone when its permission card expires.
            if let Some(s) = sessions.iter().find(|s| s["id"] == p["session_id"]
                && !s["attention"]["created_ms"].is_null()
                && s["attention"]["created_ms"] == p["created_ms"])
            {
                format!("waiting:{}@{}", s["id"], s["attention"]["created_ms"])
            } else {
                format!("request:{}@{}", p["id"], p["created_ms"])
            }
        })
        .collect();
    if let Some(sessions) = snapshot["sessions"].as_array() {
        keys.extend(
            sessions
                .iter()
                .filter(|s| {
                    s["status"] == "waiting" && !pending.iter().any(|p| p["session_id"] == s["id"])
                })
                .map(|s| format!("waiting:{}@{}", s["id"], s["attention"]["created_ms"])),
        );
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn enabled() -> DockState {
        let mut s = DockState::new();
        s.display_changed(true, true);
        s
    }
    fn keys() -> Vec<String> {
        vec!["request:1".into()]
    }
    #[test]
    fn a_dismissed_permission_stays_dismissed_when_its_hook_times_out() {
        let mut snap = json!({"pending":[{"id":1,"created_ms":10,"session_id":"codex:a"}],
            "sessions":[{"id":"codex:a","status":"waiting","attention":{"created_ms":10}}]});
        let mut s = enabled();
        s.update(attention_keys(&snap), true);
        s.close();
        let keys = attention_keys(&snap);
        snap["pending"] = json!([]);
        assert_eq!(attention_keys(&snap), keys);
        s.update(attention_keys(&snap), true);
        assert_eq!(s.mode, Mode::Collapsed);
    }

    #[test]
    fn waits_survive_unavailable_geometry_without_undoing_dismissal() {
        let mut s = DockState::new();
        s.display_changed(false, true);
        s.update(keys(), true);
        assert_eq!(s.mode, Mode::Collapsed);
        s.display_changed(true, true);
        assert_eq!(s.mode, Mode::Attention);
        s.close();
        s.display_changed(false, true);
        s.display_changed(true, true);
        s.update(keys(), true);
        assert_eq!(s.mode, Mode::Collapsed);
        s.display_changed(false, true);
        s.update(vec!["request:2".into()], true);
        s.display_changed(true, true);
        assert_eq!(s.mode, Mode::Attention);

        let mut s = DockState::new();
        s.update(keys(), true);
        s.update(vec![], true);
        s.display_changed(true, true);
        assert_eq!(s.mode, Mode::Collapsed);
        s.display_changed(false, true);
        s.update(keys(), false);
        s.display_changed(true, false);
        assert_eq!(s.mode, Mode::Collapsed);
    }

    #[test]
    fn a_second_question_in_the_same_session_reopens_attention() {
        let mut s = enabled();
        let mut snap = json!({"sessions":[{"id":"codex:a", "status":"waiting", "attention":{"created_ms":10}}]});
        s.update(attention_keys(&snap), true);
        s.close();
        s.update(attention_keys(&snap), true);
        assert_eq!(s.mode, Mode::Collapsed);
        snap["sessions"][0]["attention"]["created_ms"] = json!(20);
        s.update(attention_keys(&snap), true);
        assert_eq!(s.mode, Mode::Attention);
    }

    #[test]
    fn hover_and_wait_resolution_follow_pointer() {
        let mut s = enabled();
        s.hover(true);
        assert_eq!(s.mode, Mode::Preview);
        s.update(keys(), false);
        assert_eq!(s.mode, Mode::Attention);
        s.hover(false);
        assert_eq!(s.mode, Mode::Attention);
        s.update(vec![], true);
        assert_eq!(s.mode, Mode::Collapsed);
        s.hover(true);
        s.update(keys(), true);
        s.update(vec![], true);
        assert_eq!(s.mode, Mode::Preview);
    }
    #[test]
    fn initial_requests_and_disabled_auto_open() {
        let mut s = enabled();
        s.update(keys(), true);
        assert_eq!(s.mode, Mode::Attention);
        let mut s = enabled();
        s.update(keys(), false);
        assert_eq!(s.mode, Mode::Collapsed);
        s.hover(true);
        assert_eq!(s.mode, Mode::Attention);
        let mut s = DockState::new();
        s.update(keys(), true);
        s.display_changed(true, true);
        assert_eq!(s.mode, Mode::Attention);
    }
    #[test]
    fn dismissal_is_not_undone_by_identical_snapshots() {
        let mut s = enabled();
        s.update(keys(), true);
        s.close();
        s.update(keys(), true);
        assert_eq!(s.mode, Mode::Collapsed);
        s.update(vec!["request:1".into(), "request:2".into()], true);
        assert_eq!(s.mode, Mode::Attention);
        s.close();
        s.hover(true);
        assert_eq!(s.mode, Mode::Attention);
    }
    #[test]
    fn full_panel_has_priority_and_display_changes_keep_attention() {
        let mut s = enabled();
        s.mode = Mode::Full;
        s.hover(true);
        s.update(keys(), true);
        s.display_changed(false, true);
        assert_eq!(s.mode, Mode::Full);
        s.mode = Mode::Attention;
        s.display_changed(false, true);
        assert_eq!(s.mode, Mode::Attention);
        s.mode = Mode::Preview;
        s.display_changed(true, true);
        assert_eq!(s.mode, Mode::Collapsed);
    }
    #[test]
    fn waiting_keys_deduplicate_pending_and_ignore_ordinary_updates() {
        let snap = json!({"pending":[{"id":1,"created_ms":10,"session_id":"a"}],
            "sessions":[{"id":"a","status":"waiting"},{"id":"b","status":"waiting","last_event_ms":20}]});
        let mut changed = snap.clone();
        changed["sessions"][1]["last_event_ms"] = json!(40);
        assert_eq!(attention_keys(&snap).len(), 2);
        assert_eq!(attention_keys(&snap), attention_keys(&changed));
        let mut s = enabled();
        s.update(attention_keys(&snap), true);
        s.close();
        s.update(attention_keys(&changed), true);
        assert_eq!(s.mode, Mode::Collapsed);
        s.update(vec![], true);
        s.update(attention_keys(&snap), true);
        assert_eq!(s.mode, Mode::Attention);
    }
}
