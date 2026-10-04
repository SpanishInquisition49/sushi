--!nonstrict
-- lib/fmt.luau — formatting helpers shared by the bar widget and the panel.

local F = {}

function F.round(x) return math.floor((tonumber(x) or 0) + 0.5) end

--- Compact token count: 950, 1.2k, 3.4M.
function F.tokens(n)
    n = tonumber(n) or 0
    if n >= 1e9 then return string.format("%.1fG", n / 1e9) end
    if n >= 1e6 then return string.format("%.1fM", n / 1e6) end
    if n >= 1e3 then return string.format("%.1fk", n / 1e3) end
    return tostring(math.floor(n))
end

--- Palette role for a usage percentage: calm, warning, critical.
function F.percentColor(p)
    p = tonumber(p) or 0
    if p >= 90 then return "error" end
    if p >= 70 then return "tertiary" end
    return "primary"
end

--- "2h 15m", "3d 4h", "now" — time left until `resetsAtMs`.
function F.resetIn(resetsAtMs, nowMs)
    if not resetsAtMs then return "?" end
    local s = math.floor((resetsAtMs - nowMs) / 1000)
    if s <= 0 then return "now" end
    local d, h, m = math.floor(s / 86400), math.floor(s % 86400 / 3600), math.floor(s % 3600 / 60)
    if d > 0 then return string.format("%dd %dh", d, h) end
    if h > 0 then return string.format("%dh %dm", h, m) end
    return string.format("%dm", math.max(1, m))
end

--- "2:13" or "1:02:03" for a duration in milliseconds.
function F.duration(ms)
    local total = math.max(0, math.floor((tonumber(ms) or 0) / 1000))
    local h, m, sec = math.floor(total / 3600), math.floor(total % 3600 / 60), total % 60
    if h > 0 then return string.format("%d:%02d:%02d", h, m, sec) end
    return string.format("%d:%02d", m, sec)
end

--- Sum of a Tokens table's 4 fields (input/output/cache_read/cache_write).
function F.tokenSum(t)
    t = t or {}
    return (t.input or 0) + (t.output or 0) + (t.cache_read or 0) + (t.cache_write or 0)
end

--- "$0.42", "<$0.01", "$0": an estimated cost, never a billed amount.
function F.cost(usd)
    local n = tonumber(usd) or 0
    if n <= 0 then return "$0" end
    if n < 0.01 then return "<$0.01" end
    return string.format("$%.2f", n)
end

--- "HH:MM" to minutes since midnight, or nil if not a valid time.
local function minutesOf(hhmm)
    local h, m = tostring(hhmm or ""):match("^(%d%d?):(%d%d)$")
    if not h then return nil end
    h, m = tonumber(h), tonumber(m)
    return (h % 24) * 60 + (m % 60)
end

--- Monday-first day index (0=Mon..6=Sun) and minutes since midnight, local time, for `epochMs` —
--- nil if the runtime's `os.date` is unavailable (some plugin sandboxes do not expose it: the
--- manual focus override in `F.isFocusActive` still works then, only the schedule cannot be
--- computed — see the README).
local function localClock(epochMs)
    if type(os) ~= "table" or type(os.date) ~= "function" then return nil end
    local ok, t = pcall(os.date, "*t", math.floor((tonumber(epochMs) or 0) / 1000))
    if not ok or type(t) ~= "table" or not t.wday then return nil end
    return (t.wday + 5) % 7, t.hour * 60 + t.min
end

--- Whether "focus mode" (a quiet, compact pet) is active right now: `settings.focusManual`
--- ("on"/"off") overrides everything; otherwise a configured schedule — `focusDays` (7 chars,
--- '1'/'0', Monday first) and `focusStart`/`focusEnd` ("HH:MM", which may wrap past midnight,
--- e.g. 22:00-06:00).
function F.isFocusActive(settings, nowMs)
    settings = settings or {}
    local manual = settings.focusManual or "auto"
    if manual == "on" then return true end
    if manual == "off" then return false end
    if not settings.focusEnabled then return false end
    local dayIdx, nowMin = localClock(nowMs)
    if not dayIdx then return false end
    local days = tostring(settings.focusDays or "1111100")
    local start = minutesOf(settings.focusStart)
    local stop = minutesOf(settings.focusEnd)
    if not start or not stop or start == stop then return false end
    if start < stop then
        return days:sub(dayIdx + 1, dayIdx + 1) == "1" and nowMin >= start and nowMin < stop
    end
    if nowMin >= start then return days:sub(dayIdx + 1, dayIdx + 1) == "1" end
    local previousDay = (dayIdx + 6) % 7
    return nowMin < stop and days:sub(previousDay + 1, previousDay + 1) == "1"
