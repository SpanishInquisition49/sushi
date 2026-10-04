// fmt.js — formatting helpers shared by the pet window and the panel (port of lib/fmt.luau).

export const round = (x) => Math.floor((Number(x) || 0) + 0.5);

/** Compact token count: 950, 1.2k, 3.4M. */
export function tokens(n) {
  n = Number(n) || 0;
  if (n >= 1e9) return (n / 1e9).toFixed(1) + "G";
  if (n >= 1e6) return (n / 1e6).toFixed(1) + "M";
  if (n >= 1e3) return (n / 1e3).toFixed(1) + "k";
  return String(Math.floor(n));
}

/** Compact in-game currency count: "0", "42", "1.2k" (see src/care.rs's `currency`). */
export function coins(n) {
  return tokens(n);
}

/** Palette role for a usage percentage: calm, warning, critical. */
export function percentColor(p) {
  p = Number(p) || 0;
  if (p >= 90) return "error";
  if (p >= 70) return "tertiary";
  return "primary";
}

/** "2h 15m", "3d 4h", "now": time left until `resetsAtMs`. */
export function resetIn(resetsAtMs, nowMs) {
  if (!resetsAtMs) return "?";
  const s = Math.floor((resetsAtMs - nowMs) / 1000);
  if (s <= 0) return "now";
  const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60);
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  return `${Math.max(1, m)}m`;
}

