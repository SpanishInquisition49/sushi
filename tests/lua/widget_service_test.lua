-- Tests of the bar widget (plugin/bar_widget.luau) and of the background service (plugin/service.luau).
package.path = (arg[0]:match("^(.*)/") or ".") .. "/?.lua;" .. package.path
local H = require("helpers")
local find, texts, byKey = H.find, H.texts, H.byKey
local NOW = H.NOW

local function working(extra)
    local s = H.session("s1", "my-project", "working", 9999000, {
        { tool = "Edit", kind = "write", label = "src/lib.rs", ok = nil, ts_ms = NOW - 1000 } },
        { turn_started_ms = NOW - 133000, tool_calls = 17, files_changed = 3, files = { "src/lib.rs", "src/main.rs", "NOTES.md" },
          lines_added = 45, lines_removed = 12, commands = 4, failures = 1, last_result = "Refactored the parser." })
    s.last_tool = H.tool("Edit: src/lib.rs", "write")
    return s
end

local function widget(cfg, snap, up)
    local t = H.load("bar_widget.luau", cfg)
    t.env.noctalia.state.set("daemonUp", up ~= false)
    t.env.noctalia.state.set("snapshot", snap)
    t.env.update()
    return t
end

local snap = { sessions = { working(), H.session("s2", "other", "idle", 5, {}) }, pending = {}, agents = H.AGENTS }