end

--- Shorten `text` to at most `n` characters, adding an ellipsis.
function F.clip(text, n)
    text = tostring(text or "")
    if utf8 and utf8.len then
        local len = utf8.len(text)
        if len and len > n then return text:sub(1, utf8.offset(text, n) - 1) .. "…" end
        return text
    end
    if #text > n then return text:sub(1, n - 1) .. "…" end
    return text
end

--- The session to describe: one that is working or waiting (most recent first),
--- otherwise the one with the most recent turn.
function F.activeSession(snap)
    local best, bestScore, bestEvent = nil, -1, -1
    for _, s in ipairs((snap and snap.sessions) or {}) do
        local busy = (s.status == "working" or s.status == "waiting")
        local score = busy and 2 or ((s.activity and s.activity.turn_started_ms) and 1 or 0)
        local ev = s.last_event_ms or 0
        if score > bestScore or (score == bestScore and ev > bestEvent) then
            best, bestScore, bestEvent = s, score, ev
        end
    end
    return best
end

--- Label of an agent ("Claude Code", "pi"...) from the snapshot's `agents`.
function F.agentLabel(snap, id)
    local a = snap and snap.agents and snap.agents[id]
    return (a and a.label) or tostring(id or "agent")
end

--- Plan limits ({ data, error }) of the agent that has them, or nil.
function F.limits(snap)
    for _, a in pairs((snap and snap.agents) or {}) do
        if a.limits then return a.limits end
    end
    return nil
end

--- Does any agent offer the built-in chat? (Assumed yes when the snapshot does not say.)
function F.canChat(snap)
    if not (snap and snap.agents) then return true end
    for _, a in pairs(snap.agents) do
        if a.capabilities and a.capabilities.chat then return true end
    end
    return false
end

--- How many things wait for you: the pending requests, plus the sessions blocked on their own
--- terminal with nothing to answer here (Antigravity asking, or a request whose hook gave up).
function F.needsYou(snap)
    local pending = (snap and snap.pending) or {}
    local asked = {}
    for _, p in ipairs(pending) do asked[p.session_id or ""] = true end
    local n = #pending
    for _, s in ipairs((snap and snap.sessions) or {}) do
        if s.status == "waiting" and not asked[s.id or ""] then n = n + 1 end
    end
    return n
end

-- ── Following a session's steps in the Live viewer ──

