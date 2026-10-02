// panel.js — the wide window (port of plugin/panel.luau).
//
//  left   the pet (with a status bubble), the session name and its clock, the steps of the
//         current turn (done ✓, running ◌, failed ✗, "Done") and the sessions
//  right  tabs Live / Chat / Usage / Settings, permission requests with Allow / Deny, and the live
//         viewer: the diff of the file being edited (typewriter effect, line numbers, syntax
//         colors), the terminal of a command, or the file that was read; the Chat tab talks to the
//         daemon's headless chat
//
// Clicking a step shows that step in the viewer; clicking a session pins it (click it again to
// follow the active session). Everything is drawn from the daemon's snapshot.

import { invoke, listen, emit } from "./api.js";
import { store, setting, start, saveSettings, DEFAULT_SETTINGS } from "./store.js";
import { ui, esc } from "./ui.js";
import { Pet, configFrom, CHARACTER_IDS } from "./pet.js";
import * as Fmt from "./fmt.js";
import * as Viewer from "./viewer.js";

const HERO_SIZE = 92;
const FRAME_MS = 33; // ~30 fps
const TYPE_WINDOW_MS = 8000; // only steps this recent get the typewriter effect
const MAX_RAIL = 7;
const SPINNER_STEP_MS = 40;

const $ = (id) => document.getElementById(id);
const now = () => Date.now();

const pet = new Pet(now(), configFrom(store.settings));

// ── UI state ───────────────────────────────────────────────────────────────────

const mem = {
  get(key, fallback) {
    try {
      return localStorage.getItem("sushi-" + key) ?? fallback;
    } catch {
      return fallback;
    }
  },
  set(key, value) {
    try {
      if (value == null) localStorage.removeItem("sushi-" + key);
      else localStorage.setItem("sushi-" + key, value);
    } catch {}
  },
};

// live | chat | usage | settings (`?tab=` overrides the remembered one, for development)
let tab = new URLSearchParams(location.search).get("tab") || mem.get("tab", "live");
let pinnedSession = mem.get("session", null); // null = follow the active session
let pinnedStep = null; // ts_ms of a step chosen in the rail (null = follow the latest)
let pinnedTurn = null; // the turn that step belongs to
const seenAt = new Map(); // first time a step was drawn (drives the typewriter)
let optimistic = null; // {text, base}: a message just sent, shown until the daemon confirms it
let lastChatRev = "";
const choices = new Map(); // pending id → question index → Set(labels)

/** Make the pet react here and in the pet window (events travel through Tauri). */
function sendEvent(kind) {
  const ts = now();
  pet.onEvent(kind, ts, ts);
  emit("petEvent", { kind, ts });
}

// ── Small HTML helpers ─────────────────────────────────────────────────────────

/** Replace the content of `el` only when it changed (keeps scroll position and avoids flicker). */
function setHtml(el, html) {
  if (el._html === html) return;
  const top = el.scrollTop;
  el.innerHTML = html;
  el._html = html;
  el.scrollTop = top;
}

function btn(label, { variant = "ghost", act, data = {}, tip, off, glyph, small } = {}) {
  const d = Object.entries({ act, ...data })
    .filter(([, v]) => v !== undefined)
    .map(([k, v]) => ` data-${k}="${esc(v)}"`)
    .join("");
  return `<button class="btn ${variant}${small ? " sm" : ""}"${d}${tip ? ` title="${esc(tip)}"` : ""}${off ? " disabled" : ""}>${
    glyph ? ui.glyph({ name: glyph, size: 16 }) : ""
  }${label ? `<span>${esc(label)}</span>` : ""}</button>`;
}

const STATUS = {
  idle: { label: "idle", color: "on_surface_variant" },
  working: { label: "working", color: "primary" },
  waiting: { label: "waiting", color: "error" },
};

// ── Selection ──────────────────────────────────────────────────────────────────

const snap = () => store.snapshot;

function currentSession() {
  if (pinnedSession) {
    const s = snap().sessions.find((x) => x.id === pinnedSession);
    if (s) return s;
  }
  return Fmt.activeSession(snap());
}

const stepsOf = (s) => s?.activity?.recent || [];

/** The step shown in the viewer, and whether it is the latest one (follows the session). */
function currentStep(s) {
  const list = stepsOf(s);
  if (pinnedStep != null) {
    if (pinnedTurn === s.activity?.turn_started_ms) {
      const e = list.find((x) => x.ts_ms === pinnedStep);
      if (e) return [e, false];
    }
    pinnedStep = null; // a new turn started, or the step scrolled out of the list
  }
  return [list.at(-1), true];
}

