// Run: node --experimental-vm-modules --test tests/js/dock-window.test.mjs
import { readFile } from "node:fs/promises";
import { strict as assert } from "node:assert";
import { test } from "node:test";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";

const transitionSource = await readFile(new URL("../../app/ui/js/dock-transition.js", import.meta.url), "utf8");
const source = await readFile(new URL("../../app/ui/js/dock-window.js", import.meta.url), "utf8");

async function harness(fetchLayout, reduced = false, surface = {}) {
  const classes = new Set();
  const properties = new Map();
  const listeners = new Map();
  const mounted = [];
  const presentations = [];
  const compactModes = [];
  const calls = [];
  const renderAcks = [];
  const timers = new Map();
  let time = 0, nextId = 1, hovered = false;
  let point = { x: -10, y: -10 };
  const events = new Map();
  const root = {
    set innerHTML(html) {
      // The mount fixture consists of divs; record actual ancestry for clipping checks.
      const parents = [];
      for (const tag of html.matchAll(/<\/?div\b[^>]*>/g)) {
        if (tag[0].startsWith("</")) { parents.pop(); continue; }
        const id = tag[0].match(/id="([^"]+)"/)?.[1];
        if (nodes[id]) nodes[id].parentId = parents.at(-1) ?? null;
        parents.push(id);
      }
    },
    addEventListener: (name, fn) => events.set(name, fn),
    matches: () => hovered,
  };
  function advance(ms) {
    const end = time + ms;
    while (true) {
      const next = [...timers].filter(([, t]) => t.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
      if (!next) break;
      time = next[1].at;
      timers.delete(next[0]);
      next[1].fn();
    }
    time = end;
  }
  function mouse(inside, side = "left") {
    hovered = inside;
    if (inside) {
      const wing = side === "none" ? null : {};
      point = { x: 20, y: 16 };
      events.get("mousemove")?.({ ...{clientX: point.x, clientY: point.y}, target: { closest: () => wing } });
    } else { point = { x: -10, y: -10 }; events.get("mouseleave")?.(); }
  }
  const schedule = (fn, delay) => {
    const id = nextId++;
    timers.set(id, { fn, at: time + delay });
    return id;
  };
  const nodes = { dockHoverZone: {style: {}}, dockSurface: {style: {}}, dockBackdrop: {style: {}}, pillRoot: { style: {} }, compactRoot: { style: {} }, panelRoot: { style: {} } };
  const document = {
    getElementById: (id) => nodes[id],
    documentElement: {
      dataset: {},
      classList: { toggle: (name, on) => on ? classes.add(name) : classes.delete(name) },
      style: { setProperty: (name, value) => properties.set(name, value) },
    },
  };
  const context = createContext({ document, console: { error() {} },
    window: { matchMedia: () => ({ matches: reduced }), getComputedStyle: el => el.style },
    setTimeout: schedule, clearTimeout: (id) => timers.delete(id),
    requestAnimationFrame: (fn) => schedule(fn, 16),
  });
  const dependencies = {
    "./api.js": {
      listen: async (name, callback) => { listeners.set(name, callback); },
      invoke: async (command, args) => {
        if (command === "set_dock_hover") { calls.push(args.hovered); return; }
        if (command === "get_dock_pointer") return {...point};
        if (command === "finish_dock_transition") return;
        if (command === "ack_dock_render") { renderAcks.push({ ...args }); return; }
        assert.equal(command, "get_dock_layout");
        assert.ok(listeners.has("dockLayout"));
        assert.ok(listeners.has("dockMode"));
        assert.equal(mounted.length, 0, "must not measure content before getting layout");
        return fetchLayout(listeners);
      },
    },
    "./pet-window.js": {
      mount: () => mounted.push(properties.get("--notch-inset")),
      setPresentation: (mode, { animate }) => presentations.push({ mode, animate }),
    },
    "./panel.js": { mount: () => mounted.push("panel") },
    "./compact-panel.js": { mount: () => mounted.push("compact"), setLayout() {}, setMode: mode => compactModes.push(mode) },
    "./dock-transition.js": { createDockTransition: () => ({ update: () => false, animating: false, containsHoverPoint: () => surface.visible !== false }) },
  };
  const module = new SourceTextModule(source, { context });
  await module.link((name) => {
    if (name === "./dock-transition.js" && surface.real) {
      const real = new SourceTextModule(transitionSource, { context });
      return real;
    }
    const exports = dependencies[name];
    return new SyntheticModule(Object.keys(exports), function () {
      for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
    }, { context });
  });
  await module.evaluate();
  await module.namespace.mount(root);
  return { classes, properties, listeners, nodes, mounted, presentations,
    advance, mouse, calls, compactModes, renderAcks,
    move: (x,y) => { point = {x,y}; events.get("mousemove")?.({clientX:x,clientY:y,target:{closest:()=>null}}); },
    syntheticLeave: () => events.get("mouseleave")?.(),
    setPointer: next => { point = next; },
    flush: async () => { await Promise.resolve(); await Promise.resolve(); },
    dataset: document.documentElement.dataset,
    dismiss: () => events.get("click")?.({ target: { closest: () => ({}) } }) };
}

