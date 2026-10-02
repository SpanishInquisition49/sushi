// api.js — the bridge to the Rust side (Tauri). Opened in a plain browser (no Tauri) it falls back
// to a mock with sample data, so the interface can be developed and screenshotted without a daemon.

import { mockState } from "./mock.js";

const T = window.__TAURI__;
export const isTauri = !!T;

const listeners = new Map(); // event → Set(callback), for the mock
const local = (name, payload) => (listeners.get(name) || []).forEach((f) => f(payload));

let mockSettings = {};
try {
  mockSettings = JSON.parse(localStorage.getItem("sushi-settings") || "{}");
} catch {}

const mock = {
  get_state: async () => mockState(),
  get_settings: async () => mockSettings,
  set_settings: async ({ settings }) => {
    mockSettings = settings;
    try {
      localStorage.setItem("sushi-settings", JSON.stringify(settings));
    } catch {}
    local("settings", settings);
  },
};

/** Call a Rust command. */
export function invoke(cmd, args = {}) {
  if (T) return T.core.invoke(cmd, args);
  return Promise.resolve(mock[cmd] ? mock[cmd](args) : console.info("[mock] invoke", cmd, args));
}

/** Listen to an event from Rust or another window; the callback gets the payload. */
export function listen(name, fn) {
  if (T) return T.event.listen(name, (e) => fn(e.payload));
  if (!listeners.has(name)) listeners.set(name, new Set());
  listeners.get(name).add(fn);
  return Promise.resolve();
}

/** Send an event to every window (including this one). */
export function emit(name, payload) {
  if (T) return T.event.emit(name, payload);
  local(name, payload);
}

export const startDragging = () => (T ? T.window.getCurrentWindow().startDragging() : undefined);