-- ── what the widget shows next to the pet ─────────────────────────────────────────────────
do
    local usage = texts(widget({ widgetInfo = "usage" }, snap).out())
    assert(usage == "44%", "usage only: " .. usage)
    local session = widget({ widgetInfo = "session" }, snap)
    local tx = texts(session.out())
    assert(tx:find("2:13", 1, true) and tx:find("3", 1, true) and tx:find("44%", 1, true), "session: " .. tx)
    assert(#find(session.out(), function(n) return n.k == "glyph" and n.p.name == "pencil" end) == 1, "a pencil for the files")
    assert(not tx:find("+45", 1, true), "no line counts in the default view")
    local detailed = widget({ widgetInfo = "detailed" }, snap)
    tx = texts(detailed.out())
    assert(tx:find("+45", 1, true) and tx:find("-12", 1, true), "detailed: lines: " .. tx)
    assert(#find(detailed.out(), function(n) return n.k == "glyph" and n.p.name == "terminal-2" end) == 1, "commands")
    assert(#find(detailed.out(), function(n) return n.k == "glyph" and n.p.name == "alert-triangle" end) == 1, "failures")
    assert(widget({}, snap).out() and texts(widget({}, snap).out()):find("2:13", 1, true), "the default is the session view")

    local idle = widget({ widgetInfo = "detailed" }, { sessions = { H.session("s2", "other", "idle", 5, {}) }, pending = {}, agents = H.AGENTS })
    assert(texts(idle.out()) == "44%", "nothing but the percentage when idle")
    local ask = widget({}, { sessions = snap.sessions, pending = { { id = 1 } }, agents = H.AGENTS })
    assert(texts(ask.out()):find("!1", 1, true), "a pending request shows !1")
    local off = widget({}, snap, false)
    assert(texts(off.out()) == "", "the daemon is down: only the pet")
    print("widget: usage / session / detailed views, idle, pending request, daemon down")
end

-- ── the tooltip ────────────────────────────────────────────────────────────────────────────
do
    local t = widget({}, snap)
    local rows = {}
    for _, r in ipairs(t.tooltip()) do rows[#rows + 1] = r.key .. "=" .. r.value end
    local all = table.concat(rows, "\n")
    assert(all:find("my%-project=working · 2:13 · 17 tools"), all)
    assert(all:find("Changes=3 files · +45 -12", 1, true) and all:find("Commands=4 run · 1 failed", 1, true), all)
    assert(all:find("5-hour limit=44%", 1, true) and all:find("other=idle", 1, true), all)
    local down = widget({}, snap, false)
    assert(down.tooltip()[1].value:find("not running", 1, true), "the tooltip says the daemon is down")
    print("tooltip: progress of the active session, limits, other sessions")
end

-- ── right click, sounds ────────────────────────────────────────────────────────────────────
do
    local pet = widget({}, snap)
    pet.env.onRightClick()
    assert(pet.state.petEvent.kind == "love", "right click pets it")
    -- the widget owns the speakers: it loads the sounds and plays the greeting once ready
    local t = widget({}, snap)
    t.env.update(); t.env.NOW = NOW + 100; t.env.update()
    assert(#t.sounds.loads >= 1, "sounds start loading")
    while #t.sounds.callbacks > 0 do table.remove(t.sounds.callbacks, 1)(true) end
    for _ = 1, 20 do t.env.NOW = t.env.NOW + 66; t.env.update() end
    assert(#t.sounds.plays >= 1 and t.sounds.plays[1] == "greeting", "the greeting is played: " .. table.concat(t.sounds.plays, ","))
    local quiet = widget({ sounds = false }, snap)
    for _ = 1, 20 do quiet.env.NOW = quiet.env.NOW + 66; quiet.env.update() end
    assert(#quiet.sounds.loads == 0 and #quiet.sounds.plays == 0, "sounds can be switched off")
    print("widget: right click pets, the greeting plays once loaded, sounds can be switched off")
end

-- ── the service ────────────────────────────────────────────────────────────────────────────
do
    local PATH = "/run/sushi/state.json"
    local function body(extra, updated)
        return '{"updated_ms":' .. (updated or NOW) .. ',"sessions":[],"pending":[]' .. (extra or "") .. "}"
    end
    local t = H.load("service.luau")
    local e = t.env

    t.files[PATH] = body(',"chat":{"busy":true}'); e.update()
    t.files[PATH] = body(',"chat":{"busy":false}', NOW + 1); e.update()
    local iv = t.intervals
    assert(iv[#iv - 1] == 250 and iv[#iv] == 1000, "250 ms while the chat streams, then back to 1 s: " .. table.concat(iv, ","))
    t.files[PATH] = '{"updated_ms":' .. (NOW + 2) .. ',"sessions":[{"id":"a","status":"working"}],"pending":[]}'; e.update()
    assert(iv[#iv] == 250, "250 ms while a session works: " .. table.concat(iv, ","))
    t.files[PATH] = '{"updated_ms":' .. (NOW + 3) .. ',"sessions":[{"id":"a","status":"idle"}],"pending":[]}'; e.update()
    assert(iv[#iv] == 1000, "1 s when everything is idle")

    assert(t.state.daemonUp == true and type(t.state.snapshot) == "table", "publishes the snapshot")
    t.files[PATH] = body("", NOW - 60000); e.update()
    assert(t.state.daemonUp == false, "an old file means the daemon is gone")
    t.files[PATH] = nil; e.update()
    assert(t.state.daemonUp == false, "no file, no daemon")

    -- a pending request opens the panel, but not on the very first read after loading
    local t2 = H.load("service.luau")
    t2.files[PATH] = body(',"pending":[]'); t2.env.update()
    t2.files[PATH] = '{"updated_ms":' .. (NOW + 1) .. ',"sessions":[],"pending":[{"id":1,"created_ms":5}]}'; t2.env.update()
    local opened = false
    for _, c in ipairs(t2.calls) do if c:find("panel-open", 1, true) then opened = true end end
    assert(opened, "a new request opens the panel: " .. table.concat(t2.calls, " ; "))
    local before = #t2.calls
    t2.files[PATH] = '{"updated_ms":' .. (NOW + 2) .. ',"sessions":[],"pending":[{"id":1,"created_ms":5}]}'; t2.env.update()
    assert(#t2.calls == before, "the same request does not open it again")
    local t3 = H.load("service.luau", { autoOpen = false })
    t3.files[PATH] = body(); t3.env.update()
    t3.files[PATH] = '{"updated_ms":' .. (NOW + 1) .. ',"sessions":[],"pending":[{"id":9,"created_ms":6}]}'; t3.env.update()
    assert(#t3.calls == 0, "auto-open can be switched off")

    -- commands from outside
    e.onIpc("tab", "chat"); assert(t.state.panelTab.tab == "chat")
    e.onIpc("tab", "nonsense"); assert(t.state.panelTab.tab == "chat", "unknown tabs are ignored")
    e.onIpc("emote", "dance"); assert(t.state.petEvent.kind == "dance")
    t.state.petEvent = nil; e.onIpc("emote", "explode"); assert(t.state.petEvent == nil, "unknown emotes are ignored")
    print("service: polling speed, daemon down, auto-open of the panel, tab and emote commands")
end

-- ── feeding a file ────────────────────────────────────────────────────────────────────────
do
    local s = H.load("service.luau")
    s.env.onIpc("feed", "/home/x/My%20Notes.md")
    assert(s.state.fedFile and s.state.fedFile.path == "/home/x/My%20Notes.md", "a plain path is taken as it is")
    s.env.onIpc("feed", "file:///home/x/My%20Notes.md")
    assert(s.state.fedFile.path == "/home/x/My Notes.md" and s.state.fedFile.name == "My Notes.md", "file:// URIs are decoded")
    assert(s.state.petEvent.kind == "eat" and s.state.panelTab.tab == "chat", "the pet eats it, the chat opens")
    assert(s.calls[#s.calls]:find("panel-open scanna/sushi:panel", 1, true), "and the panel with it")
    local n = #s.calls; s.state.fedFile = nil
    s.env.onIpc("feed", "not a path"); s.env.onIpc("feed", nil)
    assert(#s.calls == n and s.state.fedFile == nil, "nothing to feed: nothing happens")

    -- middle click: the file copied in the file manager (wl-paste), else the plain text, else a sad pet
    local w = widget({}, snap)
    w.env.onMiddleClick()
    assert(w.calls[#w.calls] == "wl-paste --no-newline --type text/uri-list", w.calls[#w.calls])
    w.runs[#w.runs].cb({ exitCode = 0, stdout = "# copied\r\nfile://host/tmp/a%2Bb.rs\r\nfile:///tmp/other\r\n" })
    assert(w.state.fedFile.path == "/tmp/a+b.rs" and w.state.petEvent.kind == "eat", "the first file of the uri-list")
    w.env.onMiddleClick()
    w.runs[#w.runs].cb({ exitCode = 1, stdout = "", stderr = "No suitable type of content copied" })
    assert(w.calls[#w.calls] == "wl-paste --no-newline --type text/plain", "falls back to plain text")
    w.runs[#w.runs].cb({ exitCode = 0, stdout = "hello there" })
    assert(w.state.petEvent.kind == "sad" and w.state.fedFile.path == "/tmp/a+b.rs", "no file: the pet is sad, nothing changes")
    print("feed: IPC paths and URIs, middle click from the clipboard, nothing to eat")
end
