// The notch's two intermediate presentations. Request controls are shared with panel.js.
import { invoke, listen } from "./api.js";
import { store, start } from "./store.js";
import { ui, esc } from "./ui.js";
import { Pet, configFrom } from "./pet.js";
import * as Fmt from "./fmt.js";
import * as Requests from "./requests.js";
import * as Viewer from "./viewer.js";

let root, mode = "preview", adaptive = false, probe, measureFrame = null;
let measuredHeight = null;
const header = () => `<header><div class="compact-pet"></div><strong class="compact-title"></strong><button class="btn sm ghost" data-act="compact-full" title="Open full panel">Open full panel</button><button class="btn sm ghost" data-act="compact-close" aria-label="Minimize panel">×</button></header>`;

function scheduleMeasure() {
  if (!adaptive || !probe || measureFrame !== null) return;
  measureFrame = requestAnimationFrame(() => {
    measureFrame = null;
    if (!adaptive) return;
    const height = Math.min(380, Math.ceil(probe.getBoundingClientRect().height));
    if (!Number.isFinite(height) || height < 1 || height === measuredHeight) return;
    measuredHeight = height;
    invoke("fit_attention", { height }).catch(error => {
      measuredHeight = null;
      console.error("Unable to fit attention panel", error);
    });
  });
}
export function setLayout(layout) {
  adaptive = layout?.hasNotch === true && layout?.lateralAvailable === true;
  if (adaptive) scheduleMeasure();
}
const pet = new Pet(Date.now(), configFrom(store.settings));
function replace(el, html) {
  if (el._html === html) return;
  const scroll = el.scrollTop;
  el.innerHTML = html; el._html = html; el.scrollTop = scroll;
}
function rows() {
  const snap = store.snapshot, now = Date.now();
  if (!store.up) return '<p class="muted">Daemon is not running. Start <code>sushi daemon</code>.</p>';
  const session = Fmt.activeSession(snap);
  const details = Fmt.sessionRows(session, now).map(({ key, value }) =>
    `<div class="compact-detail"><span class="muted">${esc(key)}</span><span>${esc(value)}</span></div>`).join("");
  const limits = Fmt.allLimits(snap).flatMap(({label, data, error}) => {
    if (error) return [`<div class="muted small">${esc(label)}: ${esc(error)}</div>`];
    const windows = [["5-hour", data?.five_hour], ["Weekly", data?.seven_day],
      ...(data?.models || []).map(m => [m.kind ? `${m.model} · ${m.kind}` : m.model, m])];
    return windows.filter(([, w]) => w).map(([name, w]) => {
      const pct = Math.max(0, Math.min(100, Number(w.percent) || 0));
      return `<div class="compact-limit"><div><span>${esc(label)} · ${esc(name)}</span><strong>${esc(Fmt.round(w.percent))}%</strong></div><div class="track"><div class="bar" style="width:${pct}%;background:var(--primary)"></div></div></div>`;
    });
  }).join("");
  return (details || '<p class="muted">No active sessions.</p>') + limits;
}
function waitingCard(session, snap) {
  const detail = session.attention?.detail;
  const body = detail?.type === "questions"
    ? (detail.questions || []).map(q => `<div class="compact-question"><strong>${esc(q.header || "")}</strong><p>${esc(q.question)}</p>${q.options?.length
      ? `<ul>${q.options.map(o => `<li>${esc(o.label)}${o.description ? ` — ${esc(o.description)}` : ""}</li>`).join("")}</ul>` : ""}</div>`).join("")
    : detail ? Viewer.render(detail) : "";
  const where = session.agent === "codex" ? "Continue in the Codex app or CLI where this session is running." : "Continue in this session’s terminal.";
  return `<div class="terminal-wait"><strong>${esc(session.name || session.id)}</strong><div class="muted">${esc(Fmt.agentLabel(snap, session.agent))} is waiting for you. ${where}</div>${body}</div>`;
}
function render() {
  if (!root) return;
  const snap = store.snapshot;
  const title = mode === "attention" ? `${Fmt.needsYou(snap)} waiting for you` : "Session details";
  replace(root.querySelector(".compact-title"), esc(title));
  const requests = store.up ? snap.pending.map(p => Requests.permissionCard(p)).join("") : "";
  const asked = new Set(snap.pending.map(p => p.session_id));
  const waiting = store.up ? snap.sessions.filter(s => s.status === "waiting" && !asked.has(s.id)).map(s => waitingCard(s, snap)).join("") : "";
  const attention = requests + waiting || rows();
  replace(root.querySelector(".compact-content"), mode === "attention" ? attention : rows());
  // A separate natural-height copy can be measured while the visible panel is hidden,
  // constrained by its native frame, or fading out. Its width is always the attention width.
  replace(probe.querySelector(".compact-title"), esc(`${Fmt.needsYou(snap)} waiting for you`));
  replace(probe.querySelector(".compact-content"), attention);
  scheduleMeasure();
}
export function setMode(next) { mode = next; render(); }
export function mount(container) {
  root = container;
  root.innerHTML = `<div class="compact-panel">${header()}<div class="compact-content scroll grow"></div></div>`;
  probe = document.createElement("div");
  probe.className = "compact-panel compact-measure";
  probe.setAttribute("inert", "");
  probe.setAttribute("aria-hidden", "true");
  probe.innerHTML = `${header()}<div class="compact-content"></div>`;
  root.appendChild(probe);
  const resize = new ResizeObserver(scheduleMeasure);
  resize.observe(probe);
  document.fonts?.ready.then(scheduleMeasure);
  document.fonts?.addEventListener("loadingdone", scheduleMeasure);
  root.addEventListener("click", event => {
    const el = event.target.closest("[data-act]");
    if (!el || el.disabled) return;
    if (el.dataset.act === "compact-full") invoke("open_panel");
    else if (el.dataset.act === "compact-close") invoke("close_panel");
    else Requests.handleAction(el.dataset);
  });
  Requests.subscribe(render);
  listen("petEvent", ev => ev && pet.onEvent(ev.kind, ev.ts, Date.now()));
  start(() => { pet.onSnapshot(store.snapshot, store.up, Date.now()); render(); },
    () => { pet.configure(configFrom(store.settings)); render(); });
  setInterval(() => {
    const now = Date.now(); pet.tick(now);
    replace(root.querySelector(".compact-pet"), pet.build(32, { room: 2, margin: 2, maxHeight: 40, detail: false }));
  }, 66);
  setInterval(render, 1000);
}
