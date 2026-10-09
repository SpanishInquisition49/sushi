// pet-window.js — the small always-on-top window: the pet plus a few numbers (port of plugin/bar_widget.luau).
//   "!N"        requests waiting for you, and sessions waiting in their own terminal (when any)
//   "2:13"      how long the active session's current turn has been running
//   pencil "3"  files changed this turn ("detailed" also adds +lines -lines, commands, failures)
//   "44%"       plan usage of the 5-hour window (falls back to the context window of the most
//               recently active session when plan limits are unavailable)
// The tooltip carries the full progress of the active session.
// Click → poke it, then open / close the panel · double click → nap or wake up · drag → move the
// window · right click → pet it (hold it down longer for a bigger cuddle) · middle click → feed it
// whatever is on the clipboard.
// Drop a file on it → it eats the file and the chat opens with the file attached (handled on the
// Rust side, which sends the pet's reactions as petEvent and the file as fedFile).

import { invoke, listen, emit, startDragging } from "./api.js";
import { store, setting, start } from "./store.js";
import { ui } from "./ui.js";
import { Pet, configFrom } from "./pet.js";
import * as Fmt from "./fmt.js";
import * as Sounds from "./sounds.js";

const PET_SIZE = 52; // sushi width in px (it is ~0.8x as tall)
const PET_SIZE_SHRUNK = 26; // while napping: just the character, no numbers
const FRAME_MS = 66; // ~15 fps
const DRAG_PX = 4; // moving the pointer this far turns a click into a drag

const now = () => Date.now();
const pet = new Pet(now(), configFrom(store.settings));
let lastHtml = "";
let lastTip = "";
// Set by `mount()`: normally `#root` itself, but the macOS notch window mounts this into a
// sub-container (see dock-window.js), so `render()` must not re-look it up by a fixed id.
let rootEl = null;
// Standard keeps other platforms unchanged; macOS camera displays use one left wing.
let presentation = "standard";
let notchLayout = null;
let presentationKey = "standard";

export function setPresentation(mode, { layout = null } = {}) {
  const key = mode === "lateral"
    ? [mode, layout?.lateralAvailable, layout?.notchWidth, layout?.topInset, layout?.leftWingWidth, layout?.rightWingWidth].join(":")
    : mode;
  if (presentationKey === key) return;
  presentationKey = key;
  presentation = mode;
  notchLayout = layout;
  if (rootEl) {
    render();
    fitWindow(isShrunk());
  }
}

const snap = () => store.snapshot;

/** Text and color for the percentage next to the pet: null when there is nothing to show. */
function headline() {
  let worst = null;
  for (const { data } of Fmt.allLimits(snap())) if (data?.five_hour && (!worst || data.five_hour.percent > worst)) worst = data.five_hour.percent;
  if (worst != null) return [Fmt.round(worst) + "%", Fmt.percentColor(worst)];
  let best = null;
  for (const s of snap().sessions) if (s.context && (!best || (s.last_event_ms || 0) > (best.last_event_ms || 0))) best = s;
  if (best) return [Fmt.round(best.context.percent) + "%", Fmt.percentColor(best.context.percent)];
  return null;
}

const STATUS_LABEL = { idle: "idle", working: "working", waiting: "waiting" };

function tooltipRows() {
  if (!store.up) return [["Sushi", "daemon is not running: start `sushi daemon`"]];
  const t = now();
  const rows = [];
  const active = Fmt.activeSession(snap());
  for (const r of Fmt.sessionRows(active, t)) rows.push([r.key, r.value]);
  const allLim = Fmt.allLimits(snap());
  const multiLim = allLim.length > 1;
  for (const { label, data, error } of allLim) {
    if (data) {
      for (const [name, w] of [["5-hour limit", data.five_hour], ["Weekly limit", data.seven_day]]) {
        if (w) rows.push([multiLim ? `${label} ${name}` : name, `${Fmt.round(w.percent)}% · resets in ${Fmt.resetIn(w.resets_at_ms, t)}`]);
      }
      for (const m of data.models || []) {
        const name = m.kind ? `${m.model} · ${m.kind}` : m.model;
        rows.push([multiLim ? `${label} ${name}` : name, `${Fmt.round(m.percent)}% · resets in ${Fmt.resetIn(m.resets_at_ms, t)}`]);
      }
    } else if (error) rows.push([multiLim ? `${label} limits` : "Plan limits", String(error)]);
  }
  for (const s of snap().sessions) {
    if (s === active) continue; // the active session is described above
    let value = STATUS_LABEL[s.status] || String(s.status);
    if (s.context) value += ` · ctx ${Fmt.round(s.context.percent)}%`;
    const tool = Fmt.lastToolText(s);
    if (tool && s.status !== "idle") value += " · " + Fmt.clip(tool, 50);
    rows.push([s.name || s.id, value]);
  }
  if (!rows.length) rows.push(["Sushi", "no active sessions"]);
  return rows;
}

/** Small pieces describing the active session's turn (time, changes, commands). */
function sessionChips(info) {
  const s = Fmt.activeSession(snap());
  if (!s || s.status === "idle") return [];
  return chips(info, s.activity || {}, Fmt.turnElapsed(s, now()));
}