const notch = { hasNotch: true, notchWidth: 200, topInset: 32, lateralAvailable: true, leftWingWidth: 88, rightWingWidth: 0 };

test("the persistent pet is outside the panel's animated clipping subtree", async () => {
  const h = await harness(() => notch);
  assert.equal(h.nodes.pillRoot.parentId, null);
  assert.equal(h.nodes.compactRoot.parentId, "dockSurface");
  assert.equal(h.nodes.panelRoot.parentId, "dockSurface");
});

const protectedLayout = (id, revision) => ({ ...notch, mode: "collapsed", transitionId: id,
  viewport: { x: 662, y: 768, width: 288, height: 32 },
  frames: { collapsed: { x: 0, y: 0, width: 288, height: 32 },
    preview: { x: -22, y: 0, width: 420, height: 252 },
    attention: { x: -92, y: 0, width: 560, height: 232 },
    full: { x: -262, y: 0, width: 900, height: 632 } },
  renderAck: { transitionId: id, revision } });

test("render acknowledgement follows layout application and a rendering opportunity, once per revision", async () => {
  const final = protectedLayout(2, 10);
  const h = await harness(() => final, false, { real: true });
  assert.equal(h.nodes.pillRoot.style.left, "0px");
  assert.deepEqual(h.renderAcks, []);
  h.advance(16);
  assert.deepEqual(h.renderAcks, []);
  h.listeners.get("dockMode")(final);
  h.listeners.get("dockLayout")(final);
  h.advance(16);
  assert.deepEqual(h.renderAcks, [{ id: 2, revision: 10 }]);
  h.advance(100);
  assert.equal(h.renderAcks.length, 1);
});

test("a newer settlement in the same transition invalidates the old render acknowledgement", async () => {
  const h = await harness(() => protectedLayout(2, 10), false, { real: true });
  h.advance(16);
  h.listeners.get("dockLayout")(protectedLayout(2, 11));
  h.advance(32);
  assert.deepEqual(h.renderAcks, [{ id: 2, revision: 11 }]);
});

test("new transitions and display changes discard queued render acknowledgements", async () => {
  for (const changed of [protectedLayout(3, 11), { hasNotch: false, mode: "collapsed", transitionId: 3, displayRevision: 1 }]) {
    const h = await harness(() => protectedLayout(2, 10), false, { real: true });
    h.advance(16);
    h.listeners.get("dockLayout")(changed);
    h.listeners.get("dockLayout")(protectedLayout(2, 10));
    h.advance(32);
    assert.deepEqual(h.renderAcks, changed.renderAck ? [{ id: 3, revision: 11 }] : []);
  }
});

test("outgoing compact content stays intact and stale dock events cannot restore it", async () => {
  const h = await harness(() => ({ ...notch, mode: "attention", transitionId: 1 }));
  h.listeners.get("dockMode")({ mode: "collapsed", transitionId: 2 });
  h.listeners.get("dockMode")({ mode: "attention", transitionId: 1 });
  h.listeners.get("dockLayout")({ ...notch, mode: "attention", transitionId: 1 });
  assert.equal(h.dataset.dockMode, "collapsed");
  assert.deepEqual(h.compactModes, ["attention"]);
});

test("motion in a transparent native envelope does not reopen the preview", async () => {
  const surface = { visible: false };
  const h = await harness(() => ({ ...notch, viewport: {}, frames: {}, mode: "collapsed" }), false, surface);
  h.mouse(true); h.advance(100);
  assert.deepEqual(h.calls, []);
  surface.visible = true;
  h.mouse(true);
  assert.deepEqual(h.calls, [true]);
  surface.visible = false;
  h.mouse(true); h.advance(100); await h.flush();
  assert.deepEqual(h.calls, [true, false]);
});

