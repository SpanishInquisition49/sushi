// Shared request cards, selections and submission state for the full and compact panels.
import { invoke, emit } from "./api.js";
import { store } from "./store.js";
import { ui, esc } from "./ui.js";
import * as Fmt from "./fmt.js";
import * as Viewer from "./viewer.js";
const snap = () => store.snapshot;
const choices = new Map();
const busy = new Set();
const errors = new Map();
const subscribers = new Set();
export function subscribe(callback) { subscribers.add(callback); return () => subscribers.delete(callback); }
function notify(result = null) { for (const callback of subscribers) callback(result); }

async function settle(id, send, eventKind) {
  if (busy.has(id)) return;
  busy.add(id); errors.delete(id); notify();
  let succeeded = false;
  try {
    await send();
    store.snapshot = { ...snap(), pending: snap().pending.filter((p) => p.id !== id) };
    choices.delete(id);
    emit("petEvent", { kind: eventKind, ts: Date.now() });
    succeeded = true;
  } catch (error) { errors.set(id, String(error)); }
  finally { busy.delete(id); notify(succeeded ? { settled: id } : null); }
}
const decide = (action, id) => settle(id, () => invoke("decide", { action, id }), action === "deny" ? "deny" : "approve");

function btn(label, { variant = "ghost", act, data = {}, tip, off, glyph, small } = {}) {
  const request = snap().pending.find((p) => String(p.id) === String(data.id));
  off = off || busy.has(data.id) || request?.answerable === false;
  const d = Object.entries({ act, ...data })
    .filter(([, v]) => v !== undefined)
    .map(([k, v]) => ` data-${k}="${esc(v)}"`)
    .join("");
  return `<button class="btn ${variant}${small ? " sm" : ""}"${d}${tip ? ` title="${esc(tip)}"` : ""}${off ? " disabled" : ""}>${
    glyph ? ui.glyph({ name: glyph, size: 16 }) : ""
  }${label ? `<span>${esc(label)}</span>` : ""}</button>`;
}

const picked = (id, qi) => choices.get(id)?.get(qi) || new Set();

function questionsAnswers(p) {
  const answers = {};
  const list = p.detail?.questions || [];
  for (const [qi, q] of list.entries()) {
    const chosen = picked(p.id, qi);
    const labels = (q.options || []).filter((o) => chosen.has(o.label)).map((o) => o.label);
    if (!labels.length) return null; // not answered yet
    answers[q.question] = labels.join(", ");
  }
  return answers;
}

function sendAnswers(p) {
  const answers = questionsAnswers(p);
  if (!answers) return;
  settle(p.id, () => invoke("answer", { id: p.id, answers }), "approve");
}

function choose(p, qi, label) {
  const q = (p.detail?.questions || [])[qi];
  if (!q) return;
  if (!choices.has(p.id)) choices.set(p.id, new Map());
  const c = choices.get(p.id);
  if (q.multi) {
    const set = new Set(c.get(qi) || []);
    set.has(label) ? set.delete(label) : set.add(label);
    c.set(qi, set);
  } else c.set(qi, new Set([label]));
  const list = p.detail?.questions || [];
  if (list.length === 1 && !q.multi) sendAnswers(p); // one question, one choice: nothing more to ask
  else notify();
}

function questionCard(p, who) {
  const list = p.detail?.questions || [];
  const children = [
    ui.row({ align: "center", gap: 8 }, [
      ui.glyph({ name: "help-circle", size: 18, color: "primary" }),
      ui.label({ text: `${who} is asking you`, fontSize: 14, fontWeight: "semibold", color: "on_surface", maxLines: 1, flexGrow: 1 }),
    ]),
  ];
  for (const [qi, q] of list.entries()) {
    const chosen = picked(p.id, qi);
    const block = [];
    if (q.header) block.push(ui.label({ text: q.header, fontSize: 10, fontWeight: "bold", color: "primary" }));
    block.push(ui.label({ text: q.question, fontSize: 14, fontWeight: "semibold", color: "on_surface", maxLines: 4, maxWidth: 520 }));
    if (q.multi) block.push(ui.label({ text: "Choose one or more", fontSize: 11, color: "on_surface_variant" }));
    for (const o of q.options || []) {
      const on = chosen.has(o.label);
      block.push(
        `<div class="option">${btn(on && q.multi ? "✓ " + o.label : o.label, {
          variant: on ? "primary" : "outline", act: "choose", data: { id: p.id, qi, label: o.label }, tip: o.description || undefined,
        })}${ui.label({ text: o.description || "", fontSize: 11, color: "on_surface_variant", maxLines: 2, maxWidth: 360, flexGrow: 1 })}</div>`,
      );
    }
    children.push(ui.column({ gap: 6 }, block));
  }
  // Several questions (or a multiple choice) need an explicit send.
  if (!(list.length === 1 && !list[0].multi)) {
    children.push(btn("Send answers", { variant: "primary", act: "send-answers", data: { id: p.id }, off: questionsAnswers(p) == null }));
  }
  children.push(ui.label({ text: "You can also answer it in the terminal.", fontSize: 11, color: "on_surface_variant" }));
  return ui.column({ gap: 10, padding: 10, fill: "primary/0.10", radius: 12 }, children);
}