function chips(info, a, elapsed) {
  const chips = [];
  if (elapsed != null) chips.push(ui.label({ text: Fmt.duration(elapsed), fontSize: 12, color: "on_surface_variant" }));
  if (a.files_changed > 0) {
    chips.push(ui.glyph({ name: "pencil", size: 12, color: "primary" }), ui.label({ text: String(a.files_changed), fontSize: 12, fontWeight: "bold", color: "primary" }));
    if (info === "detailed") chips.push(ui.label({ text: "+" + (a.lines_added || 0), fontSize: 11, color: "primary" }), ui.label({ text: "-" + (a.lines_removed || 0), fontSize: 11, color: "error" }));
  }
  if (info === "detailed") {
    if (a.commands > 0) chips.push(ui.glyph({ name: "terminal-2", size: 12, color: "on_surface_variant" }), ui.label({ text: String(a.commands), fontSize: 12, color: "on_surface_variant" }));
    if (a.failures > 0) chips.push(ui.glyph({ name: "alert-triangle", size: 12, color: "error" }), ui.label({ text: String(a.failures), fontSize: 12, fontWeight: "bold", color: "error" }));
  }
  return chips;
}

/** The pill: the pet, then `pending` ("!N"), the session chips and the headline percentage —
 *  or, while napping (nothing pending, nothing working), just the character on its own. */
function pill(pending, sessionChips, h, shrunk) {
  const focusDim = Fmt.isFocusActive(store.settings);
  const character = pet.build(shrunk ? PET_SIZE_SHRUNK : PET_SIZE, shrunk
    ? { room: 3, margin: 2, maxHeight: 40, focusDim }
    : { room: 6, margin: 4, mouth: true, maxHeight: 74, focusDim });
  if (shrunk) return ui.row({ gap: 0, align: "center", cls: "pill" }, [character]);
  const children = [character];
  if (pending > 0) children.push(ui.label({ text: "!" + pending, fontSize: 13, fontWeight: "bold", color: "error" }));
  children.push(...sessionChips);
  if (h) children.push(ui.label({ text: h[0], fontSize: 13, fontWeight: "bold", color: h[1] }));
  return ui.row({ gap: 6, align: "center", cls: "pill" }, children);
}

/** One priority indicator; shape and text both distinguish disconnected / working / idle. */
function lateralIndicator() {
  if (!store.up) return ["×", "on_surface_variant", "daemon disconnected"];
  const pending = Fmt.needsYou(snap());
  if (pending > 0) return [pending > 99 ? "!99+" : "!" + pending, "error", `${pending} requests need you`];
  const usage = headline();
  if (usage) return [...usage, "usage " + usage[0]];
  return snap().sessions.some((s) => s.status === "working")
    ? ["•••", "primary", "working"]
    : ["—", "on_surface_variant", "idle"];
}

function lateralPill() {
  if (!notchLayout?.lateralAvailable) return "";
  const height = Math.max(1, Math.min(24, notchLayout.topInset - 4));
  const character = pet.build(20, {
    room: Math.min(2, height / 4), margin: 2, maxHeight: height,
    detail: false, focusDim: Fmt.isFocusActive(store.settings),
  });
  const [text, color] = lateralIndicator();
  return '<div class="notch-pill"><div class="notch-wing"><div class="notch-pet">' + character
    + '</div><div class="notch-indicator">'
    + ui.label({ text, fontSize: 13, fontWeight: "bold", color })
    + '</div></div><div class="notch-camera" aria-hidden="true"></div></div>';
}

const isShrunk = () => pet.cur === "nap";

function render() {
  const info = setting("widgetInfo");
  const shrunk = isShrunk();
  const html = presentation === "lateral" ? lateralPill() : store.up
    ? pill(Fmt.needsYou(snap()), !shrunk && info !== "usage" ? sessionChips(info) : [], shrunk ? null : headline(), shrunk)
    : pill(0, [], null, false);
  const root = rootEl;
  if (html !== lastHtml) {
    lastHtml = html;
    root.innerHTML = html;
  }
  const tip = (presentation === "lateral" ? [["Sushi", lateralIndicator()[2]], ...tooltipRows()] : tooltipRows())
    .map(([k, v]) => `${k}: ${v}`).join("\n");
  if (tip !== lastTip) {
    lastTip = tip;
    root.title = tip;
  }
}