test("reserves the camera space before mounting and toggles panel interaction", async () => {
  const h = await harness(() => notch);
  assert.ok(h.classes.has("has-notch"));
  assert.deepEqual(h.mounted, ["32px", "panel", "compact"]);
  h.listeners.get("dockMode")({ expanded: true });
  assert.ok(h.classes.has("expanded"));
  assert.equal(h.nodes.pillRoot.style.pointerEvents, "auto");
  assert.equal(h.nodes.panelRoot.style.visibility, "visible");
  h.listeners.get("dockMode")({ expanded: false });
  assert.equal(h.nodes.pillRoot.style.visibility, "visible");
  assert.equal(h.nodes.panelRoot.style.pointerEvents, "none");
});

test("a display event received during initialization wins over the initial response", async () => {
  const h = await harness((listeners) => {
    listeners.get("dockLayout")({ ...notch, topInset: 38 });
    return notch;
  });
  assert.equal(h.properties.get("--notch-inset"), "38px");
  h.listeners.get("dockLayout")({ hasNotch: false, notchWidth: 0, topInset: 0 });
  assert.equal(h.classes.has("has-notch"), false);
  assert.equal(h.properties.get("--notch-inset"), "0px");
});

test("displays without a notch retain the standard layout", async () => {
  const h = await harness(() => ({ hasNotch: false, notchWidth: 0, topInset: 0 }));
  assert.equal(h.classes.has("has-notch"), false);
  assert.deepEqual(h.mounted, ["0px", "panel", "compact"]);
});

test("a failed initial fetch does not discard a newer display event", async () => {
  const h = await harness((listeners) => {
    listeners.get("dockLayout")(notch);
    throw new Error("initial fetch failed");
  });
  assert.ok(h.classes.has("has-notch"));
  assert.deepEqual(h.mounted, ["32px", "panel", "compact"]);
});


test("hover requests an intermediate panel and leaves after 100 ms", async () => {
  const h = await harness(() => notch);
  h.mouse(true);
  assert.deepEqual(h.calls, [true]);
  h.listeners.get("dockMode")({ mode: "preview" });
  assert.equal(h.dataset.dockPresentation, "preview");
  assert.equal(h.nodes.compactRoot.style.visibility, "visible");
  assert.equal(h.nodes.panelRoot.style.visibility, "hidden");
  h.mouse(false); h.advance(99); assert.deepEqual(h.calls, [true]);
  h.advance(1); assert.deepEqual(h.calls, [true, false]);
});

test("moving from a replaced wing into the preview cancels delayed leave", async () => {
  const h = await harness(() => notch);
  h.mouse(true); h.mouse(true);
  h.listeners.get("dockMode")({ mode: "preview" });
  h.mouse(false); h.advance(50); h.mouse(true); h.advance(200);
  assert.deepEqual(h.calls, [true]);
});

test("attention remains visible when leaving and full mode takes precedence", async () => {
  const h = await harness(() => ({ ...notch, mode: "attention" }));
  assert.equal(h.dataset.dockMode, "attention");
  h.mouse(true); h.mouse(false); h.advance(100);
  assert.equal(h.nodes.compactRoot.style.visibility, "visible");
  h.listeners.get("dockMode")({ mode: "full" });
  h.mouse(true); assert.equal(h.dataset.dockMode, "full");
  assert.equal(h.nodes.panelRoot.style.visibility, "visible");
});

test("explicit dismissal suppresses hover until the pointer leaves and reenters", async () => {
  const h = await harness(() => ({ ...notch, mode: "attention" }));
  h.mouse(true);
  h.dismiss();
  h.listeners.get("dockMode")({ mode: "collapsed" });
  h.mouse(true);
  assert.deepEqual(h.calls, [true]);
  h.mouse(false); h.mouse(true);
  assert.deepEqual(h.calls, [true, true]);
});

test("non-notch hover is ignored and initial layout restores the backend mode", async () => {
  const h = await harness(() => ({ hasNotch: false }));
  h.mouse(true); assert.deepEqual(h.calls, []);
  const attention = await harness(() => ({ hasNotch: false, mode: "attention" }));
  assert.equal(attention.nodes.compactRoot.style.visibility, "visible");
  const preview = await harness(() => ({ ...notch, mode: "preview" }));
  assert.equal(preview.dataset.dockMode, "preview");
  assert.equal(preview.properties.get("--left-wing-width"), "88px");
});

test("opening and closing the panel retains the lateral layout even while hovered", async () => {
  const h = await harness(() => notch);
  h.mouse(true);
  h.listeners.get("dockMode")({ expanded: true });
  assert.equal(h.dataset.dockPresentation, "panel");
  h.listeners.get("dockMode")({ expanded: false });
  h.advance(5000);
  assert.equal(h.dataset.dockPresentation, "lateral");
  assert.equal(h.nodes.pillRoot.style.pointerEvents, "auto");
});

