--!nonstrict
-- bar_widget.luau — Nigiri, the animated pet that sits on the notch.
-- The pet's mood follows the daemon (idle / working / alert / sleep). Next to it:
--   "!N"        requests waiting for you, and sessions waiting in their own terminal (when any)
--   "2:13"      how long the active session's current turn has been running
--   pencil "3"  files changed this turn ("detailed" also adds +lines -lines, commands, failures)
--   "44%"       plan usage of the 5-hour window (falls back to the context window of
--               the most recently active session when plan limits are unavailable)
-- The tooltip carries the full progress of the active session: what it is doing,
-- changes, commands, context and the agent's last result.
-- What is shown next to the pet is the plugin setting "widgetInfo"
-- (usage = only the percentage, session = + time and files, detailed = + more).
-- Left click   → toggle the panel attached to the notch.
-- Right click  → pet it (it blushes and sends hearts).
-- Middle click → feed it the file copied in the file manager: it eats it and the chat opens
--                with the file attached (see lib/feed.luau).

local Pet = require("./lib/pet.luau")
local Fmt = require("./lib/fmt.luau")
local Sounds = require("./lib/sounds.luau")
local Feed = require("./lib/feed.luau")
Pet.setUi(ui)
Feed.init(noctalia)

local PANEL_ID = "scanna/sushi:panel"
local PET_SIZE = 26   -- sushi width in px (it is ~0.8x as tall); the bar is one control tall
local FRAME_MS = 66   -- ~15 fps

local snapshot = { sessions = {}, pending = {} }
local daemonUp = false
local pet = Pet.new(noctalia.nowMs(), Pet.configFrom(noctalia.getConfig))