function setTab(id) {
  tab = id;
  mem.set("tab", id);
  renderRight();
  if (id === "chat") setTimeout(() => $("chatInput")?.focus(), 0);
}

function pinSession(id) {
  pinnedSession = pinnedSession === id ? null : id;
  pinnedStep = null;
  mem.set("session", pinnedSession);
  renderAll();
}

function pinStep(s, e) {
  if (pinnedStep === e.ts_ms) pinnedStep = null;
  else {
    [pinnedStep, pinnedTurn] = [e.ts_ms, s.activity?.turn_started_ms];
    if (tab !== "live") return setTab("live");
  }
  renderAll();
}

// ── Decisions ──────────────────────────────────────────────────────────────────

/** Settle a pending request: take its card away at once (the daemon's next snapshot confirms it, and
 *  brings it back if the answer was refused), let the pet react, send the request, and close the
 *  panel if nothing is left to answer. */
function settle(id, send, eventKind) {
  const kept = snap().pending.filter((p) => p.id !== id);
  store.snapshot = { ...snap(), pending: kept };
  sendEvent(eventKind); // love / sad, with their own sounds
  Promise.resolve(send()).catch((e) => console.warn("sushi: request failed:", e));
  if (!kept.length && setting("closeAfterDecision")) invoke("close_panel");
  else renderAll();
}

const decide = (action, id) => settle(id, () => invoke("decide", { action, id }), action === "approve" ? "approve" : "deny");

const picked = (id, qi) => choices.get(id)?.get(qi) || new Set();

function questionsAnswers(p) {
  const answers = {};
  const list = p.detail?.questions || [];
  for (const [qi, q] of list.entries()) {
    const chosen = picked(p.id, qi);
    const labels = (q.options || []).filter((o) => chosen.has(o.label)).map((o) => o.label);
    if (!labels.length) return null; // not answered yet
    answers[q.question] = labels.join(", ");
  }
  return answers;
}

function sendAnswers(p) {
  const answers = questionsAnswers(p);
  if (!answers) return;
  choices.delete(p.id);
  settle(p.id, () => invoke("answer", { id: p.id, answers }), "approve");
}

function choose(p, qi, label) {
  const q = (p.detail?.questions || [])[qi];
  if (!q) return;
  if (!choices.has(p.id)) choices.set(p.id, new Map());
  const c = choices.get(p.id);
  if (q.multi) {
    const set = new Set(c.get(qi) || []);
    set.has(label) ? set.delete(label) : set.add(label);
    c.set(qi, set);
  } else c.set(qi, new Set([label]));
  const list = p.detail?.questions || [];
  if (list.length === 1 && !q.multi) sendAnswers(p); // one question, one choice: nothing more to ask
  else renderRight();
}

// ── Left column ────────────────────────────────────────────────────────────────

const SPINNER_ANGLE = (t) => Math.floor(t / SPINNER_STEP_MS) * 30;

function counts() {
  const working = snap().sessions.filter((s) => s.status === "working").length;
  return [working, snap().pending.length];
}

function petBlock(nWorking, nPending) {
  // The bubble follows what the sessions are doing, not the pet's passing emote, so clicking an
  // animation does not make it vanish and reappear.
  const badge = nPending > 0 ? "alert" : nWorking > 0 ? "work" : undefined;
  return ui.row({ cls: "poke", data: { act: "poke" }, title: "Poke it" }, [
    pet.build(HERO_SIZE, { room: 20, maxHeight: 124, margin: 20, badge, badgeSlot: true }),
  ]);
}

function titleBlock(s, nWorking, nPending, t) {
  let sub = s ? Fmt.agentLabel(snap(), s.agent) : "Coding agents";
  if (s) {
    const elapsed = Fmt.turnElapsed(s, t);
    if (elapsed != null) sub += (s.status === "idle" ? " · last turn " : " · ") + Fmt.duration(elapsed);
  }
  return ui.column({ gap: 2 }, [
    ui.label({ text: s ? s.name : "No session", fontSize: 20, fontWeight: "bold", color: "on_surface", maxLines: 1 }),
    ui.label({ text: sub, fontSize: 12, color: "on_surface_variant", maxLines: 1 }),
    ui.label({ text: pet.caption(snap().sessions.length, nWorking, nPending), fontSize: 12, color: "primary", maxLines: 2 }),
  ]);
}

