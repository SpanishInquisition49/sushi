-- Tests of the pet (lib/pet.luau), its sounds (lib/sounds.luau) and the highlighter.
package.path = (arg[0]:match("^(.*)/") or ".") .. "/?.lua;" .. package.path
local H = require("helpers")

local function loadMod(name, ui)
    local f = assert(loadfile(H.PLUGIN .. "lib/" .. name .. ".luau")); local m = f()
    if ui and m.setUi then m.setUi(ui) end
    return m
end
local ui = H.makeUi()
local Pet = loadMod("pet", ui)
local Sounds = loadMod("sounds")
local High = loadMod("highlight")

local function new(cfg, now)
    local p = Pet.new(now or 1e6, cfg or { character = "tofu", fidgets = false, napAfterSec = 0 })
    p.override = nil
    return p
end
local function run(pet, from, ms) local t = from; for _ = 1, math.floor(ms / 33) do t = t + 33; pet:tick(t) end return t end

-- every node must have sane integer sizes
local function check(n, path)
    if type(n) ~= "table" then return end
    for _, key in ipairs({ "width", "height" }) do
        local v = n.p[key]
        if v ~= nil then assert(type(v) == "number" and v == v and v >= 0 and v == math.floor(v), path .. "." .. key .. "=" .. tostring(v)) end
    end
    if n.p.radius then assert(n.p.radius >= 0 and n.p.radius == math.floor(n.p.radius), path .. ".radius") end
    for i, c in ipairs(n.c) do check(c, path .. "/" .. n.k .. i) end
end

