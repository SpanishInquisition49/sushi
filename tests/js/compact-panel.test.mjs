import { readFile } from "node:fs/promises";
import { strict as assert } from "node:assert";
import { test } from "node:test";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";
import * as UI from "../../app/ui/js/ui.js";
import * as Fmt from "../../app/ui/js/fmt.js";
import * as Characters from "../../app/ui/js/pet.js";
import * as Viewer from "../../app/ui/js/viewer.js";
const source=await readFile(new URL("../../app/ui/js/compact-panel.js",import.meta.url),"utf8");
async function harness() {
  const nodes = Object.fromEntries([".compact-title",".compact-content",".compact-pet"].map(k=>[k,{innerHTML:"",scrollTop:0}]));
  const events=new Map(), calls=[], fits=[], frames=[];
  const probeNodes = Object.fromEntries([".compact-title", ".compact-content"].map(k => [k, { innerHTML: "", scrollTop: 0 }]));
  let naturalHeight = 180, resized;
  const probe = { innerHTML: "", setAttribute() {}, querySelector: k => probeNodes[k],
    getBoundingClientRect: () => ({ height: naturalHeight }) };
  const root={innerHTML:"",querySelector:k=>nodes[k],appendChild() {},addEventListener:(k,f)=>events.set(k,f)};
  const store={up:true,settings:{character:"dango"},snapshot:{pending:[],agents:{codex:{label:"Codex",limits:{data:{five_hour:{percent:42}}}}},sessions:[{id:"s",name:"API <test>",agent:"codex",status:"working",context:{percent:30,window:200000},activity:{turn_started_ms:Date.now()-65000,files_changed:2,files:["a.rs","b.rs"],recent:[{tool:"Bash",label:"cargo test"}]}}]}};
  let refresh, requestRefresh;
  const deps={
    "./api.js":{invoke:(cmd,args)=>{ if (cmd === "fit_attention") fits.push(args.height); else calls.push(cmd); return Promise.resolve(); },listen(){}},
    "./store.js":{store,start:(state,settings)=>{refresh=state;settings();state();}},
    "./ui.js":UI,"./fmt.js":Fmt,"./pet.js":Characters,"./viewer.js":Viewer,
    "./requests.js":{permissionCard:p=>`<div>request ${p.id}</div>`,subscribe:f=>requestRefresh=f,handleAction:d=>calls.push(d.act)},
  };
  const context=createContext({Date,setInterval(){},console,
    document: { createElement: () => probe, fonts: { ready: Promise.resolve(), addEventListener() {} } },
    ResizeObserver: class { constructor(fn) { resized = fn; } observe() {} },
    requestAnimationFrame: fn => { frames.push(fn); return frames.length; },
  });
  const module=new SourceTextModule(source,{context});
  await module.link(name=>{const ex=deps[name];return new SyntheticModule(Object.keys(ex),function(){for(const[k,v]of Object.entries(ex))this.setExport(k,v);},{context});});
  await module.evaluate();module.namespace.mount(root);
  return {nodes,events,calls,store,refresh,requestRefresh,setMode:module.namespace.setMode,
    fits, probeNodes, setLayout: module.namespace.setLayout,
    measure: height => { naturalHeight = height; resized(); },
    flush: () => { const pending = frames.splice(0); pending.forEach(fn => fn()); },
  };
}

test("preview shows active session details, context and plan utilization with escaped labels",async()=>{
  const h=await harness(), html=h.nodes[".compact-content"].innerHTML;
  assert.match(html,/API &lt;test&gt;/);assert.match(html,/working/);assert.match(html,/Changes/);
  assert.match(html,/cargo test/);assert.match(html,/30%/);assert.match(html,/42%/);
});