function railRow(s, e, t) {
  const running = e.ok == null;
  let icon;
  if (e.ok === true) icon = ui.glyph({ name: "circle-check-filled", size: 18, color: Viewer.COLORS.ok });
  else if (e.ok === false) icon = ui.glyph({ name: "circle-x-filled", size: 18, color: Viewer.COLORS.fail });
  else icon = ui.glyph({ name: "loader", size: 18, color: "on_surface", rotate: SPINNER_ANGLE(t) });
  return ui.row(
    {
      cls: "item" + (pinnedStep === e.ts_ms ? " selected" : ""), data: { act: "pin-step", ts: e.ts_ms },
      align: "center", gap: 10, paddingH: 8, paddingV: 5, radius: 8, title: `${e.tool} ${e.label || ""}`,
    },
    [
      icon,
      ui.label({ text: e.tool, fontSize: 15, fontWeight: running ? "bold" : "semibold", color: running ? "on_surface" : "on_surface_variant" }),
      ui.label({ text: e.label || "", fontSize: 11, color: "on_surface_variant", maxLines: 1, flexGrow: 1 }),
    ],
  );
}

function rail(s, t) {
  const rows = [];
  if (s) {
    for (const e of stepsOf(s).slice(-MAX_RAIL)) rows.push(railRow(s, e, t));
    if (s.status === "idle" && s.activity?.finished_ms) {
      rows.push(
        ui.row({ align: "center", gap: 10, paddingH: 8, paddingV: 5 }, [
          ui.glyph({ name: "circle-check-filled", size: 18, color: "on_surface_variant", opacity: 0.6 }),
          ui.label({ text: "Done", fontSize: 15, fontWeight: "semibold", color: "on_surface_variant", opacity: 0.7 }),
        ]),
      );
    }
  }
  if (!rows.length) rows.push(ui.label({ text: "No steps yet.", fontSize: 12, color: "on_surface_variant" }));
  return ui.column({ gap: 1 }, rows);
}

function sessionList(current) {
  const rank = (s) => (s.status === "working" || s.status === "waiting" ? 1 : 0);
  const list = [...snap().sessions].sort(
    (a, b) => rank(b) - rank(a) || (b.last_event_ms || 0) - (a.last_event_ms || 0) || String(a.name).localeCompare(String(b.name)),
  );
  const multi = Fmt.multiAgent(snap());
  const rows = list.map((s) => {
    const st = STATUS[s.status] || STATUS.idle;
    return ui.row(
      {
        cls: "item" + (s === current ? " selected" : ""), data: { act: "pin-session", id: s.id }, align: "center", gap: 8,
        paddingH: 8, paddingV: 5, radius: 8,
        title: pinnedSession === s.id ? "Pinned: click to follow the active session" : `Show ${s.name || s.id}`,
      },
      [
        ui.box({ width: 8, height: 8, radius: 4, fill: st.color }),
        ui.label({ text: s.name || s.id, fontSize: 12, fontWeight: "semibold", color: "on_surface", maxLines: 1, flexGrow: 1 }),
        ui.label({ text: multi ? Fmt.agentLabel(snap(), s.agent) : "", fontSize: 10, color: "on_surface_variant", maxLines: 1 }),
        ui.label({ text: s.context ? Fmt.round(s.context.percent) + "%" : "", fontSize: 11, color: s.context ? Fmt.percentColor(s.context.percent) : "on_surface_variant" }),
      ],
    );
  });
  if (!rows.length) rows.push(ui.label({ text: "No active sessions", fontSize: 12, color: "on_surface_variant" }));
  return ui.column({ gap: 2 }, rows);
}

// ── Right column: permissions ──────────────────────────────────────────────────

function questionCard(p, who) {
  const list = p.detail?.questions || [];
  const children = [
    ui.row({ align: "center", gap: 8 }, [
      ui.glyph({ name: "help-circle", size: 18, color: "primary" }),
      ui.label({ text: `${who} is asking you`, fontSize: 14, fontWeight: "semibold", color: "on_surface", maxLines: 1, flexGrow: 1 }),
    ]),
  ];
  for (const [qi, q] of list.entries()) {
    const chosen = picked(p.id, qi);
    const block = [];
    if (q.header) block.push(ui.label({ text: q.header, fontSize: 10, fontWeight: "bold", color: "primary" }));
    block.push(ui.label({ text: q.question, fontSize: 14, fontWeight: "semibold", color: "on_surface", maxLines: 4, maxWidth: 520 }));
    if (q.multi) block.push(ui.label({ text: "Choose one or more", fontSize: 11, color: "on_surface_variant" }));
    for (const o of q.options || []) {
      const on = chosen.has(o.label);
      block.push(
        `<div class="option">${btn(on && q.multi ? "✓ " + o.label : o.label, {
          variant: on ? "primary" : "outline", act: "choose", data: { id: p.id, qi, label: o.label }, tip: o.description || undefined,
        })}${ui.label({ text: o.description || "", fontSize: 11, color: "on_surface_variant", maxLines: 2, maxWidth: 360, flexGrow: 1 })}</div>`,
      );
    }
    children.push(ui.column({ gap: 6 }, block));
  }
  // Several questions (or a multiple choice) need an explicit send.
  if (!(list.length === 1 && !list[0].multi)) {
    children.push(btn("Send answers", { variant: "primary", act: "send-answers", data: { id: p.id }, off: questionsAnswers(p) == null }));
  }
  children.push(ui.label({ text: "You can also answer it in the terminal.", fontSize: 11, color: "on_surface_variant" }));
  return ui.column({ gap: 10, padding: 10, fill: "primary/0.10", radius: 12 }, children);
}

