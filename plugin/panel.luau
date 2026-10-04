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
-- again to follow the active session). A file fed to the pet ("Feed a file", or
-- from the bar widget) is attached to the next chat message. Everything is drawn from the
-- daemon's snapshot.

local Pet = require("./lib/pet.luau")
local Fmt = require("./lib/fmt.luau")
local Highlight = require("./lib/highlight.luau")
local Viewer = require("./lib/viewer.luau")
local Chat = require("./lib/chat.luau")
local Feed = require("./lib/feed.luau")
Pet.setUi(ui)
Viewer.init(ui, Highlight)
Chat.init(ui)
Feed.init(noctalia)

local HERO_SIZE = 92
local LEFT_W = 292
local FRAME_MS = 33          -- cap the redraw rate at ~30 fps
local TYPE_WINDOW_MS = 20000 -- only steps this recent get the typewriter effect (they may be shown a little late)
local MAX_RAIL = 6           -- rows of the step rail ("Done" included) with a single session
local SESSION_ROWS_MIN = 3   -- session rows the rail's own row budget leaves room for (see railBudget)
local SESSION_ROW_H = 28     -- height of a session row (paddingV 5 + a 12 px label + gap)
local SESSIONS_MAX_ROWS = 5  -- sessions is a fixed-height scroll box (not flexGrow): it must never
                              -- fight the rail for space or render on top of it

local snapshot = { sessions = {}, pending = {}, usage = {} }
local daemonUp = false
local pet = Pet.new(noctalia.nowMs(), Pet.configFrom(noctalia.getConfig))
local lastDrawMs = 0

--- Focus mode has no quick toggle here (unlike the app): set `focusEnabled` and the hours/days in
--- Noctalia's own settings for this plugin. `focusManual` stays "auto" — nothing writes it back.
local function focusSettings()
    return {
        focusEnabled = noctalia.getConfig("focusEnabled"),
        focusStart = noctalia.getConfig("focusStart"),
        focusEnd = noctalia.getConfig("focusEnd"),
        focusDays = noctalia.getConfig("focusDays"),
        focusManual = "auto",
    }
end
local focusDim = false

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
local pinnedStep = nil   -- ts_ms of a step chosen in the rail (nil = follow the session)
local follow = { key = nil, since = 0 } -- the step the viewer follows (see Fmt.followStep)
local pinnedTurn = nil   -- the turn that step belongs to
local seenAt = {}        -- first time a step was drawn (drives the typewriter)
local viewCache = {}     -- finished viewer trees, so they are not rebuilt every frame
local viewCacheSize = 0
local chatInputKey = 0   -- bumped after each send: recreates (empties) the text field
local chatDraft = ""     -- what is typed so far (the send button needs it)
local optimistic = nil   -- {text, file, base}: a message just sent, shown until the daemon confirms it

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

