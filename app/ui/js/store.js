// store.js — what both windows share: the daemon's snapshot, whether the daemon is up, and the
// app settings. The Rust side polls the daemon and emits "state" / "settings" events.

import { invoke, listen } from "./api.js";

export const store = {
  up: false,
  snapshot: { sessions: [], pending: [], usage: {} },
  settings: {},
};

export const DEFAULT_SETTINGS = {
  character: "nigiri_salmon",
  dockedWindow: true,
  sounds: true,
  fidgets: true,
  napAfterSec: 120,
  widgetInfo: "session", // usage | session | detailed
  autoOpen: true,
  closeAfterDecision: true,
  chatAgent: "claude",
  chatModel: "",
  // Focus mode: a quiet, compact pet during configured hours (see `Fmt.isFocusActive`), or
  // forced on/off regardless of the schedule. Client-local, no daemon involved.
  focusEnabled: false,
  focusStart: "09:00",
  focusEnd: "18:00",
  focusDays: "1111100", // Mon..Sun, '1' = quiet that day
  focusManual: "auto", // "auto" | "on" | "off"
  gamificationEnabled: true,
};

export const setting = (key) => store.settings[key] ?? DEFAULT_SETTINGS[key];

function applyState(payload) {
  if (!payload || typeof payload !== "object") return;
  store.up = payload.up === true;
  const snap = payload.snapshot;
  if (snap && typeof snap === "object") {
    snap.sessions = snap.sessions || [];
    snap.pending = snap.pending || [];
    store.snapshot = snap;
  } else if (!store.up) {
    store.snapshot = { sessions: [], pending: [], usage: {} };
  }
}

/** Load the current state and settings, then keep them up to date. `onState` / `onSettings` run on
 *  every change (and once right now). */
export async function start(onState, onSettings) {
  store.settings = (await invoke("get_settings")) || {};
  applyState(await invoke("get_state"));
  onSettings?.();
  onState?.();
  listen("state", (p) => {
    applyState(p);
    onState?.();
  });
  listen("settings", (s) => {
    store.settings = s || {};
    onSettings?.();
  });
}

/** Change some settings and tell every window. */
export function saveSettings(patch) {
  store.settings = { ...store.settings, ...patch };
  return invoke("set_settings", { settings: store.settings });
}
