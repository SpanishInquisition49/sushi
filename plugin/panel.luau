--!nonstrict
-- panel.luau — the wide window attached to the notch.
--
--  left   the pet (with a status bubble), the session name and its clock, the steps
--         of the current turn (done ✓, running ◌, failed ✗, "Done") and the sessions
--  right  tabs Live / Chat / Usage, permission requests with Allow / Deny, and the live
--         viewer: the diff of the file being edited (typewriter effect, line numbers,
--         syntax colors), the terminal of a command, or the file that was read;
--         the Chat tab ("Ask <agent> anything") talks to the daemon's headless chat
--
-- Clicking a step shows that step in the viewer; clicking a session pins it (click it
-- again to follow the active session). Everything is drawn from the daemon's snapshot.

local Pet = require("./lib/pet.luau")
local Fmt = require("./lib/fmt.luau")
local Highlight = require("./lib/highlight.luau")
local Viewer = require("./lib/viewer.luau")
local Chat = require("./lib/chat.luau")
Pet.setUi(ui)
Viewer.init(ui, Highlight)
Chat.init(ui)

local HERO_SIZE = 92
local LEFT_W = 292
local FRAME_MS = 33          -- cap the redraw rate at ~30 fps
local TYPE_WINDOW_MS = 8000  -- only steps this recent get the typewriter effect
local MAX_RAIL = 7

local snapshot = { sessions = {}, pending = {}, usage = {} }
local daemonUp = false
local pet = Pet.new(noctalia.nowMs(), Pet.configFrom(noctalia.getConfig))
local lastDrawMs = 0

-- ── UI state (also kept in noctalia.state so it survives the panel being rebuilt) ──

local function recall(key, field)
    local v = noctalia.state.get(key)
    if type(v) == "table" and type(v[field]) == "string" and v[field] ~= "" then return v[field] end
    return nil
end

local function remember(key, field, value)
    noctalia.state.set(key, { [field] = value or "" })
end

local pinnedSession = recall("selectedSession", "sel") -- nil = follow the active session
local tab = recall("panelTab", "tab") or "live"
local pinnedStep = nil   -- ts_ms of a step chosen in the rail (nil = follow the latest)
local pinnedTurn = nil   -- the turn that step belongs to
local seenAt = {}        -- first time a step was drawn (drives the typewriter)
local viewCache = {}     -- finished viewer trees, so they are not rebuilt every frame
local viewCacheSize = 0
local chatInputKey = 0   -- bumped after each send: recreates (empties) the text field
local chatDraft = ""     -- what is typed so far (the send button needs it)
local optimistic = nil   -- {text, base}: a message just sent, shown until the daemon confirms it

--- Make the pet react here and in the bar widget (events are shared through noctalia.state).
local function sendEvent(kind)
    local ts = noctalia.nowMs()
    pet:onEvent(kind, ts, ts)
    noctalia.state.set("petEvent", { kind = kind, ts = ts })
end

local fmt = Fmt.tokens

local STATUS = {
    idle = { label = "idle", color = "on_surface_variant" },
    working = { label = "working", color = "primary" },
    waiting = { label = "waiting", color = "error" },
}

local function binPath()
    local p = noctalia.getConfig("binPath")
    if type(p) ~= "string" or p == "" then p = "~/.cargo/bin/sushi" end
    return noctalia.expandPath(p)
end

local function closeAfterDecision()
    local v = noctalia.getConfig("closeAfterDecision")
    return v ~= false and v ~= "false"
end