-- Only this widget plays sounds (the panel's pet is silent), so nothing is heard twice.
local soundsOn = noctalia.getConfig("sounds")
soundsOn = (soundsOn ~= false and soundsOn ~= "false")
pet:enableSounds(soundsOn)
if soundsOn then Sounds.init(noctalia, noctalia.pluginDir()) end

local widgetInfo = noctalia.getConfig("widgetInfo")
if widgetInfo ~= "usage" and widgetInfo ~= "detailed" then widgetInfo = "session" end

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

--- Tamagotchi care (see src/care.rs): the widget shells out to the `sushi` CLI, the same way
--- panel.luau does for every daemon-mutating action.
local function binPath()
    local p = noctalia.getConfig("binPath")
    if type(p) ~= "string" or p == "" then p = "~/.cargo/bin/sushi" end
    return noctalia.expandPath(p)
end

local STATUS_LABEL = { idle = "idle", working = "working", waiting = "waiting" }

local function pendingCount() return Fmt.needsYou(snapshot) end

local function planLimits()
    local l = Fmt.limits(snapshot)
    return l and l.data or nil
end

--- Most recently active session that reports a context size.
local function activeContext()
    local best
    for _, s in ipairs(snapshot.sessions or {}) do
        if s.context and (not best or (s.last_event_ms or 0) > (best.last_event_ms or 0)) then best = s end
    end
    return best
end

--- Text and color for the percentage next to the pet: nil when there is nothing to show.
local function headline()
    local lim = planLimits()
    if lim and lim.five_hour then
        local p = lim.five_hour.percent
        return Fmt.round(p) .. "%", Fmt.percentColor(p)
    end
    local s = activeContext()
    if s then return Fmt.round(s.context.percent) .. "%", Fmt.percentColor(s.context.percent) end
    return nil
end

local function tooltipRows()
    if not daemonUp then
        return { { key = "Sushi", value = "daemon is not running: start `sushi daemon`" } }
    end
    local rows = {}
    local now = noctalia.nowMs()
    local active = Fmt.activeSession(snapshot)
    for _, row in ipairs(Fmt.sessionRows(active, now)) do rows[#rows + 1] = row end
    local lim = planLimits()
    if lim then
        for _, w in ipairs({ { "5-hour limit", lim.five_hour }, { "Weekly limit", lim.seven_day } }) do
            if w[2] then
                rows[#rows + 1] = { key = w[1], value = Fmt.round(w[2].percent) .. "% · resets in " .. Fmt.resetIn(w[2].resets_at_ms, now) }
            end
        end
        for _, m in ipairs(lim.models or {}) do
            rows[#rows + 1] = { key = m.model .. " weekly", value = Fmt.round(m.percent) .. "% · resets in " .. Fmt.resetIn(m.resets_at_ms, now) }
        end
    elseif Fmt.limits(snapshot) and Fmt.limits(snapshot).error then
        rows[#rows + 1] = { key = "Plan limits", value = tostring(Fmt.limits(snapshot).error) }
    end
    for _, s in ipairs(snapshot.sessions or {}) do
        if s ~= active then -- the active session is described above
            local value = STATUS_LABEL[s.status] or tostring(s.status)
            if s.context then value = value .. " · ctx " .. Fmt.round(s.context.percent) .. "%" end
            local tool = Fmt.lastToolText(s)
            if tool and s.status ~= "idle" then value = value .. " · " .. Fmt.clip(tool, 50) end
            rows[#rows + 1] = { key = s.name or s.id, value = value }
        end
    end
    if #rows == 0 then rows[1] = { key = "Sushi", value = "no active sessions" } end
    return rows
end

--- Small pieces describing the active session's turn (time, changes, commands).
local function sessionChips()
    local chips = {}
    local s = Fmt.activeSession(snapshot)
    if not s or s.status == "idle" then return chips end
    local a = s.activity or {}
    local now = noctalia.nowMs()
    local elapsed = Fmt.turnElapsed(s, now)
    if elapsed then
        chips[#chips + 1] = ui.label({ text = Fmt.duration(elapsed), fontSize = 12, color = "on_surface_variant" })
    end
    if (a.files_changed or 0) > 0 then
        chips[#chips + 1] = ui.glyph({ name = "pencil", size = 12, color = "primary" })
        chips[#chips + 1] = ui.label({ text = tostring(a.files_changed), fontSize = 12, fontWeight = "bold", color = "primary" })
        if widgetInfo == "detailed" then
            chips[#chips + 1] = ui.label({ text = "+" .. (a.lines_added or 0), fontSize = 11, color = "primary" })
            chips[#chips + 1] = ui.label({ text = "-" .. (a.lines_removed or 0), fontSize = 11, color = "error" })
        end
    end
    if widgetInfo == "detailed" then
        if (a.commands or 0) > 0 then
            chips[#chips + 1] = ui.glyph({ name = "terminal-2", size = 12, color = "on_surface_variant" })
            chips[#chips + 1] = ui.label({ text = tostring(a.commands), fontSize = 12, color = "on_surface_variant" })
        end
        if (a.failures or 0) > 0 then
            chips[#chips + 1] = ui.glyph({ name = "alert-triangle", size = 12, color = "error" })
            chips[#chips + 1] = ui.label({ text = tostring(a.failures), fontSize = 12, fontWeight = "bold", color = "error" })
        end
    end
    return chips
end

local function render()
    local children = { pet:build(PET_SIZE, { room = 4, margin = 3, mouth = true, maxHeight = 30, focusDim = focusDim }) }
    if daemonUp then
        local pending = pendingCount()
        if pending > 0 then
            children[#children + 1] = ui.label({ text = "!" .. pending, fontSize = 12, fontWeight = "bold", color = "error" })
        end
        if widgetInfo ~= "usage" then
            for _, chip in ipairs(sessionChips()) do children[#children + 1] = chip end
        end
        local text, color = headline()
        if text then
            children[#children + 1] = ui.label({ text = text, fontSize = 12, fontWeight = "bold", color = color })
        end
    end
    local container = barWidget.isVertical() and ui.column or ui.row
    barWidget.render(container({ gap = 4, align = "center" }, children))
end

local function refreshMood()
    pet:onSnapshot(snapshot, daemonUp, noctalia.nowMs())
    barWidget.setTooltip(tooltipRows())
end

noctalia.state.watch("snapshot", function(val)
    if type(val) == "table" then snapshot = val end
    refreshMood()
end)

--- Make the pet react here and in the panel (events are shared through noctalia.state).
local function sendEvent(kind)
    local ts = noctalia.nowMs()
    pet:onEvent(kind, ts, ts)
    noctalia.state.set("petEvent", { kind = kind, ts = ts })
end

function onRightClick()
    -- Petting it here counts the same as the panel's heart button, including its cooldown
    -- (snapshot.care.next_pet_ms): a right click while it is still cooling down is a no-op,
    -- instead of letting a flurry of clicks max affection out in one burst.
    local c = snapshot.care or {}
    if (c.next_pet_ms or 0) > noctalia.nowMs() then return end
    sendEvent("love")
    noctalia.runAsync({ binPath(), "care", "pet" }, function() end)
end

function onMiddleClick()
    Feed.fromClipboard(function(path)
        if path then
            Feed.feed(path)
            sendEvent("eat")
        else
            sendEvent("sad") -- nothing to eat: copy a file first
        end
    end)
end

-- Reactions triggered from the panel (approve → love, deny → sad, buttons).
noctalia.state.watch("petEvent", function(val)
    if type(val) == "table" then pet:onEvent(val.kind, val.ts, noctalia.nowMs()) end
end)

noctalia.state.watch("daemonUp", function(val)
    daemonUp = (val == true)
    refreshMood()
end)

noctalia.setUpdateInterval(FRAME_MS)

local lastTooltipMs = 0

function update()
    local now = noctalia.nowMs()
    if now - lastTooltipMs > 1000 then
        lastTooltipMs = now
        barWidget.setTooltip(tooltipRows())
    end
    focusDim = Fmt.isFocusActive(focusSettings(), now)
    pet:setFocus(focusDim)
    pet:tick(now)
    for _, name in ipairs(pet:drainSounds()) do Sounds.play(name) end
    render()
end

function onClick()
    noctalia.togglePanel(PANEL_ID)
end

do
    local snap = noctalia.state.get("snapshot")
    if type(snap) == "table" then snapshot = snap end
    daemonUp = (noctalia.state.get("daemonUp") == true)
end

refreshMood()
pet:tick(noctalia.nowMs())
render()
