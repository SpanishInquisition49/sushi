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
import { ui, esc, col } from "./ui.js";
import { Pet, configFrom, CHARACTER_IDS, ACCESSORIES, accessoryArt } from "./pet.js";
import * as Fmt from "./fmt.js";
import * as Viewer from "./viewer.js";

const HERO_SIZE = 92;
const FRAME_MS = 33; // ~30 fps
const TYPE_WINDOW_MS = 20000; // only steps this recent get the typewriter effect (they may be shown a little late)
const MAX_RAIL = 6; // rows of the step rail ("Done" included) with a single session
const SESSION_ROWS_MIN = 3; // session rows the rail always leaves room for below it
const SESSION_ROW_H = 28; // height of a session row in px
const SESSIONS_MAX_ROWS = 5; // sessions is capped (not flex-grow): it must never fight the rail for space or overlap it
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
let pinnedStep = null; // ts_ms of a step chosen in the rail (null = follow the session)
const follow = { key: null, since: 0 }; // the step the viewer follows (see Fmt.followStep)
let typing = false; // the viewer is typing a diff out (redrawn at the full frame rate)
let pinnedTurn = null; // the turn that step belongs to
const seenAt = new Map(); // first time a step was drawn (drives the typewriter)
let optimistic = null; // {text, file, base}: a message just sent, shown until the daemon confirms it
let fedFile = null; // {path, name}: a file fed to the pet, sent with the next chat message
let lastChatRev = "";
let lastRailRev = ""; // see renderLeft: keeps the rail (now a short scroll box) pinned to the newest step
const choices = new Map(); // pending id → question index → Set(labels)

/** Make the pet react here and in the pet window (events travel through Tauri). */
function sendEvent(kind) {
  const ts = now();
  pet.onEvent(kind, ts, ts);
  emit("petEvent", { kind, ts });
}

// Tamagotchi care (see src/care.rs): the 3 emote buttons that double as care actions (love/eat/nap
// each gated by its own cooldown server-side — see `next_*_ms` below); dance stays purely cosmetic.
// The cosmetic reaction only plays once the daemon actually accepts the action, so a refused click
// (still cooling down) does not mislead with an animation that did nothing.
const CARE_ACTION = { love: "care_pet", eat: "care_feed", nap: "care_nap" };
const CARE_NEXT_KEY = { love: "next_pet_ms", eat: "next_feed_ms", nap: "next_nap_ms" };
const EMOTES = [["heart", "Pet it", "love"], ["cookie", "Feed it", "eat"], ["music", "Dance!", "dance"], ["bed", "Nap time", "nap"]];

/** Seconds left before `kind`'s care action is available again (0 once its cooldown has elapsed).
 *  See `care::CARE_COOLDOWN_MS`: each of love/eat/nap has its own independent clock. */
function careCooldown(kind) {
  const key = CARE_NEXT_KEY[kind];
  if (!key) return 0;
  const next = snap().care?.[key] || 0;
  return Math.max(0, Math.ceil((next - now()) / 1000));
}

/** The 4 emote buttons: greyed out with a countdown while their care action is still cooling
 *  down, so repeated clicking can no longer max a need out in one burst (see careCooldown). */
function emotesContent() {
  return EMOTES.map(([g, tip, kind]) => {
    const cooldown = careCooldown(kind);
    return btn("", { act: "emote", data: { kind }, glyph: g, tip: cooldown > 0 ? `${tip} · ready in ${cooldown}s` : tip, small: true, off: cooldown > 0 });
  }).join("");
}

// Three interchangeable rounds, any of which can report a score to the same `care_play` (see
// src/care.rs's `Care::play`): it is game-agnostic by design, so adding a game here never needs a
// daemon change. `playGame` picks which; `playState` (idle | running | done) is shared.
const PLAY_GAMES = ["tap", "catch", "memory"];
let playGame = PLAY_GAMES.includes(mem.get("playGame", "tap")) ? mem.get("playGame", "tap") : "tap";
let playState = "idle";
let playResult = null; // { line, rewarded, newBest } for the "done" view, set by whichever game finished
let playBest = {
  tap: Number(mem.get("playBest", 0)) || 0, // kept under its original key for existing high scores
  catch: Number(mem.get("playBest_catch", 0)) || 0,
  memory: Number(mem.get("playBest_memory", 0)) || 0,
};

// tap: tap the pet as many times as possible in 8s; score = taps, capped at 100.
let playTaps = 0;
let playEndsAt = 0;
// catch: the lit side jumps between left/right on its own; tap the lit one, not the dark one.
let catchHits = 0;
let catchEndsAt = 0;
let catchLeftIsTarget = true;
let catchNextRerollAt = 0;
const CATCH_DURATION_MS = 8000, CATCH_REROLL_MS = 850, CATCH_SCORE_SCALE = 9;
// memory: Simon-style growing pattern; no time limit, since getting it wrong is its own end.
let memLevel = 1;
let memSeq = [];
let memPhase = "flash"; // flash | gap | input
let memActivePad = null;
let memShowIdx = 0;
let memInputIdx = 0;
let memNextAt = 0;
const MEMORY_FLASH_MS = 420, MEMORY_GAP_MS = 180, MEMORY_SCORE_PER_LEVEL = 14;
const randomPad = () => Math.floor(Math.random() * 4);

