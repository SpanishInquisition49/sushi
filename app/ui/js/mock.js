// mock.js — sample state for opening the interface in a plain browser (no Tauri, no daemon).
// Add `?mock=idle`, `?mock=ask`, `?mock=down` to the URL to see other situations.

const mode = new URLSearchParams(location.search).get("mock") || "work";

export function mockState() {
  const now = Date.now();
  if (mode === "down") return { up: false, snapshot: null };

  const diff = {
    type: "diff", lang: "rust", file: "/home/me/proj/src/paths.rs", more: 0,
    lines: [
      { kind: "ctx", n: 4, text: "fn runtime_dir() -> PathBuf {" },
      { kind: "del", n: 5, text: '    PathBuf::from("/tmp")' },
      { kind: "add", n: 5, text: '    if let Some(d) = env::var_os("XDG_RUNTIME_DIR") {' },
      { kind: "add", n: 6, text: "        return PathBuf::from(d); // a string with \"quotes\"" },
      { kind: "add", n: 7, text: "    }" },
      { kind: "ctx", n: 8, text: "}" },
    ],
  };
  const steps = [
    { tool: "Read", label: "src/paths.rs", ok: true, ts_ms: now - 30000, detail: { type: "file", lang: "rust", file: "src/paths.rs", start: 1, lines: ["use std::env;", "use std::path::PathBuf;", "", "pub fn socket_path() -> PathBuf {", "    runtime_dir().join(\"sushi.sock\")", "}"] } },
    { tool: "Bash", label: "cargo test", ok: false, ts_ms: now - 20000, detail: { type: "terminal", command: "cargo test", output: ["running 77 tests", "test result: FAILED. 76 passed; 1 failed"] } },
    { tool: "Edit", label: "src/paths.rs", ok: mode === "idle" ? true : null, ts_ms: now - 2000, added: 3, removed: 1, detail: diff },
  ];
  const working = mode === "work";
  const sessions = [
    {
      id: "claude:abc", name: "sushi", agent: "claude", status: working ? "working" : "idle", last_event_ms: now - 1000,
      last_tool: { kind: "write", text: "Edit: src/paths.rs" },
      context: { percent: 42, window: 200000, history: [10, 14, 18, 25, 31, 36, 40, 42] },
      activity: {
        recent: steps, tool_calls: 3, files_changed: 1, lines_added: 3, lines_removed: 1, commands: 1, failures: 1,
        files: ["/home/me/proj/src/paths.rs"], turn_started_ms: now - 95000, turn_ms: 95000,
        finished_ms: working ? null : now - 5000, last_result: working ? null : "Done: the runtime directory now works on macOS.",
      },
    },
    { id: "codex:x1", name: "api", agent: "codex", status: "idle", last_event_ms: now - 600000, context: { percent: 12, window: 200000 }, activity: { recent: [] } },
  ];
  const pending = mode === "ask"
    ? [{
        id: 7, kind: "permission", tool_name: "Bash", tool: "rm -rf target", session_name: "sushi", session_id: "claude:abc", agent: "claude", created_ms: now - 3000,
        detail: { type: "terminal", command: "rm -rf target", output: [] },
      }]
    : [];
  return {
    up: true,
    snapshot: {
      version: 2,
      agents: {
        claude: { label: "Claude Code", capabilities: { chat: true, limits: true }, limits: { data: { five_hour: { percent: 44, resets_at_ms: now + 7200000 }, seven_day: { percent: 71, resets_at_ms: now + 3 * 86400000 }, models: [{ model: "Opus", percent: 18, resets_at_ms: now + 86400000 }] } } },
        codex: { label: "Codex", capabilities: { chat: true } },
      },
      sessions, pending,
      usage: { claude: { today: { input: 12400, output: 88000, cache_read: 2400000 }, total: { input: 340000, output: 1900000 } } },
      chat: { busy: false, error: null, agent: "claude", messages: [{ role: "user", text: "What does SIGKILL do?" }, { role: "assistant", text: "SIGKILL ends a process immediately; it cannot be caught or ignored." }] },
    },
  };
}
