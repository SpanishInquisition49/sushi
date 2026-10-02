--!nonstrict
-- service.luau — reads the sushi daemon's state.json and shares it with
-- the widget and the panel through noctalia.state.
--
--   "snapshot"  table  {sessions = {...}, pending = {...}, usage = {...}}
--   "daemonUp"  bool   false when state.json is missing or stale
--
-- The daemon rewrites state.json at least every 5 s, so a file older than
-- STALE_MS means the daemon is gone.

local Feed = require("./lib/feed.luau")
Feed.init(noctalia)

local PANEL_ID = "scanna/sushi:panel"
local STALE_MS = 15000

local autoOpen = true
local interval = 1000
local lastRaw = nil
local lastUp = nil
local knownPending = {} -- ["<id>@<created_ms>"] = true
local firstRead = true

local function reloadConfig()
    local v = noctalia.getConfig("autoOpen")
    autoOpen = (v ~= false and v ~= "false")
    interval = 1000
    noctalia.setUpdateInterval(interval)
end

reloadConfig()

local function statePath()
    local rt = noctalia.getenv("XDG_RUNTIME_DIR")
    if type(rt) ~= "string" or rt == "" then rt = "/tmp" end
    return rt .. "/sushi/state.json"
end

local function setUp(up)
    if lastUp ~= up then
        lastUp = up
        noctalia.state.set("daemonUp", up)
    end
end

-- Open the panel when a request we have not seen before shows up.
local function checkNewPending(pending)
    local fresh = false
    local seen = {}
    for _, p in ipairs(pending) do
        local key = tostring(p.id) .. "@" .. tostring(p.created_ms)
        seen[key] = true
        if not knownPending[key] then fresh = true end
    end
    knownPending = seen
    if fresh and autoOpen and not firstRead then
        noctalia.runAsync({ "noctalia", "msg", "panel-open", PANEL_ID }, function() end)
    end
end

function update()
    local raw = noctalia.readFile(statePath())
    if type(raw) ~= "string" or raw == "" then
        lastRaw = nil
        setUp(false)
        firstRead = false
        return
    end

    local ok, snap = pcall(function() return noctalia.json.decode(raw) end)
    if not ok or type(snap) ~= "table" then return end

    local age = noctalia.nowMs() - (tonumber(snap.updated_ms) or 0)
    setUp(age < STALE_MS)

    -- Look more often while something moves (a session at work or waiting, the chat streaming an
    -- answer), so steps and text show up as they happen; reading an unchanged file costs little.
    local busy = snap.chat and snap.chat.busy
    for _, s in ipairs(snap.sessions or {}) do
        if s.status == "working" or s.status == "waiting" then busy = true end
    end
    local want = busy and 250 or 1000
    if want ~= interval then
        interval = want
        noctalia.setUpdateInterval(interval)
    end

    if raw ~= lastRaw then
        lastRaw = raw
        snap.sessions = snap.sessions or {}
        snap.pending = snap.pending or {}
        checkNewPending(snap.pending)
        noctalia.state.set("snapshot", snap)
    end
    firstRead = false
end

function onConfigChanged()
    reloadConfig()
end

-- Choose the tab the panel opens on, e.g. from a keybinding:
--   noctalia msg plugin scanna/sushi:state all tab chat      (live | chat | usage)
--   noctalia msg panel-open scanna/sushi:panel
-- Feed the pet a file (it opens the chat with the file attached):
--   noctalia msg plugin scanna/sushi:state all feed /path/to/file
function onIpc(event, payload)
    if event == "tab" and (payload == "live" or payload == "chat" or payload == "usage") then
        noctalia.state.set("panelTab", { tab = payload })
    elseif event == "emote" and (payload == "love" or payload == "eat" or payload == "dance" or payload == "nap") then
        -- Make the pet do something, e.g. from a keybinding: ... all emote dance
        noctalia.state.set("petEvent", { kind = payload, ts = noctalia.nowMs() })
    elseif event == "feed" and type(payload) == "string" then
        -- Feed the pet a file, e.g. from a file manager action: ... all feed /path/to/file
        local path = Feed.pathFrom(payload)
        if path then
            Feed.feed(path)
            noctalia.state.set("petEvent", { kind = "eat", ts = noctalia.nowMs() })
        end
    end
end