/** Seconds left before a mini-game round can earn a reward again (0 once ready); see
 *  `care::PLAY_COOLDOWN_MS`. Playing while this is positive still works, it just won't pay out.
 *  Shared by all three games — the cooldown is on the reward, not any one of them. */
function playCooldown() {
  return Math.max(0, Math.ceil(((snap().care?.next_play_ms || 0) - now()) / 1000));
}

/** Records the result of a finished round: updates that game's best (kept per-game, since "best"
 *  means a different thing for each one), then reports the score once (`Care::play` itself
 *  rate-limits the actual reward — this just makes sure it is asked once). */
function finishPlay(score, metric, line) {
  const best = playBest[playGame];
  const newBest = metric > best;
  if (newBest) {
    playBest[playGame] = metric;
    mem.set(playGame === "tap" ? "playBest" : `playBest_${playGame}`, String(metric));
  }
  playState = "done";
  playResult = { line: line(newBest), rewarded: playCooldown() === 0, newBest };
  invoke("care_play", { score: Math.min(100, score) }).catch(() => {});
}

function tickTap() {
  if (playGame === "tap" && playState === "running" && now() >= playEndsAt) {
    finishPlay(playTaps, playTaps, (nb) => (nb ? `${playTaps} taps — new best!` : `${playTaps} taps!`));
  }
}

function tickCatch() {
  if (playGame !== "catch" || playState !== "running") return;
  const t = now();
  if (t >= catchNextRerollAt) {
    catchLeftIsTarget = Math.random() < 0.5;
    catchNextRerollAt = t + CATCH_REROLL_MS;
  }
  if (t >= catchEndsAt) {
    finishPlay(catchHits * CATCH_SCORE_SCALE, catchHits, (nb) => (nb ? `${catchHits} catches — new best!` : `${catchHits} catches!`));
  }
}

/** A hit on `side` ("left" | "right"); only the currently-lit side scores — the wrong one is just
 *  a miss, never a penalty (this project never punishes a care action, see src/care.rs). */
function catchTap(side) {
  if (playGame !== "catch" || playState !== "running") return;
  if ((side === "left") === catchLeftIsTarget) {
    catchHits++;
    catchLeftIsTarget = Math.random() < 0.5;
    catchNextRerollAt = now() + CATCH_REROLL_MS;
  }
}

function tickMemory() {
  if (playGame !== "memory" || playState !== "running" || memPhase === "input") return;
  const t = now();
  if (t < memNextAt) return;
  if (memPhase === "flash") {
    memPhase = "gap";
    memActivePad = null;
    memNextAt = t + MEMORY_GAP_MS;
  } else {
    memShowIdx++;
    if (memShowIdx >= memSeq.length) {
      memPhase = "input";
      memInputIdx = 0;
      memActivePad = null;
    } else {
      memPhase = "flash";
      memActivePad = memSeq[memShowIdx];
      memNextAt = t + MEMORY_FLASH_MS;
    }
  }
}

/** A tap on pad 0..3 during the input phase: right pad advances (and, on a completed sequence,
 *  grows it and replays); a wrong one ends the round — there's no time limit, so this is the only
 *  way it ends. */
function memoryTap(pad) {
  if (playGame !== "memory" || playState !== "running" || memPhase !== "input") return;
  if (pad === memSeq[memInputIdx]) {
    memInputIdx++;
    if (memInputIdx >= memSeq.length) {
      memLevel++;
      memSeq.push(randomPad());
      memShowIdx = 0;
      memPhase = "flash";
      memActivePad = memSeq[0];
      memNextAt = now() + MEMORY_FLASH_MS;
    }
    return;
  }
  const completed = memLevel - 1;
  memPhase = "idle";
  finishPlay(completed * MEMORY_SCORE_PER_LEVEL, completed, (nb) =>
    nb ? `${completed} rounds remembered — new best!` : `${completed} rounds remembered!`,
  );
}

function tickPlay() {
  tickTap();
  tickCatch();
  tickMemory();
}

const NEED_COLOR = (pct) => (pct < 35 ? "error" : pct < 60 ? "tertiary" : "primary");

/** The hunger / energy / affection meters (see src/care.rs), compact: one row, three mini-bars
 *  each with a one-letter label — hover (or long-press on touch) for the full name and percent.
 *  (Was 3 full-width labeled bars; that crowded out the sessions list below it.) */
