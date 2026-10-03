// main.js — one page, up to two windows: `?view=pet` is the small always-on-top pet, `?view=panel`
// the panel. On docked platforms (see app/src-tauri/src/dock*.rs — macOS, Windows, and Linux
// where the compositor speaks wlr-layer-shell) there is only one window and `?view=dock` loads
// both, swapping between the pet pill and the full panel in place instead of opening a second,
// centered window.

const v = new URLSearchParams(location.search).get("view");
const view = v === "dock" ? "dock" : v === "pet" ? "pet" : "panel";
document.documentElement.dataset.view = view;

const root = document.getElementById("root");
const mod = await import(view === "dock" ? "./dock-window.js" : view === "pet" ? "./pet-window.js" : "./panel.js");
mod.mount(root);
