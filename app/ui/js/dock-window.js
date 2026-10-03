// dock-window.js — docked platforms only (macOS, Windows, and Linux where the compositor speaks
// wlr-layer-shell; see the dock*.rs files under app/src-tauri/src): the one window that is the
// pet pill when collapsed and the full panel when expanded. It mounts the exact same
// pet-window.js and panel.js used on every platform into two sibling containers, toggled by the
// native side resizing/repositioning the window and telling this page which one to show — no
// logic is duplicated, so the two stay aligned the same way they already do everywhere else.

import { listen } from "./api.js";
import * as PetWindow from "./pet-window.js";
import * as Panel from "./panel.js";

function show(el, visible) {
  el.style.visibility = visible ? "visible" : "hidden";
  el.style.pointerEvents = visible ? "auto" : "none";
}

export function mount(root) {
  root.innerHTML = '<div id="pillRoot"></div><div id="panelRoot"></div>';
  const pillRoot = document.getElementById("pillRoot");
  const panelRoot = document.getElementById("panelRoot");
  show(pillRoot, true);
  show(panelRoot, false);

  // The Rust side owns the expand/collapse decision (a click on the pill, a new permission
  // request with "open on request" on, losing focus, …) and resizes the native window to match;
  // this just follows along with which content should be visible inside it.
  listen("dockMode", (m) => {
    const expanded = !!m?.expanded;
    document.documentElement.classList.toggle("expanded", expanded);
    show(pillRoot, !expanded);
    show(panelRoot, expanded);
  });

  PetWindow.mount(pillRoot);
  Panel.mount(panelRoot);
}