function careMeters() {
  const c = snap().care || {};
  const meter = (letter, label, pct) => {
    const p = Math.max(0, Math.min(100, pct ?? 100));
    return ui.column({ flexGrow: 1, gap: 2, title: `${label} ${Math.round(p)}%` }, [
      ui.label({ text: letter, fontSize: 9, color: "on_surface_variant" }),
      `<div style="height:5px;border-radius:3px;overflow:hidden;background:${col("surface_variant")};">` +
        `<div style="width:${p}%;height:100%;background:${col(NEED_COLOR(p))};"></div></div>`,
    ]);
  };
  return ui.row({ gap: 10 }, [meter("H", "Hunger", c.hunger), meter("E", "Energy", c.energy), meter("A", "Affection", c.affection)]);
}

/** The shop grid: buy with currency earned from real agent usage, equip at most one at a time
 *  (see ACCESSORIES in pet.js, mirrored server-side in src/care.rs so a tampered client can't
 *  grant itself a free item). */
function shopContent() {
  const c = snap().care || {};
  const owned = new Set(c.owned || []);
  const currency = c.currency || 0;
  const cards = Object.entries(ACCESSORIES).map(([id, a]) => {
    const isEquipped = c.equipped === id;
    const action = isEquipped
      ? btn("Unequip", { act: "accessory-equip", data: { id: "" }, small: true })
      : owned.has(id)
        ? btn("Equip", { act: "accessory-equip", data: { id }, small: true, variant: "primary" })
        : btn(`Buy · ${a.cost}`, { act: "accessory-buy", data: { id }, small: true, variant: "primary", off: currency < a.cost });
    return ui.row({ align: "center", justify: "space_between" }, [
      ui.row({ align: "center", gap: 8 }, [
        ui.column({ width: 22, height: 22, align: "center", justify: "center" }, [accessoryArt(id, 20)]),
        ui.label({ text: a.label, fontSize: 12, color: "on_surface" }),
      ]),
      action,
    ]);
  });
  return ui.column({ gap: 10 }, [
    ui.row({ align: "center", justify: "space_between" }, [
      ui.label({ text: "Shop", fontSize: 12, fontWeight: "semibold", color: "on_surface" }),
      ui.label({ text: `🪙 ${Fmt.coins(currency)}`, fontSize: 12, color: "on_surface_variant" }),
    ]),
    ui.column({ gap: 8 }, cards),
  ]);
}

const GAME_META = {
  tap: {
    label: "Tap the pet!", switcherLabel: "Tap",
    hint: (cd) => cd > 0
      ? `Tap as many times as you can in 8 seconds. You can still practice, but the next reward is ${cd}s away.`
      : "Tap as many times as you can in 8 seconds. A round can only earn a reward once every 5 minutes.",
    bestLine: (n) => `Best: ${n}`,
  },
  catch: {
    label: "Catch it!", switcherLabel: "Catch",
    hint: (cd) => cd > 0
      ? `The lit side jumps around on its own — tap it, not the dark one, for 8 seconds. Practice freely; the next reward is ${cd}s away.`
      : "The lit side jumps around on its own — tap it, not the dark one, for 8 seconds. A round can only earn a reward once every 5 minutes.",
    bestLine: (n) => `Best: ${n}`,
  },
  memory: {
    label: "Memory", switcherLabel: "Memory",
    hint: (cd) => cd > 0
      ? `Watch the pads light up, then repeat the pattern — it only gets longer, no time limit. Practice freely; the next reward is ${cd}s away.`
      : "Watch the pads light up, then repeat the pattern — it only gets longer, no time limit. A round can only earn a reward once every 5 minutes.",
    bestLine: (n) => `Best: ${n} rounds`,
  },
};

function gameSwitcher() {
  return ui.row({ gap: 6 }, PLAY_GAMES.map((g) =>
    btn(GAME_META[g].switcherLabel, { act: "play-game", data: { id: g }, small: true, variant: playGame === g ? "primary" : "ghost" }),
  ));
}

