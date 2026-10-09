import { readFile } from "node:fs/promises";
import { strict as assert } from "node:assert";
import { test } from "node:test";
import { createContext, SourceTextModule } from "node:vm";

const source = await readFile(new URL("../../app/ui/js/dock-transition.js", import.meta.url), "utf8");
const viewport = { x: 640, y: 548, width: 420, height: 252 };
const frames = {
  collapsed: { x: 22, y: 0, width: 288, height: 32 },
  preview: { x: 0, y: 0, width: 420, height: 252 },
  attention: { x: -70, y: 0, width: 560, height: 412 },
  full: { x: -240, y: 0, width: 900, height: 632 },
};
function layout(id, mode, fromMode = null) {
  return { viewport, frames, topInset: 32, lateralAvailable: true, leftWingWidth: 88, transitionId: id,
    ...(fromMode ? { transition: { id, fromMode, toMode: mode, durationMs: 160, from: frames[fromMode], to: frames[mode] } } : {}) };
}
async function harness(reduced = false) {
  const animations = [], completed = [];
  function element(opacity = 0) {
    return { style: { opacity }, computed: {}, animate(keyframes, options) {
      let resolve, reject;
      const finished = new Promise((yes, no) => { resolve = yes; reject = no; });
      const animation = { element: this, keyframes, options, finished, resolve, cancelled: false,
        cancel() { this.cancelled = true; reject(new Error("cancelled")); } };
      animations.push(animation);
      return animation;
    } };
  }
  const surface = element(), backdrop = element(), roots = { pill: element(1), compact: element(), panel: element() };
  let motionChanged;
  const motion = { matches: reduced, addEventListener: (_, callback) => { motionChanged = callback; } };
  const context = createContext({ window: {
    matchMedia: () => motion,
    getComputedStyle: element => ({ ...element.style, ...element.computed }),
  } });
  const module = new SourceTextModule(source, { context });
  await module.link(() => {}); await module.evaluate();
  const transition = module.namespace.createDockTransition(surface, roots, backdrop, id => completed.push(id));
  async function finish() { animations.filter(a => !a.cancelled).forEach(a => a.resolve()); await new Promise(resolve => setImmediate(resolve)); }
  return { surface, backdrop, roots, animations, completed, transition, finish,
    reduceMotion: () => { motion.matches = true; motionChanged(); } };
}

test("closing clips and fades outgoing content for 160 ms with wings already anchored", async () => {
  const h = await harness();
  h.transition.update(layout(1, "preview"), "preview");
  h.transition.update(layout(2, "collapsed", "preview"), "collapsed");
  assert.equal(h.roots.compact.style.visibility, "visible");
  assert.equal(h.roots.pill.style.left, "22px");
  assert.equal(h.roots.pill.style.width, "288px");
  assert.equal(h.roots.compact.style.height, "220px");
  assert.equal(h.animations.length, 4);
  assert.equal(h.animations[0].options.duration, 160);
  assert.match(h.animations[0].keyframes[1].clipPath, /inset\(0px 110px 220px 22px round 0px 0px 8px 8px\)/);
  await h.finish();
  assert.equal(h.roots.compact.style.visibility, "hidden");
  assert.deepEqual(h.completed, [2]);
  // The final native resize only changes local coordinates, never screen coordinates.
  const final = { ...layout(2, "collapsed"), viewport: { x: 662, y: 768, width: 288, height: 32 },
    frames: Object.fromEntries(Object.entries(frames).map(([key, frame]) => [key, { ...frame, x: frame.x - 22 }])) };
  h.transition.update(final, "collapsed");
  assert.equal(h.roots.pill.style.left, "0px");
  assert.equal(final.viewport.x + Number.parseFloat(h.roots.pill.style.left), 662);
});

test("duplicate and stale events cannot restart a transition or complete it twice", async () => {
  const h = await harness();
  h.transition.update(layout(1, "preview"), "preview");
  const closing = layout(2, "collapsed", "preview");
  h.transition.update(closing, "collapsed");
  h.transition.update(closing, "collapsed");
  h.transition.update(layout(1, "preview"), "preview");
  assert.equal(h.animations.length, 4);
  await h.finish(); await h.finish();
  assert.deepEqual(h.completed, [2]);
});

for (const target of ["preview", "attention"]) {
  test(`${target === "preview" ? "mouse reentry" : "a new request"} reverses a closing transition from its current appearance`, async () => {
    const h = await harness();
    h.transition.update(layout(1, "preview"), "preview");
    h.transition.update(layout(2, "collapsed", "preview"), "collapsed");
    h.surface.computed.clipPath = "inset(0px 29px 110px 37px round 0px 0px 14px 14px)";
    h.roots.compact.computed.opacity = "0.5";
    h.roots.pill.computed.opacity = "0.5";
    h.transition.update(layout(3, target, "collapsed"), target);
    assert.equal(h.animations[0].cancelled, true);
    assert.equal(h.animations[4].keyframes[0].clipPath, h.surface.computed.clipPath);
    assert.equal(h.animations[5].keyframes[0].opacity, 0.5);
    await h.finish();
    assert.deepEqual(h.completed, [3]);
  });
}