/** A plan the agent wants approved: that dialog only exists in the terminal, so it is shown here for
 *  reading and goes away once it has been dealt with there. */
function planCard(p, who) {
  const children = [
    ui.row({ align: "center", gap: 8 }, [
      ui.glyph({ name: "list-check", size: 18, color: "primary" }),
      ui.label({ text: `${who} has a plan ready`, fontSize: 14, fontWeight: "semibold", color: "on_surface", maxLines: 1, flexGrow: 1 }),
    ]),
  ];
  if (p.detail && typeof p.detail === "object") children.push(Viewer.render(p.detail, { bodyLines: 14 }));
  children.push(ui.label({ text: "Review and approve it in the terminal.", fontSize: 11, color: "on_surface_variant" }));
  return ui.column({ gap: 8, padding: 10, fill: "primary/0.10", radius: 12 }, children);
}

function permissionCard(p) {
  let who = p.session_name || p.session_id;
  if (p.agent && Fmt.multiAgent(snap())) who = Fmt.agentLabel(snap(), p.agent) + " · " + who;
  if (p.kind === "question" && p.detail?.type === "questions") return questionCard(p, who);
  if (p.kind === "plan") return planCard(p, who);
  const what = p.tool_name || p.tool || "a tool";
  const children = [
    ui.row({ align: "center", gap: 8 }, [
      ui.glyph({ name: "alert-triangle", size: 18, color: "#f59e0b" }),
      ui.label({ text: `${who} wants to use ${what}`, fontSize: 14, fontWeight: "semibold", color: "on_surface", maxLines: 1, flexGrow: 1 }),
    ]),
  ];
  if (p.detail && typeof p.detail === "object") children.push(Viewer.render(p.detail, { maxRows: 9 }));
  else if (p.tool) children.push(ui.label({ text: p.tool, fontSize: 12, color: "on_surface_variant", maxLines: 3 }));
  children.push(
    `<div class="btn-row">${btn("Allow", { variant: "primary", act: "allow", data: { id: p.id } })}${btn("Deny", { variant: "outline", act: "deny", data: { id: p.id } })}</div>`,
  );
  return ui.column({ gap: 8, padding: 10, fill: "#f59e0b/0.10", radius: 12 }, children);
}

// ── Right column: Live ─────────────────────────────────────────────────────────

function stepLine(e) {
  const right = [];
  if (e.added > 0 || e.removed > 0) right.push(ui.label({ text: `+${e.added || 0} -${e.removed || 0}`, fontSize: 11, color: "on_surface_variant" }));
  const status = e.ok === true ? "done" : e.ok === false ? "failed" : "running";
  right.push(ui.label({ text: status, fontSize: 11, fontWeight: "semibold", color: e.ok === false ? Viewer.COLORS.fail : e.ok === true ? Viewer.COLORS.ok : "primary" }));
  return ui.row({ align: "center", gap: 8 }, [
    ui.label({ text: `${e.tool}  ${e.label || ""}`, fontSize: 12, color: "on_surface_variant", maxLines: 1, flexGrow: 1 }),
    ui.row({ gap: 8, align: "center" }, right),
  ]);
}

/** The viewer for a step. The latest step of a working session is typed out. */
function viewerFor(s, e, t, following) {
  const detail = e.detail;
  const opts = { ok: e.ok, maxRows: 22 };
  if (following && detail?.type === "diff" && t - e.ts_ms < TYPE_WINDOW_MS) {
    const key = `${s.id}:${e.ts_ms}`;
    const first = seenAt.get(key) ?? t;
    seenAt.set(key, first);
    const total = Viewer.addedChars(detail);
    const speed = Math.max(1 / 22, total / 1500); // chars per ms; never slower than 22 ms/char, done in 1.5 s
    const n = Math.floor((t - first) * speed);
    if (n < total || e.ok == null) {
      opts.typed = Math.min(n, total);
      opts.cursor = Math.floor(t / 450) % 2 === 0;
    }
  }
  return Viewer.render(detail, opts);
}

