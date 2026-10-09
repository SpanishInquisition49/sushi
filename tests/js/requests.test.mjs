import { readFile } from "node:fs/promises";
import { strict as assert } from "node:assert";
import { test } from "node:test";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import * as UI from "../../app/ui/js/ui.js";
import * as Fmt from "../../app/ui/js/fmt.js";
import * as Viewer from "../../app/ui/js/viewer.js";
const source = await readFile(new URL("../../app/ui/js/requests.js", import.meta.url), "utf8");
async function harness(pending, send = async () => {}) {
  const store = { snapshot: { pending, sessions: [], agents: {} } }, calls = [], changes = [];
  const context = createContext({ Date });
  const deps = {
    "./api.js": { invoke: async (cmd, args) => { calls.push({ cmd, ...args }); return send(cmd, args); }, emit: (cmd, args) => calls.push({ cmd, ...args }) },
    "./store.js": { store }, "./ui.js": UI, "./fmt.js": Fmt, "./viewer.js": Viewer,
  };
  const module = new SourceTextModule(source, { context });
  await module.link(name => { const ex = deps[name]; return new SyntheticModule(Object.keys(ex), function() {
    for (const [k,v] of Object.entries(ex)) this.setExport(k,v);
  }, { context }); });
  await module.evaluate();
  const api = module.namespace; api.subscribe(result => changes.push(result));
  const flush = async () => { for (let i=0; i<6; i++) await Promise.resolve(); };
  return { api, store, calls, changes, flush };
}
const permission = { id: 1, session_id: "a", session_name: "API", tool_name: "Bash" };
const question = (multi = false) => ({ id: 2, session_id: "a", kind: "question", detail: { type: "questions", questions: [
  { question: "Choose", multi, options: [{label:"A"}, {label:"B"}] },
] } });

test("approval waits for success, blocks duplicates and notifies both presentations", async () => {
  let finish;
  const h = await harness([permission], () => new Promise(resolve => { finish = resolve; }));
  const second = []; h.api.subscribe(r => second.push(r));
  h.api.handleAction({act:"allow",id:"1"}); h.api.handleAction({act:"deny",id:"1"});
  assert.equal(h.calls.length, 1); assert.equal(h.store.snapshot.pending.length, 1);
  assert.match(h.api.permissionCard(permission), / disabled/);
  finish(); await h.flush();
  assert.equal(h.store.snapshot.pending.length, 0);
  assert.equal(h.calls[0].action, "approve");
  assert.equal(h.changes.at(-1).settled, 1); assert.equal(second.at(-1).settled, 1);
});

test("failed submissions retain the request, show escaped errors and allow retry", async () => {
  let fail = true;
  const h = await harness([permission], async () => { if (fail) throw new Error("<failed>"); });
  h.api.handleAction({act:"deny",id:"1"}); await h.flush();
  assert.equal(h.store.snapshot.pending.length, 1);
  assert.match(h.api.permissionCard(permission), /role="alert".*&lt;failed&gt;/);
  assert.doesNotMatch(h.api.permissionCard(permission), / disabled/);
  fail=false; h.api.handleAction({act:"deny",id:"1"}); await h.flush();
  assert.equal(h.store.snapshot.pending.length, 0);
  assert.equal(h.calls[1].action, "deny");
});

test("single-choice questions send immediately and multiple-choice selections survive rerender", async () => {
  const h = await harness([question()]);
  h.api.handleAction({act:"choose",id:"2",qi:"0",label:"B"}); await h.flush();
  assert.equal(h.calls[0].cmd, "answer"); assert.equal(h.calls[0].answers.Choose, "B");
  const q=question(true), multi=await harness([q]);
  multi.api.handleAction({act:"choose",id:"2",qi:"0",label:"A"});
  assert.equal(multi.calls.length, 0); assert.match(multi.api.permissionCard(q), /✓ A/);
  multi.api.permissionCard(q); // A second view renders the same selection.
  multi.api.handleAction({act:"choose",id:"2",qi:"0",label:"B"});
  multi.api.handleAction({act:"send-answers",id:"2"}); await multi.flush();
  assert.equal(multi.calls[0].answers.Choose, "A, B");
});

test("all questions need answers and plans retain approve-edits and deny actions", async () => {
  const q=question(); q.detail.questions.push({question:"Second",options:[{label:"C"}]});
  const h=await harness([q]);
  h.api.handleAction({act:"choose",id:"2",qi:"0",label:"A"});
  h.api.handleAction({act:"send-answers",id:"2"}); assert.equal(h.calls.length,0);
  h.api.handleAction({act:"choose",id:"2",qi:"1",label:"C"});
  h.api.handleAction({act:"send-answers",id:"2"}); await h.flush();
  assert.equal(h.calls[0].answers.Second,"C");
  for (const [act, action] of [["plan-approve","approve"],["plan-approve-edits","approve-edits"],["deny","deny"]]) {
    const h=await harness([{...permission,kind:"plan"}]);
    h.api.handleAction({act,id:"1"}); await h.flush(); assert.equal(h.calls[0].action, action);
  }
});

test("unanswerable requests never invoke a response and keep terminal guidance", async () => {
  const p={...permission,kind:"plan",answerable:false}, h=await harness([p]);
  h.api.handleAction({act:"plan-approve",id:"1"}); assert.equal(h.calls.length,0);
  assert.match(h.api.permissionCard(p), /approve it in the terminal/);
});