test("attention lists requests and terminal waits once and preserves scrolling on updates",async()=>{
  const h=await harness();h.store.snapshot.pending=[{id:1,session_id:"s"}];
  h.store.snapshot.sessions[0].status="waiting";
  h.store.snapshot.sessions.push({id:"terminal",name:"Other",status:"waiting",agent:"codex"});
  h.setMode("attention");const body=h.nodes[".compact-content"];
  assert.match(body.innerHTML,/request 1/);assert.match(body.innerHTML,/Other/);
  assert.match(body.innerHTML,/Codex app or CLI/);assert.equal((body.innerHTML.match(/terminal-wait/g)||[]).length,1);
  body.scrollTop=75;h.store.snapshot.pending.push({id:2,session_id:"s"});h.requestRefresh();
  assert.equal(body.scrollTop,75);assert.match(body.innerHTML,/request 2/);
  h.setMode("preview");assert.doesNotMatch(body.innerHTML,/request 2/);
});

test("disconnected state explains the daemon and compact actions use the existing commands",async()=>{
  const h=await harness();h.store.up=false;h.refresh();assert.match(h.nodes[".compact-content"].innerHTML,/sushi daemon/);
  const click=act=>h.events.get("click")({target:{closest:()=>({dataset:{act}})}});
  click("compact-full");click("compact-close");click("allow");
  assert.deepEqual(h.calls,["open_panel","close_panel","allow"]);
});

test("Codex questions show escaped text and options with guidance to answer in Codex",async()=>{
  const h=await harness();
  const session=h.store.snapshot.sessions[0];
  session.status="waiting";
  session.attention={created_ms:10,detail:{type:"questions",questions:[
    {header:"Choice",question:"Proceed <now>?",options:[{label:"Yes <please>",description:"Keep & continue"}]},
    {question:"Explain",options:[]},
  ]}};
  h.setMode("attention");
  let html=h.nodes[".compact-content"].innerHTML;
  assert.match(html,/Proceed &lt;now&gt;\?/);
  assert.match(html,/Yes &lt;please&gt;/);
  assert.match(html,/Keep &amp; continue/);
  assert.match(html,/Explain/);
  assert.match(html,/Codex app or CLI/);
  assert.doesNotMatch(html,/data-act="(?:choose|allow|deny)"/);
  session.attention.detail={type:"text",title:"request_user_input",body:""};
  h.refresh();html=h.nodes[".compact-content"].innerHTML;
  assert.doesNotMatch(html,/Proceed|Yes &lt;please&gt;/);
  session.status="working";h.refresh();
  assert.doesNotMatch(h.nodes[".compact-content"].innerHTML,/waiting for you/);
});


test("attention measures while hidden, batches updates and ignores identical rounded heights", async () => {
  const h = await harness();
  h.setLayout({hasNotch: true, lateralAvailable: true});
  h.store.snapshot.pending = [{id: 1, session_id: "s"}];
  h.refresh(); h.requestRefresh(); h.measure(180.2); h.flush();
  assert.deepEqual(h.fits, [181]);
  assert.match(h.probeNodes[".compact-content"].innerHTML, /request 1/);
  assert.doesNotMatch(h.nodes[".compact-content"].innerHTML, /request 1/);
  h.measure(180.8); h.flush();
  assert.deepEqual(h.fits, [181]);
  h.setMode("attention"); h.measure(140); h.flush();
  assert.deepEqual(h.fits, [181, 140]);
});

test("long messages cap at 380 and removing requests shrinks without losing scroll", async () => {
  const h = await harness();
  h.setLayout({hasNotch: true, lateralAvailable: true}); h.setMode("attention");
  h.nodes[".compact-content"].scrollTop = 75;
  h.measure(900); h.flush(); h.measure(850); h.flush();
  assert.deepEqual(h.fits, [380]);
  h.requestRefresh(); h.measure(210); h.flush();
  assert.deepEqual(h.fits, [380, 210]);
  assert.equal(h.nodes[".compact-content"].scrollTop, 75);
});

test("non-notch layouts do not request adaptive sizing, including queued measurements", async () => {
  const h = await harness();
  h.measure(200); h.flush(); assert.deepEqual(h.fits, []);
  h.setLayout({hasNotch: true, lateralAvailable: true});
  h.setLayout({hasNotch: false}); h.flush();
  assert.deepEqual(h.fits, []);
});