-- ── configuration ──────────────────────────────────────────────────────────────────────────
do
    local cfg = Pet.configFrom(function() return nil end)
    assert(cfg.character == "nigiri_salmon" and cfg.fidgets == true and cfg.napAfterSec == 120)
    cfg = Pet.configFrom(function(k) return ({ character = "bogus", fidgets = false, napAfterSec = 0 })[k] end)
    assert(cfg.character == "nigiri_salmon" and cfg.fidgets == false and cfg.napAfterSec == 0, "an unknown character falls back")
    assert(#Pet.CHARACTER_IDS == 30)
    print("config: defaults and validation; 30 characters")
end

-- ── every character in every mood, at both sizes ───────────────────────────────────────────
do
    local moods = { "idle", "working", "alert", "sleep", "nap", "worried", "stuffed", "happy", "love", "sad", "annoyed", "dizzy", "startled",
        "wave", "greeting", "eat", "dance", "hop", "wiggle", "yawn", "peek", "squish", "wink", "blush", "sneeze", "hum", "think",
        "spin", "bounce", "sip", "hiccup", "jiggle", "budget_alert", "milestone", "policy_match", "grew_up" }
    local frames = 0
    for _, c in ipairs(Pet.CHARACTER_IDS) do
        for _, m in ipairs(moods) do
            local now = 1e6
            local pet = new({ character = c, fidgets = false, napAfterSec = 0 }, now)
            for _, wk in ipairs({ "read", "write", "run", "search", "think" }) do
                pet.workKind = wk
                for _ = 1, 10 do
                    now = now + 80
                    pet.override = { mood = m, startMs = now - 300, untilMs = now + 700 }
                    pet:tick(now)
                    check(pet:build(26, { room = 4, margin = 3, maxHeight = 30 }), c .. "/" .. m)
                    check(pet:build(92, { room = 24, maxHeight = 120, margin = 20, badge = "work", badgeSlot = true }), c .. "/" .. m)
                    frames = frames + 2
                end
                if m ~= "working" then break end
            end
        end
    end
    print("rendering: " .. frames .. " frames, " .. #Pet.CHARACTER_IDS .. " characters x " .. #moods .. " moods, all sizes valid")
end

-- ── the badge slot keeps its kind: Noctalia would otherwise give the bubble's nodes (blue fill,
-- dimmed dots) to the body when the bubble goes ───────────────────────────────────────────────
do
    local pet = new({ character = "ikura", fidgets = false, napAfterSec = 0 })
    local function slot(badge)
        local row = pet:build(92, { room = 20, maxHeight = 124, margin = 20, badge = badge, badgeSlot = true }).c[2]
        local kinds = {}
        for i, n in ipairs(row.c) do kinds[i] = n.k end
        return row.c[1], table.concat(kinds, ",")
    end
    local work, workKinds = slot("work")
    local none, noneKinds = slot(nil)
    assert(workKinds == noneKinds, "the frame row changes shape with the badge: " .. workKinds .. " vs " .. noneKinds)
    assert(work.p.key == "badge" and none.p.key == "badge", "the badge slot is keyed")
    assert(work.p.width == none.p.width, "the badge slot keeps its width")
    print("badge slot: same kind, key and width with and without the bubble")
end

-- ── gamification accessories: pins stack in the same keyed slot, sizes stay sane ───────────
do
    local pet = new({ character = "ikura", fidgets = false, napAfterSec = 0, gamification = true })
    pet.streakBadge, pet.stepsBadge = 3, 5 -- gold streak, diamond steps
    pet.growthTier, pet.equipped = 2, "bow" -- tamagotchi care: a growth pin and an equipped accessory
    local tree = pet:build(92, { room = 20, maxHeight = 124, margin = 20, badge = "work", badgeSlot = true, accessories = true })
    check(tree, "accessories")
    local slot = tree.c[2].c[1]
    assert(slot.p.key == "badge" and #slot.c == 5, "status bubble + streak + steps + growth + accessory pins: " .. #slot.c)
    -- Without opts.accessories the same pet shows no pins, same key and width.
    local plain = pet:build(92, { room = 20, maxHeight = 124, margin = 20, badge = "work", badgeSlot = true })
    local plainSlot = plain.c[2].c[1]
    assert(plainSlot.p.key == "badge" and #plainSlot.c == 1 and plainSlot.p.width == slot.p.width, "accessories are opt-in, the slot stays the same size")
    print("accessories: gamification + growth + equipped-accessory pins stack in the badge slot, sizes stay sane")
end

-- ── height budgets: the bar and the panel hero never overflow ──────────────────────────────
do
    for _, c in ipairs(Pet.CHARACTER_IDS) do
        local pet = new({ character = c, fidgets = true, napAfterSec = 0 })
        local now, maxBar, maxHero = 1e6, 0, 0
        for step = 1, 600 do
            now = now + 33; pet:tick(now)
            if step % 40 == 0 then pet:onEvent("love", step, now) end
            maxBar = math.max(maxBar, pet:build(26, { room = 4, margin = 3, maxHeight = 30 }).p.height)
            maxHero = math.max(maxHero, pet:build(92, { room = 24, maxHeight = 120 }).p.height)
        end
        assert(maxBar <= 30 and maxHero <= 120, c .. " overflows: " .. maxBar .. " / " .. maxHero)
    end
    print("heights: every character fits 30 px in the bar and 120 px in the panel")
end

-- ── what drives the mood ───────────────────────────────────────────────────────────────────
do
    local function snapWith(tool, ctx, extra)
        local kinds = { Read = "read", Edit = "write", Write = "write", Bash = "run", WebSearch = "search", WebFetch = "search",
            Task = "think", Grep = "read", Mystery = "other" }
        local lt = tool and { name = tool:match("^[%w_]+"), kind = kinds[tool:match("^[%w_]+")] or "other", text = tool }
        local sessions = { { id = "a", status = "working", last_event_ms = 10, last_tool = lt, context = ctx and { percent = ctx } or nil } }
        for _, id in ipairs(extra or {}) do sessions[#sessions + 1] = { id = id, status = "idle" } end
        return { sessions = sessions, pending = {} }
    end
    local pet = new({ character = "ramen", fidgets = false, napAfterSec = 0 })
    for tool, kind in pairs({ ["Read: /x"] = "read", ["Edit: /y"] = "write", ["Write: z"] = "write", ["Bash: cargo test"] = "run",
        ["WebSearch"] = "search", ["WebFetch: http"] = "search", ["Task: sub"] = "think", ["Grep: foo"] = "read", ["Mystery: ?"] = "read" }) do
        pet:onSnapshot(snapWith(tool), true, 1e6)
        assert(pet.workKind == kind, tool .. " -> " .. pet.workKind)
    end
    pet:onSnapshot(snapWith("Read: x", 95), true, 1e6); assert(pet.baseMood == "stuffed")
    pet:onSnapshot(snapWith("Read: x", 50), true, 1e6); assert(pet.baseMood == "working")
    local hot = snapWith("Read: x", 95); hot.agents = { claude = { limits = { data = { five_hour = { percent = 95 } } } } }
    pet:onSnapshot(hot, true, 1e6); assert(pet.baseMood == "worried", "limits beat the context")
    hot.pending = { { id = 1 } }; pet:onSnapshot(hot, true, 1e6); assert(pet.baseMood == "alert", "a request beats everything")
    -- an agent asking in its own terminal (Antigravity) has no request here, but still needs you
    local asking = { sessions = { { id = "antigravity:c1", status = "waiting", last_event_ms = 1 } }, pending = {} }
    pet:onSnapshot(asking, true, 1e6); assert(pet.baseMood == "alert", "a waiting session calls you")
    assert(pet:caption(1, 0, 1) == "I need you!")

    -- the service says "daemon up" a moment before it sends the sessions: they are not newcomers
    local p2 = new()
    p2:onSnapshot({ sessions = {}, pending = {} }, true, 1e6)
    p2:onSnapshot(snapWith("Read: x", nil, { "b" }), true, 1e6 + 300)
    assert(not p2.override or p2.override.mood ~= "wave", "no wave for sessions that were already there")
    p2:onSnapshot(snapWith("Read: x", nil, { "b", "c" }), true, 1e6 + 6000)
    assert(p2.override and p2.override.mood == "wave", "a session that starts later gets a wave")
    -- the same after the daemon went away and came back
    p2.override = nil
    p2:onSnapshot({ sessions = {}, pending = {} }, false, 1e6 + 7000)
    p2:onSnapshot({ sessions = {}, pending = {} }, true, 1e6 + 8000)
    p2:onSnapshot(snapWith("Read: x", nil, { "b" }), true, 1e6 + 8300)
    assert(not p2.override or p2.override.mood ~= "wave", "no wave after the daemon restarts")

    local p3 = new(); p3:onEvent("eat", 1, 2e6)
    assert(p3:mood(2e6) == "eat" and p3:mood(2e6 + 2700) == "happy" and p3:mood(2e6 + 3900) == "idle", "eat → happy → idle")
    local p4 = new(); p4:onEvent("nap", 5, 3e6); assert(p4:mood(3e6 + 10) == "nap", "a forced nap works even if auto-nap is off")
    p4:onSnapshot(snapWith("Read: x"), true, 3e6 + 20); assert(p4:mood(3e6 + 30) ~= "nap", "activity wakes it")
    local p5 = new(); p5:onEvent("dance", 10, 4e6); assert(p5:mood(4e6 + 5) == "dance")
    p5.override = nil; p5:onEvent("love", 10, 4e6 + 10); assert(p5.override == nil, "an old timestamp is ignored")

    local chat = new()
    chat:onSnapshot({ sessions = { { id = "a", status = "idle" } }, pending = {}, chat = { busy = true } }, true, 2e6)
    assert(chat.baseMood == "think", "thinks while the chat answers")
    chat:onSnapshot({ sessions = { { id = "a", status = "idle" } }, pending = {}, chat = { busy = false, messages = { {} } } }, true, 2e6 + 100)
    assert(chat.override and chat.override.mood == "happy", "happy when it answered")

    -- a failed tool worries it, but failures already there on the pet's first look don't count
    local function withFails(status, fails, extra)
        local s = { id = "a", status = status, activity = { failures = fails } }
        return { sessions = { s }, pending = extra and extra.pending or {} }
    end
    local p6 = new()
    p6:onSnapshot(withFails("working", 0), true, 5e6)
    assert(not p6.override or p6.override.mood ~= "worried", "no false alarm for failures already on the first look")
    p6:onSnapshot(withFails("working", 1), true, 5e6 + 300)
    assert(p6.override and p6.override.mood == "worried", "a failed tool worries it")
    -- three failures in the same turn overwhelm it instead
    local p7 = new()
    p7:onSnapshot(withFails("working", 0), true, 6e6)
    p7:onSnapshot(withFails("working", 3), true, 6e6 + 300)
    assert(p7.override and p7.override.mood == "dizzy", "a pile of failures overwhelms it")
    -- succeeding after a failure earns a bigger celebration
    local p8 = new()
    p8:onSnapshot(withFails("working", 0), true, 7e6)
    p8:onSnapshot(withFails("working", 1), true, 7e6 + 300)
    p8:onSnapshot(withFails("idle", 1), true, 7e6 + 600)
    assert(p8.override and p8.override.mood == "happy" and p8.override.untilMs - p8.override.startMs > 1400,
        "a bigger celebration after recovering from a failure")
    -- a pending question still comes first
    local p9 = new()
    p9:onSnapshot(withFails("working", 0), true, 8e6)
    p9:onSnapshot(withFails("waiting", 1, { pending = { { id = 1 } } }), true, 8e6 + 300)
    assert(p9.baseMood == "alert" and (not p9.override or p9.override.mood ~= "worried"), "a pending question beats a failure")
    print("mood: tool kinds, priorities, wave, eat chain, nap, events, chat, failures")
end

-- ── the daemon's one-shot event bus (budget alerts, milestones, policy flags) ───────────────
do
    local function snapWithEvents(events)
        return { sessions = { { id = "a", status = "idle" } }, pending = {}, events = events }
    end
    local p = new()
    -- Events already in the capped list on the very first snapshot are not replayed.
    p:onSnapshot(snapWithEvents({ { id = 1, kind = "budget_alert" } }), true, 1e6)
    assert(not p.override or p.override.mood ~= "budget_alert", "no replay on the first snapshot")
    assert(p.lastSeenEventId == 1)
    -- The same event seen again (unchanged list) does not retrigger.
    p:onSnapshot(snapWithEvents({ { id = 1, kind = "budget_alert" } }), true, 1e6 + 100)
    assert(not p.override or p.override.mood ~= "budget_alert", "an already-seen event id does not retrigger")
    -- A genuinely new event triggers its mood once.
    p:onSnapshot(snapWithEvents({ { id = 1, kind = "budget_alert" }, { id = 2, kind = "milestone" } }), true, 1e6 + 200)
    assert(p.override and p.override.mood == "milestone", "a new event id triggers its mood")
    assert(p.lastSeenEventId == 2)
    p:onSnapshot(snapWithEvents({ { id = 3, kind = "policy_match" } }), true, 1e6 + 300)
    assert(p.override and p.override.mood == "policy_match", "policy_match reacts too")
    -- A growth tier crossed (src/care.rs's "grew_up") is the same kind of one-shot event.
    p:onSnapshot(snapWithEvents({ { id = 4, kind = "grew_up" } }), true, 1e6 + 400)
    assert(p.override and p.override.mood == "grew_up", "grew_up reacts too")
    print("events: one-shot daemon events (budget_alert, milestone, policy_match, grew_up) fire once per id")
end

-- ── tamagotchi care: needs read from the snapshot, droopy fidgets when one runs low ─────────
do
    local function snapWithCare(care)
        return { sessions = { { id = "a", status = "idle" } }, pending = {}, care = care }
    end
    local p = new()
    p:onSnapshot(snapWithCare({ hunger = 80, energy = 60, affection = 90, growth_tier = 1, currency = 12, owned = { "bow" }, equipped = "bow" }), true, 1e6)
    assert(p.hunger == 80 and p.energy == 60 and p.affection == 90, "needs are read from the snapshot")
    assert(p.growthTier == 1 and p.currency == 12 and p.equipped == "bow", "growth/currency/equipped are read from the snapshot")

    -- Low hunger biases idle fidgets toward "yawn"/"think" (a visual cue only — never a punishment:
    -- the pet still idles normally, it just leans droopy more often).
    local needy = new({ character = "tofu", fidgets = true, napAfterSec = 0 }, 1e6)
    needy:onSnapshot(snapWithCare({ hunger = 10, energy = 100, affection = 100 }), true, 1e6)
    local full = new({ character = "tofu", fidgets = true, napAfterSec = 0 }, 2e6)
    full:onSnapshot(snapWithCare({ hunger = 100, energy = 100, affection = 100 }), true, 2e6)
    local needyDroopy, fullDroopy, now = 0, 0, 1e6
    for _ = 1, 400 do
        now = now + 50
        needy.nextFidget, full.nextFidget = now, now -- force a fidget pick on every tick
        needy:tick(now); full:tick(now)
        if needy.cur == "yawn" or needy.cur == "think" then needyDroopy = needyDroopy + 1 end
        if full.cur == "yawn" or full.cur == "think" then fullDroopy = fullDroopy + 1 end
    end
    assert(needyDroopy > fullDroopy * 1.3, "low hunger leans the fidget pool droopy: " .. needyDroopy .. " vs " .. fullDroopy)
    print("care: needs/growth/currency/equipped read from the snapshot; low needs lean fidgets droopy")
end

-- ── focus mode: sounds and fidgets are suppressed while focused ────────────────────────────
do
    local pet = new({ character = "tofu", fidgets = true, napAfterSec = 0 })
    pet:enableSounds(true)
    pet:setFocus(true)
    pet:onEvent("sad", 1, 1e6) -- a mood change is needed to re-evaluate the sound cue
    pet:tick(1e6 + 10)
    assert(#pet:drainSounds() == 0, "no sound while focused")
    pet:setFocus(false)
    pet:onEvent("love", 2, 2e6)
    pet:tick(2e6 + 10)
    assert(#pet:drainSounds() > 0, "sounds resume once focus ends")
    print("focus: mutes sounds while active")
end

-- ── personality: a few caption lines are flavored by character, the rest stays shared ──────
do
    local feisty = new({ character = "wasabi", fidgets = false, napAfterSec = 0 })
    feisty.cur = "happy"
    assert(feisty:caption(1, 0, 0) == "Ha! Done!", "wasabi (feisty) has its own happy line")

    local calm = new({ character = "tofu", fidgets = false, napAfterSec = 0 })
    calm.cur = "happy"
    assert(calm:caption(1, 0, 0) == "Finished, calmly.", "tofu (calm) has a different happy line")

    local classic = new({ character = "nigiri_salmon", fidgets = false, napAfterSec = 0 })
    classic.cur = "idle"
    assert(classic:caption(1, 0, 0) == "All quiet", "classic keeps the plain idle line")

    local sleepy = new({ character = "bao", fidgets = false, napAfterSec = 0 })
    sleepy.cur = "idle"
    assert(sleepy:caption(1, 0, 0) == "All quiet, zzz-ish", "bao (sleepy) flavors the idle line too")

    -- Moods outside the flavored set stay identical across personalities.
    feisty.cur, calm.cur = "worried", "worried"
    assert(feisty:caption(1, 0, 0) == calm:caption(1, 0, 0), "unflavored moods are shared verbatim")
    print("personality: flavored lines differ by character, everything else is shared")
end

-- ── idle quirks ────────────────────────────────────────────────────────────────────────────
do
    local function fidgets(char, on)
        local pt = new({ character = char, fidgets = on, napAfterSec = 0 })
        local t, seen = 1e6, {}
        pt:onSnapshot({ sessions = { { id = "a", status = "idle" } }, pending = {} }, true, t)
        for _ = 1, 30 * 60 * 40 do t = t + 33; pt:tick(t); seen[pt.cur] = true end
        return seen
    end
    local sk = fidgets("sake", true)
    for _, m in ipairs({ "hop", "wiggle", "yawn", "peek", "squish", "wink", "blush", "sneeze", "hum", "think", "spin", "bounce", "hiccup", "sip" }) do assert(sk[m], "sake never did " .. m) end
    assert(fidgets("takoyaki", true).jiggle and not fidgets("takoyaki", true).hiccup, "only some characters have their own quirks")
    local off = fidgets("tofu", false)
    for _, m in ipairs({ "hop", "wiggle", "yawn", "peek", "squish", "wink", "blush", "sneeze", "hum", "think", "spin", "bounce" }) do assert(not off[m], "quirk with fidgets off: " .. m) end
    print("quirks: all of them happen (sake also hiccups and sips); none when switched off")
end

-- ── the gaze ───────────────────────────────────────────────────────────────────────────────
do
    local pet = new(); local t = 1e6
    pet:lookAt(1, 0); t = run(pet, t, 1300); assert(pet.gx > 0.9 and math.abs(pet.gy) < 0.1, "looks right")
    pet:lookAt(0, 1); t = run(pet, t, 1300); assert(pet.gy > 0.9 and math.abs(pet.gx) < 0.1, "looks down")
    pet:lookAt(nil); pet.nextGlance = t + 1e9; pet.tx, pet.ty = 0, 0
    t = run(pet, t, 2000); assert(math.abs(pet.gx) < 0.1 and math.abs(pet.gy) < 0.1, "back to the centre")
    print("gaze: follows lookAt and relaxes")
end

-- ── sounds ─────────────────────────────────────────────────────────────────────────────────
do
    local names = {}
    for _, n in ipairs(Sounds.NAMES) do names[n] = true end
    assert(#Sounds.NAMES == 34)
    for n in pairs(names) do
        local f = io.open(H.PLUGIN .. "sounds/" .. n .. ".wav", "rb"); assert(f, "missing " .. n)
        assert(f:read(4) == "RIFF", n .. " is not a wav"); f:close()
    end

    local pet = Pet.new(1e6, { character = "tofu", fidgets = false, napAfterSec = 0 }) -- keeps its greeting
    pet:enableSounds(true)
    local t = run(pet, 1e6, 100)
    local d = pet:drainSounds(); assert(d[1] == "greeting" and #d == 1, "the greeting plays once: " .. table.concat(d, ","))
    pet.override = nil
    local quiet = new(); run(quiet, 1e6, 3000); assert(#quiet:drainSounds() == 0, "silent unless enabled")

    t = run(pet, t, 2500); pet:drainSounds()
    pet:poke(t); t = run(pet, t, 200); d = pet:drainSounds()
    assert(d[1] == "poke" and d[2] == "annoyed", "poke: " .. table.concat(d, ","))
    t = run(pet, t, 1500); pet:drainSounds()
    pet:onEvent("approve", 1, t); t = run(pet, t, 200); d = pet:drainSounds()
    assert(#d == 1 and d[1] == "approve", "approve has its own sound: " .. table.concat(d, ","))
    t = run(pet, t, 2500); pet:drainSounds()
    pet:onEvent("deny", 2, t); t = run(pet, t, 200); d = pet:drainSounds()
    assert(#d == 1 and d[1] == "deny", "deny has its own sound")
    t = run(pet, t, 2500); pet:drainSounds()
    pet:onEvent("love", 3, t); t = run(pet, t, 200); d = pet:drainSounds()
    assert(#d == 1 and d[1] == "love", "petting keeps the love sound")

    local napper = new(); napper:enableSounds(true); napper:onEvent("nap", 1, 1e6)
    local tn = run(napper, 1e6, 3000); napper:drainSounds()
    napper:onSnapshot({ sessions = { { id = "a", status = "working" } }, pending = {} }, true, tn); tn = run(napper, tn, 300)
    d = napper:drainSounds(); assert(d[1] == "wake" or d[2] == "wake", "waking up plays 'wake': " .. table.concat(d, ","))

    local a = new(); a:enableSounds(true); local ta = run(a, 1e6, 2500); a:drainSounds()
    a:onSnapshot({ sessions = { { id = "a", status = "idle" } }, pending = { { id = 1 } } }, true, ta); run(a, ta, 200)
    assert(a:drainSounds()[1] == "alert", "a request plays the alert")

    local all = {}
    for _, ch in ipairs(Pet.CHARACTER_IDS) do
        local p = new({ character = ch, fidgets = true, napAfterSec = 40 }); p:enableSounds(true)
        local tt = 1e6
        for step = 1, 30 * 60 * 20 do
            tt = tt + 33; p:tick(tt)
            if step % 2000 == 0 then p:poke(tt); p:onEvent("approve", step, tt) end
            for _, n in ipairs(p:drainSounds()) do all[n] = true end
        end
    end
    for n in pairs(all) do assert(names[n], "a sound cue without a file: " .. n) end
    local cap = new(); cap:enableSounds(true); for _ = 1, 40 do cap:cue("poke") end
    assert(#cap:drainSounds() == 12, "the queue is capped")

    -- the loader
    local loads, plays, waiting, inFlight, maxInFlight = {}, {}, {}, 0, 0
    local fake = { sound = {
        load = function(name, path, cb)
            inFlight = inFlight + 1; maxInFlight = math.max(maxInFlight, inFlight)
            loads[#loads + 1] = name; waiting[#waiting + 1] = cb
            assert(path:find("/sounds/" .. name .. ".wav", 1, true))
        end,
        play = function(name) plays[#plays + 1] = name end } }
    assert(Sounds.init(fake, "/plugin") == true)
    assert(#loads == 4, "loads in small groups")
    Sounds.play("hello"); assert(#plays == 0, "asked for before it is loaded")
    while #waiting > 0 do local cb = table.remove(waiting, 1); inFlight = inFlight - 1; cb(true) end
    assert(#loads == 34 and maxInFlight <= 4, "all 34 loaded, at most 4 at a time")
    assert(plays[1] == "hello", "the early request plays once loaded")
    Sounds.play("poke"); assert(plays[#plays] == "poke" and Sounds.isReady("poke"))
    assert(Sounds.init({}, "/x") == false, "no sound API: nothing happens")
    print("sounds: 34 files, cues for every moment, one source, capped queue, grouped loading")
end

-- ── syntax colors ──────────────────────────────────────────────────────────────────────────
do
    local function kinds(lang, line)
        local out = {}
        for _, tok in ipairs(High.tokens(lang, line)) do
            for name, color in pairs(High.COLORS) do if color == tok.color then out[#out + 1] = name .. ":" .. tok.text end end
        end
        return table.concat(out, "|")
    end
    assert(kinds("ts", "const TVA = 0.196 // tax"):find("comment:// tax", 1, true), "a comment after code")
    assert(kinds("ts", "const TVA = 0.196"):find("number:0.196", 1, true) and kinds("ts", "const TVA = 0.196"):find("keyword:const", 1, true))
    assert(kinds("rust", 'let x = foo("a\\"b", 42);'):find('string:"a\\"b"', 1, true), "escaped quotes stay inside the string")
    assert(kinds("python", "x = 1  # ok"):find("comment:# ok", 1, true))
    assert(kinds("json", '"name": "korus"'):find('key:"name"', 1, true))
    for _, l in ipairs({ "", "//", "a //", "-- x", "'", 'unterminated "s', "é à 日本語 'x'" }) do High.tokens("ts", l); High.tokens("lua", l); High.tokens("markdown", l) end
    print("highlight: keywords, numbers, strings, comments; odd input is safe")
end