/** The running view for whichever game is active (see tickTap/tickCatch/tickMemory for the rules). */
function runningContent() {
  if (playGame === "tap") {
    const remaining = Math.max(0, playEndsAt - now());
    return ui.column({ gap: 10 }, [
      ui.row({ align: "center", justify: "space_between" }, [
        ui.label({ text: `${(remaining / 1000).toFixed(1)}s`, fontSize: 16, fontWeight: "bold", color: "on_surface" }),
        ui.label({ text: `${playTaps} taps`, fontSize: 16, fontWeight: "bold", color: "primary" }),
      ]),
      `<button class="btn primary" data-act="play-tap" style="height:72px;font-size:20px;">Tap!</button>`,
    ]);
  }
  if (playGame === "catch") {
    const remaining = Math.max(0, catchEndsAt - now());
    const side = (isLeft) => {
      const on = isLeft === catchLeftIsTarget;
      return `<button class="btn ${on ? "primary" : "ghost"}" data-act="catch-tap" data-side="${isLeft ? "left" : "right"}" style="height:72px;flex:1;font-size:22px;">${on ? "●" : "○"}</button>`;
    };
    return ui.column({ gap: 10 }, [
      ui.row({ align: "center", justify: "space_between" }, [
        ui.label({ text: `${(remaining / 1000).toFixed(1)}s`, fontSize: 16, fontWeight: "bold", color: "on_surface" }),
        ui.label({ text: `${catchHits} hits`, fontSize: 16, fontWeight: "bold", color: "primary" }),
      ]),
      `<div style="display:flex;gap:8px;">${side(true)}${side(false)}</div>`,
    ]);
  }
  const pads = [0, 1, 2, 3].map((p) => {
    const on = memActivePad === p;
    return `<button class="btn ${on ? "primary" : "ghost"}" data-act="memory-tap" data-pad="${p}" style="height:56px;flex:1;font-size:14px;"${memPhase !== "input" ? " disabled" : ""}>${p + 1}</button>`;
  });
  return ui.column({ gap: 10 }, [
    ui.row({ align: "center", justify: "space_between" }, [
      ui.label({ text: `Level ${memLevel}`, fontSize: 16, fontWeight: "bold", color: "on_surface" }),
      ui.label({ text: memPhase === "input" ? "Your turn" : "Watch...", fontSize: 12, color: "on_surface_variant" }),
    ]),
    `<div style="display:flex;gap:8px;">${pads.join("")}</div>`,
  ]);
}

/** The mini-games: a switcher at top picks which, each reports a 0..100 score to the same
 *  `care_play` (see src/care.rs's `Care::play` for how that becomes a reward, and its 5-minute
 *  cooldown — shared across all three, so switching games does not reset it). */