const resultCard = (text) =>
  ui.column({ gap: 4, padding: 12, fill: "surface_variant/0.35", radius: 10 }, [
    ui.label({ text: "Result", fontSize: 11, color: "on_surface_variant" }),
    ui.label({ text: text, fontSize: 13, color: "on_surface", maxLines: 8 }),
  ]);

function liveContent(s, t) {
  const nodes = [];
  if (!s) {
    nodes.push(Viewer.render(null, { empty: "No agent session yet. Start Claude Code, Codex, opencode or pi in a terminal and it shows up here." }));
  } else {
    const [e, following] = currentStep(s);
    if (e) nodes.push(stepLine(e), viewerFor(s, e, t, following));
    else nodes.push(Viewer.render(null, { empty: s.status === "idle" ? "Idle: nothing running." : "Waiting for the first step of this turn…" }));
    if (s.activity?.last_result && s.status === "idle") nodes.push(resultCard(s.activity.last_result));
  }
  return ui.column({ gap: 10 }, nodes);
}

// ── Right column: Chat ─────────────────────────────────────────────────────────

const chatAgent = () => setting("chatAgent") || "claude";

function bubble(m, streaming) {
  const mine = m.role === "user";
  let text = m.text || "";
  if (!text && streaming) text = "…";
  const box = ui.column({ padding: 10, radius: 12, fill: mine ? "primary/0.22" : "surface_variant/0.45", maxWidth: 430 }, [
    ui.label({ text, fontSize: 13, color: "on_surface" }),
  ]);
  return ui.row({ justify: mine ? "end" : "start" }, [box]);
}

function chatBusy() {
  return snap().chat?.busy === true || optimistic != null;
}

/** The conversation log and the buttons; the input itself is never rebuilt (it keeps focus and text). */
function renderChat() {
  const chat = snap().chat || {};
  const msgs = chat.messages || [];
  // The daemon's state caught up with what we sent: drop the provisional message.
  if (optimistic && msgs.length > optimistic.base) optimistic = null;
  const who = Fmt.agentLabel(snap(), chat.agent || chatAgent());
  const rows = msgs.map((m, i) => bubble(m, chat.busy && i === msgs.length - 1));
  if (optimistic) rows.push(bubble({ role: "user", text: optimistic.text }), bubble({ role: "assistant", text: "" }, true));
  if (!rows.length) {
    rows.push(
      ui.column({ gap: 6, padding: 6 }, [
        ui.label({ text: `Ask ${who} anything`, fontSize: 16, fontWeight: "semibold", color: "on_surface" }),
        ui.label({
          text: `Quick questions and answers, right from here. This chat has no tools and no access to your files; it uses your ${who} login.`,
          fontSize: 12, color: "on_surface_variant", maxLines: 4, maxWidth: 430,
        }),
      ]),
    );
  }
  if (chat.error) rows.push(ui.label({ text: String(chat.error), fontSize: 12, color: "error", maxLines: 4, maxWidth: 430 }));

  // Keep the log pinned to the bottom whenever it grows or the answer streams in.
  const last = msgs.at(-1);
  const rev = `${msgs.length}:${(last?.text || "").length}:${optimistic ? 1 : 0}:${chat.error || ""}`;
  const log = $("chatLog");
  setHtml(log, ui.column({ gap: 8 }, rows));
  if (rev !== lastChatRev) {
    lastChatRev = rev;
    log.scrollTop = log.scrollHeight;
  }

  const busy = chatBusy();
  const input = $("chatInput");
  input.disabled = busy;
  input.placeholder = busy ? `${who} is answering…` : `Ask ${who} anything…`;
  const ready = input.value.trim() !== "";
  setHtml(
    $("chatBtns"),
    (busy
      ? btn("", { variant: "outline", act: "chat-stop", glyph: "player-stop", tip: "Stop", small: true })
      : btn("", { variant: ready ? "primary" : "ghost", act: "chat-send", glyph: "send", tip: "Send", small: true, off: !ready })) +
      btn("", { act: "chat-clear", glyph: "trash", tip: "New conversation", small: true }),
  );
}

function submitChat() {
  const input = $("chatInput");
  const text = input.value.trim();
  const chat = snap().chat || {};
  if (!text || chat.busy || optimistic) return;
  optimistic = { text, base: (chat.messages || []).length };
  input.value = "";
  const model = setting("chatModel");
  invoke("chat_send", { text, model: model || null, agent: chatAgent() }).catch((e) => {
    optimistic = null;
    console.warn("sushi: chat failed:", e);
    renderChat();
  });
  renderChat();
}