--- `extra`: more CLI arguments (`--accept-edits` to approve a plan with auto-accepted edits).
local function decide(action, id, extra)
    local argv = { binPath(), action, tostring(id) }
    if extra then argv[#argv + 1] = extra end
    settle(id, argv, (action == "approve") and "approve" or "deny")
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
local function currentStep(s, now)
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
    return Fmt.followStep(follow, s.id, list, now), true
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

-- Tamagotchi care (see src/care.rs): the 3 emote buttons that double as care actions, each
-- gated by its own cooldown server-side (see next_*_ms below) so repeated clicking can no
-- longer max a need out in one burst. Dance stays purely cosmetic. The reaction only plays once
-- the daemon actually accepts the action, never on a click refused for still cooling down.
local CARE_ACTION = { love = "pet", eat = "feed", nap = "nap" }
local CARE_NEXT_KEY = { love = "next_pet_ms", eat = "next_feed_ms", nap = "next_nap_ms" }

--- Seconds left before `kind`'s care action is available again (0 once ready); see
--- src/care.rs's CARE_COOLDOWN_MS — love/eat/nap each run on their own independent clock.
local function careCooldown(kind, now)
    local key = CARE_NEXT_KEY[kind]
    if not key then return 0 end
    local c = snapshot.care or {}
    return math.max(0, math.ceil(((c[key] or 0) - now) / 1000))
end

local function action(glyph, tip, kind, now)
    local cooldown = careCooldown(kind, now)
    return ui.button({
        key = "emote-" .. kind, glyph = glyph, variant = "ghost", controlSize = "sm",
        tooltip = (cooldown > 0) and (tip .. " · ready in " .. cooldown .. "s") or tip,
        enabled = cooldown == 0,
        onClick = function()
            sendEvent(kind)
            if CARE_ACTION[kind] then noctalia.runAsync({ binPath(), "care", CARE_ACTION[kind] }, function() end) end
        end,
        onHover = "onGaze",
    })
end

local function counts()
    local working = 0
    for _, s in ipairs(snapshot.sessions or {}) do
        if s.status == "working" then working = working + 1 end
    end
    return working, Fmt.needsYou(snapshot)
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
        pet:build(HERO_SIZE, { room = 20, maxHeight = 124, margin = 20, badge = badge, badgeSlot = true, focusDim = focusDim, accessories = true }),
    })
end

--- Feed the pet a file: it eats it and the chat opens with the file attached.
local function feed(path)
    if not path then
        sendEvent("sad") -- nothing to eat: copy a file in the file manager first
        return
    end
    Feed.feed(path)
    sendEvent("eat")
    tab = "chat"
    render()
end