/** A plan the agent wants approved (Claude Code's "Ready to code?"). While its hook waits it can be
 *  approved here; after that it stays for reading until it has been dealt with in the terminal. */
function planCard(p, who) {
  const children = [
    ui.row({ align: "center", gap: 8 }, [
      ui.glyph({ name: "list-check", size: 18, color: "primary" }),
      ui.label({ text: `${who} has a plan ready`, fontSize: 14, fontWeight: "semibold", color: "on_surface", maxLines: 1, flexGrow: 1 }),
    ]),
  ];
  if (p.detail && typeof p.detail === "object") children.push(Viewer.render(p.detail, { bodyLines: 14 }));
  if (p.answerable === false) {
    children.push(ui.label({ text: "Review and approve it in the terminal.", fontSize: 11, color: "on_surface_variant" }));
  } else {
    const data = { id: p.id };
    children.push(
      `<div class="btn-row">${btn("Approve", { variant: "primary", act: "plan-approve", data, tip: "Claude asks before each edit" })}${btn("Approve, auto-accept edits", { variant: "outline", act: "plan-approve-edits", data })}${btn("Keep planning", { variant: "outline", act: "deny", data, tip: "Then tell Claude what to change in the terminal" })}</div>`,
    );
    children.push(ui.label({ text: "You can also answer it in the terminal.", fontSize: 11, color: "on_surface_variant" }));
  }
  return ui.column({ gap: 8, padding: 10, fill: "primary/0.10", radius: 12 }, children);
}

function card(p) {
  let who = p.session_name || p.session_id;
  if (p.agent && Fmt.multiAgent(snap())) who = Fmt.agentLabel(snap(), p.agent) + " · " + who;
  if (p.kind === "question" && p.detail?.type === "questions") return questionCard(p, who);
  if (p.kind === "plan") return planCard(p, who);
  const what = p.tool_name || p.tool || "a tool";
  const children = [
    ui.row({ align: "center", gap: 8 }, [
      ui.glyph({ name: "alert-triangle", size: 18, color: "#f59e0b" }),
      ui.label({ text: `${who} wants to use ${what}`, fontSize: 14, fontWeight: "semibold", color: "on_surface", maxLines: 1, flexGrow: 1 }),
    ]),
  ];
  if (p.detail && typeof p.detail === "object") children.push(Viewer.render(p.detail, { maxRows: 9 }));
  else if (p.tool) children.push(ui.label({ text: p.tool, fontSize: 12, color: "on_surface_variant", maxLines: 3 }));
  children.push(
    `<div class="btn-row">${btn("Allow", { variant: "primary", act: "allow", data: { id: p.id } })}${btn("Deny", { variant: "outline", act: "deny", data: { id: p.id } })}</div>`,
  );
  return ui.column({ gap: 8, padding: 10, fill: "#f59e0b/0.10", radius: 12 }, children);
}


export function permissionCard(p) {
  const error = errors.get(p.id);
  return `<div data-request-id="${esc(p.id)}" aria-busy="${busy.has(p.id)}">${card(p)}${error ? `<div class="request-error" role="alert">${esc(error)}</div>` : ""}</div>`;
}
export function handleAction(d) {
  const actions = ["allow", "deny", "plan-approve", "plan-approve-edits", "choose", "send-answers"];
  if (!actions.includes(d.act)) return false;
  const id = /^\d+$/.test(String(d.id)) ? Number(d.id) : d.id;
  const p = snap().pending.find((p) => p.id === id);
  if (!p || p.answerable === false || busy.has(id)) return true;
  switch (d.act) {
    case "allow": case "plan-approve": void decide("approve", id); break;
    case "plan-approve-edits": void decide("approve-edits", id); break;
    case "deny": void decide("deny", id); break;
    case "choose": choose(p, Number(d.qi), d.label); break;
    case "send-answers": sendAnswers(p); break;
  }
  return true;
}