test("display changes update wing dimensions and restore standard mode without a notch", async () => {
  const h = await harness(() => notch);
  assert.equal(h.properties.get("--notch-width"), "200px");
  assert.equal(h.properties.get("--left-wing-width"), "88px");
  assert.equal(h.properties.get("--right-wing-width"), "0px");
  h.listeners.get("dockLayout")({ ...notch, notchWidth: 260, topInset: 38 });
  assert.equal(h.properties.get("--notch-width"), "260px");
  assert.equal(h.properties.get("--notch-inset"), "38px");
  h.listeners.get("dockLayout")({ hasNotch: false, topInset: 0 });
  assert.equal(h.presentations.at(-1).mode, "standard");
  assert.equal(h.properties.get("--left-wing-width"), "0px");
});

test("invalid camera geometry hides wings but leaves the expanded panel usable", async () => {
  const h = await harness(() => ({ ...notch, lateralAvailable: false }));
  assert.equal(h.nodes.pillRoot.style.visibility, "hidden");
  assert.equal(h.nodes.pillRoot.style.pointerEvents, "none");
  h.listeners.get("dockMode")({ expanded: true });
  assert.equal(h.nodes.panelRoot.style.visibility, "visible");
  h.listeners.get("dockMode")({ expanded: false });
  assert.equal(h.nodes.pillRoot.style.visibility, "hidden");
  h.listeners.get("dockLayout")(notch);
  assert.equal(h.nodes.pillRoot.style.visibility, "visible");
});

test("a panel event during layout fetch is retained through initialization", async () => {
  const h = await harness((listeners) => {
    listeners.get("dockMode")({ expanded: true });
    return notch;
  });
  assert.equal(h.dataset.dockPresentation, "panel");
  assert.equal(h.nodes.panelRoot.style.visibility, "visible");
  assert.equal(h.nodes.pillRoot.style.visibility, "visible");
});


const frames = {
  collapsed: {x:22,y:0,width:288,height:32},
  preview: {x:0,y:0,width:420,height:252},
  attention: {x:-70,y:0,width:560,height:232},
  full: {x:-240,y:0,width:900,height:632},
};
const geometry = (mode, id=1) => ({...notch, mode, transitionId:id,
  viewport:{x:640,y:548,width:420,height:252}, frames});

test("stationary pointer on the persistent left wing survives replacement and repeated layouts for five seconds", async () => {
  const h = await harness(() => geometry("collapsed"), false, {real:true});
  h.move(40,16);
  h.listeners.get("dockLayout")(geometry("preview",2));
  for (let i=0;i<50;i++) {
    h.syntheticLeave(); h.advance(100); await h.flush();
    h.listeners.get("dockLayout")(geometry("preview",2));
    h.advance(16); await h.flush();
  }
  assert.deepEqual(h.calls,[true]);
  assert.equal(h.nodes.dockHoverZone.style.width,"88px");
  assert.equal(h.nodes.dockHoverZone.style.pointerEvents,"auto");
  h.move(200,100); h.move(400,16); h.advance(100); await h.flush();
  assert.deepEqual(h.calls,[true,false]);
});

test("camera and right band never hover; dismissal waits for a real exit rather than a resize leave", async () => {
  const h = await harness(() => geometry("collapsed"), false, {real:true});
  h.move(150,16); h.move(320,16); h.advance(200); await h.flush();
  assert.deepEqual(h.calls,[]);
  h.move(40,16); h.listeners.get("dockLayout")(geometry("preview",2));
  h.dismiss(); h.listeners.get("dockLayout")(geometry("collapsed",3));
  h.syntheticLeave(); h.advance(100); await h.flush();
  h.move(40,16); assert.deepEqual(h.calls,[true]);
  h.mouse(false); h.advance(100); await h.flush();
  h.move(40,16); assert.deepEqual(h.calls,[true,true]);
});

test("attention height changes recheck the stationary pointer and preserve reentry cancellation", async () => {
  const h = await harness(() => geometry("attention"), false, {real:true});
  h.move(40,16);
  const shorter = geometry("attention",2);
  shorter.frames = {...frames, attention:{...frames.attention,height:172}};
  h.listeners.get("dockLayout")(shorter); h.syntheticLeave();
  h.advance(100); await h.flush(); assert.deepEqual(h.calls,[true]);
  h.mouse(false); h.advance(50); h.move(40,16); h.advance(200); await h.flush();
  assert.deepEqual(h.calls,[true]);
  h.mouse(false); h.advance(100); await h.flush();
  assert.deepEqual(h.calls,[true,false]);
  assert.equal(h.dataset.dockMode,"attention");
});