/** "2:13" or "1:02:03" for a duration in milliseconds. */
export function duration(ms) {
  const total = Math.max(0, Math.floor((Number(ms) || 0) / 1000));
  const h = Math.floor(total / 3600), m = Math.floor((total % 3600) / 60), sec = total % 60;
  const two = (n) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${two(m)}:${two(sec)}` : `${m}:${two(sec)}`;
}

/** Sum of a `Tokens` object's 4 fields (input/output/cache_read/cache_write). */
export const tokenSum = (t) => (t?.input || 0) + (t?.output || 0) + (t?.cache_read || 0) + (t?.cache_write || 0);

/** "$0.42", "<$0.01", "$0": an estimated cost, never a billed amount. */
export function cost(usd) {
  const n = Number(usd) || 0;
  if (n <= 0) return "$0";
  if (n < 0.01) return "<$0.01";
  return "$" + n.toFixed(2);
}

/** Monday-first day index (0=Mon..6=Sun) from JS's Sunday-first `Date#getDay`. */
const mondayFirst = (jsDay) => (jsDay + 6) % 7;

/** "HH:MM" to minutes since midnight, or null if not a valid time. */
function minutesOf(hhmm) {
  const m = /^(\d{1,2}):(\d{2})$/.exec(String(hhmm || "").trim());
  if (!m) return null;
  const h = Number(m[1]), mi = Number(m[2]);
  return Number.isFinite(h) && Number.isFinite(mi) ? (h % 24) * 60 + (mi % 60) : null;
}

/** Whether "focus mode" (a quiet, compact pet) is active right now: `settings.focusManual`
 *  ("on"/"off") overrides everything; otherwise a configured schedule — `focusDays` (7 chars,
 *  '1'/'0', Monday first) and `focusStart`/`focusEnd` ("HH:MM", which may wrap past midnight,
 *  e.g. 22:00–06:00). All client-local settings; no daemon involved. */
export function isFocusActive(settings, now = new Date()) {
  const manual = settings?.focusManual || "auto";
  if (manual === "on") return true;
  if (manual === "off") return false;
  if (!settings?.focusEnabled) return false;
  const days = String(settings.focusDays || "1111100");
  const start = minutesOf(settings.focusStart);
  const end = minutesOf(settings.focusEnd);
  if (start == null || end == null || start === end) return false;
  const nowMin = now.getHours() * 60 + now.getMinutes();
  const day = mondayFirst(now.getDay());
  if (start < end) return days[day] === "1" && nowMin >= start && nowMin < end;
  if (nowMin >= start) return days[day] === "1";
  const previousDay = (day + 6) % 7;
  return nowMin < end && days[previousDay] === "1";
}

/** Shorten `text` to at most `n` characters, adding an ellipsis. */
export function clip(text, n) {
  const chars = Array.from(String(text ?? ""));
  return chars.length > n ? chars.slice(0, n - 1).join("") + "…" : chars.join("");
}

/** The session to describe: one working or waiting (most recent first), else the latest turn. */
export function activeSession(snap) {
  let best = null, bestScore = -1, bestEvent = -1;
  for (const s of snap?.sessions || []) {
    const busy = s.status === "working" || s.status === "waiting";
    const score = busy ? 2 : s.activity?.turn_started_ms ? 1 : 0;
    const ev = s.last_event_ms || 0;
    if (score > bestScore || (score === bestScore && ev > bestEvent)) [best, bestScore, bestEvent] = [s, score, ev];
  }
  return best;
}

/** Label of an agent ("Claude Code", "pi"...) from the snapshot's `agents`. */
export const agentLabel = (snap, id) => snap?.agents?.[id]?.label || String(id ?? "agent");

/** Plan limits ({ id, label, data, error }) of every agent that reports them (Claude Code,
 *  Codex, Copilot, Antigravity today — see `agent::Capabilities::limits`). */
export function allLimits(snap) {
  const out = [];
  for (const [id, a] of Object.entries(snap?.agents || {})) if (a.limits) out.push({ id, label: a.label, ...a.limits });
  return out;
}

/** Plan limits ({ data, error }) of the first agent that has them, or null — for spots that show
 *  a single number and don't need to tell agents apart. */
export function limits(snap) {
  return allLimits(snap)[0] || null;
}

/** Does any agent offer the built-in chat? (Assumed yes when the snapshot does not say.) */
export function canChat(snap) {
  if (!snap?.agents) return true;
  return Object.values(snap.agents).some((a) => a.capabilities?.chat);
}

/** How many things wait for you: the pending requests, plus the sessions blocked on their own
 *  terminal with nothing to answer here (Antigravity asking, or a request whose hook gave up). */
export function needsYou(snap) {
  const pending = snap?.pending || [];
  const asked = new Set(pending.map((p) => p.session_id));
  return pending.length + (snap?.sessions || []).filter((s) => s.status === "waiting" && !asked.has(s.id)).length;
}

// ── Following a session's steps in the Live viewer ──

/** How long the typewriter takes to type a diff's added lines (as the viewers type it). */
export function typingMs(detail) {
  if (detail?.type !== "diff") return 0;
  const total = (detail.lines || []).filter((l) => l.kind === "add").reduce((n, l) => n + Array.from(l.text || "").length, 0);
  return Math.min(total * 22, 1500);
}

/** How long a step stays in the viewer before the next one replaces it: an edit until it is typed
 *  out and a moment to read it, a command long enough to see it, a read or a search briefly. */
export function dwellMs(e) {
  const type = e?.detail?.type;
  if (type === "diff") return Math.max(1800, typingMs(e.detail) + 1200);
  if (type === "terminal") return 1500;
  if (type === "file") return 700;
  return 400;
}

const worthStopping = (e) => e?.detail?.type === "diff" || e?.detail?.type === "terminal";

/** The step to show while following a session. Steps arrive faster than they can be read, so each
 *  stays for `dwellMs` before the next; reads and searches with an edit or a command after them are
 *  skipped, and it never lags more than three steps. `state` ({key, since}) is kept by the caller across frames. */
export function followStep(state, sessionId, list, now) {
  if (!list.length) {
    state.key = null;
    return null;
  }
  const last = list.length - 1;
  const keyOf = (e) => `${sessionId}:${e.ts_ms}`;
  let i = list.findIndex((e) => keyOf(e) === state.key);
  if (i < 0) i = last; // a new session, a new turn, or the step scrolled out of the list
  else if (i < last && now - state.since >= dwellMs(list[i])) {
    i++;
    while (i < last && !worthStopping(list[i])) i++;
    i = Math.max(i, last - 3);
  } else return list[i];
  state.key = keyOf(list[i]);
  state.since = now;
  return list[i];
}

/** More than one agent has a session right now. */
export const multiAgent = (snap) => new Set((snap?.sessions || []).map((s) => s.agent).filter(Boolean)).size > 1;

/** "Bash: cargo test": the last tool a session used, or null. */
export const lastToolText = (s) => (s?.last_tool && typeof s.last_tool === "object" ? s.last_tool.text : null);

/** How long the current turn has been running (or took, once finished); null if unknown. */
export function turnElapsed(s, nowMs) {
  const a = s?.activity;
  if (!a) return null;
  if (s.status !== "idle" && a.turn_started_ms) return nowMs - a.turn_started_ms;
  return a.turn_ms ?? null;
}

/** "3 files · +45 -12", or null when nothing was changed this turn. */
export function changesText(a) {
  if (!a || !(a.files_changed > 0)) return null;
  const files = a.files_changed === 1 ? "1 file" : `${a.files_changed} files`;
  return `${files} · +${a.lines_added || 0} -${a.lines_removed || 0}`;
}

/** "4 run · 1 failed", or null when no command ran. */
export function commandsText(a) {
  if (!a || !(a.commands > 0)) return null;
  return `${a.commands} run` + (a.failures > 0 ? ` · ${a.failures} failed` : "");
}

/** `{ day, tokens, estimated_cost_usd }` entries of `usage_history.by_day`, oldest first, capped
 *  to the most recent `max` days — for the Usage tab's day-by-day bar row (see src/usage_history.rs). */
export function byDaySeries(usageHistory, max = 14) {
  const byDay = usageHistory?.by_day || {};
  return Object.keys(byDay).sort().slice(-max).map((day) => ({ day, ...byDay[day] }));
}

/** Tooltip rows describing the active session's progress: [{key, value}]. */
export function sessionRows(s, nowMs) {
  if (!s) return [];
  const a = s.activity || {};
  const rows = [];
  let head = s.status === "working" ? "working" : s.status === "waiting" ? "waiting for you" : "idle";
  const elapsed = turnElapsed(s, nowMs);
  if (elapsed != null) head += (s.status === "idle" ? " · last turn " : " · ") + duration(elapsed);
  if (a.tool_calls > 0) head += ` · ${a.tool_calls} tools`;
  rows.push({ key: s.name || s.id, value: head });
  const ch = changesText(a);
  if (ch) {
    const names = (a.files || []).slice(0, 3).map((f) => f.split("/").pop() || f);
    rows.push({ key: "Changes", value: clip(`${ch} (${names.join(", ")}${a.files_changed > 3 ? ", …" : ""})`, 80) });
  }
  const cm = commandsText(a);
  if (cm) rows.push({ key: "Commands", value: cm });
  const last = (a.recent || []).at(-1);
  if (last && s.status !== "idle") rows.push({ key: last.ok == null ? "Now" : "Last step", value: clip(`${last.tool} ${last.label || ""}`, 70) });
  if (s.context) rows.push({ key: "Context", value: `${round(s.context.percent)}% of ~${tokens(s.context.window)}` });
  if (a.last_result && (s.status === "idle" || elapsed == null)) rows.push({ key: "Result", value: clip(a.last_result, 90) });
  return rows;
}