// ── Right column: Usage ────────────────────────────────────────────────────────

const stat = (label, value, color) =>
  ui.column({ gap: 1 }, [
    ui.label({ text: label, fontSize: 10, color: "on_surface_variant" }),
    ui.label({ text: value, fontSize: 13, fontWeight: "semibold", color: color || "on_surface" }),
  ]);

function limitBar(label, w, t) {
  const pct = Math.max(0, Math.min(100, Number(w.percent) || 0));
  return ui.column({ gap: 3 }, [
    ui.row({ align: "center", justify: "space_between" }, [
      ui.label({ text: label, fontSize: 12, color: "on_surface" }),
      ui.label({ text: `${Fmt.round(w.percent)}% · resets in ${Fmt.resetIn(w.resets_at_ms, t)}`, fontSize: 11, color: "on_surface_variant" }),
    ]),
    `<div class="track"><div class="bar" style="width:${pct}%;background:var(--${Fmt.percentColor(w.percent).replace("_", "-")})"></div></div>`,
  ]);
}

function limitsSection(t) {
  const lim = Fmt.limits(snap()) || {};
  const data = lim.data;
  if (!data) {
    return ui.label({ text: "Plan limits unavailable" + (lim.error ? ": " + lim.error : ""), fontSize: 11, color: "on_surface_variant", maxLines: 2 });
  }
  const bars = [];
  if (data.five_hour) bars.push(limitBar("5-hour limit", data.five_hour, t));
  if (data.seven_day) bars.push(limitBar("Weekly limit", data.seven_day, t));
  for (const m of data.models || []) bars.push(limitBar(`${m.model} weekly`, m, t));
  return ui.column({ gap: 8 }, bars);
}

function contextTrend(s) {
  const ctx = s?.context;
  if (!(ctx && Array.isArray(ctx.history) && ctx.history.length >= 2)) return null;
  const values = ctx.history.map((p) => Math.max(0, Math.min(1, (Number(p) || 0) / 100)));
  const W = 400, H = 48;
  const pts = values.map((v, i) => `${((i / (values.length - 1)) * W).toFixed(1)},${(H - 2 - v * (H - 4)).toFixed(1)}`);
  const color = `var(--${Fmt.percentColor(ctx.percent).replace("_", "-")})`;
  return ui.column({ gap: 3 }, [
    ui.row({ align: "center", justify: "space_between" }, [
      ui.label({ text: `Context trend · ${s.name || ""}`, fontSize: 11, color: "on_surface_variant" }),
      ui.label({ text: `${Fmt.round(ctx.percent)}% of ~${Fmt.tokens(ctx.window)}`, fontSize: 11, color: Fmt.percentColor(ctx.percent) }),
    ]),
    `<svg viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" style="width:100%;height:${H}px;color:${color}">
       <polygon points="0,${H} ${pts.join(" ")} ${W},${H}" fill="currentColor" opacity=".18"/>
       <polyline points="${pts.join(" ")}" fill="none" stroke="currentColor" stroke-width="2" vector-effect="non-scaling-stroke" stroke-linejoin="round"/>
     </svg>`,
  ]);
}

function usageContent(s, t) {
  const u = snap().usage?.claude || {};
  const today = u.today || {}, total = u.total || {};
  const a = s?.activity || {};
  const nodes = [ui.label({ text: "Plan usage", fontSize: 12, fontWeight: "semibold", color: "on_surface" }), limitsSection(t)];
  if (s) {
    nodes.push(
      ui.separator({ spacing: 4 }),
      ui.label({ text: `This turn · ${s.name || ""}`, fontSize: 12, fontWeight: "semibold", color: "on_surface" }),
      ui.row({ gap: 20 }, [
        stat("Tools", String(a.tool_calls || 0)),
        stat("Files", a.files_changed > 0 ? `${a.files_changed}  +${a.lines_added || 0} -${a.lines_removed || 0}` : "0"),
        stat("Commands", Fmt.commandsText(a) || "0", a.failures > 0 ? "error" : undefined),
      ]),
    );
    const trend = contextTrend(s);
    if (trend) nodes.push(trend);
  }
  nodes.push(
    ui.separator({ spacing: 4 }),
    ui.label({ text: `Today: ${Fmt.tokens(today.output)} out · ${Fmt.tokens(today.input)} in · ${Fmt.tokens(today.cache_read)} cache`, fontSize: 12, color: "on_surface" }),
    ui.label({ text: `Total (recent transcripts): ${Fmt.tokens(total.output)} out · ${Fmt.tokens(total.input)} in`, fontSize: 11, color: "on_surface_variant" }),
  );
  return ui.column({ gap: 10 }, nodes);
}