--- Settle a pending request: take its card away at once (the daemon's next snapshot confirms it,
--- and brings it back if the answer was refused), let the pet react, run the CLI `argv`, and
--- close the panel if nothing is left to answer.
local function settle(id, argv, eventKind)
    local kept = {}
    for _, p in ipairs(snapshot.pending or {}) do
        if p.id ~= id then kept[#kept + 1] = p end
    end
    local copy = {}
    for k, v in pairs(snapshot) do copy[k] = v end -- do not touch a table shared with the state
    copy.pending = kept
    snapshot = copy
    sendEvent(eventKind) -- love / sad, with their own sounds
    noctalia.runAsync(argv, function(res)
        if type(res) == "table" and res.exitCode ~= 0 then
            noctalia.log("sushi: " .. tostring(argv[2]) .. " failed: " .. tostring(res.stderr))
        end
    end)
    if #kept == 0 and closeAfterDecision() then
        panel.close() -- "one click, back to work"
    else
        render()
    end
end

local function decide(action, id)
    settle(id, { binPath(), action, tostring(id) }, (action == "approve") and "approve" or "deny")
end

-- ── Gaze ──────────────────────────────────────────────────────────────────────
-- Noctalia gives no global pointer position, so the eyes react to what the pointer is
-- over: onHover(state, key) fires on enter ("true") and leave ("false"). The first word
-- of the key says what it is; the pet sits at the top left, so the targets are
-- relative to that (x: -1 left … 1 right, y: -1 up … 1 down).

local GAZE = {
    perm = { 1.0, 0.3 }, tab = { 0.9, -0.3 }, close = { 1.0, -0.7 },
    emote = { 0.0, 0.9 }, step = { 0.0, 0.9 }, session = { 0.0, 1.0 }, pet = { 0.0, 0.0 },
}

function onGaze(state, key)
    local kind = tostring(key or ""):match("^(%a+)")
    local target = kind and GAZE[kind]
    if tostring(state) == "true" and target then
        pet:lookAt(target[1], target[2])
    else
        pet:lookAt(nil)
    end
end

-- ── Selection ─────────────────────────────────────────────────────────────────

local function currentSession()
    if pinnedSession then
        for _, s in ipairs(snapshot.sessions or {}) do
            if s.id == pinnedSession then return s end
        end
    end
    return Fmt.activeSession(snapshot)
end

local function stepsOf(s) return (s and s.activity and s.activity.recent) or {} end

--- The step shown in the viewer, and whether it is the latest one (follows the session).
local function currentStep(s)
    local list = stepsOf(s)
    if pinnedStep then
        local turn = s.activity and s.activity.turn_started_ms
        if pinnedTurn == turn then
            for _, e in ipairs(list) do
                if e.ts_ms == pinnedStep then return e, false end
            end
        end
        pinnedStep = nil -- a new turn started, or the step scrolled out of the list
    end
    return list[#list], true
end

local function setTab(id)
    tab = id
    remember("panelTab", "tab", id)
    render()
end

local function pinSession(s)
    if pinnedSession == s.id then pinnedSession = nil else pinnedSession = s.id end
    pinnedStep = nil
    remember("selectedSession", "sel", pinnedSession)
    render()
end

local function pinStep(s, e)
    if pinnedStep == e.ts_ms then
        pinnedStep = nil
    else
        pinnedStep, pinnedTurn = e.ts_ms, s.activity and s.activity.turn_started_ms
        if tab ~= "live" then setTab("live"); return end
    end
    render()
end

-- ── Left column ───────────────────────────────────────────────────────────────

local SPINNER = { "loader", "loader-2", "loader-3", "loader-quarter" }

local function action(glyph, tip, kind)
    return ui.button({
        key = "emote-" .. kind, glyph = glyph, variant = "ghost", controlSize = "sm", tooltip = tip,
        onClick = function() sendEvent(kind) end, onHover = "onGaze",
    })
end

local function counts()
    local working = 0
    for _, s in ipairs(snapshot.sessions or {}) do
        if s.status == "working" then working = working + 1 end
    end
    return working, #(snapshot.pending or {})
end

local function petBlock(nWorking, nPending)
    -- The bubble follows what the sessions are doing, not the pet's passing emote, so
    -- clicking an animation does not make it vanish and reappear.
    local badge
    if nPending > 0 then
        badge = "alert"
    elseif nWorking > 0 then
        badge = "work"
    end
    return ui.row({ key = "pet", onHover = "onGaze", onClick = function() pet:poke(noctalia.nowMs()) end, tooltip = "Poke it" }, {
        pet:build(HERO_SIZE, { room = 20, maxHeight = 124, margin = 20, badge = badge, badgeSlot = true }),
    })
end

local function titleBlock(s, nWorking, nPending, now)
    local sub = s and Fmt.agentLabel(snapshot, s.agent) or "Coding agents"
    if s then
        local elapsed = Fmt.turnElapsed(s, now)
        if elapsed then sub = sub .. ((s.status == "idle") and " · last turn " or " · ") .. Fmt.duration(elapsed) end
    end
    return ui.column({ gap = 2 }, {
        ui.label({ text = s and s.name or "No session", fontSize = 20, fontWeight = "bold", color = "on_surface", maxLines = 1 }),
        ui.label({ text = sub, fontSize = 12, color = "on_surface_variant", maxLines = 1 }),
        ui.label({ text = pet:caption(#(snapshot.sessions or {}), nWorking, nPending), fontSize = 12, color = "primary", maxLines = 2 }),
    })
end

local function railRow(s, e, now)
    local running = (e.ok == nil)
    local icon
    if e.ok == true then
        icon = ui.glyph({ name = "circle-check-filled", size = 18, color = Viewer.COLORS.ok })
    elseif e.ok == false then
        icon = ui.glyph({ name = "circle-x-filled", size = 18, color = Viewer.COLORS.fail })
    else
        icon = ui.glyph({ name = SPINNER[math.floor(now / 120) % #SPINNER + 1], size = 18, color = "on_surface" })
    end
    return ui.row({
        key = "step-" .. tostring(e.ts_ms), align = "center", gap = 10, paddingH = 8, paddingV = 5, radius = 8,
        fill = (pinnedStep == e.ts_ms) and "surface_variant/0.55" or nil,
        onClick = function() pinStep(s, e) end, onHover = "onGaze",
        tooltip = e.tool .. " " .. (e.label or ""),
    }, {
        icon,
        ui.label({
            text = e.tool, fontSize = 15, fontWeight = running and "bold" or "semibold",
            color = running and "on_surface" or "on_surface_variant",
        }),
        ui.label({ text = e.label or "", fontSize = 11, color = "on_surface_variant", maxLines = 1, flexGrow = 1 }),
    })
end

local function rail(s, now)
    local rows = {}
    if s then
        local list = stepsOf(s)
        for i = math.max(1, #list - MAX_RAIL + 1), #list do rows[#rows + 1] = railRow(s, list[i], now) end
        local a = s.activity or {}
        if s.status == "idle" and a.finished_ms then
            rows[#rows + 1] = ui.row({ key = "step-done", align = "center", gap = 10, paddingH = 8, paddingV = 5 }, {
                ui.glyph({ name = "circle-check-filled", size = 18, color = "on_surface_variant", opacity = 0.6 }),
                ui.label({ text = "Done", fontSize = 15, fontWeight = "semibold", color = "on_surface_variant", opacity = 0.7 }),
            })
        end
    end
    if #rows == 0 then
        rows[1] = ui.label({ text = "No steps yet.", fontSize = 12, color = "on_surface_variant" })
    end
    return ui.column({ gap = 1 }, rows)
end

local function sessionList(current)
    local list = {}
    for _, s in ipairs(snapshot.sessions or {}) do list[#list + 1] = s end
    local function rank(s) return (s.status == "working" or s.status == "waiting") and 1 or 0 end
    table.sort(list, function(a, b)
        if rank(a) ~= rank(b) then return rank(a) > rank(b) end
        if (a.last_event_ms or 0) ~= (b.last_event_ms or 0) then return (a.last_event_ms or 0) > (b.last_event_ms or 0) end
        return tostring(a.name) < tostring(b.name)
    end)
    local rows = {}
    local multi = Fmt.multiAgent(snapshot)
    for _, s in ipairs(list) do
        local st = STATUS[s.status] or STATUS.idle
        local pinned = (pinnedSession == s.id)
        rows[#rows + 1] = ui.row({
            key = "session-" .. tostring(s.id), align = "center", gap = 8, paddingH = 8, paddingV = 5, radius = 8,
            fill = (s == current) and "surface_variant/0.55" or nil,
            onClick = function() pinSession(s) end, onHover = "onGaze",
            tooltip = pinned and "Pinned: click to follow the active session" or ("Show " .. (s.name or s.id)),
        }, {
            ui.box({ width = 8, height = 8, radius = 4, fill = st.color }),
            ui.label({ text = s.name or s.id, fontSize = 12, fontWeight = "semibold", color = "on_surface", maxLines = 1, flexGrow = 1 }),
            ui.label({ text = multi and Fmt.agentLabel(snapshot, s.agent) or "", fontSize = 10, color = "on_surface_variant", maxLines = 1 }),
            ui.label({
                text = s.context and (Fmt.round(s.context.percent) .. "%") or "",
                fontSize = 11, color = s.context and Fmt.percentColor(s.context.percent) or "on_surface_variant",
            }),
        })
    end
    if #rows == 0 then
        rows[1] = ui.label({ text = "No active sessions", fontSize = 12, color = "on_surface_variant" })
    end
    return ui.scroll({ flexGrow = 1, gap = 2 }, rows)
end

local function leftColumn(s, now)
    local nWorking, nPending = counts()
    return ui.column({ width = LEFT_W, gap = 10, padding = 14 }, {
        petBlock(nWorking, nPending),
        titleBlock(s, nWorking, nPending, now),
        ui.row({ gap = 2 }, {
            action("heart", "Pet it", "love"),
            action("cookie", "Feed it", "eat"),
            action("music", "Dance!", "dance"),
            action("moon", "Nap time", "nap"),
        }),
        ui.separator({ spacing = 2, color = "outline", opacity = 0.2 }),
        rail(s, now),
        ui.separator({ spacing = 2, color = "outline", opacity = 0.2 }),
        ui.label({ text = "Sessions", fontSize = 11, color = "on_surface_variant" }),
        sessionList(s),
    })
end

-- ── Right column: permissions ─────────────────────────────────────────────────

-- Questions the agent asks (Claude Code's AskUserQuestion). The options are buttons; the answers
-- go back to the agent through the hook. Anything else (free text) is answered in the terminal, where the
-- same question is also on screen.

local choices = {} -- [pending id] = { [question index] = { [label] = true } }

local function picked(id, qi)
    return (choices[id] and choices[id][qi]) or {}
end

local function questionsAnswers(p)
    local answers = {}
    for qi, q in ipairs((p.detail and p.detail.questions) or {}) do
        local chosen = picked(p.id, qi)
        local labels = {}
        for _, o in ipairs(q.options or {}) do
            if chosen[o.label] then labels[#labels + 1] = o.label end
        end
        if #labels == 0 then return nil end -- not answered yet
        answers[q.question] = table.concat(labels, ", ")
    end
    return answers
end

local function sendAnswers(p)
    local answers = questionsAnswers(p)
    if not answers then return end
    choices[p.id] = nil
    settle(p.id, { binPath(), "answer", tostring(p.id), noctalia.json.encode(answers) }, "approve")
end

local function choose(p, qi, q, label)
    local c = choices[p.id] or {}
    choices[p.id] = c
    if q.multi then
        local set = c[qi] or {}
        set[label] = (not set[label]) or nil
        c[qi] = set
    else
        c[qi] = { [label] = true }
    end
    local list = (p.detail and p.detail.questions) or {}
    if #list == 1 and not q.multi then
        sendAnswers(p) -- one question, one choice: nothing more to ask
    else
        render()
    end
end

local function questionCard(p, who)
    local list = (p.detail and p.detail.questions) or {}
    local children = {
        ui.row({ align = "center", gap = 8 }, {
            ui.glyph({ name = "help-circle", size = 18, color = "primary" }),
            ui.label({ text = who .. " is asking you", fontSize = 14, fontWeight = "semibold", color = "on_surface", maxLines = 1, flexGrow = 1 }),
        }),
    }
    for qi, q in ipairs(list) do
        local chosen = picked(p.id, qi)
        local block = {}
        if (q.header or "") ~= "" then
            block[#block + 1] = ui.label({ text = q.header, fontSize = 10, fontWeight = "bold", color = "primary" })
        end
        block[#block + 1] = ui.label({ text = q.question, fontSize = 14, fontWeight = "semibold", color = "on_surface", maxLines = 4, maxWidth = 520 })
        if q.multi then
            block[#block + 1] = ui.label({ text = "Choose one or more", fontSize = 11, color = "on_surface_variant" })
        end
        for oi, o in ipairs(q.options or {}) do
            local on = chosen[o.label] == true
            block[#block + 1] = ui.row({ key = "q-" .. p.id .. "-" .. qi .. "-" .. oi, align = "center", gap = 10 }, {
                ui.button({
                    text = (on and q.multi) and ("✓ " .. o.label) or o.label, variant = on and "primary" or "outline",
                    onHover = "onGaze", tooltip = o.description ~= "" and o.description or nil,
                    onClick = function() choose(p, qi, q, o.label) end,
                }),
                ui.label({ text = o.description or "", fontSize = 11, color = "on_surface_variant", maxLines = 2, maxWidth = 360, flexGrow = 1 }),
            })
        end
        children[#children + 1] = ui.column({ gap = 6 }, block)
    end
    -- Several questions (or a multiple choice) need an explicit send.
    if not (#list == 1 and not list[1].multi) then
        children[#children + 1] = ui.row({ gap = 8, align = "center" }, {
            ui.button({
                key = "q-send-" .. p.id, text = "Send answers", variant = "primary", onHover = "onGaze",
                enabled = questionsAnswers(p) ~= nil, onClick = function() sendAnswers(p) end,
            }),
        })
    end
    children[#children + 1] = ui.label({ text = "You can also answer it in the terminal.", fontSize = 11, color = "on_surface_variant" })
    return ui.column({ key = "perm-" .. tostring(p.id), gap = 10, padding = 10, fill = "primary/0.10", radius = 12 }, children)
end

-- A plan the agent wants approved (Claude Code's ExitPlanMode): that dialog only exists in the terminal, so it is
-- shown here for reading and goes away once it has been dealt with there.
local function planCard(p, who)
    local children = {
        ui.row({ align = "center", gap = 8 }, {
            ui.glyph({ name = "list-check", size = 18, color = "primary" }),
            ui.label({ text = who .. " has a plan ready", fontSize = 14, fontWeight = "semibold", color = "on_surface", maxLines = 1, flexGrow = 1 }),
        }),
    }
    if type(p.detail) == "table" then
        children[#children + 1] = Viewer.render(p.detail, { bodyLines = 14, ok = nil })
    end
    children[#children + 1] = ui.label({ text = "Review and approve it in the terminal.", fontSize = 11, color = "on_surface_variant" })
    return ui.column({ key = "perm-" .. tostring(p.id), gap = 8, padding = 10, fill = "primary/0.10", radius = 12 }, children)
end

local function permissionCard(p)
    local who = (p.session_name ~= "" and p.session_name) or p.session_id
    if p.agent and Fmt.multiAgent(snapshot) then who = Fmt.agentLabel(snapshot, p.agent) .. " · " .. who end
    if p.kind == "question" and type(p.detail) == "table" and p.detail.type == "questions" then
        return questionCard(p, who)
    elseif p.kind == "plan" then
        return planCard(p, who)
    end
    local what = (p.tool_name ~= nil and p.tool_name ~= "") and p.tool_name or (p.tool ~= "" and p.tool or "a tool")
    local children = {
        ui.row({ align = "center", gap = 8 }, {
            ui.glyph({ name = "alert-triangle", size = 18, color = "#f59e0b" }),
            ui.label({ text = who .. " wants to use " .. what, fontSize = 14, fontWeight = "semibold", color = "on_surface", maxLines = 1, flexGrow = 1 }),
        }),
    }
    if type(p.detail) == "table" then
        children[#children + 1] = Viewer.render(p.detail, { maxRows = 9, ok = nil })
    elseif p.tool ~= "" then
        children[#children + 1] = ui.label({ text = p.tool, fontSize = 12, color = "on_surface_variant", maxLines = 3 })
    end
    children[#children + 1] = ui.row({ gap = 8 }, {
        ui.button({ key = "perm-allow-" .. tostring(p.id), text = "Allow", variant = "primary", onHover = "onGaze", onClick = function() decide("approve", p.id) end }),
        ui.button({ key = "perm-deny-" .. tostring(p.id), text = "Deny", variant = "outline", onHover = "onGaze", onClick = function() decide("deny", p.id) end }),
    })
    return ui.column({ key = "perm-" .. tostring(p.id), gap = 8, padding = 10, fill = "#f59e0b/0.10", radius = 12 }, children)
end

-- ── Right column: Live ────────────────────────────────────────────────────────

local function stepLine(e)
    local right = {}
    if (e.added or 0) > 0 or (e.removed or 0) > 0 then
        right[#right + 1] = ui.label({ text = string.format("+%d -%d", e.added or 0, e.removed or 0), fontSize = 11, color = "on_surface_variant" })
    end
    local status = (e.ok == true) and "done" or ((e.ok == false) and "failed" or "running")
    right[#right + 1] = ui.label({
        text = status, fontSize = 11, fontWeight = "semibold",
        color = (e.ok == false) and Viewer.COLORS.fail or ((e.ok == true) and Viewer.COLORS.ok or "primary"),
    })
    return ui.row({ align = "center", gap = 8 }, {
        ui.label({ text = e.tool .. "  " .. (e.label or ""), fontSize = 12, color = "on_surface_variant", maxLines = 1, flexGrow = 1 }),
        ui.row({ gap = 8, align = "center" }, right),
    })
end

--- The viewer for a step. The latest step of a working session is typed out; everything
--- else is built once and cached.
local function viewerFor(s, e, now, following)
    local detail = e.detail
    local opts = { ok = e.ok, maxRows = 22 }
    if following and type(detail) == "table" and detail.type == "diff" and (now - e.ts_ms) < TYPE_WINDOW_MS then
        local key = tostring(s.id) .. ":" .. tostring(e.ts_ms)
        local first = seenAt[key] or now
        seenAt[key] = first
        local total = Viewer.addedChars(detail)
        local speed = math.max(1 / 22, total / 1500) -- chars per ms; never slower than 22 ms/char, done in 1.5 s
        local n = math.floor((now - first) * speed)
        if n < total or e.ok == nil then
            opts.typed = math.min(n, total)
            opts.cursor = (math.floor(now / 450) % 2 == 0)
        end
    end
    if opts.typed ~= nil then return Viewer.render(detail, opts) end

    local size = type(detail) == "table" and #(detail.lines or detail.output or {}) or 0
    local key = table.concat({ tostring(s.id), tostring(e.ts_ms), tostring(e.ok), type(detail) == "table" and detail.type or "-", size }, ":")
    local hit = viewCache[key]
    if hit then return hit end
    if viewCacheSize > 60 then viewCache, viewCacheSize = {}, 0 end
    local tree = Viewer.render(detail, opts)
    viewCache[key] = tree
    viewCacheSize = viewCacheSize + 1
    return tree
end

local function resultCard(text)
    return ui.column({ gap = 4, padding = 12, fill = "surface_variant/0.35", radius = 10 }, {
        ui.label({ text = "Result", fontSize = 11, color = "on_surface_variant" }),
        ui.label({ text = text, fontSize = 13, color = "on_surface", maxLines = 8 }),
    })
end

local function liveContent(s, now)
    local nodes = {}
    if not s then
        nodes[1] = Viewer.render(nil, { empty = "No agent session yet. Start Claude Code, Codex, opencode or pi in a terminal and it shows up here." })
    else
        local e, following = currentStep(s)
        if e then
            nodes[#nodes + 1] = stepLine(e)
            nodes[#nodes + 1] = viewerFor(s, e, now, following)
        else
            nodes[#nodes + 1] = Viewer.render(nil, { empty = (s.status == "idle") and "Idle: nothing running." or "Waiting for the first step of this turn…" })
        end
        local a = s.activity or {}
        if a.last_result and s.status == "idle" then nodes[#nodes + 1] = resultCard(a.last_result) end
    end
    return ui.scroll({ flexGrow = 1, gap = 10 }, nodes)
end

-- ── Right column: Chat ────────────────────────────────────────────────────────

--- The model to ask for: empty means the agent's own default.
local function chatModel()
    local m = noctalia.getConfig("chatModel")
    if type(m) ~= "string" then return "" end
    return m
end

--- The agent the chat talks to (`claude`, `copilot`, `pi`, `codex`).
local function chatAgent()
    local a = noctalia.getConfig("chatAgent")
    if type(a) ~= "string" or a == "" then return "claude" end
    return a
end

local function chatContent()
    local chat = snapshot.chat or {}
    -- The daemon's state caught up with what we sent: drop the provisional message.
    if optimistic and #(chat.messages or {}) > optimistic.base then optimistic = nil end
    return Chat.view(chat, {
        optimistic = optimistic and optimistic.text or nil,
        label = Fmt.agentLabel(snapshot, chat.agent or chatAgent()),
        inputKey = chatInputKey,
        draftReady = chatDraft:match("%S") ~= nil,
    })
end

function onChatChange(text)
    local ready = (tostring(text or ""):match("%S") ~= nil)
    local was = (chatDraft:match("%S") ~= nil)
    chatDraft = tostring(text or "")
    if ready ~= was then render() end
end

function onChatSubmit(text)
    text = tostring(text or ""):match("^%s*(.-)%s*$")
    local chat = snapshot.chat or {}
    if text == "" or chat.busy or optimistic then return end
    optimistic = { text = text, base = #(chat.messages or {}) }
    chatInputKey = chatInputKey + 1
    chatDraft = ""
    local argv = { binPath(), "chat", text, "--agent", chatAgent() }
    if chatModel() ~= "" then
        argv[#argv + 1] = "--model"
        argv[#argv + 1] = chatModel()
    end
    noctalia.runAsync(argv, function(res)
        if type(res) == "table" and res.exitCode ~= 0 then
            optimistic = nil
            noctalia.log("sushi: chat failed: " .. tostring(res.stderr))
        end
    end)
    render()
end

function onChatSend() onChatSubmit(chatDraft) end

function onChatStop()
    noctalia.runAsync({ binPath(), "chat-stop" }, function() end)
end

function onChatClear()
    optimistic = nil
    chatInputKey = chatInputKey + 1
    chatDraft = ""
    noctalia.runAsync({ binPath(), "chat-clear" }, function() end)
    render()
end

-- ── Right column: Usage ───────────────────────────────────────────────────────

local function stat(label, value, color)
    return ui.column({ gap = 1 }, {
        ui.label({ text = label, fontSize = 10, color = "on_surface_variant" }),
        ui.label({ text = value, fontSize = 13, fontWeight = "semibold", color = color or "on_surface" }),
    })
end

local function limitBar(key, label, w, now)
    return ui.column({ key = key, gap = 3 }, {
        ui.row({ align = "center", justify = "space_between" }, {
            ui.label({ text = label, fontSize = 12, color = "on_surface" }),
            ui.label({
                text = string.format("%d%% · resets in %s", Fmt.round(w.percent), Fmt.resetIn(w.resets_at_ms, now)),
                fontSize = 11, color = "on_surface_variant",
            }),
        }),
        ui.progress({
            progress = math.max(0, math.min(1, (tonumber(w.percent) or 0) / 100)),
            fill = Fmt.percentColor(w.percent), track = "outline/0.25", height = 6, radius = 3, flexGrow = 1,
        }),
    })
end

local function limitsSection(now)
    local lim = Fmt.limits(snapshot) or {}
    local data = lim.data
    if not data then
        return ui.label({
            text = "Plan limits unavailable" .. (lim.error and (": " .. tostring(lim.error)) or ""),
            fontSize = 11, color = "on_surface_variant", maxLines = 2,
        })
    end
    local bars = {}
    if data.five_hour then bars[#bars + 1] = limitBar("l5h", "5-hour limit", data.five_hour, now) end
    if data.seven_day then bars[#bars + 1] = limitBar("l7d", "Weekly limit", data.seven_day, now) end
    for i, m in ipairs(data.models or {}) do
        bars[#bars + 1] = limitBar("lm" .. i, m.model .. " weekly", m, now)
    end
    return ui.column({ gap = 8 }, bars)
end

local function contextTrend(s)
    local ctx = s and s.context
    if not (ctx and type(ctx.history) == "table" and #ctx.history >= 2) then return nil end
    local values = {}
    for i, p in ipairs(ctx.history) do values[i] = math.max(0, math.min(1, (tonumber(p) or 0) / 100)) end
    return ui.column({ gap = 3 }, {
        ui.row({ align = "center", justify = "space_between" }, {
            ui.label({ text = "Context trend · " .. (s.name or ""), fontSize = 11, color = "on_surface_variant" }),
            ui.label({ text = string.format("%d%% of ~%s", Fmt.round(ctx.percent), fmt(ctx.window)), fontSize = 11, color = Fmt.percentColor(ctx.percent) }),
        }),
        ui.graph({ values = values, color = Fmt.percentColor(ctx.percent), lineWidth = 2, fillOpacity = 0.18, height = 48, flexGrow = 1 }),
    })
end

local function usageContent(s, now)
    local u = (snapshot.usage or {}).claude or {}
    local today, total = u.today or {}, u.total or {}
    local a = (s and s.activity) or {}
    local nodes = {
        ui.label({ text = "Plan usage", fontSize = 12, fontWeight = "semibold", color = "on_surface" }),
        limitsSection(now),
    }
    if s then
        nodes[#nodes + 1] = ui.separator({ spacing = 4, color = "outline", opacity = 0.2 })
        nodes[#nodes + 1] = ui.label({ text = "This turn · " .. (s.name or ""), fontSize = 12, fontWeight = "semibold", color = "on_surface" })
        nodes[#nodes + 1] = ui.row({ gap = 20 }, {
            stat("Tools", tostring(a.tool_calls or 0)),
            stat("Files", (a.files_changed or 0) > 0 and (a.files_changed .. string.format("  +%d -%d", a.lines_added or 0, a.lines_removed or 0)) or "0"),
            stat("Commands", Fmt.commandsText(a) or "0", (a.failures or 0) > 0 and "error" or nil),
        })
        local trend = contextTrend(s)
        if trend then nodes[#nodes + 1] = trend end
    end
    nodes[#nodes + 1] = ui.separator({ spacing = 4, color = "outline", opacity = 0.2 })
    nodes[#nodes + 1] = ui.label({
        text = string.format("Today: %s out · %s in · %s cache", fmt(today.output), fmt(today.input), fmt(today.cache_read)),
        fontSize = 12, color = "on_surface",
    })
    nodes[#nodes + 1] = ui.label({
        text = string.format("Total (recent transcripts): %s out · %s in", fmt(total.output), fmt(total.input)),
        fontSize = 11, color = "on_surface_variant",
    })
    return ui.scroll({ flexGrow = 1, gap = 10 }, nodes)
end

-- ── Assembly ──────────────────────────────────────────────────────────────────

local function tabButton(id, label)
    return ui.button({
        key = "tab-" .. id, text = label, variant = (tab == id) and "primary" or "ghost", controlSize = "sm",
        onClick = function() setTab(id) end, onHover = "onGaze",
    })
end

local function rightColumn(s, now)
    local children = {
        ui.row({ align = "center", gap = 6 }, {
            tabButton("live", "Live"),
            Fmt.canChat(snapshot) and tabButton("chat", "Chat") or ui.spacer({}),
            tabButton("usage", "Usage"),
            ui.spacer({ flexGrow = 1 }),
            ui.button({ key = "close", glyph = "x", variant = "ghost", onClick = "onClose", onHover = "onGaze" }),
        }),
    }
    if not daemonUp then
        children[#children + 1] = ui.label({ text = "Daemon is not running.", fontSize = 14, fontWeight = "semibold", color = "error" })
        children[#children + 1] = ui.label({ text = "Start `sushi daemon` to see your sessions.", fontSize = 12, color = "on_surface_variant" })
        return ui.column({ flexGrow = 1, gap = 10, padding = 14 }, children)
    end
    for _, p in ipairs(snapshot.pending or {}) do children[#children + 1] = permissionCard(p) end
    if tab == "usage" then
        children[#children + 1] = usageContent(s, now)
    elseif tab == "chat" and Fmt.canChat(snapshot) then
        children[#children + 1] = chatContent()
    else
        children[#children + 1] = liveContent(s, now)
    end
    return ui.column({ flexGrow = 1, gap = 10, padding = 14 }, children)
end

function render()
    local now = noctalia.nowMs()
    local s = currentSession()
    panel.render(ui.row({ flexGrow = 1, gap = 0 }, {
        leftColumn(s, now),
        ui.separator({ orientation = "vertical", color = "outline", opacity = 0.2 }),
        rightColumn(s, now),
    }))
end

-- ── State ─────────────────────────────────────────────────────────────────────

local function dataChanged()
    pet:onSnapshot(snapshot, daemonUp, noctalia.nowMs())
    render()
end

noctalia.state.watch("snapshot", function(val)
    if type(val) == "table" then snapshot = val end
    dataChanged()
end)

-- Reactions triggered from the bar widget (right click) or elsewhere.
noctalia.state.watch("petEvent", function(val)
    if type(val) == "table" then pet:onEvent(val.kind, val.ts, noctalia.nowMs()) end
end)

noctalia.state.watch("daemonUp", function(val)
    daemonUp = (val == true)
    dataChanged()
end)

function onOpen(_context)
    local snap = noctalia.state.get("snapshot")
    if type(snap) == "table" then snapshot = snap end
    daemonUp = (noctalia.state.get("daemonUp") == true)
    pinnedStep = nil
    -- The saved choices may have been changed by another entry since this panel loaded.
    tab = recall("panelTab", "tab") or tab
    pinnedSession = recall("selectedSession", "sel")
    if panel.setNeedsFrameTick then panel.setNeedsFrameTick(true) end
    dataChanged()
end

-- Animation: advance the pet and redraw.
function onFrameTick(_deltaMs)
    local now = noctalia.nowMs()
    if now - lastDrawMs < FRAME_MS then return end
    lastDrawMs = now
    pet:tick(now)
    render()
end

function onClose() panel.close() end