--- How long the typewriter takes to type a diff's added lines (as the viewer types it).
function F.typingMs(detail)
    if type(detail) ~= "table" or detail.type ~= "diff" then return 0 end
    local total = 0
    for _, l in ipairs(detail.lines or {}) do
        local text = l.text or ""
        if l.kind == "add" then total = total + (utf8 and utf8.len and utf8.len(text) or #text) end
    end
    return math.min(total * 22, 1500)
end

--- How long a step stays in the viewer before the next one replaces it: an edit until it is typed
--- out and a moment to read it, a command long enough to see it, a read or a search briefly.
function F.dwellMs(e)
    local d = e and e.detail
    local t = type(d) == "table" and d.type or nil
    if t == "diff" then return math.max(1800, F.typingMs(d) + 1200) end
    if t == "terminal" then return 1500 end
    if t == "file" then return 700 end
    return 400
end

local function worthStopping(e)
    local d = e and e.detail
    return type(d) == "table" and (d.type == "diff" or d.type == "terminal")
end

--- The step to show while following a session. Steps arrive faster than they can be read, so each
--- stays for `dwellMs` before the next; reads and searches with an edit or a command after them are
--- skipped, and it never lags more than three steps. `state` ({key, since}) is kept by the caller.
function F.followStep(state, sessionId, list, now)
    local last = #list
    if last == 0 then
        state.key = nil
        return nil
    end
    local function keyOf(e) return tostring(sessionId) .. ":" .. tostring(e.ts_ms) end
    local i = nil
    for j, e in ipairs(list) do
        if keyOf(e) == state.key then i = j end
    end
    if not i then
        i = last -- a new session, a new turn, or the step scrolled out of the list
    elseif i < last and now - (state.since or 0) >= F.dwellMs(list[i]) then
        i = i + 1
        while i < last and not worthStopping(list[i]) do i = i + 1 end
        i = math.max(i, last - 3)
    else
        return list[i]
    end
    state.key = keyOf(list[i])
    state.since = now
    return list[i]
end

--- More than one agent has a session right now.
function F.multiAgent(snap)
    local seen, n = {}, 0
    for _, s in ipairs((snap and snap.sessions) or {}) do
        if s.agent and not seen[s.agent] then seen[s.agent], n = true, n + 1 end
    end
    return n > 1
end

--- "Bash: cargo test": the last tool a session used, or nil.
function F.lastToolText(s)
    local t = s and s.last_tool
    if type(t) == "table" then return t.text end
    return nil
end

--- How long the current turn has been running (or took, once finished); nil if unknown.
function F.turnElapsed(s, nowMs)
    local a = s and s.activity
    if not a then return nil end
    if s.status ~= "idle" and a.turn_started_ms then return nowMs - a.turn_started_ms end
    return a.turn_ms
end

--- "3 files · +45 −12", or nil when nothing was changed this turn.
function F.changesText(a)
    if not a or (a.files_changed or 0) == 0 then return nil end
    local files = a.files_changed == 1 and "1 file" or (a.files_changed .. " files")
    return string.format("%s · +%d -%d", files, a.lines_added or 0, a.lines_removed or 0)
end

--- "4 run · 1 failed", or nil when no command ran.
function F.commandsText(a)
    if not a or (a.commands or 0) == 0 then return nil end
    local text = a.commands .. " run"
    if (a.failures or 0) > 0 then text = text .. " · " .. a.failures .. " failed" end
    return text
end

--- Tooltip rows describing the active session's progress.
function F.sessionRows(s, nowMs)
    if not s then return {} end
    local a = s.activity or {}
    local rows = {}
    local head = (s.status == "working") and "working" or (s.status == "waiting" and "waiting for you" or "idle")
    local elapsed = F.turnElapsed(s, nowMs)
    if elapsed then head = head .. (s.status == "idle" and " · last turn " or " · ") .. F.duration(elapsed) end
    if (a.tool_calls or 0) > 0 then head = head .. " · " .. a.tool_calls .. " tools" end
    rows[#rows + 1] = { key = s.name or s.id, value = head }
    local ch = F.changesText(a)
    if ch then
        local names = {}
        for i, f in ipairs(a.files or {}) do
            if i > 3 then break end
            names[#names + 1] = (f:match("([^/]+)$") or f)
        end
        rows[#rows + 1] = { key = "Changes", value = F.clip(ch .. " (" .. table.concat(names, ", ") .. ((a.files_changed or 0) > 3 and ", …" or "") .. ")", 80) }
    end
    local cm = F.commandsText(a)
    if cm then rows[#rows + 1] = { key = "Commands", value = cm } end
    local recent = a.recent or {}
    local last = recent[#recent]
    if last and s.status ~= "idle" then
        rows[#rows + 1] = { key = (last.ok == nil) and "Now" or "Last step", value = F.clip(last.tool .. " " .. (last.label or ""), 70) }
    end
    if s.context then
        rows[#rows + 1] = { key = "Context", value = F.round(s.context.percent) .. "% of ~" .. F.tokens(s.context.window) }
    end
    if a.last_result and (s.status == "idle" or not elapsed) then
        rows[#rows + 1] = { key = "Result", value = F.clip(a.last_result, 90) }
    end
    return rows
end

return F
