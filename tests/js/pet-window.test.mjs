import { readFile } from "node:fs/promises";
import { strict as assert } from "node:assert";
import { test } from "node:test";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import * as Characters from "../../app/ui/js/pet.js";
import * as Format from "../../app/ui/js/fmt.js";
import * as UI from "../../app/ui/js/ui.js";

const source = await readFile(new URL("../../app/ui/js/pet-window.js", import.meta.url), "utf8");
const notch = { hasNotch: true, notchWidth: 200, topInset: 32, lateralAvailable: true, leftWingWidth: 88, rightWingWidth: 0 };
async function harness(up = true, working = true, mode = "lateral", layout = notch) {
  let now = 100000, tick;
  const calls = [];
  const snapshot = { sessions: working ? [{ id: "test", status: "working", activity: {
    files_changed: 3, lines_added: 123, lines_removed: 45, commands: 2, failures: 1, turn_started_ms: 0,
  } }] : [], pending: [], agents: {}, events: [] };
  const settings = { character: "dango", widgetInfo: "detailed", fidgets: false, napAfterSec: 1 };
  const pointer = new Map();
  const events = new Map();
  const root = { innerHTML: "", addEventListener: (name, fn) => pointer.set(name, fn),
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 288, height: 32 }),
    querySelector: () => ({ getBoundingClientRect: () => ({ left: 0, top: 0, width: 36, height: 32 }) }),
  };
  let refresh, refreshSettings;
  const store = { up, snapshot, settings };
  const context = createContext({
    Date: class extends Date { static now() { return now; } },
    setInterval: (fn) => { tick = fn; },
    document: { documentElement: { dataset: { view: "dock" } },
      body: { appendChild() {} },
      // Browser text metrics are checked separately on the Mac. These test the sizing
      // contract independently of browser fonts.
      createElement: () => ({ style: {}, remove() {}, firstElementChild: {
        getBoundingClientRect: () => ({ width: 420, height: 80 }),
      } }),
    },
  });
  const dependencies = {
    "./api.js": { invoke: async (cmd, args) => calls.push({ cmd, ...args }), listen: (name, fn) => events.set(name, fn), emit: (name, args) => calls.push({ cmd: name, ...args }), startDragging: () => calls.push({ cmd: "drag" }) },
    "./store.js": { store, setting: (key) => settings[key], start: (onState, onSettings) => { refresh = onState; refreshSettings = onSettings; onSettings(); onState(); } },
    "./ui.js": UI, "./pet.js": Characters, "./fmt.js": Format,
    "./sounds.js": { init() {}, play() {} },
  };
  const module = new SourceTextModule(source, { context });
  await module.link((name) => {
    const exports = dependencies[name];
    return new SyntheticModule(Object.keys(exports), function () {
      for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
    }, { context });
  });
  await module.evaluate();
  module.namespace.setPresentation(mode, { layout });
  module.namespace.mount(root);
  return { root, calls, store, pointer, events, refresh, refreshSettings, set: module.namespace.setPresentation, advance: (ms) => { now += ms; tick(); } };
}

