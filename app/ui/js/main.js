// main.js — one page, two windows: `?view=pet` is the small always-on-top pet, `?view=panel` the panel.

const view = new URLSearchParams(location.search).get("view") === "pet" ? "pet" : "panel";
document.documentElement.dataset.view = view;

const root = document.getElementById("root");
const mod = await import(view === "pet" ? "./pet-window.js" : "./panel.js");
mod.mount(root);