function playContent() {
  const meta = GAME_META[playGame];
  const switcher = gameSwitcher();
  if (playState === "idle") {
    const best = playBest[playGame];
    return ui.column({ gap: 10 }, [
      switcher,
      ui.row({ align: "center", justify: "space_between" }, [
        ui.label({ text: meta.label, fontSize: 12, fontWeight: "semibold", color: "on_surface" }),
        best > 0 ? ui.label({ text: meta.bestLine(best), fontSize: 11, color: "on_surface_variant" }) : "",
      ]),
      ui.label({ text: meta.hint(playCooldown()), fontSize: 11, color: "on_surface_variant", maxLines: 3 }),
      btn("Start", { act: "play-start", variant: "primary", small: true }),
    ]);
  }
  if (playState === "running") return ui.column({ gap: 10 }, [switcher, runningContent()]);
  return ui.column({ gap: 10 }, [
    switcher,
    ui.label({
      text: playResult.line, fontSize: 14, fontWeight: "semibold", color: playResult.newBest ? "primary" : "on_surface",
    }),
    ui.label({
      text: playResult.rewarded ? "Sent to your pet — check its currency and affection." : "That one was just for fun — the next reward was still cooling down.",
      fontSize: 11, color: "on_surface_variant",
    }),
    btn("Play again", { act: "play-again", small: true }),
  ]);
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

/** The step shown in the viewer, and whether it follows the session (rather than being pinned). */
function currentStep(s, t) {
  const list = stepsOf(s);
  if (pinnedStep != null) {
    if (pinnedTurn === s.activity?.turn_started_ms) {
      const e = list.find((x) => x.ts_ms === pinnedStep);
      if (e) return [e, false];
    }
    pinnedStep = null; // a new turn started, or the step scrolled out of the list
  }
  return [Fmt.followStep(follow, s.id, list, t), true];
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

const decide = (action, id) => settle(id, () => invoke("decide", { action, id }), action === "deny" ? "deny" : "approve");

/** Toggle `focusManual` between forced on and the configured schedule ("auto"). */
function toggleFocus() {
  saveSettings({ focusManual: (setting("focusManual") || "auto") === "on" ? "auto" : "on" });
}

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
  return [working, Fmt.needsYou(snap())];
}

function petBlock(nWorking, nPending) {
  // The bubble follows what the sessions are doing, not the pet's passing emote, so clicking an
  // animation does not make it vanish and reappear.
  const badge = nPending > 0 ? "alert" : nWorking > 0 ? "work" : undefined;
  const focusDim = Fmt.isFocusActive(store.settings);
  return ui.row({ cls: "poke", data: { act: "poke" }, title: focusDim ? "Poke it (focus mode)" : "Poke it" }, [
    pet.build(HERO_SIZE, { room: 20, maxHeight: 124, margin: 20, badge, badgeSlot: true, focusDim, accessories: true }),
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
  // Flagged by a configured policy rule (see policy.rs): shown for every agent, even though real
  // enforcement only exists for Claude Code's own permissions (see the README).
  const policyBadge = e.policy ? ui.glyph({ name: "alert-triangle", size: 14, color: e.policy.level === "deny" ? "error" : "#f59e0b" }) : null;
  const title = e.policy ? `${e.tool} ${e.label || ""} — policy: ${e.policy.label}` : `${e.tool} ${e.label || ""}`;
  return ui.row(
    {
      cls: "item" + (pinnedStep === e.ts_ms ? " selected" : pinnedStep == null && follow.key === `${s.id}:${e.ts_ms}` ? " shown" : ""),
      data: { act: "pin-step", ts: e.ts_ms },
      align: "center", gap: 10, paddingH: 8, paddingV: 5, radius: 8, title,
    },
    [
      icon,
      ui.label({ text: e.tool, fontSize: 15, fontWeight: running ? "bold" : "semibold", color: running ? "on_surface" : "on_surface_variant" }),
      ui.label({ text: e.label || "", fontSize: 11, color: "on_surface_variant", maxLines: 1, flexGrow: 1 }),
      policyBadge,
    ],
  );
}

/** How many session rows the rail's own row budget assumes (see `rail`) — sessions itself is now
 *  capped independently (see SESSIONS_MAX_ROWS), this is just about leaving the rail reasonably
 *  short rather than a long list of steps. */
const sessionRowsShown = () => Math.max(1, Math.min(snap().sessions.length, SESSION_ROWS_MIN));

function rail(s, t) {
  const rows = [];
  if (s) {
    // Fewer steps when there are several sessions, so a long turn's step list doesn't dominate.
    const done = s.status === "idle" && !!s.activity?.finished_ms;
    const room = MAX_RAIL - sessionRowsShown() + 1 - (done ? 1 : 0);
    for (const e of stepsOf(s).slice(-room)) rows.push(railRow(s, e, t));
    if (done) {
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

/** A plan the agent wants approved (Claude Code's "Ready to code?"). While its hook waits it can be
 *  approved here; after that it stays for reading until it has been dealt with in the terminal. */
function planCard(p, who) {
  const children = [
    ui.row({ align: "center", gap: 8 }, [
      ui.glyph({ name: "list-check", size: 18, color: "primary" }),
      ui.label({ text: `${who} has a plan ready`, fontSize: 14, fontWeight: "semibold", color: "on_surface", maxLines: 1, flexGrow: 1 }),
    ]),
  ];
  if (p.detail && typeof p.detail === "object") children.push(Viewer.render(p.detail, { bodyLines: 14 }));
  if (p.answerable === false) {
    children.push(ui.label({ text: "Review and approve it in the terminal.", fontSize: 11, color: "on_surface_variant" }));
  } else {
    const data = { id: p.id };
    children.push(
      `<div class="btn-row">${btn("Approve", { variant: "primary", act: "plan-approve", data, tip: "Claude asks before each edit" })}${btn("Approve, auto-accept edits", { variant: "outline", act: "plan-approve-edits", data })}${btn("Keep planning", { variant: "outline", act: "deny", data, tip: "Then tell Claude what to change in the terminal" })}</div>`,
    );
    children.push(ui.label({ text: "You can also answer it in the terminal.", fontSize: 11, color: "on_surface_variant" }));
  }
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

function stepLine(e, t) {
  const right = [];
  if (e.added > 0 || e.removed > 0) right.push(ui.label({ text: `+${e.added || 0} -${e.removed || 0}`, fontSize: 11, color: "on_surface_variant" }));
  const status = e.ok === true ? "done" : e.ok === false ? "failed" : `running · ${Fmt.duration(t - e.ts_ms)}`;
  right.push(ui.label({ text: status, fontSize: 11, fontWeight: "semibold", color: e.ok === false ? Viewer.COLORS.fail : e.ok === true ? Viewer.COLORS.ok : "primary" }));
  return ui.row({ align: "center", gap: 8 }, [
    ui.label({ text: `${e.tool}  ${e.label || ""}`, fontSize: 12, color: "on_surface_variant", maxLines: 1, flexGrow: 1 }),
    ui.row({ gap: 8, align: "center" }, right),
  ]);
}

/** The viewer for a step. The latest step of a working session is typed out. */
function viewerFor(s, e, t, following) {
  const detail = e.detail;
  const opts = { ok: e.ok, maxRows: 34 };
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
      typing = n < total;
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
  typing = false;
  if (!s) {
    nodes.push(Viewer.render(null, { empty: "No agent session yet. Start Claude Code, Codex, opencode or pi in a terminal and it shows up here." }));
  } else {
    const [e, following] = currentStep(s, t);
    if (e) nodes.push(stepLine(e, t), viewerFor(s, e, t, following));
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
  const body = [];
  if (m.file) {
    body.push(
      ui.row({ gap: 4, align: "center" }, [
        ui.glyph({ name: "file-text", size: 13, color: "primary" }),
        ui.label({ text: m.file, fontSize: 11, fontWeight: "semibold", color: "primary", maxLines: 1 }),
      ]),
    );
  }
  if (text) body.push(ui.label({ text, fontSize: 13, color: "on_surface" }));
  const box = ui.column({ padding: 10, radius: 12, gap: 4, fill: mine ? "primary/0.22" : "surface_variant/0.45", maxWidth: 430 }, body);
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
  if (optimistic) rows.push(bubble({ role: "user", text: optimistic.text, file: optimistic.file }), bubble({ role: "assistant", text: "" }, true));
  if (!rows.length) {
    rows.push(
      ui.column({ gap: 6, padding: 6 }, [
        ui.label({ text: `Ask ${who} anything`, fontSize: 16, fontWeight: "semibold", color: "on_surface" }),
        ui.label({
          text: `Quick questions and answers, right from here. This chat has no tools and no access to your files, but you can drop a file on the pet to ask about it. It uses your ${who} login.`,
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
  input.placeholder = busy ? `${who} is answering…` : fedFile ? `Ask about ${fedFile.name}…` : `Ask ${who} anything…`;
  const ready = input.value.trim() !== "" || fedFile != null;
  setHtml(
    $("chatFile"),
    fedFile
      ? ui.row({ gap: 6, align: "center", paddingH: 8, paddingV: 4, radius: 8, fill: "primary/0.12" }, [
          ui.glyph({ name: "file-text", size: 14, color: "primary" }),
          ui.label({ text: fedFile.name, fontSize: 12, fontWeight: "semibold", color: "on_surface", maxLines: 1, flexGrow: 1 }),
        ]) + btn("", { act: "feed-discard", glyph: "x", tip: "Don't send this file", small: true })
      : "",
  );
  $("chatFile").classList.toggle("hidden", !fedFile);
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
  if ((!text && !fedFile) || chat.busy || optimistic) return;
  const file = fedFile;
  fedFile = null;
  optimistic = { text, file: file?.name, base: (chat.messages || []).length };
  input.value = "";
  const model = setting("chatModel");
  invoke("chat_send", { text, model: model || null, agent: chatAgent(), file: file?.path || null }).catch((e) => {
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

/** One agent's row in the comparative usage table: today's tokens and an estimated cost, or
 *  "not tracked yet" when nothing reads that agent's token usage (see `usage::Capabilities::context`
 *  — only Claude Code today; never fabricated for the others). */
function usageRow(label, data) {
  const right = !data?.available
    ? ui.label({ text: "not tracked yet", fontSize: 11, color: "on_surface_variant" })
    : ui.label({
        text: `${Fmt.tokens(Fmt.tokenSum(data.today))} today · ${Fmt.cost(data.estimated_cost_usd)} total (est.)`,
        fontSize: 11, color: "on_surface_variant",
      });
  return ui.row({ align: "center", justify: "space_between" }, [ui.label({ text: label, fontSize: 12, color: "on_surface" }), right]);
}

function usageContent(s, t) {
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
    ui.label({ text: "Usage by agent", fontSize: 12, fontWeight: "semibold", color: "on_surface" }),
    ui.column(
      { gap: 4 },
      Object.entries(snap().usage || {}).map(([id, data]) => usageRow(Fmt.agentLabel(snap(), id), data)),
    ),
  );
  if (setting("gamificationEnabled")) {
    const stats = snap().stats || {};
    nodes.push(
      ui.separator({ spacing: 4 }),
      ui.label({ text: "Stats", fontSize: 12, fontWeight: "semibold", color: "on_surface" }),
      ui.row({ gap: 20 }, [
        stat("Streak", stats.streak_days > 0 ? `🔥 ${stats.streak_days}d` : "—"),
        stat("Sessions", String(stats.total_sessions || 0)),
        stat("Steps watched", String(stats.total_tool_calls || 0)),
      ]),
    );
  }
  return ui.column({ gap: 10 }, nodes);
}

// ── Right column: Settings ─────────────────────────────────────────────────────

const prettify = (id) => id.replace(/_/g, " ").replace(/^./, (c) => c.toUpperCase());

/** 7 day checkboxes for `focusDays` (Mon..Sun, '1' = quiet that day): not a plain `data-setting`
 *  control (one checkbox cannot hold a 7-character string), handled in `onSettingChange` instead. */
function focusDaysControl() {
  const days = String(setting("focusDays") || "1111100").padEnd(7, "0");
  const names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
  const items = names
    .map(
      (n, i) =>
        `<label style="display:flex;flex-direction:column;align-items:center;gap:2px;font-size:11px;color:var(--on-surface-variant)">` +
        `<input type="checkbox" data-focusday="${i}"${days[i] === "1" ? " checked" : ""}>${n[0]}</label>`,
    )
    .join("");
  return `<div style="display:flex;gap:6px;">${items}</div>`;
}

function settingsView() {
  const row = (label, hint, control) =>
    `<label class="setting"><span class="setting-text"><b>${esc(label)}</b><small>${esc(hint)}</small></span>${control}</label>`;
  const select = (key, options) =>
    `<select data-setting="${key}">${options.map(([v, l]) => `<option value="${esc(v)}"${String(setting(key)) === String(v) ? " selected" : ""}>${esc(l)}</option>`).join("")}</select>`;
  const check = (key) => `<input type="checkbox" data-setting="${key}"${setting(key) ? " checked" : ""}>`;
  const text = (key, ph = "") => `<input type="text" data-setting="${key}" value="${esc(setting(key))}" placeholder="${esc(ph)}">`;
  const time = (key) => `<input type="time" data-setting="${key}" value="${esc(setting(key))}">`;
  const num = (key, min, max) => `<input type="number" data-setting="${key}" min="${min}" max="${max}" value="${esc(setting(key))}">`;
  return [
    row("Character", "Who lives in your pet window.", select("character", CHARACTER_IDS.map((id) => [id, prettify(id)]))),
    row(
      "Dock to the screen edge",
      "Pins the pet above everything else (macOS, Windows, and Linux compositors that support it) instead of a window you drag anywhere. Off = a classic, freely movable window. Takes effect after restarting the app.",
      check("dockedWindow"),
    ),
    row("Sounds", "Play the pet's little sounds.", check("sounds")),
    row("Random fidgets", "Hop, yawn, wink… while idle.", check("fidgets")),
    row("Nap after (seconds)", "Idle time before it falls asleep. 0 = never.", num("napAfterSec", 0, 3600)),
    row("Pet window shows", "What goes next to the pet.", select("widgetInfo", [["usage", "Usage only"], ["session", "Session"], ["detailed", "Detailed"]])),
    row("Open on permission request", "Bring the panel up when an agent asks.", check("autoOpen")),
    row("Close after Allow / Deny", "One click, back to work.", check("closeAfterDecision")),
    row("Chat agent", "Which agent answers in the Chat tab.", select("chatAgent", [["claude", "Claude Code"], ["copilot", "GitHub Copilot"], ["pi", "pi"], ["codex", "Codex"]])),
    row("Chat model", "Empty = the agent's own default.", text("chatModel", "default")),
    row("Focus mode", "A quiet, compact pet during these hours (the moon button next to the emotes forces it on/off).", check("focusEnabled")),
    row("Focus hours", "Quiet from / to (local time); crosses midnight if \"to\" is earlier than \"from\".", `${time("focusStart")} – ${time("focusEnd")}`),
    row("Focus days", "Which days the schedule above applies.", focusDaysControl()),
    row("Milestones", "A little celebration (and a streak / step count in Usage) for long-term use.", check("gamificationEnabled")),
  ].join("");
}

// ── Assembly ───────────────────────────────────────────────────────────────────

function tabs() {
  const t = (id, label) => btn(label, { variant: tab === id ? "primary" : "ghost", act: "tab", data: { id }, small: true });
  return (
    `<div class="tabs">${t("live", "Live")}${Fmt.canChat(snap()) ? t("chat", "Chat") : ""}${t("usage", "Usage")}${t("shop", "Shop")}${t("play", "Play")}` +
    `<span class="grow"></span>${btn("", { act: "tab", data: { id: "settings" }, glyph: "settings", tip: "Settings", small: true, variant: tab === "settings" ? "primary" : "ghost" })}` +
    `${btn("", { act: "close", glyph: "x", tip: "Close" })}</div>`
  );
}

function renderLeft(t) {
  const s = currentSession();
  const [nWorking, nPending] = counts();
  setHtml($("title"), titleBlock(s, nWorking, nPending, t));
  setHtml($("careMeters"), careMeters());
  setHtml($("emotes"), emotesContent());
  const railEl = $("rail");
  setHtml(railEl, rail(s, t));
  // The rail is a short scroll box (see SESSIONS_MAX_ROWS' sibling note): without this, the
  // newest command could scroll out of view behind older ones instead of always being the one
  // you see (same "pin to the bottom on new content" idiom as the chat log above).
  const steps = stepsOf(s);
  const railRev = `${s?.id || ""}:${steps.length}:${steps.at(-1)?.ts_ms || 0}`;
  if (railRev !== lastRailRev) {
    lastRailRev = railRev;
    railEl.scrollTop = railEl.scrollHeight;
  }
  setHtml($("sessions"), sessionList(s));
  $("sessions").style.maxHeight = `${SESSIONS_MAX_ROWS * SESSION_ROW_H}px`;
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
  show("shop", tab === "shop" && !down);
  show("play", tab === "play" && !down);
  if (down) {
    setHtml(
      $("content"),
      ui.column({ gap: 10 }, [
        ui.label({ text: "Daemon is not running.", fontSize: 14, fontWeight: "semibold", color: "error" }),
        ui.label({ text: "Start `sushi daemon` to see your sessions.", fontSize: 12, color: "on_surface_variant" }),
      ]),
    );
  } else if (tab === "usage") setHtml($("content"), usageContent(s, t));
  else if (tab === "live") {
    setHtml($("content"), liveContent(s, t));
    // Keep the line being typed in sight (only while typing: then the page is free to scroll).
    if (typing) $("content").querySelector(".caret")?.scrollIntoView({ block: "nearest" });
  }
  else if (tab === "chat") renderChat();
  else if (tab === "shop") setHtml($("shop"), shopContent());
  else if (tab === "play") setHtml($("play"), playContent());
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
    case "emote":
      sendEvent(d.kind);
      if (CARE_ACTION[d.kind]) void invoke(CARE_ACTION[d.kind]).catch(() => {});
      return;
    case "poke": return pet.poke(now());
    case "accessory-buy": return void invoke("care_buy", { id: d.id }).catch(() => {});
    case "accessory-equip": return void invoke("care_equip", { id: d.id }).catch(() => {});
    case "play-game":
      playGame = d.id;
      mem.set("playGame", d.id);
      playState = "idle";
      playResult = null;
      return renderRight();
    case "play-start":
      playState = "running";
      playResult = null;
      if (playGame === "tap") {
        playTaps = 0;
        playEndsAt = now() + 8000;
      } else if (playGame === "catch") {
        catchHits = 0;
        catchEndsAt = now() + CATCH_DURATION_MS;
        catchLeftIsTarget = Math.random() < 0.5;
        catchNextRerollAt = now() + CATCH_REROLL_MS;
      } else {
        memLevel = 1;
        memSeq = [randomPad()];
        memShowIdx = 0;
        memInputIdx = 0;
        memPhase = "flash";
        memActivePad = memSeq[0];
        memNextAt = now() + MEMORY_FLASH_MS;
      }
      return renderRight();
    case "play-tap":
      if (playGame === "tap" && playState === "running") playTaps++;
      return renderRight();
    case "catch-tap":
      catchTap(d.side);
      return renderRight();
    case "memory-tap":
      memoryTap(Number(d.pad));
      return renderRight();
    case "play-again":
      playState = "idle";
      playResult = null;
      return renderRight();
    case "focus-toggle": return toggleFocus();
    case "allow": return decide("approve", id);
    case "deny": return decide("deny", id);
    case "plan-approve": return decide("approve", id);
    case "plan-approve-edits": return decide("approve-edits", id);
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
    case "feed-discard":
      fedFile = null;
      return renderChat();
    case "chat-clear":
      optimistic = null;
      $("chatInput").value = "";
      return void invoke("chat_clear").then(renderChat);
  }
}

function onSettingChange(e) {
  const dayEl = e.target.closest("[data-focusday]");
  if (dayEl) {
    const days = String(setting("focusDays") || "1111100").padEnd(7, "0").split("");
    days[Number(dayEl.dataset.focusday)] = dayEl.checked ? "1" : "0";
    saveSettings({ focusDays: days.join("") });
    return;
  }
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

// petBlock, title, then care (the meters + its action buttons) — care is about the pet rather
// than any one session, but still reads after the title/status line it sits just below. The
// session list is capped at SESSIONS_MAX_ROWS (own scrollbar) rather than flex-grow, so it can
// never fight the rail for space or render on top of it — the rail is the one that flex-grows/
// scrolls into whatever is left at the bottom, since sessions is the thing you most need to
// always reach.
const SKELETON = `
<div class="panel">
  <aside class="left">
    <div id="petBlock"></div>
    <div id="title"></div>
    ${ui.separator({ spacing: 2 })}
    <div id="careMeters"></div>
    <div class="emotes">
      <span id="emotes" style="display:contents"></span>
      ${btn("", { act: "focus-toggle", glyph: "moon-stars", tip: "Toggle focus mode (quiet, compact pet)", small: true })}
    </div>
    ${ui.separator({ spacing: 2 })}
    <div class="muted small">Sessions</div>
    <div id="sessions" class="scroll"></div>
    ${ui.separator({ spacing: 2 })}
    <div id="rail" class="scroll grow"></div>
  </aside>
  <div class="vsep"></div>
  <main class="right">
    <div id="tabs"></div>
    <div id="perms" class="perms"></div>
    <div id="content" class="scroll grow"></div>
    <div id="chat" class="chat grow hidden">
      <div id="chatLog" class="scroll grow"></div>
      <div id="chatFile" class="chat-file hidden"></div>
      <div class="chat-input"><input id="chatInput" type="text" autocomplete="off" spellcheck="true"><span id="chatBtns" class="btn-row"></span></div>
    </div>
    <div id="settings" class="scroll grow hidden"></div>
    <div id="shop" class="scroll grow hidden"></div>
    <div id="play" class="scroll grow hidden"></div>
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
  // A file dropped on either window (the Rust side sends it): attach it to the next message and show the chat.
  listen("fedFile", (f) => {
    if (!f?.path) return;
    fedFile = f;
    if (Fmt.canChat(snap())) setTab("chat");
    renderChat();
  });

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
      pet.setFocus(Fmt.isFocusActive(store.settings));
      pet.tick(t);
      const [nWorking, nPending] = counts();
      setHtml($("petBlock"), petBlock(nWorking, nPending));
      if (t - slow >= 120) {
        slow = t;
        renderLeft(t);
        tickPlay();
        if (tab === "live" || tab === "usage" || tab === "play") renderRight();
      } else if (tab === "live" && typing) renderRight(); // the typewriter runs at the full frame rate
    }
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
}