test("the left wing renders one status and no detailed numeric indicators", async () => {
  const h = await harness();
  assert.equal(h.calls.at(-1).width, 288);
  assert.equal(h.calls.at(-1).height, 32);
  assert.match(h.root.innerHTML, /class="notch-pet|notch-wing notch-pet/);
  assert.match(h.root.innerHTML, /class="notch-camera" aria-hidden="true"/);
  assert.equal((h.root.innerHTML.match(/class="notch-wing"/g) || []).length, 1);
  assert.ok(h.root.innerHTML.indexOf('class="notch-pet"') < h.root.innerHTML.indexOf('class="notch-indicator"'));
  assert.ok(h.root.innerHTML.indexOf('class="notch-indicator"') < h.root.innerHTML.indexOf('class="notch-camera"'));
  assert.match(h.root.innerHTML, />•••<\/span>/);
  assert.doesNotMatch(h.root.innerHTML, />3<\/span>|>\+123<\/span>|>-45<\/span>/);
  assert.match(h.root.title, /working/);
});

test("requests override usage, count waiting sessions once, and cap large counts", async () => {
  const h = await harness();
  h.store.snapshot.agents.codex = { label: "Codex", limits: { data: { five_hour: { percent: 72 } } } };
  h.store.snapshot.pending = [{ session_id: "test" }];
  h.store.snapshot.sessions[0].status = "waiting";
  h.refresh();
  assert.match(h.root.innerHTML, />!1<\/span>/);
  assert.doesNotMatch(h.root.innerHTML, />72%<\/span>/);
  h.store.snapshot.sessions.push({ id: "other", status: "waiting" });
  h.refresh();
  assert.match(h.root.innerHTML, />!2<\/span>/);
  h.store.snapshot.pending = Array.from({ length: 100 }, () => ({ session_id: "test" }));
  h.refresh();
  assert.match(h.root.innerHTML, />!99\+<\/span>/);
  assert.match(h.root.title, /101 requests need you/);
  assert.equal(h.calls.filter((c) => c.cmd === "fit_pet").length, 1);
});

test("100% and !99+ stay inside the single left wing without refitting", async () => {
  const h = await harness();
  h.store.snapshot.agents.codex = { limits: { data: { five_hour: { percent: 100 } } } };
  h.refresh();
  assert.match(h.root.innerHTML, /class="notch-indicator">.*>100%<\/span><\/div><\/div><div class="notch-camera"/);
  h.store.snapshot.pending = Array.from({ length: 100 }, () => ({ session_id: "test" }));
  h.refresh();
  assert.match(h.root.innerHTML, /class="notch-indicator">.*>!99\+<\/span><\/div><\/div><div class="notch-camera"/);
  assert.equal(h.calls.filter(c => c.cmd === "fit_pet").length, 1);
});

test("usage keeps existing plan priority and most recent context fallback", async () => {
  const h = await harness();
  h.store.snapshot.agents = {
    codex: { limits: { data: { five_hour: { percent: 40 } } } },
    claude: { limits: { data: { five_hour: { percent: 87 } } } },
  };
  h.store.snapshot.sessions[0].context = { percent: 95 };
  h.refresh();
  assert.match(h.root.innerHTML, />87%<\/span>/);
  h.store.snapshot.agents = {};
  h.store.snapshot.sessions.push({ id: "latest", status: "working", context: { percent: 63 }, last_event_ms: 120000 });
  h.refresh();
  assert.match(h.root.innerHTML, />63%<\/span>/);
});

test("disconnected, idle and working states have distinct symbols and tooltip descriptions", async () => {
  const h = await harness(false);
  assert.match(h.root.innerHTML, />×<\/span>/);
  assert.match(h.root.title, /daemon disconnected/);
  assert.ok(h.root.innerHTML.includes("height:23px"));
  h.store.up = true;
  h.refresh();
  assert.match(h.root.innerHTML, />•••<\/span>/);
  h.store.snapshot.sessions = [];
  h.refresh();
  assert.match(h.root.innerHTML, />—<\/span>/);
  assert.match(h.root.title, /idle/);
});

test("sleep, settings changes and reactions retain the lateral frame", async () => {
  const h = await harness(true, false);
  h.advance(2000);
  const fits = h.calls.filter((c) => c.cmd === "fit_pet");
  assert.equal(fits.length, 1);
  assert.equal(fits[0].width, 288);
  assert.equal(fits[0].height, 32);
  h.store.settings.widgetInfo = "usage";
  h.refreshSettings();
  h.events.get("petEvent")({ kind: "love", ts: 102000 });
  h.advance(1000);
  assert.equal(h.calls.filter((c) => c.cmd === "fit_pet").length, 1);
  assert.match(h.root.innerHTML, />—<\/span>/);
});

test("changed camera geometry refits even when presentation mode stays lateral", async () => {
  const h = await harness();
  h.set("lateral", { layout: { ...notch, notchWidth: 260, topInset: 20 } });
  assert.equal(h.calls.at(-1).width, 348);
  assert.equal(h.calls.at(-1).height, 20);
  assert.ok(Number(h.root.innerHTML.match(/height:(\d+)px/)[1]) <= 16);
  const count = h.calls.length;
  h.set("lateral", { layout: { ...notch, notchWidth: 260, topInset: 20 } });
  assert.equal(h.calls.length, count);
  h.set("standard");
  assert.equal(h.calls.at(-1).width, 436);
  assert.equal(h.calls.at(-1).height, 96);
  assert.match(h.root.innerHTML, />\+123<\/span>/);
});

test("invalid lateral geometry renders nothing", async () => {
  const h = await harness(true, true, "lateral", { ...notch, lateralAvailable: false });
  assert.equal(h.root.innerHTML, "");
});

test("both parts of the left wing open the panel; the camera spacer cannot trigger a click", async () => {
  const h = await harness();
  for (const wing of ["pet", "indicator"]) {
    const e = { button: 0, screenX: 10, screenY: 10, target: { closest: () => wing } };
    h.pointer.get("mousedown")(e);
    h.pointer.get("mouseup")(e);
  }
  assert.equal(h.calls.filter((c) => c.cmd === "toggle_panel").length, 2);
  const camera = { button: 0, target: { closest: () => null } };
  h.pointer.get("mousedown")(camera);
  h.pointer.get("mouseup")(camera);
  assert.equal(h.calls.filter((c) => c.cmd === "toggle_panel").length, 2);
});

test("lateral wings preserve middle-click feeding, cuddles and nap gestures", async () => {
  const h = await harness();
  const target = { closest: () => "pet" };
  const e = { button: 1, target, preventDefault() {} };
  h.pointer.get("mousedown")(e);
  h.pointer.get("mouseup")(e);
  assert.equal(h.calls.at(-1).cmd, "feed_clipboard");
  h.pointer.get("mousedown")({ ...e, button: 2 });
  h.advance(800);
  h.pointer.get("contextmenu")({ ...e, button: 2 });
  assert.ok(h.calls.some((c) => c.cmd === "petEvent" && c.kind === "snuggle"));
  assert.equal(h.calls.at(-1).cmd, "care_pet");
  h.pointer.get("dblclick")({ ...e, button: 0 });
  assert.equal(h.calls.at(-1).kind, "nap-toggle");
  const count = h.calls.length;
  const camera = { ...e, target: { closest: () => null } };
  h.pointer.get("mouseup")(camera);
  h.pointer.get("dblclick")(camera);
  h.pointer.get("contextmenu")(camera);
  assert.equal(h.calls.length, count);
});

test("standard dock windows keep detailed indicators and shrinking on nap", async () => {
  const h = await harness(true, true, "standard");
  assert.match(h.root.innerHTML, />3<\/span>/);
  assert.match(h.root.innerHTML, />\+123<\/span>/);
  assert.equal(h.calls.at(-1).height, 96);
  const sleeping = await harness(true, false, "standard");
  sleeping.advance(2000);
  assert.match(sleeping.root.innerHTML, /height:40px/);
  assert.doesNotMatch(sleeping.root.innerHTML, /notch-wing/);
});

test("every character fits both a 24pt and a reduced 16pt viewport across reactions", () => {
  for (const character of Characters.CHARACTER_IDS) {
    const pet = new Characters.Pet(0, Characters.configFrom({ character, fidgets: false }));
    for (const height of [24, 16]) for (const kind of ["love", "poke", "nap-toggle", "eat", "hello"]) {
      pet.onEvent(kind, 1000, 1000);
      pet.tick(1200);
      const html = pet.build(20, { room: 2, margin: 2, maxHeight: height, detail: false });
      const actualHeight = Number(html.match(/height:(\d+)px/)[1]);
      assert.ok(actualHeight <= height, `${character}, ${kind}: ${actualHeight}pt`);
    }
  }
});
