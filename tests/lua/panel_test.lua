-- Tests of plugin/panel.luau: the live view, permissions (Allow / Deny, questions, plans),
-- the chat tab, the gaze handlers and the pet's frame.
package.path = (arg[0]:match("^(.*)/") or ".") .. "/?.lua;" .. package.path
local H = require("helpers")
local find, texts, byKey, button = H.find, H.texts, H.byKey, H.button
local NOW = H.NOW

local function open(cfg, snap, daemonUp)
    local t = H.load("panel.luau", cfg)
    t.env.noctalia.state.set("daemonUp", daemonUp ~= false)
    t.env.noctalia.state.set("snapshot", snap)
    t.env.onOpen({})
    return t
end

local function snapshot(sessions, pending)
    return { sessions = sessions, pending = pending or {}, agents = H.AGENTS }
end

-- ── the live view ──────────────────────────────────────────────────────────────────────────
do
    local snap = snapshot({ H.session("a", "alpha", "idle", 10, {}), H.session("b", "beta", "working", 5, H.steps(NOW)) })
    local t = open({}, snap)
    local e = t.env
    local tx = texts(t.out())
    assert(tx:find("beta", 1, true), "the working session is the active one")
    assert(#byKey(t.out(), "step-") == 2, "the rail has the two steps")
    assert(tx:find("Edit", 1, true) and tx:find("running", 1, true), "the latest step is running")
    assert(not tx:find("0.2", 1, true), "typewriter: the added line is not there yet")
    e.NOW = NOW + 3000; e.onFrameTick(33)
    tx = texts(t.out())
    assert(tx:find("0.2", 1, true) and tx:find("TVA", 1, true), "after 3 s the whole added line is typed")

    byKey(t.out(), "step-")[1].p.onClick()
    tx = texts(t.out())
    assert(tx:find("types.ts", 1, true) and tx:find("read", 1, true) and tx:find("interface", 1, true), "a pinned step shows its file")
    byKey(t.out(), "step-")[1].p.onClick()
    assert(texts(t.out()):find("TVA", 1, true), "unpinned: back to the latest step")

    byKey(t.out(), "session-a")[1].p.onClick()
    assert(texts(t.out()):find("Idle: nothing running", 1, true), "a pinned idle session")
    byKey(t.out(), "session-a")[1].p.onClick()
    assert(texts(t.out()):find("Edit", 1, true), "unpinned: follows the active session again")

    button(t.out(), "Usage").p.onClick()
    tx = texts(t.out())
    assert(tx:find("Plan usage", 1, true) and tx:find("5-hour limit", 1, true) and tx:find("44%", 1, true) and tx:find("Context trend", 1, true))
    assert(t.state.panelTab.tab == "usage", "the tab is remembered")
    button(t.out(), "Live").p.onClick()
    assert(texts(t.out()):find("Edit", 1, true))
    print("view: active session, rail, typewriter, pinning steps and sessions, tabs")
end

-- ── permissions: Allow / Deny ──────────────────────────────────────────────────────────────
do
    local pending = { { id = 7, kind = "permission", session_id = "b", session_name = "beta", tool = "Edit: x", tool_name = "Edit", detail = H.DIFF, created_ms = NOW } }
    local snap = snapshot({ H.session("b", "beta", "waiting", 5, H.steps(NOW)) }, pending)
    local t = open({}, snap)
    local tx = texts(t.out())
    assert(tx:find("beta wants to use Edit", 1, true), tx)
    assert(button(t.out(), "Allow") and button(t.out(), "Deny"), "Allow and Deny")
    assert(tx:find("0.196", 1, true), "the card previews the change")
    button(t.out(), "Allow").p.onClick()
    assert(t.calls[1]:find("approve 7", 1, true), "Allow runs `approve 7`: " .. tostring(t.calls[1]))
    assert(t.calls[2] == "PANEL_CLOSE", "the panel closes after the last request")

    local t2 = open({ closeAfterDecision = false }, snap)
    button(t2.out(), "Deny").p.onClick()
    assert(t2.calls[1]:find("deny 7", 1, true) and t2.calls[2] == nil, "Deny runs `deny 7`; the panel stays open")

    local two = { pending[1], { id = 8, kind = "permission", session_id = "b", session_name = "beta", tool = "Bash: ls", tool_name = "Bash", detail = H.TERM, created_ms = NOW } }
    local t3 = open({}, snapshot(snap.sessions, two))
    button(t3.out(), "Allow").p.onClick()
    assert(t3.calls[2] == nil, "one request left: the panel stays open")
    assert(texts(t3.out()):find("npm test", 1, true), "the second request shows its command")

    -- a refused answer brings the card back with the next snapshot
    local t4 = open({}, snap)
    button(t4.out(), "Allow").p.onClick()
    t4.runs[1].cb({ exitCode = 1, stderr = "no pending request with id 7" })
    t4.env.noctalia.state.set("snapshot", snapshot(snap.sessions, pending))
    assert(button(t4.out(), "Allow"), "the card is back")
    print("permissions: preview, Allow, Deny, close-after-decision, several requests, refused answers")
end

-- ── questions: answered from the notch ─────────────────────────────────────────────────────
do
    local colour = { header = "Colour", question = "Which colour do you prefer?", multi = false,
        options = { { label = "Red", description = "warm" }, { label = "Blue", description = "cool" } } }
    local extras = { header = "Extras", question = "Which extras?", multi = true,
        options = { { label = "Sound", description = "" }, { label = "Hearts", description = "" }, { label = "Confetti", description = "" } } }
    local function qpending(...)
        return { { id = 3, kind = "question", session_id = "b", session_name = "beta", tool = "", tool_name = "AskUserQuestion",
            detail = { type = "questions", questions = { ... } }, created_ms = NOW } }
    end
    local sessions = { H.session("b", "beta", "waiting", 5, {}) }

    -- one single-choice question: one click answers it
    local t = open({}, snapshot(sessions, qpending(colour)))
    local tx = texts(t.out())
    assert(tx:find("beta is asking you", 1, true) and tx:find("Which colour do you prefer?", 1, true), tx)
    assert(button(t.out(), "Red") and button(t.out(), "Blue"), "the options are buttons")
    assert(not button(t.out(), "Allow") and not button(t.out(), "Deny"), "no Allow / Deny on a question")
    assert(not button(t.out(), "Send answers"), "no Send button for a single question")
    button(t.out(), "Blue").p.onClick()
    assert(t.calls[1]:find(" answer 3 ", 1, true), "runs `answer 3 <json>`: " .. tostring(t.calls[1]))
    assert(t.calls[1]:find('"Which colour do you prefer?":"Blue"', 1, true), "the answer is {question: label}: " .. t.calls[1])
    assert(t.calls[2] == "PANEL_CLOSE", "the panel closes afterwards")

    -- two questions, one of them multiple choice: needs Send, which waits until everything is answered
    local t2 = open({}, snapshot(sessions, qpending(colour, extras)))
    local send = button(t2.out(), "Send answers")
    assert(send and send.p.enabled == false, "Send is disabled at first")
    button(t2.out(), "Red").p.onClick()
    assert(#t2.calls == 0, "choosing does not send yet")
    assert(button(t2.out(), "Send answers").p.enabled == false, "one question still open")
    button(t2.out(), "Hearts").p.onClick()
    button(t2.out(), "Sound").p.onClick()
    assert(button(t2.out(), "Send answers").p.enabled == true, "all answered")
    assert(button(t2.out(), "✓ Hearts") and button(t2.out(), "✓ Sound"), "chosen options are marked")
    button(t2.out(), "✓ Hearts").p.onClick()           -- toggling off
    button(t2.out(), "Hearts").p.onClick()             -- and on again
    button(t2.out(), "Send answers").p.onClick()
    local call = t2.calls[1]
    assert(call:find(' answer 3 ', 1, true), call)
    assert(call:find('"Which colour do you prefer?":"Red"', 1, true), call)
    assert(call:find('"Which extras?":"Sound, Hearts"', 1, true), "multi-select labels in option order: " .. call)

    -- changing the single choice replaces it
    local t3 = open({}, snapshot(sessions, qpending(colour, extras)))
    button(t3.out(), "Red").p.onClick(); button(t3.out(), "Blue").p.onClick(); button(t3.out(), "Sound").p.onClick()
    button(t3.out(), "Send answers").p.onClick()
    assert(t3.calls[1]:find('"Which colour do you prefer?":"Blue"', 1, true) and not t3.calls[1]:find('"Red"', 1, true), t3.calls[1])
    print("questions: options as buttons, one-click answers, several questions, multi-select, Send, no Allow/Deny")
end

-- ── following the steps: each stays long enough to be seen, reads are skipped, the lag is capped ──
do
    local Fmt = H.load("panel.luau").env.require("./lib/fmt.luau")
    local function diff(ts) return { ts_ms = ts, detail = { type = "diff", lines = { { kind = "add", text = string.rep("x", 40) } } } } end
    local function read(ts) return { ts_ms = ts, detail = { type = "file", lines = {} } } end
    local function run(ts) return { ts_ms = ts, detail = { type = "terminal", command = "ls", output = {} } } end
    local st = { key = nil, since = 0 }
    assert(Fmt.followStep(st, "s", { diff(1) }, 0).ts_ms == 1, "the first step")
    local list = { diff(1), read(2), run(3) }
    assert(Fmt.followStep(st, "s", list, 500).ts_ms == 1, "an edit stays while it is typed out")
    assert(Fmt.followStep(st, "s", list, 2100).ts_ms == 3, "then the next command, skipping the read")
    assert(Fmt.followStep(st, "s", list, 2200).ts_ms == 3)
    local many = { diff(1), diff(2), diff(3), diff(4), diff(5), diff(6) }
    st = { key = nil, since = 0 }
    Fmt.followStep(st, "s", { diff(1) }, 0)
    assert(Fmt.followStep(st, "s", many, 2100).ts_ms == 3, "never more than three steps behind")
    assert(Fmt.followStep(st, "t", many, 2101).ts_ms == 6, "another session starts at its latest step")
    assert(Fmt.followStep(st, "t", {}, 2102) == nil and st.key == nil)
    print("follow: steps stay to be seen, reads skipped, lag capped")
end

-- ── plans: approved, approved with auto-accepted edits, or sent back; read-only once the hook gave up ──
do
    local function plan(answerable)
        return { { id = 5, kind = "plan", session_id = "b", session_name = "beta", tool = "", tool_name = "ExitPlanMode",
            detail = { type = "text", title = "Plan", body = "1. Do the thing\n2. Check it" }, created_ms = NOW, answerable = answerable } }
    end
    local sessions = { H.session("b", "beta", "waiting", 5, {}) }
    local t = open({}, snapshot(sessions, plan(true)))
    local tx = texts(t.out())
    assert(tx:find("beta has a plan ready", 1, true) and tx:find("Do the thing", 1, true), tx)
    assert(not button(t.out(), "Allow") and not button(t.out(), "Deny"), "a plan has no Allow / Deny")
    button(t.out(), "Approve").p.onClick()
    assert(t.calls[1]:find(" approve 5$"), t.calls[1])
    local t2 = open({}, snapshot(sessions, plan(true)))
    button(t2.out(), "Approve, auto-accept edits").p.onClick()
    assert(t2.calls[1]:find(" approve 5 %-%-accept%-edits$"), t2.calls[1])
    local t3 = open({}, snapshot(sessions, plan(true)))
    button(t3.out(), "Keep planning").p.onClick()
    assert(t3.calls[1]:find(" deny 5$"), t3.calls[1])

    local t4 = open({}, snapshot(sessions, plan(false)))
    assert(texts(t4.out()):find("approve it in the terminal", 1, true), "tells where to approve it")
    assert(not button(t4.out(), "Approve"), "nothing waits for an answer any more")
    print("plans: Approve, auto-accept edits, Keep planning; read-only once the hook gave up")
end

-- ── robustness ─────────────────────────────────────────────────────────────────────────────
do
    local t = H.load("panel.luau")
    t.env.noctalia.state.set("daemonUp", false); t.env.onOpen({})
    assert(texts(t.out()):find("Daemon is not running", 1, true))
    t.env.noctalia.state.set("daemonUp", true)
    t.env.noctalia.state.set("snapshot", { sessions = {}, pending = {} })
    assert(texts(t.out()):find("No agent session yet", 1, true), "empty state")
    -- a daemon from before this version: no activity, details, kinds or history
    t.env.noctalia.state.set("snapshot", { sessions = { { id = "x", name = "old", status = "working", last_event_ms = 1 } },
        pending = { { id = 1, session_id = "x", session_name = "old", tool = "Bash: ls" } } })
    assert(texts(t.out()):find("old", 1, true) and button(t.out(), "Allow"), "an old pending request is an ordinary permission")
    -- odd details never crash the view
    local odd = H.session("o", "odd", "working", 1, { { ts_ms = NOW, tool = "Task", kind = "think", label = "sub", ok = nil } })
    for _, d in ipairs({ { type = "text", title = "Subagent", body = "do things" }, { type = "mystery" }, {} }) do
        odd.activity.recent[1].detail = d
        t.env.noctalia.state.set("snapshot", { sessions = { odd }, pending = {} })
    end
    -- a new turn clears a pinned step
    local t4 = open({}, snapshot({ H.session("a", "alpha", "working", 5, H.steps(NOW)) }))
    byKey(t4.out(), "step-")[1].p.onClick()
    assert(texts(t4.out()):find("types.ts", 1, true))
    local b = H.session("a", "alpha", "working", 6, H.steps(NOW)); b.activity.turn_started_ms = 9999999
    t4.env.noctalia.state.set("snapshot", snapshot({ b }))
    assert(texts(t4.out()):find("TVA", 1, true), "a new turn: back to following the latest step")
    print("robustness: daemon off, empty, old daemon, odd details, new turn")
end

-- ── the chat tab ──────────────────────────────────────────────────────────────────────────
do
    local base = snapshot({ H.session("a", "alpha", "idle", 10, {}) })
    local t = open({}, base)
    local e = t.env
    local function set(chat) local s = {}; for k, v in pairs(base) do s[k] = v end; s.chat = chat; e.noctalia.state.set("snapshot", s) end
    set({ busy = false, messages = {} })
    button(t.out(), "Chat").p.onClick()
    assert(t.state.panelTab.tab == "chat")
    assert(texts(t.out()):find("Ask Claude Code anything", 1, true), "empty state")
    local input = find(t.out(), function(n) return n.k == "input" end)[1]
    assert(input and input.p.onSubmit == "onChatSubmit" and input.p.submitOnEnter == true, "text field, Enter sends")
    local function sendBtn() return find(t.out(), function(n) return n.k == "button" and n.p.glyph == "send" end)[1] end
    assert(sendBtn().p.enabled == false, "send is disabled with an empty field")
    e.onChatChange("hel")
    assert(sendBtn().p.enabled == true and sendBtn().p.variant == "primary")

    local keyBefore = find(t.out(), function(n) return n.k == "input" end)[1].p.key
    e.onChatSubmit("  how are you?  ")
    assert(t.calls[#t.calls]:find("chat how are you? --agent claude", 1, true), "argv call with the agent: " .. tostring(t.calls[#t.calls]))
    assert(not t.calls[#t.calls]:find("--model", 1, true), "no model chosen: the agent's default")
    assert(texts(t.out()):find("how are you?", 1, true), "provisional bubble")
    assert(find(t.out(), function(n) return n.k == "input" end)[1].p.key ~= keyBefore, "the field is recreated so it empties")
    local n = #t.calls; e.onChatSubmit("another"); assert(#t.calls == n, "ignored while a message is pending")
    set({ busy = true, messages = { { role = "user", text = "how are you?" }, { role = "assistant", text = "I am fi" } } })
    local tx = texts(t.out())
    assert(tx:find("I am fi", 1, true) and select(2, tx:gsub("how are you%?", "")) == 1, "no duplicate of the user message")
    assert(find(t.out(), function(n) return n.k == "button" and n.p.glyph == "player-stop" end)[1], "stop button while answering")
    e.onChatStop(); assert(t.calls[#t.calls]:find("chat-stop", 1, true))
    set({ busy = false, messages = { { role = "user", text = "how are you?" }, { role = "assistant", text = "I am fine!" } } })
    assert(texts(t.out()):find("I am fine!", 1, true))
    assert(find(t.out(), function(n) return n.k == "scroll" and n.p.stickToBottom == true end)[1], "the log stays at the bottom")
    set({ busy = false, error = "Not logged in", messages = {} })
    assert(texts(t.out()):find("Not logged in", 1, true))
    e.onChatClear(); assert(t.calls[#t.calls]:find("chat-clear", 1, true))
    local m = #t.calls; e.onChatSubmit("   "); e.onChatSubmit(nil); assert(#t.calls == m, "empty messages never reach the daemon")
    print("chat: tab, send, provisional bubble, streaming, stop, clear, errors")
end

-- ── the gaze ───────────────────────────────────────────────────────────────────────────────
do
    local pending = { { id = 7, kind = "permission", session_id = "b", session_name = "beta", tool_name = "Edit", detail = H.DIFF, created_ms = NOW } }
    local t = open({}, snapshot({ H.session("b", "beta", "working", 5, H.steps(NOW)) }, pending))
    local kinds = {}
    for _, n in ipairs(find(t.out(), function(n) return n.p.onHover == "onGaze" end)) do kinds[tostring(n.p.key):match("^(%a+)")] = true end
    for _, k in ipairs({ "perm", "tab", "close", "emote", "step", "session", "pet" }) do assert(kinds[k], "hover handler on " .. k) end
    local e = t.env
    local function frames() for _ = 1, 30 do e.NOW = e.NOW + 40; e.onFrameTick(33) end end
    e.onGaze("true", "perm-allow-7"); frames(); e.onGaze("false", "perm-allow-7"); frames()
    e.onGaze("true", "unknown-thing"); e.onGaze("true", nil); e.onGaze(nil, nil); frames()
    print("gaze: hover handlers everywhere that matters; unknown keys are harmless")
end

-- ── the status bubble must not make the pet jump ────────────────────────────────────────────
do
    local snap = snapshot({ H.session("b", "beta", "working", 5, H.steps(NOW)) })
    local t = open({}, snap)
    local e = t.env
    local function frameOf() return byKey(t.out(), "pet")[1].c[1].p end
    local function bubbles() return #find(t.out(), function(n) return n.p.fill == "#2f6fed" end) end
    local function settle() for _ = 1, 80 do e.NOW = e.NOW + 40; e.onFrameTick(33) end end
    settle(); settle()
    local base = frameOf()
    assert(bubbles() == 1, "a blue bubble while the session works")
    for _, kind in ipairs({ "love", "eat", "dance", "nap" }) do
        e.noctalia.state.set("petEvent", { kind = kind, ts = e.NOW + 1 }); e.NOW = e.NOW + 2
        for step = 1, 40 do
            e.NOW = e.NOW + 50; e.onFrameTick(33)
            local f = frameOf()
            assert(f.width == base.width and f.height == base.height, kind .. ": the frame changed size at step " .. step)
            assert(bubbles() == 1, kind .. ": the bubble disappeared at step " .. step)
        end
    end
    e.noctalia.state.set("snapshot", snapshot({ H.session("b", "beta", "idle", 5, {}) })); settle()
    assert(bubbles() == 0 and frameOf().width == base.width, "no bubble when idle, same room")
    print("pet frame: same size and bubble through emotes")
end

-- ── several agents ─────────────────────────────────────────────────────────────────────────
do
    local pi = H.session("pi:p", "pi-proj", "working", 20, H.steps(NOW))
    pi.agent, pi.context = "pi", nil
    pi.last_tool = H.tool("bash: ls", "run")
    local claude = H.session("claude:c", "claude-proj", "idle", 10, {})
    local pending = { { id = 1, agent = "pi", session_id = "pi:p", session_name = "pi-proj", tool_name = "bash", tool = "bash: ls", kind = "permission" } }
    local t = open({}, snapshot({ pi, claude }, pending))
    local tx = texts(t.out())
    assert(tx:find("Claude Code", 1, true) and tx:find("pi", 1, true), "each session says which agent it belongs to")
    assert(tx:find("pi · pi-proj wants to use bash", 1, true), "the request says who asks: " .. tx)
    assert(button(t.out(), "Chat"), "Claude Code has a chat")

    -- With only agents that cannot chat, the tab goes away.
    local agents = { opencode = H.AGENTS.opencode }
    local oc = H.session("oc:o", "oc-proj", "working", 20, H.steps(NOW))
    oc.agent, oc.context = "opencode", nil
    local only = { sessions = { oc }, pending = {}, agents = agents }
    local t2 = open({}, only)
    assert(not button(t2.out(), "Chat"), "no chat tab without a chat-capable agent")
    assert(not texts(t2.out()):find("Claude Code", 1, true), "no Claude wording for an opencode-only setup")
    print("agents: labels, who asks, chat tab only when some agent can chat")
end

-- ── the chat with another agent ────────────────────────────────────────────────────────────
do
    local base = snapshot({ H.session("a", "alpha", "idle", 10, {}) })
    local t = open({ chatAgent = "copilot", chatModel = "auto" }, base)
    button(t.out(), "Chat").p.onClick()
    assert(texts(t.out()):find("Ask GitHub Copilot anything", 1, true), "the chat names its agent")
    t.env.onChatSubmit("hello")
    local call = t.calls[#t.calls]
    assert(call:find("chat hello --agent copilot --model auto", 1, true), call)
    print("chat agent: named in the tab, passed to the daemon with its model")
end

-- ── a file fed to the pet ─────────────────────────────────────────────────────────────────
do
    local base = snapshot({ H.session("a", "alpha", "idle", 10, {}) })
    base.chat = { busy = false, messages = {} }
    local t = open({}, base)
    local e = t.env
    assert(not byKey(t.out(), "chat-attachment")[1], "no chip without a file")

    byKey(t.out(), "emote-feed")[1].p.onClick()
    t.runs[#t.runs].cb({ exitCode = 0, stdout = "file:///home/x/report%20v2.txt\n" })
    assert(t.state.fedFile.path == "/home/x/report v2.txt" and t.state.petEvent.kind == "eat", "a copied file is eaten")
    assert(byKey(t.out(), "chat-attachment")[1] and texts(t.out()):find("report v2.txt", 1, true), "the chat shows the file")
    local sendBtn = find(t.out(), function(n) return n.k == "button" and n.p.glyph == "send" end)[1]
    assert(sendBtn.p.enabled == true, "a file alone can be sent")
    e.onFeedDiscard()
    assert(not byKey(t.out(), "chat-attachment")[1] and t.state.fedFile.path == "", "the chip's x drops it")

    -- the Feed button reads the clipboard
    byKey(t.out(), "emote-feed")[1].p.onClick()
    t.runs[#t.runs].cb({ exitCode = 0, stdout = "file:///tmp/a.rs\n" })
    assert(t.state.fedFile.path == "/tmp/a.rs")
    e.onChatSubmit("")
    local call = t.calls[#t.calls]
    assert(call:find("chat --file /tmp/a.rs --agent claude", 1, true), "only the file: " .. call)
    assert(t.state.fedFile.path == "", "sent: the file is no longer attached")
    assert(texts(t.out()):find("a.rs", 1, true), "the provisional bubble names the file")
    e.noctalia.state.set("snapshot", (function() local s = {}; for k, v in pairs(base) do s[k] = v end
        s.chat = { busy = false, messages = { { role = "user", text = "what is it?", file = "a.rs" }, { role = "assistant", text = "A test." } } }; return s end)())
    assert(texts(t.out()):find("a.rs | what is it?", 1, true), "the log shows the file above the question")

    -- fed from the bar widget while the panel shows another tab
    local t2 = open({}, base)
    byKey(t2.out(), "tab-usage")[1].p.onClick()
    t2.env.noctalia.state.set("fedFile", { path = "/tmp/b.md", name = "b.md", ts = 1 })
    assert(byKey(t2.out(), "chat-attachment")[1], "switches to the chat")
    print("feed: feed button, chip, discard, send with a file, file in the log")
end

-- ── many steps must not push the sessions out of the panel ──────────────────────────────────
do
    local steps = {}
    for i = 1, 12 do steps[i] = { ts_ms = NOW - 20000 + i * 1000, tool = "Bash", kind = "run", label = "echo " .. i, ok = true, added = 0, removed = 0 } end
    local function sessions(n, status)
        local list = { H.session("a", "alpha", status, 20, steps, { finished_ms = NOW }) }
        for i = 2, n do list[i] = H.session("s" .. i, "other" .. i, "idle", i, {}) end
        return list
    end
    local function railRows(t) return #byKey(t.out(), "step-") end
    local function sessionScroll(t)
        return find(t.out(), function(n) return n.k == "scroll" and #byKey(n, "session-") > 0 end)[1]
    end
    for _, n in ipairs({ 1, 2, 4 }) do
        local t = open({}, snapshot(sessions(n, "idle")))
        local shown = math.min(n, 3)
        assert(railRows(t) == 6 - shown + 1, n .. " sessions: rail rows (Done included) " .. railRows(t))
        assert(byKey(t.out(), "step-done")[1], "Done is still there")
        assert(sessionScroll(t).p.minHeight == shown * 28, n .. " sessions: the list keeps room for " .. shown .. " rows")
        assert(#byKey(sessionScroll(t), "session-") == n, "every session is listed")
    end
    local t = open({}, snapshot(sessions(2, "working")))
    assert(railRows(t) == 5 and not byKey(t.out(), "step-done")[1], "a working turn uses the whole budget for steps")
    assert(texts(t.out()):find("echo 12", 1, true) and not texts(t.out()):find("echo 7 ", 1, true), "the latest steps are kept")
    print("rail: bounded by the number of sessions; the session list always has room")
end