function onFeedDiscard()
    Feed.clear()
    render()
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
    -- Flagged by a configured policy rule (see policy.rs): shown for every agent, even though real
    -- enforcement only exists for Claude Code's own permissions (see the README).
    local tooltip = e.tool .. " " .. (e.label or "")
    local children = {
        icon,
        ui.label({
            text = e.tool, fontSize = 15, fontWeight = running and "bold" or "semibold",
            color = running and "on_surface" or "on_surface_variant",
        }),
        ui.label({ text = e.label or "", fontSize = 11, color = "on_surface_variant", maxLines = 1, flexGrow = 1 }),
    }
    if e.policy then
        tooltip = tooltip .. " — policy: " .. tostring(e.policy.label)
        children[#children + 1] = ui.glyph({ name = "alert-triangle", size = 14, color = (e.policy.level == "deny") and "error" or "#f59e0b" })
    end
    return ui.row({
        key = "step-" .. tostring(e.ts_ms), align = "center", gap = 10, paddingH = 8, paddingV = 5, radius = 8,
        fill = (pinnedStep == e.ts_ms) and "surface_variant/0.55"
            or ((pinnedStep == nil and follow.key == tostring(s.id) .. ":" .. tostring(e.ts_ms)) and "surface_variant/0.22" or nil),
        onClick = function() pinStep(s, e) end, onHover = "onGaze",
        tooltip = tooltip,
    }, children)
end

--- How many session rows the rail's own row budget assumes (see railBudget) — sessions itself is
--- capped independently (see SESSIONS_MAX_ROWS), this is just about leaving the rail reasonably
--- short rather than a long list of steps.
local function sessionRowsShown()
    return math.max(1, math.min(#(snapshot.sessions or {}), SESSION_ROWS_MIN))
end

--- How many rows the rail may take: fewer when there are sessions to list, so a turn with many
--- steps cannot push the session list out of the panel.
local function railBudget()
    return MAX_RAIL - sessionRowsShown() + 1
end

local function rail(s, now)
    local rows = {}
    local rev = 0
    if s then
        local list = stepsOf(s)
        local a = s.activity or {}
        local done = (s.status == "idle" and a.finished_ms) and true or false
        local room = railBudget() - (done and 1 or 0)
        for i = math.max(1, #list - room + 1), #list do rows[#rows + 1] = railRow(s, list[i], now) end
        -- Keep the rail pinned to the newest step: it's a short scroll box (see leftColumn's
        -- note), so without this the latest command could scroll out of view behind older ones.
        local last = list[#list]
        rev = (last and last.ts_ms or 0) * 1000 + #list
        if done then
            rows[#rows + 1] = ui.row({ key = "step-done", align = "center", gap = 10, paddingH = 8, paddingV = 5 }, {
                ui.glyph({ name = "circle-check-filled", size = 18, color = "on_surface_variant", opacity = 0.6 }),
                ui.label({ text = "Done", fontSize = 15, fontWeight = "semibold", color = "on_surface_variant", opacity = 0.7 }),
            })
        end
    end
    if #rows == 0 then
        rows[1] = ui.label({ text = "No steps yet.", fontSize = 12, color = "on_surface_variant" })
    end
    return ui.scroll({ flexGrow = 1, gap = 1, stickToBottom = true, scrollToBottomRev = rev }, rows)
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
    return ui.scroll({ height = SESSIONS_MAX_ROWS * SESSION_ROW_H, gap = 2 }, rows)
end

--- The hunger / energy / affection meters (see src/care.rs); low = "error" track color, not a
--- punishment — they never drop below a floor, this just nudges the color so it's noticeable.
local function needColor(pct)
    if pct < 35 then return "error" end
    if pct < 60 then return "tertiary" end
    return "primary"
end

--- Compact: one row, three mini-bars each with a one-letter label — hover for the full name and
--- percent. (Was 3 full-width labeled bars; that crowded out the sessions list below it.)
local function careMeters()
    local c = snapshot.care or {}
    local function meter(letter, label, pct)
        pct = pct or 100
        local p = math.max(0, math.min(100, pct))
        return ui.column({ flexGrow = 1, gap = 2, tooltip = label .. " " .. math.floor(p + 0.5) .. "%" }, {
            ui.label({ text = letter, fontSize = 9, color = "on_surface_variant" }),
            ui.progress({ progress = p / 100, fill = needColor(p), track = "outline/0.25", height = 5, radius = 3 }),
        })
    end
    return ui.row({ gap = 10 }, { meter("H", "Hunger", c.hunger), meter("E", "Energy", c.energy), meter("A", "Affection", c.affection) })
end

-- petBlock, title, then care (the meters + its action buttons) — care is about the pet rather
-- than any one session, but still reads after the title/status line it sits just below. The
-- session list is a fixed-height scroll box (SESSIONS_MAX_ROWS), not flexGrow, so it can never
-- fight the rail for space or render on top of it — the rail is the one that flexGrows/scrolls
-- into whatever is left at the bottom, since sessions is the thing you most need to always reach.
local function leftColumn(s, now)
    local nWorking, nPending = counts()
    return ui.column({ width = LEFT_W, gap = 10, padding = 14 }, {
        petBlock(nWorking, nPending),
        titleBlock(s, nWorking, nPending, now),
        ui.separator({ spacing = 2, color = "outline", opacity = 0.2 }),
        careMeters(),
        ui.row({ gap = 2 }, {
            action("heart", "Pet it", "love", now),
            action("cookie", "Feed it", "eat", now),
            action("music", "Dance!", "dance", now),
            action("moon", "Nap time", "nap", now),
            ui.button({
                key = "emote-feed", glyph = "file-plus", variant = "ghost", controlSize = "sm",
                tooltip = "Feed it a file (copy one in the file manager first)",
                onClick = function() Feed.fromClipboard(feed) end, onHover = "onGaze",
            }),
        }),
        ui.separator({ spacing = 2, color = "outline", opacity = 0.2 }),
        ui.label({ text = "Sessions", fontSize = 11, color = "on_surface_variant" }),
        sessionList(s),
        ui.separator({ spacing = 2, color = "outline", opacity = 0.2 }),
        rail(s, now),
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

-- A plan the agent wants approved (Claude Code's "Ready to code?"). While its hook waits it can be approved here;
-- after that it stays for reading until it has been dealt with in the terminal.
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
    if p.answerable == false then
        children[#children + 1] = ui.label({ text = "Review and approve it in the terminal.", fontSize = 11, color = "on_surface_variant" })
    else
        local id = tostring(p.id)
        children[#children + 1] = ui.row({ gap = 8 }, {
            ui.button({ key = "plan-approve-" .. id, text = "Approve", variant = "primary", onHover = "onGaze", tooltip = "Claude asks before each edit", onClick = function() decide("approve", p.id) end }),
            ui.button({ key = "plan-edits-" .. id, text = "Approve, auto-accept edits", variant = "outline", onHover = "onGaze", onClick = function() decide("approve", p.id, "--accept-edits") end }),
            ui.button({ key = "plan-keep-" .. id, text = "Keep planning", variant = "outline", onHover = "onGaze", tooltip = "Then tell Claude what to change in the terminal", onClick = function() decide("deny", p.id) end }),
        })
        children[#children + 1] = ui.label({ text = "You can also answer it in the terminal.", fontSize = 11, color = "on_surface_variant" })
    end
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

local function stepLine(e, now)
    local right = {}
    if (e.added or 0) > 0 or (e.removed or 0) > 0 then
        right[#right + 1] = ui.label({ text = string.format("+%d -%d", e.added or 0, e.removed or 0), fontSize = 11, color = "on_surface_variant" })
    end
    local status = (e.ok == true) and "done" or ((e.ok == false) and "failed" or ("running · " .. Fmt.duration(now - (e.ts_ms or now))))
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
    local opts = { ok = e.ok, maxRows = 34 }
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
        local e, following = currentStep(s, now)
        if e then
            nodes[#nodes + 1] = stepLine(e, now)
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
    local fed = Feed.current()
    return Chat.view(chat, {
        optimistic = optimistic and optimistic.text or nil,
        optimisticFile = optimistic and optimistic.file or nil,
        attachment = fed and fed.name or nil,
        label = Fmt.agentLabel(snapshot, chat.agent or chatAgent()),
        inputKey = chatInputKey,
        draftReady = chatDraft:match("%S") ~= nil or fed ~= nil,
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
    local fed = Feed.current()
    if (text == "" and not fed) or chat.busy or optimistic then return end
    optimistic = { text = text, file = fed and fed.name or nil, base = #(chat.messages or {}) }
    chatInputKey = chatInputKey + 1
    chatDraft = ""
    local argv = { binPath(), "chat" }
    if text ~= "" then argv[#argv + 1] = text end
    if fed then
        argv[#argv + 1] = "--file"
        argv[#argv + 1] = fed.path
        Feed.clear()
    end
    argv[#argv + 1] = "--agent"
    argv[#argv + 1] = chatAgent()
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

--- One agent's row in the comparative usage table: today's tokens and an estimated cost, or "not
--- tracked yet" when nothing reads that agent's usage (only Claude Code today; never fabricated).
local function usageRow(label, data)
    local right
    if not (data and data.available) then
        right = ui.label({ text = "not tracked yet", fontSize = 11, color = "on_surface_variant" })
    else
        right = ui.label({
            text = string.format("%s today · %s total (est.)", fmt(Fmt.tokenSum(data.today)), Fmt.cost(data.estimated_cost_usd)),
            fontSize = 11, color = "on_surface_variant",
        })
    end
    return ui.row({ align = "center", justify = "space_between" }, {
        ui.label({ text = label, fontSize = 12, color = "on_surface" }),
        right,
    })
end

local function usageContent(s, now)
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
    nodes[#nodes + 1] = ui.label({ text = "Usage by agent", fontSize = 12, fontWeight = "semibold", color = "on_surface" })
    local rows = {}
    for id, data in pairs(snapshot.usage or {}) do
        rows[#rows + 1] = usageRow(Fmt.agentLabel(snapshot, id), data)
    end
    nodes[#nodes + 1] = ui.column({ gap = 4 }, rows)
    local gamify = noctalia.getConfig("gamificationEnabled")
    if gamify ~= false and gamify ~= "false" then
        local st = snapshot.stats or {}
        nodes[#nodes + 1] = ui.separator({ spacing = 4, color = "outline", opacity = 0.2 })
        nodes[#nodes + 1] = ui.label({ text = "Stats", fontSize = 12, fontWeight = "semibold", color = "on_surface" })
        nodes[#nodes + 1] = ui.row({ gap = 20 }, {
            stat("Streak", (st.streak_days or 0) > 0 and ("🔥 " .. st.streak_days .. "d") or "—"),
            stat("Sessions", tostring(st.total_sessions or 0)),
            stat("Steps watched", tostring(st.total_tool_calls or 0)),
        })
    end
    return ui.scroll({ flexGrow = 1, gap = 10 }, nodes)
end

--- The shop grid: buy with currency earned from real agent usage, equip at most one at a time
--- (see Pet.ACCESSORIES, mirrored server-side in src/care.rs so a tampered client can't grant
--- itself a free item).
local function shopContent()
    local c = snapshot.care or {}
    local owned = {}
    for _, id in ipairs(c.owned or {}) do owned[id] = true end
    local currency = c.currency or 0
    local cards = {}
    for id, a in pairs(Pet.ACCESSORIES) do
        local isEquipped = c.equipped == id
        local actionBtn
        if isEquipped then
            actionBtn = ui.button({
                key = "unequip-" .. id, text = "Unequip", controlSize = "sm", variant = "outline",
                onClick = function() noctalia.runAsync({ binPath(), "care", "equip", "" }, function() end) end,
            })
        elseif owned[id] then
            actionBtn = ui.button({
                key = "equip-" .. id, text = "Equip", controlSize = "sm", variant = "primary",
                onClick = function() noctalia.runAsync({ binPath(), "care", "equip", id }, function() end) end,
            })
        else
            actionBtn = ui.button({
                key = "buy-" .. id, text = "Buy · " .. a.cost, controlSize = "sm", variant = "primary", enabled = currency >= a.cost,
                onClick = function() noctalia.runAsync({ binPath(), "care", "buy", id }, function() end) end,
            })
        end
        cards[#cards + 1] = ui.row({ key = id, align = "center", justify = "space_between" }, {
            ui.row({ align = "center", gap = 8 }, {
                ui.column({ width = 22, height = 22, align = "center", justify = "center" }, { Pet.accessoryArt(id, 20) }),
                ui.label({ text = a.label, fontSize = 12, color = "on_surface" }),
            }),
            actionBtn,
        })
    end
    return ui.scroll({ flexGrow = 1, gap = 10 }, {
        ui.row({ align = "center", justify = "space_between" }, {
            ui.label({ text = "Shop", fontSize = 12, fontWeight = "semibold", color = "on_surface" }),
            ui.label({ text = "🪙 " .. fmt(currency), fontSize = 12, color = "on_surface_variant" }),
        }),
        ui.column({ gap = 8 }, cards),
    })
end

-- ── Mini-games: three interchangeable rounds, any of which can report a score to the same
-- "care play" command (see src/care.rs's Care:play — it's game-agnostic by design, so adding a
-- game here never needs a daemon change). playGame picks which; playState (idle | running | done)
-- is shared (see app/ui/js/panel.js's mirror of this) ───────────────────────────────────────────
local playGame = "tap" -- "tap" | "catch" | "memory"
local playState = "idle"
local playResultLine = ""
local playResultRewarded = true
local playResultNewBest = false
local playBest = { tap = 0, catch = 0, memory = 0 } -- best this session (not persisted, unlike the app)

-- tap: tap the pet as many times as possible in 8s; score = taps, capped at 100.
local playTaps = 0
local playEndsAt = 0
-- catch: the lit side jumps between left/right on its own; tap the lit one, not the dark one.
local catchHits = 0
local catchEndsAt = 0
local catchLeftIsTarget = true
local catchNextRerollAt = 0
local CATCH_DURATION_MS, CATCH_REROLL_MS, CATCH_SCORE_SCALE = 8000, 850, 9
-- memory: Simon-style growing pattern; no time limit, since getting it wrong is its own end.
local memLevel = 1
local memSeq = {}
local memPhase = "flash" -- flash | gap | input
local memActivePad = nil
local memShowIdx = 1
local memInputIdx = 1
local memNextAt = 0
local MEMORY_FLASH_MS, MEMORY_GAP_MS, MEMORY_SCORE_PER_LEVEL = 420, 180, 14
local function randomPad() return math.random(0, 3) end

--- Seconds left before a round can earn a reward again (0 once ready); see
--- src/care.rs's PLAY_COOLDOWN_MS. Playing while this is positive still works for practice.
--- Shared by all three games — the cooldown is on the reward, not any one of them.
local function playCooldown(now)
    local c = snapshot.care or {}
    return math.max(0, math.ceil(((c.next_play_ms or 0) - now) / 1000))
end

--- Records the result of a finished round: updates that game's best (kept per-game, since "best"
--- means a different thing for each one), then reports the score once (Care:play itself
--- rate-limits the actual reward — this just makes sure it is asked once).
local function finishPlay(now, score, metric, line)
    local newBest = metric > playBest[playGame]
    if newBest then playBest[playGame] = metric end
    playState = "done"
    playResultRewarded = playCooldown(now) == 0
    playResultNewBest = newBest
    playResultLine = line(newBest)
    noctalia.runAsync({ binPath(), "care", "play", tostring(math.min(100, score)) }, function() end)
end

local function tickTap(now)
    if playGame == "tap" and playState == "running" and now >= playEndsAt then
        finishPlay(now, playTaps, playTaps, function(nb)
            return nb and (playTaps .. " taps — new best!") or (playTaps .. " taps!")
        end)
    end
end

local function tickCatch(now)
    if playGame ~= "catch" or playState ~= "running" then return end
    if now >= catchNextRerollAt then
        catchLeftIsTarget = math.random() < 0.5
        catchNextRerollAt = now + CATCH_REROLL_MS
    end
    if now >= catchEndsAt then
        finishPlay(now, catchHits * CATCH_SCORE_SCALE, catchHits, function(nb)
            return nb and (catchHits .. " catches — new best!") or (catchHits .. " catches!")
        end)
    end
end

--- A hit on `side` ("left" | "right"); only the currently-lit side scores — the wrong one is just
--- a miss, never a penalty (this project never punishes a care action, see src/care.rs).
local function catchTap(side, now)
    if playGame ~= "catch" or playState ~= "running" then return end
    if (side == "left") == catchLeftIsTarget then
        catchHits = catchHits + 1
        catchLeftIsTarget = math.random() < 0.5
        catchNextRerollAt = now + CATCH_REROLL_MS
    end
end

local function tickMemory(now)
    if playGame ~= "memory" or playState ~= "running" or memPhase == "input" then return end
    if now < memNextAt then return end
    if memPhase == "flash" then
        memPhase = "gap"
        memActivePad = nil
        memNextAt = now + MEMORY_GAP_MS
    else
        memShowIdx = memShowIdx + 1
        if memShowIdx > #memSeq then
            memPhase = "input"
            memInputIdx = 1
            memActivePad = nil
        else
            memPhase = "flash"
            memActivePad = memSeq[memShowIdx]
            memNextAt = now + MEMORY_FLASH_MS
        end
    end
end

--- A tap on pad 0..3 during the input phase: the right pad advances (and, on a completed
--- sequence, grows it and replays); a wrong one ends the round — there's no time limit, so this
--- is the only way it ends.
local function memoryTap(pad, now)
    if playGame ~= "memory" or playState ~= "running" or memPhase ~= "input" then return end
    if pad == memSeq[memInputIdx] then
        memInputIdx = memInputIdx + 1
        if memInputIdx > #memSeq then
            memLevel = memLevel + 1
            memSeq[#memSeq + 1] = randomPad()
            memShowIdx = 1
            memPhase = "flash"
            memActivePad = memSeq[1]
            memNextAt = now + MEMORY_FLASH_MS
        end
        return
    end
    local completed = memLevel - 1
    memPhase = "idle"
    finishPlay(now, completed * MEMORY_SCORE_PER_LEVEL, completed, function(nb)
        return nb and (completed .. " rounds remembered — new best!") or (completed .. " rounds remembered!")
    end)
end

local function tickPlay(now)
    tickTap(now)
    tickCatch(now)
    tickMemory(now)
end

local GAME_META = {
    tap = {
        label = "Tap the pet!", switcherLabel = "Tap",
        hint = function(cd)
            return (cd > 0)
                and ("Tap as many times as you can in 8 seconds. You can still practice, but the next reward is " .. cd .. "s away.")
                or "Tap as many times as you can in 8 seconds. A round can only earn a reward once every 5 minutes."
        end,
        bestLine = function(n) return "Best: " .. n end,
    },
    catch = {
        label = "Catch it!", switcherLabel = "Catch",
        hint = function(cd)
            return (cd > 0)
                and ("The lit side jumps around on its own — tap it, not the dark one, for 8 seconds. Practice freely; the next reward is " .. cd .. "s away.")
                or "The lit side jumps around on its own — tap it, not the dark one, for 8 seconds. A round can only earn a reward once every 5 minutes."
        end,
        bestLine = function(n) return "Best: " .. n end,
    },
    memory = {
        label = "Memory", switcherLabel = "Memory",
        hint = function(cd)
            return (cd > 0)
                and ("Watch the pads light up, then repeat the pattern — it only gets longer, no time limit. Practice freely; the next reward is " .. cd .. "s away.")
                or "Watch the pads light up, then repeat the pattern — it only gets longer, no time limit. A round can only earn a reward once every 5 minutes."
        end,
        bestLine = function(n) return "Best: " .. n .. " rounds" end,
    },
}
local GAME_ORDER = { "tap", "catch", "memory" }

local function startGame(now)
    playState = "running"
    if playGame == "tap" then
        playTaps, playEndsAt = 0, now + 8000
    elseif playGame == "catch" then
        catchHits = 0
        catchEndsAt = now + CATCH_DURATION_MS
        catchLeftIsTarget = math.random() < 0.5
        catchNextRerollAt = now + CATCH_REROLL_MS
    else
        memLevel = 1
        memSeq = { randomPad() }
        memShowIdx, memInputIdx = 1, 1
        memPhase = "flash"
        memActivePad = memSeq[1]
        memNextAt = now + MEMORY_FLASH_MS
    end
end

local function gameSwitcher()
    local buttons = {}
    for _, g in ipairs(GAME_ORDER) do
        buttons[#buttons + 1] = ui.button({
            key = "play-game-" .. g, text = GAME_META[g].switcherLabel, controlSize = "sm",
            variant = (playGame == g) and "primary" or "ghost",
            onClick = function() playGame, playState = g, "idle" end,
        })
    end
    return ui.row({ gap = 6 }, buttons)
end

--- The running view for whichever game is active (see tickTap/tickCatch/tickMemory for the rules).
local function runningContent(now)
    if playGame == "tap" then
        local remaining = math.max(0, playEndsAt - now)
        return ui.column({ gap = 10 }, {
            ui.row({ align = "center", justify = "space_between" }, {
                ui.label({ text = string.format("%.1fs", remaining / 1000), fontSize = 16, fontWeight = "bold", color = "on_surface" }),
                ui.label({ text = playTaps .. " taps", fontSize = 16, fontWeight = "bold", color = "primary" }),
            }),
            ui.button({ key = "play-tap", text = "Tap!", variant = "primary", height = 72, onClick = function() playTaps = playTaps + 1 end }),
        })
    end
    if playGame == "catch" then
        local remaining = math.max(0, catchEndsAt - now)
        local function side(isLeft)
            local on = (isLeft == catchLeftIsTarget)
            return ui.button({
                key = "catch-" .. (isLeft and "left" or "right"), text = on and "●" or "○",
                variant = on and "primary" or "ghost", height = 72, flexGrow = 1,
                onClick = function() catchTap(isLeft and "left" or "right", noctalia.nowMs()) end,
            })
        end
        return ui.column({ gap = 10 }, {
            ui.row({ align = "center", justify = "space_between" }, {
                ui.label({ text = string.format("%.1fs", remaining / 1000), fontSize = 16, fontWeight = "bold", color = "on_surface" }),
                ui.label({ text = catchHits .. " hits", fontSize = 16, fontWeight = "bold", color = "primary" }),
            }),
            ui.row({ gap = 8 }, { side(true), side(false) }),
        })
    end
    local pads = {}
    for p = 0, 3 do
        local on = memActivePad == p
        pads[#pads + 1] = ui.button({
            key = "memory-pad-" .. p, text = tostring(p + 1), variant = on and "primary" or "ghost",
            height = 56, flexGrow = 1, enabled = memPhase == "input",
            onClick = function() memoryTap(p, noctalia.nowMs()) end,
        })
    end
    return ui.column({ gap = 10 }, {
        ui.row({ align = "center", justify = "space_between" }, {
            ui.label({ text = "Level " .. memLevel, fontSize = 16, fontWeight = "bold", color = "on_surface" }),
            ui.label({ text = (memPhase == "input") and "Your turn" or "Watch...", fontSize = 12, color = "on_surface_variant" }),
        }),
        ui.row({ gap = 8 }, pads),
    })
end

--- The mini-games: a switcher at top picks which, each reports a 0..100 score to the same
--- "care play" command (see src/care.rs's Care:play for how that becomes a reward, and its
--- 5-minute cooldown — shared across all three, so switching games does not reset it).
local function playContent(now)
    local meta = GAME_META[playGame]
    local switcher = gameSwitcher()
    if playState == "idle" then
        local best = playBest[playGame]
        return ui.column({ gap = 10 }, {
            switcher,
            ui.row({ align = "center", justify = "space_between" }, {
                ui.label({ text = meta.label, fontSize = 12, fontWeight = "semibold", color = "on_surface" }),
                (best > 0) and ui.label({ text = meta.bestLine(best), fontSize = 11, color = "on_surface_variant" }) or nil,
            }),
            ui.label({ text = meta.hint(playCooldown(now)), fontSize = 11, color = "on_surface_variant", maxLines = 3 }),
            ui.button({ key = "play-start", text = "Start", variant = "primary", controlSize = "sm", onClick = function() startGame(noctalia.nowMs()) end }),
        })
    end
    if playState == "running" then return ui.column({ gap = 10 }, { switcher, runningContent(now) }) end
    return ui.column({ gap = 10 }, {
        switcher,
        ui.label({
            text = playResultLine, fontSize = 14, fontWeight = "semibold", color = playResultNewBest and "primary" or "on_surface",
        }),
        ui.label({
            text = playResultRewarded and "Sent to your pet — check its currency and affection."
                or "That one was just for fun — the next reward was still cooling down.",
            fontSize = 11, color = "on_surface_variant",
        }),
        ui.button({ key = "play-again", text = "Play again", controlSize = "sm", onClick = function() playState = "idle" end }),
    })
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
            tabButton("shop", "Shop"),
            tabButton("play", "Play"),
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
    elseif tab == "shop" then
        children[#children + 1] = shopContent()
    elseif tab == "play" then
        children[#children + 1] = playContent(now)
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

-- A file fed from the bar widget or over IPC: show the chat with it attached.
noctalia.state.watch("fedFile", function()
    if Feed.current() then tab = "chat" end
    render()
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
    focusDim = Fmt.isFocusActive(focusSettings(), now)
    pet:setFocus(focusDim)
    pet:tick(now)
    tickPlay(now)
    render()
end

function onClose() panel.close() end