/** Lateral bounds are fixed by display geometry. Other platforms retain their usual fitting. */
let fitted = "";
function fitWindow(shrunk) {
  if (presentation === "lateral") {
    const width = Math.max(1, (notchLayout?.leftWingWidth || 0) + (notchLayout?.notchWidth || 0) + (notchLayout?.rightWingWidth || 0));
    const height = Math.max(1, notchLayout?.topInset || 0);
    const key = `lateral:${width}x${height}:${notchLayout?.lateralAvailable}`;
    if (fitted !== key) {
      fitted = key;
      invoke("fit_pet", { width, height, animate: false });
    }
    return;
  }
  const docked = document.documentElement.dataset.view === "dock";
  const a = { files_changed: 99, lines_added: 9999, lines_removed: 9999, commands: 99, failures: 9 };
  const info = setting("widgetInfo");
  const widest = pill(9, info !== "usage" ? chips(info, a, 9 * 3600e3 + 59 * 60e3 + 59e3) : [], ["100%", "error"], false);
  function measure(widget) {
    const probe = document.createElement("div");
    probe.style.cssText = "position:absolute;left:0;top:0;visibility:hidden;width:max-content";
    probe.innerHTML = widget;
    document.body.appendChild(probe);
    const r = probe.firstElementChild.getBoundingClientRect();
    probe.remove();
    const pad = 16; // normal pill/container padding and font allowance
    return [Math.ceil(r.width) + pad, Math.ceil(r.height) + pad];
  }
  const size = measure(docked && shrunk ? pill(0, [], null, true) : widest);
  if (size.join("x") === fitted) return;
  fitted = size.join("x");
  invoke("fit_pet", { width: size[0], height: size[1] });
}

const SNUGGLE_HOLD_MS = 700; // holding the "pet it" gesture this long makes it more intense

/** Broadcast a one-off pet reaction: react here, and tell the other windows to as well. */
function broadcast(kind) {
  const ts = now();
  pet.onEvent(kind, ts, ts);
  emit("petEvent", { kind, ts });
}

/** Click (poke, opens the panel), double click (nap), drag, right click (pet it, held longer for
 *  a bigger cuddle) and middle click (feed it whatever a file manager's "Copy" put on the
 *  clipboard — the Tauri side answers via `feed_clipboard`, see `clipboard_file.rs`). */
function wirePointer(root) {
  let down = null;
  let rightDownAt = 0;
  const inWing = (e) => presentation !== "lateral" || !!e.target.closest(".notch-wing");
  root.addEventListener("mousedown", (e) => {
    if (!inWing(e)) return;
    if (e.button === 0) down = { x: e.screenX, y: e.screenY, dragged: false };
    else if (e.button === 1) e.preventDefault(); // no autoscroll
    else if (e.button === 2) rightDownAt = now();
  });
  root.addEventListener("mousemove", (e) => {
    // The eyes follow the pointer while it is over the window.
    const r = (presentation === "lateral" ? root.querySelector(".notch-pet") : null)?.getBoundingClientRect() || root.getBoundingClientRect();
    const eyeX = r.left + (presentation === "lateral" ? r.width / 2 : 40);
    pet.lookAt(Math.max(-1, Math.min(1, (e.clientX - eyeX) / 120)), Math.max(-1, Math.min(1, (e.clientY - (r.top + r.height / 2)) / 60)));
    if (down && !down.dragged && Math.hypot(e.screenX - down.x, e.screenY - down.y) > DRAG_PX) {
      down.dragged = true;
      // In the docked window the position is pinned by the Rust side, not dragged by hand.
      if (document.documentElement.dataset.view !== "dock") startDragging();
    }
  });
  root.addEventListener("mouseup", (e) => {
    if (!inWing(e)) { down = null; return; }
    if (e.button === 0 && down && !down.dragged) {
      pet.poke(now());
      invoke("toggle_panel");
    } else if (e.button === 1) {
      invoke("feed_clipboard");
    }
    down = null;
  });
  root.addEventListener("dblclick", (e) => {
    e.preventDefault();
    if (!inWing(e)) return;
    broadcast("nap-toggle");
  });
  root.addEventListener("mouseleave", () => {
    down = null;
    pet.lookAt(null);
  });
  root.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    if (!inWing(e)) return;
    // Tamagotchi care (see src/care.rs): petting it here counts the same as the panel's heart
    // button, including its cooldown (snap().care.next_pet_ms, refreshed every snapshot) — a
    // right click while it is still cooling down is a no-op, same as a disabled button, instead
    // of letting a flurry of clicks max affection out in one burst.
    if ((snap().care?.next_pet_ms || 0) > now()) return;
    broadcast(now() - rightDownAt > SNUGGLE_HOLD_MS ? "snuggle" : "love");
    invoke("care_pet").catch(() => {});
  });
}

export function mount(root) {
  rootEl = root;
  Sounds.init();
  wirePointer(root);
  listen("petEvent", (ev) => ev && pet.onEvent(ev.kind, ev.ts, now()));

  start(
    () => {
      pet.onSnapshot(snap(), store.up, now());
      render();
    },
    () => {
      pet.configure(configFrom(store.settings));
      pet.enableSounds(setting("sounds"));
      fitWindow(isShrunk());
    },
  );

  let lastShrunk = isShrunk();
  setInterval(() => {
    const t = now();
    pet.setFocus(Fmt.isFocusActive(store.settings));
    pet.tick(t);
    for (const name of pet.drainSounds()) Sounds.play(name);
    render();
    const shrunk = isShrunk();
    if (shrunk !== lastShrunk) {
      lastShrunk = shrunk;
      fitWindow(shrunk);
    }
  }, FRAME_MS);
}
