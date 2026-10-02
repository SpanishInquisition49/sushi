-- Shared helpers for the Lua tests. Run from the repository root:  luajit tests/lua/<name>.lua
-- The plugin's scripts run here against small stand-ins for Noctalia's `ui`, `noctalia` and
-- `panel` tables, so their logic can be tested without the shell.

local H = {}

H.ROOT = (arg and arg[0] and arg[0]:match("^(.*)/tests/lua/")) or "."
H.PLUGIN = H.ROOT .. "/plugin/"

local KINDS = { "row", "column", "label", "button", "glyph", "box", "separator", "scroll", "spacer", "graph", "progress", "input", "select" }

--- A `ui` table whose constructors just record what they were given.
function H.makeUi()
    local ui = {}
    for _, k in ipairs(KINDS) do
        ui[k] = function(p, c) return { k = k, p = p or {}, c = c or {} } end
    end
    return ui
end

-- Tree helpers ---------------------------------------------------------------------------------

function H.find(n, pred, out)
    out = out or {}
    if type(n) ~= "table" then return out end
    if pred(n) then out[#out + 1] = n end
    for _, c in ipairs(n.c or {}) do H.find(c, pred, out) end
    return out
end

--- Every label's text joined with " | ".
function H.texts(n)
    local t = {}
    for _, x in ipairs(H.find(n, function(m) return m.k == "label" and m.p.text end)) do t[#t + 1] = x.p.text end
    return table.concat(t, " | ")
end

function H.byKey(tree, prefix)
    return H.find(tree, function(n) return type(n.p.key) == "string" and n.p.key:sub(1, #prefix) == prefix end)
end

function H.button(tree, text)
    return H.find(tree, function(n) return n.k == "button" and n.p.text == text end)[1]
end

--- A tiny JSON encoder (objects with string values, arrays of those) for the stand-in `noctalia.json`.
function H.encode(v)
    if type(v) == "table" then
        if #v > 0 then
            local o = {}
            for _, x in ipairs(v) do o[#o + 1] = H.encode(x) end
            return "[" .. table.concat(o, ",") .. "]"
        end
        local keys = {}
        for k in pairs(v) do keys[#keys + 1] = k end
        table.sort(keys)
        local o = {}
        for _, k in ipairs(keys) do o[#o + 1] = string.format("%q:%s", k, H.encode(v[k])) end
        return "{" .. table.concat(o, ",") .. "}"
    elseif type(v) == "string" then
        return string.format("%q", v)
    end
    return tostring(v)
end

--- A small JSON decoder (enough for the daemon's state.json).
function H.decode(s)
    local i = 1
    local function ws() i = s:find("%S", i) or #s + 1 end
    local val
    local function str()
        local j, out = i + 1, {}
        while true do
            local c = s:sub(j, j)
            if c == '"' then i = j + 1; return table.concat(out) end
            if c == "\\" then
                local n = s:sub(j + 1, j + 1)
                out[#out + 1] = ({ n = "\n", t = "\t" })[n] or n; j = j + 2
            else out[#out + 1] = c; j = j + 1 end
        end
    end
    function val()
        ws()
        local c = s:sub(i, i)
        if c == "{" then
            i = i + 1; local t = {}; ws()
            if s:sub(i, i) == "}" then i = i + 1; return t end
            while true do
                ws(); local k = str(); ws(); i = i + 1; t[k] = val(); ws()
                local d = s:sub(i, i); i = i + 1
                if d == "}" then return t end
            end
        elseif c == "[" then
            i = i + 1; local t = {}; ws()
            if s:sub(i, i) == "]" then i = i + 1; return t end
            while true do
                t[#t + 1] = val(); ws()
                local d = s:sub(i, i); i = i + 1
                if d == "]" then return t end
            end
        elseif c == '"' then return str()
        else
            local lit = s:match("^[%w%.%-%+]+", i); i = i + #lit
            if lit == "true" then return true elseif lit == "false" then return false elseif lit == "null" then return nil end
            return tonumber(lit)
        end
    end
    return val()
end

-- A stand-in runtime -------------------------------------------------------------------------

--- Load plugin/<entry> in a fresh environment. Returns an object with:
---   env      the global table of the script (call env.onOpen(), env.update(), ...)
---   state    noctalia.state's backing table
---   calls    every runAsync argv, joined with spaces (calls[i]); `runs[i]` has { argv, cb }
---   out()    the last tree handed to panel.render / barWidget.render
function H.load(entry, cfg)
    cfg = cfg or {}
    local state, watchers, rendered = {}, {}, nil
    local calls, runs = {}, {}
    local files, tooltip, intervals = {}, nil, {}
    local sounds = { loads = {}, plays = {}, callbacks = {} }
    local env = setmetatable({}, { __index = _G })
    env.NOW = 10000000
    env.ui = H.makeUi()
    env.noctalia = {
        getConfig = function(k) return cfg[k] end,
        nowMs = function() return env.NOW end,
        expandPath = function(p) return p end,
        pluginDir = function() return H.PLUGIN:gsub("/$", "") end,
        log = function() end,
        setUpdateInterval = function(ms) intervals[#intervals + 1] = ms end,
        readFile = function(p) return files[p] end,
        sound = {
            load = function(name, path, cb) sounds.loads[#sounds.loads + 1] = name; sounds.callbacks[#sounds.callbacks + 1] = cb end,
            play = function(name) sounds.plays[#sounds.plays + 1] = name end,
        },
        togglePanel = function() end,
        getenv = function() return "/run" end,
        runAsync = function(argv, cb)
            calls[#calls + 1] = table.concat(argv, " ")
            runs[#runs + 1] = { argv = argv, cb = cb }
        end,
        json = { encode = H.encode, decode = H.decode },
        state = {
            set = function(k, v) state[k] = v; for _, w in ipairs(watchers[k] or {}) do w(v) end end,
            -- a key that was never stored may read back as an empty string
            get = function(k) if state[k] == nil and k:find("^selectedSession") then return "" end return state[k] end,
            watch = function(k, f) watchers[k] = watchers[k] or {}; table.insert(watchers[k], f) end,
        },
    }
    env.panel = {
        render = function(t) rendered = t end,
        close = function() calls[#calls + 1] = "PANEL_CLOSE" end,
        setNeedsFrameTick = function() end,
    }
    env.barWidget = {
        isVertical = function() return false end,
        render = function(t) rendered = t end,
        setTooltip = function(t) tooltip = t end,
    }
    local cache = {}
    env.require = function(path)
        local rel = path:gsub("^%./", "")
        if not cache[rel] then
            local f = assert(loadfile(H.PLUGIN .. rel)); setfenv(f, env); cache[rel] = f()
        end
        return cache[rel]
    end
    local f = assert(loadfile(H.PLUGIN .. entry)); setfenv(f, env); f()
    return {
        env = env, state = state, calls = calls, runs = runs, files = files, sounds = sounds, intervals = intervals,
        out = function() return rendered end, tooltip = function() return tooltip end,
    }
end

-- Sample data ---------------------------------------------------------------------------------

H.NOW = 10000000

H.DIFF = { type = "diff", file = "src/invoice.ts", lang = "ts", more = 0, lines = {
    { n = 11, kind = "ctx", text = "" }, { n = 12, kind = "del", text = "const TVA = 0.196" },
    { n = 12, kind = "add", text = "const TVA = 0.2" }, { n = 13, kind = "ctx", text = "export function total() {" } } }
H.TERM = { type = "terminal", command = "npm test", output = { "PASS a", "2 passed" } }
H.FILE = { type = "file", file = "src/types.ts", lang = "ts", start = 1, lines = { "export interface Item {", "}" } }

function H.session(id, name, status, ev, steps, extra)
    local s = { id = id, agent = "claude", name = name, status = status, last_event_ms = ev, cwd = "/home/x/" .. name,
        context = { percent = 40, window = 1000000, tokens = 400000, history = { 10, 20, 30, 40 } },
        activity = { turn_started_ms = 9900000, tool_calls = #steps, files_changed = 1, files = { "src/invoice.ts" },
            lines_added = 1, lines_removed = 1, commands = 1, failures = 0, recent = steps } }
    for k, v in pairs(extra or {}) do s.activity[k] = v end
    return s
end

function H.steps(now)
    return {
        { ts_ms = now - 4000, tool = "Read", kind = "read", label = "src/types.ts", ok = true, added = 0, removed = 0, detail = H.FILE },
        { ts_ms = now - 2000, tool = "Edit", kind = "write", label = "src/invoice.ts", ok = nil, added = 1, removed = 1, detail = H.DIFF },
    }
end

H.LIMITS = { data = { five_hour = { percent = 44, resets_at_ms = H.NOW + 9000000 },
    seven_day = { percent = 52, resets_at_ms = H.NOW + 99000000 }, models = {} } }


--- What the daemon says about the agents: Claude Code can do everything, pi only shows sessions.
H.AGENTS = {
    claude = { label = "Claude Code", limits = H.LIMITS,
        capabilities = { approve = true, questions = true, plans = true, context = true, limits = true, chat = true } },
    copilot = { label = "GitHub Copilot", capabilities = { approve = true, questions = false, plans = false, context = false, limits = false, chat = true } },
    pi = { label = "pi", capabilities = { approve = true, questions = false, plans = false, context = false, limits = false, chat = true } },
    opencode = { label = "opencode", capabilities = { approve = true, questions = false, plans = false, context = false, limits = false, chat = false } },
}

--- A `last_tool` as the daemon publishes it, from "Bash: cargo test" and a kind.
function H.tool(text, kind)
    local name, detail = text:match("^([^:]+):%s*(.*)$")
    return { name = name or text, kind = kind, detail = detail, text = text }
end

return H