// ── Right column: Settings ─────────────────────────────────────────────────────

const prettify = (id) => id.replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase());

function settingsView() {
  const row = (label, hint, control) =>
    `<label class="setting"><span class="setting-text"><b>${esc(label)}</b><small>${esc(hint)}</small></span>${control}</label>`;
  const select = (key, options) =>
    `<select data-setting="${key}">${options.map(([v, l]) => `<option value="${esc(v)}"${String(setting(key)) === String(v) ? " selected" : ""}>${esc(l)}</option>`).join("")}</select>`;
  const check = (key) => `<input type="checkbox" data-setting="${key}"${setting(key) ? " checked" : ""}>`;
  const text = (key, ph = "") => `<input type="text" data-setting="${key}" value="${esc(setting(key))}" placeholder="${esc(ph)}">`;
  const num = (key, min, max) => `<input type="number" data-setting="${key}" min="${min}" max="${max}" value="${esc(setting(key))}">`;
  return [
    row("Character", "Who lives in your pet window.", select("character", CHARACTER_IDS.map((id) => [id, prettify(id)]))),
    row("Sounds", "Play the pet's little sounds.", check("sounds")),
    row("Random fidgets", "Hop, yawn, wink… while idle.", check("fidgets")),
    row("Nap after (seconds)", "Idle time before it falls asleep. 0 = never.", num("napAfterSec", 0, 3600)),
    row("Pet window shows", "What goes next to the pet.", select("widgetInfo", [["usage", "Usage only"], ["session", "Session"], ["detailed", "Detailed"]])),
    row("Open on permission request", "Bring the panel up when an agent asks.", check("autoOpen")),
    row("Close after Allow / Deny", "One click, back to work.", check("closeAfterDecision")),
    row("Chat agent", "Which agent answers in the Chat tab.", select("chatAgent", [["claude", "Claude Code"], ["copilot", "GitHub Copilot"], ["pi", "pi"], ["codex", "Codex"]])),
    row("Chat model", "Empty = the agent's own default.", text("chatModel", "default")),
  ].join("");
}

// ── Assembly ───────────────────────────────────────────────────────────────────

function tabs() {
  const t = (id, label) => btn(label, { variant: tab === id ? "primary" : "ghost", act: "tab", data: { id }, small: true });
  return (
    `<div class="tabs">${t("live", "Live")}${Fmt.canChat(snap()) ? t("chat", "Chat") : ""}${t("usage", "Usage")}` +
    `<span class="grow"></span>${btn("", { act: "tab", data: { id: "settings" }, glyph: "settings", tip: "Settings", small: true, variant: tab === "settings" ? "primary" : "ghost" })}` +
    `${btn("", { act: "close", glyph: "x", tip: "Close" })}</div>`
  );
}

function renderLeft(t) {
  const s = currentSession();
  const [nWorking, nPending] = counts();
  setHtml($("title"), titleBlock(s, nWorking, nPending, t));
  setHtml($("rail"), rail(s, t));
  setHtml($("sessions"), sessionList(s));
}

function renderRight() {
  const t = now();
  const s = currentSession();
  if (tab === "chat" && !Fmt.canChat(snap())) tab = "live";
  setHtml($("tabs"), tabs());
  const down = !store.up;
  setHtml(
    $("perms"),
    down ? "" : snap().pending.map((p) => permissionCard(p)).join(""),
  );
  const show = (id, on) => $(id).classList.toggle("hidden", !on);
  show("content", tab === "live" || tab === "usage" || down);
  show("chat", tab === "chat" && !down);
  show("settings", tab === "settings" && !down);
  if (down) {
    setHtml(
      $("content"),
      ui.column({ gap: 10 }, [
        ui.label({ text: "Daemon is not running.", fontSize: 14, fontWeight: "semibold", color: "error" }),
        ui.label({ text: "Start `sushi daemon` to see your sessions.", fontSize: 12, color: "on_surface_variant" }),
      ]),
    );
  } else if (tab === "usage") setHtml($("content"), usageContent(s, t));
  else if (tab === "live") setHtml($("content"), liveContent(s, t));
  else if (tab === "chat") renderChat();
  else if (tab === "settings" && !$("settings")._built) {
    $("settings").innerHTML = settingsView();
    $("settings")._built = true;
  }
}

function renderAll() {
  renderLeft(now());
  renderRight();
}

// ── Events ─────────────────────────────────────────────────────────────────────

