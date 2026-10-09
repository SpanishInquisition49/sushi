// Four presentations share one native dock window. Rust owns presentation policy and bounds.
import { invoke, listen } from "./api.js";
import * as PetWindow from "./pet-window.js";
import * as Panel from "./panel.js";
import * as Compact from "./compact-panel.js";
import { createDockTransition } from "./dock-transition.js";

function show(el, visible) {
  el.style.opacity = 1;
  el.style.visibility = visible ? "visible" : "hidden";
  el.style.pointerEvents = visible ? "auto" : "none";
}
export async function mount(root) {
  root.innerHTML = '<div id="dockHoverZone" class="dock-hover-zone" aria-hidden="true"></div><div id="pillRoot"></div><div id="dockSurface" class="dock-surface"><div id="dockBackdrop" class="dock-backdrop"></div><div id="compactRoot"></div><div id="panelRoot"></div></div>';
  const hoverZone = document.getElementById("dockHoverZone");
  const pillRoot = document.getElementById("pillRoot");
  const compactRoot = document.getElementById("compactRoot");
  const panelRoot = document.getElementById("panelRoot");
  const transition = createDockTransition(document.getElementById("dockSurface"),
    { pill: pillRoot, compact: compactRoot, panel: panelRoot }, document.getElementById("dockBackdrop"),
    id => invoke("finish_dock_transition", { id }).catch(error => console.error("Unable to complete dock transition", error)));
  let mode = "collapsed", layout = null;
  let presentationId = -1;
  let inside = false, leaveTimer = null, ignoreUntilExit = false;
  let modeChanged = false, layoutChanged = false;
  let pointerVersion = 0, pointerFrame = null;
  let renderRevision = -1;

  function acknowledgeRender() {
    const ack = layout?.renderAck;
    if (!ack || ack.revision <= renderRevision) return;
    renderRevision = ack.revision;
    // The first callback precedes paint; the next gives WebKit a rendering opportunity.
    // Native code then fences removal with an afterScreenUpdates WebKit snapshot.
    requestAnimationFrame(() => requestAnimationFrame(() => {
      if (layout?.transitionId !== ack.transitionId || layout?.renderAck?.revision !== ack.revision) return;
      invoke("ack_dock_render", { id: ack.transitionId, revision: ack.revision })
        .catch(error => console.error("Unable to acknowledge dock rendering", error));
    }));
  }

  function hover(next) {
    clearTimeout(leaveTimer); leaveTimer = null;
    if (inside === next) return;
    inside = next;
    invoke("set_dock_hover", { hovered: next }).catch(error => {
      inside = false;
      console.error("Unable to update dock presentation", error);
    });
  }
  const geometricHover = () => !!layout?.frames && !!layout?.viewport;
  async function checkPointer(finalExit = false) {
    const version = pointerVersion;
    try {
      const point = await invoke("get_dock_pointer");
      if (version !== pointerVersion || mode === "full" || !geometricHover()) return;
      if (!point || !Number.isFinite(point.x) || !Number.isFinite(point.y)) return;
      if (point.transitionId !== undefined && point.transitionId !== layout.transitionId) return;
      if (transition.containsHoverPoint(point.x, point.y, mode)) {
        clearTimeout(leaveTimer); leaveTimer = null;
        if (!ignoreUntilExit) hover(true);
      } else if (finalExit) {
        ignoreUntilExit = false;
        hover(false);
      } else leave();
    } catch (error) {
      console.error("Unable to check dock pointer", error);
      // An unavailable pointer query must not turn a resize into a close/open loop.
    }
  }
  function recheckPointer() {
    if (!geometricHover() || (!inside && !ignoreUntilExit) || pointerFrame !== null) return;
    pointerFrame = requestAnimationFrame(() => { pointerFrame = null; checkPointer(); });
  }
  function leave() {
    if (!geometricHover()) ignoreUntilExit = false;
    if ((!inside && !ignoreUntilExit) || leaveTimer !== null) return;
    leaveTimer = setTimeout(() => {
      leaveTimer = null;
      if (geometricHover()) checkPointer(true);
      else { ignoreUntilExit = false; hover(false); }
    }, 100);
  }
  function track(event) {
    pointerVersion++;
    if (mode === "full") return;
    let visibleContent;
    if (geometricHover()) {
      visibleContent = transition.containsHoverPoint(event.clientX, event.clientY, mode);
    } else {
      visibleContent = mode === "collapsed"
        ? layout?.lateralAvailable && event.target.closest(".notch-wing")
        : event.target.closest(".compact-panel") || event.target.closest(".dock-hover-zone");
    }
    if (visibleContent) {
      clearTimeout(leaveTimer); leaveTimer = null;
      if (!ignoreUntilExit) hover(true);
    } else { ignoreUntilExit = false; leave(); }
  }
  root.addEventListener("mousemove", track);
  root.addEventListener("mouseover", track);
  root.addEventListener("mouseleave", () => { pointerVersion++; leave(); });
  root.addEventListener("click", event => {
    if (event.target.closest('[data-act="compact-close"]')) {
      pointerVersion++;
      ignoreUntilExit = true;
      clearTimeout(leaveTimer); leaveTimer = null; inside = false;
    }
  });

  function present() {
    const hasNotch = layout?.hasNotch === true;
    const html = document.documentElement;
    html.dataset.dockMode = mode;
    html.dataset.dockPresentation = mode === "collapsed" ? hasNotch ? "lateral" : "standard" : mode === "full" ? "panel" : mode;
    html.classList.toggle("expanded", mode !== "collapsed");
    PetWindow.setPresentation(hasNotch ? "lateral" : "standard", { layout });
    Compact.setLayout(layout);
    const wing = layout?.frames?.collapsed;
    Object.assign(hoverZone.style, {
      left: `${wing?.x ?? 0}px`, top: `${wing?.y ?? 0}px`,
      width: `${layout?.leftWingWidth ?? 0}px`, height: `${layout?.topInset ?? 0}px`,
      pointerEvents: hasNotch && layout?.lateralAvailable && mode !== "full" ? "auto" : "none",
    });
    recheckPointer();
    // Keep the outgoing compact content intact while its surface fades away.
    if (mode === "preview" || mode === "attention") Compact.setMode(mode);
    if (transition.update(layout, mode)) { acknowledgeRender(); return; }
    show(pillRoot, hasNotch ? layout?.lateralAvailable === true : mode === "collapsed");
    show(compactRoot, mode === "preview" || mode === "attention");
    show(panelRoot, mode === "full");
  }
  function applyMode(next) {
    if (next?.transitionId !== undefined && next.transitionId < presentationId) return;
    if (next?.transitionId !== undefined) presentationId = next.transitionId;
    const value = next?.mode ?? (next?.expanded ? "full" : "collapsed");
    if (!["collapsed", "preview", "attention", "full"].includes(value)) return;
    if (value === "collapsed" && (mode === "full" || mode === "attention")) {
      inside = false; clearTimeout(leaveTimer); leaveTimer = null;
    }
    mode = value;
    if (next?.viewport) layout = next;
    present();
  }
  await listen("dockMode", next => { modeChanged = true; applyMode(next); });
  function applyLayout(next, initial = false) {
    if (next?.transitionId !== undefined && next.transitionId < presentationId) return;
    if (layout && next?.displayRevision !== layout.displayRevision) {
      pointerVersion++;
      inside = false; clearTimeout(leaveTimer); leaveTimer = null;
    }
    layout = next;
    const html = document.documentElement, hasNotch = next?.hasNotch === true;
    html.classList.toggle("has-notch", hasNotch);
    html.classList.toggle("compositor-dock", !!next?.viewport);
    html.style.setProperty("--notch-inset", `${hasNotch ? next.topInset : 0}px`);
    html.style.setProperty("--notch-width", `${hasNotch ? next.notchWidth : 0}px`);
    html.style.setProperty("--left-wing-width", `${hasNotch ? next.leftWingWidth : 0}px`);
    html.style.setProperty("--right-wing-width", `${hasNotch ? next.rightWingWidth : 0}px`);
    if (next?.mode && (!initial || !modeChanged)) applyMode(next);
    else present();
  }
  await listen("dockLayout", next => { layoutChanged = true; applyLayout(next); });
  try {
    const next = await invoke("get_dock_layout");
    if (!layoutChanged) applyLayout(next, true);
  } catch (error) {
    console.error("Unable to initialize notch layout", error);
    if (!layoutChanged) applyLayout(null, true);
  }
  PetWindow.mount(pillRoot);
  Panel.mount(panelRoot);
  Compact.mount(compactRoot);
}