test("transparent space outside the animated surface cannot count as hover", async () => {
  const h = await harness();
  h.transition.update(layout(1, "preview"), "preview");
  h.transition.update(layout(2, "collapsed", "preview"), "collapsed");
  h.surface.computed.clipPath = "inset(0px 40px 160px 50px round 0px 0px 14px 14px)";
  assert.equal(h.transition.containsPoint(20, 20), false);
  assert.equal(h.transition.containsPoint(100, 100), false);
  assert.equal(h.transition.containsPoint(100, 20), true);
  h.transition.cancel();
});

test("the menu-bar band to the right stays outside hover during opening and closing", async () => {
  const h = await harness();
  h.transition.update(layout(1, "preview", "collapsed"), "preview");
  assert.equal(h.transition.containsPoint(310, 16), false);
  assert.equal(h.transition.containsPoint(310, 40), true);
  await h.finish();
  h.transition.update(layout(2, "collapsed", "preview"), "collapsed");
  h.surface.computed.clipPath = "inset(0px 0px 0px 0px round 0px 0px 20px 20px)";
  assert.equal(h.transition.containsPoint(310, 16), false);
  assert.equal(h.transition.containsPoint(40, 16), true);
  h.transition.cancel();
});

test("reduced motion completes immediately and display changes cancel pending animation", async () => {
  const reduced = await harness(true);
  reduced.transition.update(layout(1, "preview", "collapsed"), "preview");
  assert.equal(reduced.animations.length, 0);
  assert.deepEqual(reduced.completed, [1]);
  const h = await harness();
  h.transition.update(layout(1, "preview", "collapsed"), "preview");
  h.transition.update(layout(2, "collapsed"), "collapsed");
  await h.finish();
  assert.equal(h.transition.animating, false);
  assert.deepEqual(h.completed, []);
  assert.equal(h.roots.compact.style.visibility, "hidden");
});

test("enabling reduced motion during a transition immediately settles the visible surface", async () => {
  const h = await harness();
  h.transition.update(layout(1, "preview"), "preview");
  h.transition.update(layout(2, "collapsed", "preview"), "collapsed");
  h.reduceMotion();
  assert.equal(h.transition.animating, false);
  assert.equal(h.roots.compact.style.visibility, "hidden");
  assert.equal(h.animations.every(a => a.cancelled), true);
  assert.deepEqual(h.completed, [2]);
});


test("hover uses the persistent left wing throughout clipping and excludes the camera", async () => {
  const h = await harness();
  h.transition.update(layout(1,"preview","collapsed"),"preview");
  h.surface.computed.clipPath = "inset(0px 110px 220px 22px round 0px 0px 8px 8px)";
  assert.equal(h.transition.containsHoverPoint(40,16,"preview"),true);
  assert.equal(h.transition.containsHoverPoint(150,16,"preview"),false);
  assert.equal(h.transition.containsHoverPoint(310,16,"preview"),false);
  assert.equal(h.transition.containsHoverPoint(40,100,"preview"),false);
  await h.finish();
  h.surface.computed.clipPath = "inset(0px 0px 0px 0px round 0px 0px 20px 20px)";
  assert.equal(h.transition.containsHoverPoint(100,100,"preview"),true);
  h.transition.update(layout(2,"collapsed","preview"),"collapsed");
  assert.equal(h.transition.containsHoverPoint(40,16,"collapsed"),true);
  await h.finish();
  assert.equal(h.transition.containsHoverPoint(100,100,"collapsed"),false);
});

test("changing attention height animates from the previous rectangle and fits the new content", async () => {
  const h = await harness();
  const old = layout(1,"attention");
  h.transition.update(old,"attention");
  const next = {...layout(2,"attention","attention"),
    frames:{...frames,attention:{...frames.attention,height:232}}};
  next.transition.to = next.frames.attention;
  h.transition.update(next,"attention");
  assert.equal(h.roots.compact.style.height,"200px");
  assert.match(h.animations[0].keyframes[0].clipPath,/inset\(0px -70px -160px -70px/);
  assert.match(h.animations[0].keyframes[1].clipPath,/inset\(0px -70px 20px -70px/);
  await h.finish();
  assert.deepEqual(h.completed,[2]);
});

for (const mode of ["preview", "attention", "full"]) {
  test(`the lateral pet stays opaque and unanimated while opening and closing ${mode}`, async () => {
    const h = await harness();
    h.transition.update(layout(1, "collapsed"), "collapsed");
    h.transition.update(layout(2, mode, "collapsed"), mode);
    assert.equal(h.roots.pill.style.opacity, 1);
    assert.equal(h.roots.pill.style.visibility, "visible");
    assert.equal(h.roots.pill.style.pointerEvents, "auto");
    assert.equal(h.roots.pill.style.left, "22px");
    await h.finish();
    h.transition.update(layout(3, "collapsed", mode), "collapsed");
    assert.equal(h.roots.pill.style.opacity, 1);
    assert.equal(h.roots.pill.style.visibility, "visible");
    assert.equal(h.animations.some(a => a.element === h.roots.pill), false);
    assert.equal(h.roots.pill.style.clipPath, undefined);
    await h.finish();
    assert.deepEqual(h.completed, [2, 3]);
  });
}