function onClick(e) {
  const el = e.target.closest("[data-act]");
  if (!el || el.disabled) return;
  const d = el.dataset;
  const id = d.id !== undefined && /^\d+$/.test(d.id) ? Number(d.id) : d.id;
  switch (d.act) {
    case "tab": return setTab(d.id);
    case "close": return invoke("close_panel");
    case "emote": return sendEvent(d.kind);
    case "poke": return pet.poke(now());
    case "allow": return decide("approve", id);
    case "deny": return decide("deny", id);
    case "choose": {
      const p = snap().pending.find((x) => x.id === id);
      return p && choose(p, Number(d.qi), d.label);
    }
    case "send-answers": {
      const p = snap().pending.find((x) => x.id === id);
      return p && sendAnswers(p);
    }
    case "pin-session": return pinSession(d.id);
    case "pin-step": {
      const s = currentSession();
      const step = s && stepsOf(s).find((x) => String(x.ts_ms) === d.ts);
      return step && pinStep(s, step);
    }
    case "chat-send": return submitChat();
    case "chat-stop": return void invoke("chat_stop");
    case "chat-clear":
      optimistic = null;
      $("chatInput").value = "";
      return void invoke("chat_clear").then(renderChat);
  }
}

function onSettingChange(e) {
  const el = e.target.closest("[data-setting]");
  if (!el) return;
  const key = el.dataset.setting;
  let value = el.type === "checkbox" ? el.checked : el.type === "number" ? Number(el.value) : el.value;
  if (el.type === "number" && !Number.isFinite(value)) value = DEFAULT_SETTINGS[key];
  saveSettings({ [key]: value });
}

/** The eyes follow the pointer, relative to the pet at the top left. */
function onMouseMove(e) {
  const r = $("petBlock").getBoundingClientRect();
  const cx = r.left + r.width / 2, cy = r.top + r.height / 2;
  pet.lookAt(Math.max(-1, Math.min(1, (e.clientX - cx) / 360)), Math.max(-1, Math.min(1, (e.clientY - cy) / 240)));
}

const SKELETON = `
<div class="panel">
  <aside class="left">
    <div id="petBlock"></div>
    <div id="title"></div>
    <div class="emotes">
      ${[["heart", "Pet it", "love"], ["cookie", "Feed it", "eat"], ["music", "Dance!", "dance"], ["moon", "Nap time", "nap"]]
        .map(([g, tip, kind]) => btn("", { act: "emote", data: { kind }, glyph: g, tip, small: true }))
        .join("")}
    </div>
    ${ui.separator({ spacing: 2 })}
    <div id="rail"></div>
    ${ui.separator({ spacing: 2 })}
    <div class="muted small">Sessions</div>
    <div id="sessions" class="scroll grow"></div>
  </aside>
  <div class="vsep"></div>
  <main class="right">
    <div id="tabs"></div>
    <div id="perms" class="perms"></div>
    <div id="content" class="scroll grow"></div>
    <div id="chat" class="chat grow hidden">
      <div id="chatLog" class="scroll grow"></div>
      <div class="chat-input"><input id="chatInput" type="text" autocomplete="off" spellcheck="true"><span id="chatBtns" class="btn-row"></span></div>
    </div>
    <div id="settings" class="scroll grow hidden"></div>
  </main>
</div>`;

export function mount(root) {
  root.innerHTML = SKELETON;
  root.addEventListener("click", onClick);
  root.addEventListener("change", onSettingChange);
  document.addEventListener("mousemove", onMouseMove);
  document.addEventListener("mouseleave", () => pet.lookAt(null));
  $("chatInput").addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.isComposing) {
      e.preventDefault();
      submitChat();
    }
  });
  $("chatInput").addEventListener("input", renderChat);

  listen("petEvent", (ev) => ev && pet.onEvent(ev.kind, ev.ts, now()));

  start(
    () => {
      pet.onSnapshot(snap(), store.up, now());
      renderAll();
    },
    () => {
      pet.configure(configFrom(store.settings));
      const s = $("settings");
      if (s) s._built = false; // rebuilt the next time the tab is shown
      if (tab === "settings") renderRight();
    },
  );

  // Animation: advance the pet and redraw it; the parts that move with time follow at a slower rate.
  let last = 0, slow = 0;
  const frame = () => {
    const t = now();
    if (t - last >= FRAME_MS) {
      last = t;
      pet.tick(t);
      const [nWorking, nPending] = counts();
      setHtml($("petBlock"), petBlock(nWorking, nPending));
      if (t - slow >= 120) {
        slow = t;
        renderLeft(t);
        if (tab === "live" || tab === "usage") renderRight();
      }
    }
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}
