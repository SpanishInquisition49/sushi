// pet-window.js — the small always-on-top window: the pet plus a few numbers (port of plugin/bar_widget.luau).
//   "!N"        requests waiting for you, and sessions waiting in their own terminal (when any)
//   "2:13"      how long the active session's current turn has been running
//   pencil "3"  files changed this turn ("detailed" also adds +lines -lines, commands, failures)
//   "44%"       plan usage of the 5-hour window (falls back to the context window of the most
//               recently active session when plan limits are unavailable)
// The tooltip carries the full progress of the active session.
// Click → open / close the panel · drag → move the window · right click → pet it.
// Drop a file on it → it eats the file and the chat opens with the file attached (handled on the
// Rust side, which sends the pet's reactions as petEvent and the file as fedFile).

import { invoke, listen, emit, startDragging } from "./api.js";
import { store, setting, start } from "./store.js";
import { ui } from "./ui.js";
import { Pet, configFrom } from "./pet.js";
import * as Fmt from "./fmt.js";
import * as Sounds from "./sounds.js";

const PET_SIZE = 52; // sushi width in px (it is ~0.8x as tall)
const FRAME_MS = 66; // ~15 fps
const DRAG_PX = 4; // moving the pointer this far turns a click into a drag

const now = () => Date.now();
const pet = new Pet(now(), configFrom(store.settings));
let lastHtml = "";
let lastTip = "";
// Set by `mount()`: normally `#root` itself, but the macOS notch window mounts this into a
// sub-container (see notch-window.js), so `render()` must not re-look it up by a fixed id.
let rootEl = null;

const snap = () => store.snapshot;

/** Text and color for the percentage next to the pet: null when there is nothing to show. */
function headline() {
  const lim = Fmt.limits(snap())?.data;
  if (lim?.five_hour) {
    const p = lim.five_hour.percent;
    return [Fmt.round(p) + "%", Fmt.percentColor(p)];
  }
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
  const lim = Fmt.limits(snap());
  if (lim?.data) {
    for (const [name, w] of [["5-hour limit", lim.data.five_hour], ["Weekly limit", lim.data.seven_day]]) {
      if (w) rows.push([name, `${Fmt.round(w.percent)}% · resets in ${Fmt.resetIn(w.resets_at_ms, t)}`]);
    }
    for (const m of lim.data.models || []) rows.push([`${m.model} weekly`, `${Fmt.round(m.percent)}% · resets in ${Fmt.resetIn(m.resets_at_ms, t)}`]);
  } else if (lim?.error) rows.push(["Plan limits", String(lim.error)]);
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

/** The pill: the pet, then `pending` ("!N"), the session chips and the headline percentage. */
function pill(pending, sessionChips, h) {
  const children = [pet.build(PET_SIZE, { room: 6, margin: 4, mouth: true, maxHeight: 74 })];
  if (pending > 0) children.push(ui.label({ text: "!" + pending, fontSize: 13, fontWeight: "bold", color: "error" }));
  children.push(...sessionChips);
  if (h) children.push(ui.label({ text: h[0], fontSize: 13, fontWeight: "bold", color: h[1] }));
  return ui.row({ gap: 6, align: "center", cls: "pill" }, children);
}

function render() {
  const info = setting("widgetInfo");
  const html = store.up ? pill(Fmt.needsYou(snap()), info !== "usage" ? sessionChips(info) : [], headline()) : pill(0, [], null);
  const root = rootEl;
  if (html !== lastHtml) {
    lastHtml = html;
    root.innerHTML = html;
  }
  const tip = tooltipRows().map(([k, v]) => `${k}: ${v}`).join("\n");
  if (tip !== lastTip) {
    lastTip = tip;
    root.title = tip;
  }
}

/** Size the window for the pill at its widest with the current settings, so it takes no more room
 *  than it can show. Done before the window first shows and when the settings change: tiling
 *  compositors (niri) only take a window's size when it opens, so it cannot follow every chip. */
let fitted = "";
function fitWindow() {
  const a = { files_changed: 99, lines_added: 9999, lines_removed: 9999, commands: 99, failures: 9 };
  const info = setting("widgetInfo");
  const widest = pill(9, info !== "usage" ? chips(info, a, 9 * 3600e3 + 59 * 60e3 + 59e3) : [], ["100%", "error"]);
  const probe = document.createElement("div");
  probe.style.cssText = "position:absolute;left:0;top:0;visibility:hidden;width:max-content";
  probe.innerHTML = widest;
  document.body.appendChild(probe);
  const r = probe.firstElementChild.getBoundingClientRect();
  probe.remove();
  const pad = 12 + 4; // #root's padding on both sides, and a little room for the font
  const size = [Math.ceil(r.width) + pad, Math.ceil(r.height) + pad];
  if (size.join("x") === fitted) return;
  fitted = size.join("x");
  invoke("fit_pet", { width: size[0], height: size[1] });
}

/** Click, drag and right click on the pet. */
function wirePointer(root) {
  let down = null;
  root.addEventListener("mousedown", (e) => {
    if (e.button === 0) down = { x: e.screenX, y: e.screenY, dragged: false };
  });
  root.addEventListener("mousemove", (e) => {
    // The eyes follow the pointer while it is over the window.
    const r = root.getBoundingClientRect();
    pet.lookAt(Math.max(-1, Math.min(1, (e.clientX - (r.left + 40)) / 120)), Math.max(-1, Math.min(1, (e.clientY - (r.top + r.height / 2)) / 60)));
    if (down && !down.dragged && Math.hypot(e.screenX - down.x, e.screenY - down.y) > DRAG_PX) {
      down.dragged = true;
      // In the docked window the position is pinned by the Rust side, not dragged by hand.
      if (document.documentElement.dataset.view !== "dock") startDragging();
    }
  });
  root.addEventListener("mouseup", (e) => {
    if (e.button === 0 && down && !down.dragged) invoke("toggle_panel");
    down = null;
  });
  root.addEventListener("mouseleave", () => {
    down = null;
    pet.lookAt(null);
  });
  root.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    const ts = now();
    pet.onEvent("love", ts, ts);
    emit("petEvent", { kind: "love", ts });
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
      fitWindow();
    },
  );

  setInterval(() => {
    const t = now();
    pet.tick(t);
    for (const name of pet.drainSounds()) Sounds.play(name);
    render();
  }, FRAME_MS);
}
