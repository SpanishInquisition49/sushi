// Animate a clipped surface in WebKit; native bounds change only before and after the transition.
const easing = "cubic-bezier(0.4, 0, 0.2, 1)";
function inset(shape, viewport) {
  const radius = shape.radius ?? 20;
  return `inset(${shape.y}px ${viewport.width - shape.x - shape.width}px ${viewport.height - shape.y - shape.height}px ${shape.x}px round 0px 0px ${radius}px ${radius}px)`;
}
function opacity(element) {
  return Number(window.getComputedStyle(element).opacity) || 0;
}
export function createDockTransition(surface, roots, backdrop, complete) {
  let viewport = null, shape = null, cameraBand = null, animations = [], activeId = null, lastId = -1;
  let finishActive = null;
  let compactMode = "preview", hoverWing = null;
  const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
  motion.addEventListener?.("change", () => {
    if (!motion.matches || !finishActive) return;
    const finish = finishActive;
    for (const animation of animations) animation.cancel();
    animations = [];
    finish();
  });
  function currentShape() {
    if (!viewport || !shape) return shape;
    const clip = window.getComputedStyle(surface).clipPath;
    const values = clip?.split(" round ")[0].match(/-?[\d.]+(?=px)/g)?.map(Number);
    if (values?.length !== 4) return shape;
    const [top, right, bottom, left] = values;
    const radius = clip.split(" round ")[1]?.match(/[\d.]+(?=px)/g)?.map(Number)?.[2];
    return { x: left, y: top, width: viewport.width - left - right, height: viewport.height - top - bottom, radius: radius ?? shape.radius };
  }
  function cancel() {
    for (const animation of animations) animation.cancel();
    animations = []; activeId = null; finishActive = null;
  }
  function targets(mode) {
    return { compact: mode === "preview" || mode === "attention" ? 1 : 0,
      panel: mode === "full" ? 1 : 0, backdrop: mode === "collapsed" ? 0 : 1 };
  }
  function update(layout, mode) {
    if (!layout?.viewport || !layout?.frames) {
      cancel(); viewport = null; shape = null; cameraBand = null; hoverWing = null; surface.style.clipPath = "none";
      backdrop.style.visibility = "hidden"; backdrop.style.pointerEvents = "none";
      for (const element of Object.values(roots)) {
        for (const property of ["left", "top", "width", "height", "right", "bottom"]) element.style[property] = "";
      }
      return false;
    }
    const transition = layout.transition, id = layout.transitionId ?? transition?.id ?? 0;
    if (id < lastId || (transition && id === lastId)) return true;
    lastId = id;
    // The camera band is a sibling of the animated surface and never fades or clips.
    const elements = { compact: roots.compact, panel: roots.panel, backdrop };
    Object.assign(roots.pill.style, { opacity: 1, visibility: layout.lateralAvailable ? "visible" : "hidden",
      pointerEvents: layout.lateralAvailable ? "auto" : "none" });
    const visible = Object.fromEntries(Object.entries(elements).map(([key, element]) => [key, opacity(element)]));
    let start = currentShape();
    const previousViewport = viewport;
    cancel();
    viewport = layout.viewport;
    cameraBand = layout.frames.collapsed;
    hoverWing = layout.lateralAvailable ? { ...cameraBand, width: layout.leftWingWidth } : null;
    if (start && previousViewport) {
      start = { ...start, x: start.x + previousViewport.x - viewport.x,
        y: start.y + viewport.y + viewport.height - previousViewport.y - previousViewport.height };
    } else start = { ...(transition?.from ?? layout.frames[mode]), radius: transition?.fromMode === "collapsed" || mode === "collapsed" && !transition ? 8 : 20 };
    shape = { ...(transition?.to ?? layout.frames[mode]), radius: mode === "collapsed" ? 8 : 20 };
    if (mode === "preview" || mode === "attention") compactMode = mode;
    else if (transition?.fromMode === "attention") compactMode = "attention";
    const frames = { pill: layout.frames.collapsed, compact: layout.frames[compactMode], panel: layout.frames.full };
    for (const [key, element] of Object.entries(roots)) {
      const frame = frames[key];
      if (!frame) continue;
      const topInset = key === "pill" ? 0 : layout.topInset || 0;
      Object.assign(element.style, { left: `${frame.x}px`, top: `${frame.y + topInset}px`,
        width: `${frame.width}px`, height: `${Math.max(0, frame.height - topInset)}px`, right: "auto", bottom: "auto" });
    }
    const destination = targets(mode);
    const duration = transition && !motion.matches ? transition.durationMs : 0;
    surface.style.clipPath = inset(shape, viewport);
    for (const [key, element] of Object.entries(elements)) {
      element.style.opacity = destination[key];
      element.style.visibility = duration && (visible[key] > 0 || destination[key] > 0) || destination[key] > 0 ? "visible" : "hidden";
      element.style.pointerEvents = destination[key] > 0 || duration && visible[key] > 0 ? "auto" : "none";
    }
    if (!transition) return true;
    activeId = transition.id;
    const finish = () => {
      if (activeId !== transition.id) return;
      activeId = null; finishActive = null;
      for (const [key, element] of Object.entries(elements)) {
        element.style.visibility = destination[key] ? "visible" : "hidden";
        element.style.pointerEvents = destination[key] ? "auto" : "none";
      }
      complete(transition.id);
    };
    finishActive = finish;
    if (!duration || !surface.animate) { finish(); return true; }
    animations.push(surface.animate([{ clipPath: inset(start, viewport) }, { clipPath: inset(shape, viewport) }], { duration, easing, fill: "forwards" }));
    for (const [key, element] of Object.entries(elements)) {
      animations.push(element.animate([{ opacity: visible[key] }, { opacity: destination[key] }], { duration, easing, fill: "forwards" }));
    }
    Promise.all(animations.map(animation => animation.finished)).then(finish).catch(() => {});
    return true;
  }
  return { update, cancel, get animating() { return activeId !== null; },
    containsHoverPoint(x, y, mode) {
      const contains = box => box && x >= box.x && x < box.x + box.width && y >= box.y && y < box.y + box.height;
      // The persistent wing remains a hover target while the panel contracts.
      if (contains(hoverWing)) return true;
      if (mode === "collapsed" && activeId === null) return false;
      const box = currentShape();
      if (!box || !contains(box)) return false;
      // Only the panel below the camera band is interactive, never the camera itself.
      return y >= (cameraBand?.y ?? 0) + (cameraBand?.height ?? 0);
    },
    containsPoint(x, y) {
      const box = currentShape();
      // The menu-bar band outside the camera/left wing stays transparent in every mode.
      if (cameraBand && y < cameraBand.y + cameraBand.height
        && (x < cameraBand.x || x >= cameraBand.x + cameraBand.width)) return false;
      return !box || x >= box.x && x <= box.x + box.width && y >= box.y && y <= box.y + box.height;
    } };
}
