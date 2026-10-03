--!nonstrict
-- lib/pet.luau — the animated pet. Several original characters, one animation engine.
--
-- Noctalia has no canvas, so a character is a tree of flex boxes. Spacers of
-- computed width/height act as coordinates: they place the eyes inside their
-- sockets, the body inside its frame (jump / shake) and floating decorations
-- (z, hearts, ...) beside it. The caller advances time with pet:tick(nowMs) and
-- renders pet:build(width, opts) each frame.
--
-- Characters (setting "character"): nigiri_salmon, nigiri_tuna, nigiri_tamago,
--   unagi_nigiri, ebi_nigiri, ebi_tempura, ikura, maki, uramaki, inari, ravioli,
--   bao, takoyaki, ramen, miso_soup, edamame, onigiri, dorayaki, mochi, dango,
--   bubble_tea, sake, tofu, taiyaki, ramune, kakigori, wasabi, temaki, matcha,
--   tamagoyaki.
--
-- Moods
--   steady    idle · working · alert (permission asked) · sleep (daemon down)
--             nap (idle for a long time) · worried (plan usage >= 90%)
--             stuffed (a working session's context window is >= 90% full)
--   reaction  happy (a session finished) · love (approved / petted) · sad (denied)
--             annoyed / dizzy (poked) · startled (woken up) · wave (new session)
--             eat (fed) · dance
--   fidgets   hop · wiggle · yawn · peek · squish · wink · blush · sneeze · hum ·
--             think · spin · bounce (random, while idle), plus a few per character
--             (hiccup, sip, jiggle)
--
-- While "working" the eyes and a small icon follow the tool Claude is using
-- (read, write, run, search, think). Hot dishes steam; the sake bottle is tipsy.

local Pet = {}
Pet.__index = Pet

local ui -- injected by the entry script (Pet.setUi)

function Pet.setUi(u) ui = u end

-- ── Characters ────────────────────────────────────────────────────────────────

local INK = "#2b2023"
local SHINE = "#ffffff"
local CHEEK = "#ff9aa6"
local RICE = "#fbf6ec"
local RICE_EDGE = "#e3d8c4"
local NORI = "#26382c"
local SWEAT = "#8ecbff"
local HEART = "#ff6b81"
local INARI = "#d9923f"
local INARI_DARK = "#b87430"
local DORA_TOP = "#d99a45"
local DORA_BOT = "#c4853a"
local DORA_AN = "#6b2f2a"
local BATTER = "#e2a447"
local BATTER_DARK = "#bf7f2c"
local TAIL = "#f0603c"
local DOUGH = "#f6ebd4"
local DOUGH_FOLD = "#e3d0a8"
local DOUGH_EDGE = "#e6d6b5"
local SEARED = "#d99a3e"
local BAO = "#fffaf0"
local BAO_FOLD = "#eadcc4"
local BAMBOO = "#cc9a58"
local BAMBOO_DARK = "#a97a3c"
local BOWL_CREAM = "#f4eee0"
local BOWL_EDGE = "#e0d6c0"
local BOWL_RED = "#c8453a"
local NOODLE = "#f2cf63"
local TAKO = "#d8a24c"
local SAUCE = "#5b2e1b"
local MISO = "#c98d4b"
local LACQUER = "#d94a2b"
local GOLD = "#e8b64c"
local POD = "#8cc063"
local POD_DARK = "#5f9140"
local BEAN = "#b4dc82"
local MOCHI = "#fbe4ea"
local MOCHI_EDGE = "#f0c6d2"
local BERRY = "#e8465a"
local TEA = "#d9ac80"
local PEARL = "#3b2b25"
local CERAMIC = "#f1f4f8"
local CERAMIC_EDGE = "#ccd6e0"
local SAKE_BLUE = "#3d6fb6"
local CHEEK_HOT = "#ff6f85"
local WAFFLE = "#d9973f"
local WAFFLE_DARK = "#b5752c"
local GLASS = "#2f7cb8"
local GLASS_DARK = "#1f5c8c"
local MARBLE = "#eef8ff"
local ICE = "#eef6fb"
local ICE_SHADE = "#cfe9f5"
local WASABI_GREEN = "#7fae3a"
local WASABI_RED = "#d23c3c"
local MATCHA_GREEN = "#6fae52"
local MATCHA_FOAM = "#bfe3a8"

local CHARACTERS = {
    nigiri_salmon = { kind = "nigiri", ratio = 0.80, top = "#ff8a6b", fat = "#ffc9b5" },
    nigiri_tuna   = { kind = "nigiri", ratio = 0.80, top = "#d8454f", fat = "#f0868c" },
    nigiri_tamago = { kind = "nigiri", ratio = 0.80, top = "#ffcf4d", fat = "#ffe79a" },
    maki          = { kind = "maki",   ratio = 1.00, filling = "#ff8a6b", filling2 = "#8cc76f" },
    uramaki       = { kind = "uramaki", ratio = 1.00, filling = "#ff8a6b", filling2 = "#8cc76f", filling3 = "#ffd24a" },
    unagi_nigiri  = { kind = "nigiri", ratio = 0.80, top = "#6a3a22", fat = "#b36b34" },
    ebi_nigiri    = { kind = "nigiri", ratio = 0.80, top = "#ff9c66", fat = "#fff0e6", bands = "#fff0e6" },
    ebi_tempura   = { kind = "tempura", ratio = 0.70, steam = true },
    ikura         = { kind = "ikura",  ratio = 1.00, roe = "#ff7a3d" },
    inari         = { kind = "inari",  ratio = 0.85 },
    ravioli       = { kind = "ravioli", ratio = 0.80, steam = true },
    bao           = { kind = "bao",    ratio = 0.90, steam = true },
    takoyaki      = { kind = "takoyaki", ratio = 1.00, steam = true },
    ramen         = { kind = "ramen",  ratio = 0.95, steam = true },
    miso_soup     = { kind = "miso",   ratio = 0.85, steam = true },
    edamame       = { kind = "edamame", ratio = 0.80 },
    onigiri       = { kind = "onigiri", ratio = 0.92 },
    mochi         = { kind = "mochi",  ratio = 0.88 },
    bubble_tea    = { kind = "boba",   ratio = 1.35 },
    sake          = { kind = "sake",   ratio = 1.15, tipsy = true },
    dorayaki      = { kind = "dorayaki", ratio = 0.86 },
    dango         = { kind = "dango",  ratio = 1.60, top = "#ff9eb5", mid = "#f6f1ea", bottom = "#a8d58b" },
    tofu          = { kind = "tofu",   ratio = 1.00 },
    taiyaki       = { kind = "taiyaki", ratio = 0.72, batter = WAFFLE, batterDark = WAFFLE_DARK },
    ramune        = { kind = "ramune", ratio = 1.30, glass = GLASS, glassDark = GLASS_DARK },
    kakigori      = { kind = "kakigori", ratio = 1.05, syrup = BERRY },
    wasabi        = { kind = "wasabi", ratio = 0.90 },
    temaki        = { kind = "temaki", ratio = 1.42, filling = "#ff8a6b" },
    matcha        = { kind = "matcha", ratio = 0.88, steam = true },
    tamagoyaki    = { kind = "tamagoyaki", ratio = 0.95 },
}

Pet.CHARACTER_IDS = {
    "nigiri_salmon", "nigiri_tuna", "nigiri_tamago", "unagi_nigiri", "ebi_nigiri", "ebi_tempura",
    "ikura", "maki", "uramaki", "inari", "ravioli", "bao", "takoyaki", "ramen", "miso_soup",
    "edamame", "onigiri", "dorayaki", "mochi", "dango", "bubble_tea", "sake", "tofu",
    "taiyaki", "ramune", "kakigori", "wasabi", "temaki", "matcha", "tamagoyaki",
}
local DEFAULT_CHARACTER = "nigiri_salmon"

-- ── Helpers ───────────────────────────────────────────────────────────────────

local function round(x) return math.floor(x + 0.5) end
local function clamp(x, a, b) return math.max(a, math.min(b, x)) end
local function rand(a, b) return a + math.random() * (b - a) end

-- breath period (ms), eye height scale, eye width scale
local MOOD_PARAMS = {
    idle     = { period = 3200, eyeH = 1.00, eyeW = 1.00 },
    working  = { period = 1500, eyeH = 0.95, eyeW = 1.00 },
    alert    = { period = 900,  eyeH = 1.20, eyeW = 1.15 },
    sleep    = { period = 4200, eyeH = 0.00, eyeW = 1.00 },
    nap      = { period = 5200, eyeH = 0.00, eyeW = 1.00 },
    worried  = { period = 1100, eyeH = 1.10, eyeW = 0.90 },
    stuffed  = { period = 1400, eyeH = 0.55, eyeW = 1.00 },
    happy    = { period = 700,  eyeH = 0.38, eyeW = 1.10 },
    love     = { period = 700,  eyeH = 0.38, eyeW = 1.10 },
    sad      = { period = 2400, eyeH = 0.85, eyeW = 0.95 },
    annoyed  = { period = 1200, eyeH = 0.42, eyeW = 1.10 },
    dizzy    = { period = 600,  eyeH = 1.00, eyeW = 1.00 },
    startled = { period = 500,  eyeH = 1.35, eyeW = 1.25 },
    wave     = { period = 600,  eyeH = 1.10, eyeW = 1.10 },
    greeting = { period = 600,  eyeH = 1.15, eyeW = 1.15 },
    eat      = { period = 500,  eyeH = 0.50, eyeW = 1.05 },
    dance    = { period = 600,  eyeH = 0.40, eyeW = 1.10 },
    hop      = { period = 1800, eyeH = 1.00, eyeW = 1.00 },
    wiggle   = { period = 1800, eyeH = 1.00, eyeW = 1.00 },
    yawn     = { period = 2200, eyeH = 0.30, eyeW = 1.05 },
    peek     = { period = 2600, eyeH = 1.10, eyeW = 1.05 },
    squish   = { period = 1200, eyeH = 1.00, eyeW = 1.00 },
    wink     = { period = 3200, eyeH = 1.00, eyeW = 1.00 },
    blush    = { period = 1600, eyeH = 0.90, eyeW = 1.00 },
    sneeze   = { period = 1500, eyeH = 0.30, eyeW = 1.05 },
    hum      = { period = 1800, eyeH = 0.45, eyeW = 1.05 },
    think    = { period = 3000, eyeH = 1.00, eyeW = 1.00 },
    spin     = { period = 1400, eyeH = 1.00, eyeW = 1.00 },
    bounce   = { period = 1100, eyeH = 1.00, eyeW = 1.00 },
    sip      = { period = 2200, eyeH = 0.50, eyeW = 1.05 },
    hiccup   = { period = 900,  eyeH = 0.80, eyeW = 1.00 },
    jiggle   = { period = 1200, eyeH = 1.00, eyeW = 1.00 },
}

-- Eyes drawn as thin happy bars (no shine).
local SQUINT = { sleep = true, nap = true, happy = true, love = true, yawn = true, hum = true,
    eat = true, dance = true, sip = true, stuffed = true, sneeze = true }

-- Random idle quirks: {mood, duration ms}.
local FIDGETS = {
    { "hop", 1000 }, { "wiggle", 1100 }, { "yawn", 2000 }, { "peek", 2400 }, { "squish", 1100 },
    { "wink", 900 }, { "blush", 2000 }, { "sneeze", 1500 }, { "hum", 3000 }, { "think", 2600 },
    { "spin", 1400 }, { "bounce", 1100 },
}

-- Extra quirks that suit a character.
local EXTRA_FIDGETS = {
    sake       = { { "hiccup", 900 }, { "sip", 2200 } },
    bubble_tea = { { "sip", 2200 } },
    ramen      = { { "sip", 2200 } },
    miso_soup  = { { "sip", 2200 } },
    takoyaki   = { { "jiggle", 1200 } },
    mochi      = { { "jiggle", 1200 } },
    dango      = { { "jiggle", 1200 } },
    bao        = { { "jiggle", 1200 } },
    taiyaki    = { { "wiggle", 1100 } }, -- wags its tail
    matcha     = { { "sip", 2200 } },
}

-- Sound played when a mood starts (names of the files in plugin/sounds).
local SOUND_FOR = {
    greeting = "greeting", wave = "hello", happy = "happy", love = "love", sad = "sad", startled = "startled",
    annoyed = "annoyed", dizzy = "dizzy", hop = "hop", bounce = "bounce", wiggle = "wiggle", squish = "squish",
    jiggle = "squish", yawn = "yawn", sneeze = "sneeze", hiccup = "hiccup", hum = "hum", think = "think",
    wink = "wink", blush = "blush", spin = "spin", eat = "eat", dance = "dance", nap = "nap", alert = "alert",
}

-- ── Construction & configuration ──────────────────────────────────────────────

--- Read the plugin settings; `get` is noctalia.getConfig.
function Pet.configFrom(get)
    local character = get("character")
    if type(character) ~= "string" or not CHARACTERS[character] then character = DEFAULT_CHARACTER end
    local fidgets = get("fidgets")
    local nap = tonumber(get("napAfterSec"))
    return {
        character = character,
        fidgets = (fidgets ~= false and fidgets ~= "false"),
        napAfterSec = nap and clamp(nap, 0, 3600) or 120, -- 0 = never
    }
end

function Pet:configure(cfg)
    self.cfg = cfg or { character = DEFAULT_CHARACTER, fidgets = true, napAfterSec = 120 }
    self.char = CHARACTERS[self.cfg.character] or CHARACTERS[DEFAULT_CHARACTER]
end

function Pet.new(nowMs, cfg)
    math.randomseed(nowMs or 0)
    local self = setmetatable({}, Pet)
    self:configure(cfg)
    local now = nowMs or 0
    self.baseMood = "idle"
    self.override = nil          -- {mood=, startMs=, untilMs=}
    self.lastMs = nil
    self.t = 0                   -- accumulated ms
    self.gx, self.gy = 0, 0      -- current gaze, -1..1
    self.tx, self.ty = 0, 0      -- gaze target
    self.nextGlance = 0
    self.nextBlink = now + rand(1200, 3000)
    self.blinkStart = -1e9
    self.blink = 0
    self.nextFidget = now + rand(6000, 12000)
    self.lastActive = now
    self.lastEventTs = 0
    self.prevBase = "other"
    self.pokes = {}
    self.sx, self.sy = 0, 0      -- squash/stretch
    self.lift, self.shake = 0, 0
    self.orbit, self.yawn = 0, 0
    self.deco, self.decoPhase = nil, 0
    self.winkL = 0               -- 0..1, left eye closed (wink)
    self.cheek = 0               -- 0..1, how flushed the cheeks are
    self.workKind = "read"       -- read / write / run / search / think
    self.contextPct = 0          -- the active session's context window, last seen (0..100)
    self.knownSessions = nil     -- ids seen in the last snapshot
    self.failureCounts = {}      -- session id -> activity.failures last seen
    self.turnHadFailure = false  -- a failure happened since the pet was last idle
    self.forceNap = false
    self.hover = nil
    self.prevWorking = false
    self.cur, self.curP = "idle", 0
    self:trigger("greeting", 1800, now) -- says hello when it appears
    self.sounds, self.soundsOn, self.soundMood, self.soundOverride, self.wakeCue = {}, false, nil, nil, false
    return self
end

--- Start a transient mood. `nextMood` (optional) plays right after it ends.
--- Only the entry that owns the speakers (the bar widget) turns this on.
function Pet:enableSounds(on)
    self.soundsOn = on and true or false
    if not self.soundsOn then self.sounds = {} end
end

function Pet:cue(name)
    if not (self.soundsOn and name) then return end
    self.sounds[#self.sounds + 1] = name
    if #self.sounds > 12 then table.remove(self.sounds, 1) end
end

--- The sounds to play now (and forget).
function Pet:drainSounds()
    local out = self.sounds
    self.sounds = {}
    return out
end

function Pet:trigger(mood, durationMs, nowMs, nextMood, nextDurationMs)
    self.override = {
        mood = mood, startMs = nowMs, untilMs = nowMs + durationMs,
        nextMood = nextMood, nextDur = nextDurationMs,
    }
end

--- Current mood and progress (0..1) of a transient one.
function Pet:mood(nowMs)
    local o = self.override
    if o then
        if nowMs < o.untilMs then return o.mood, (nowMs - o.startMs) / (o.untilMs - o.startMs) end
        self.override = nil
        if o.nextMood then
            self:trigger(o.nextMood, o.nextDur or 1000, o.untilMs)
            return self:mood(nowMs)
        end
    end
    local base = self.baseMood
    local napMs = (self.cfg.napAfterSec or 0) * 1000
    if base == "idle" and (self.forceNap or (napMs > 0 and nowMs - self.lastActive > napMs)) then base = "nap" end
    return base, 0
end

--- Derive the base mood from the daemon snapshot and react to transitions.
function Pet:onSnapshot(snap, daemonUp, nowMs)
    local mood = "idle"
    local working = false
    -- Requests to answer, plus sessions blocked on their own terminal with nothing to answer here.
    local pending = #((snap and snap.pending) or {})
    local asked = {}
    for _, p in ipairs((snap and snap.pending) or {}) do asked[p.session_id or ""] = true end
    for _, s in ipairs((snap and snap.sessions) or {}) do
        if s.status == "waiting" and not asked[s.id or ""] then pending = pending + 1 end
    end
    local newSession = false
    local newFails, severeFail = 0, false
    if not daemonUp then
        mood = "sleep"
        self.knownSessions = nil
        self.upSince = nil
        self.contextPct = 0
    else
        -- Sessions that were already there when the daemon came up are not newcomers: the
        -- service reports "daemon up" a moment before it sends the session list.
        if not self.upSince then self.upSince = nowMs end
        local active, ids, failCounts = nil, {}, {}
        for _, s in ipairs((snap and snap.sessions) or {}) do
            if s.id then
                ids[s.id] = true
                if self.knownSessions and not self.knownSessions[s.id] then newSession = true end
                -- A tool failed since the last snapshot (not on the pet's very first look,
                -- so an already-failed turn on startup doesn't trigger a false alarm).
                local fails = (s.activity and s.activity.failures) or 0
                failCounts[s.id] = fails
                if self.knownSessions and fails > (self.failureCounts[s.id] or 0) then
                    newFails = newFails + 1
                    if fails >= 3 then severeFail = true end
                end
            end
            if s.status == "working" then
                working = true
                if not active or (s.last_event_ms or 0) > (active.last_event_ms or 0) then active = s end
            end
        end
        self.knownSessions = ids
        self.failureCounts = failCounts
        -- What kind of work is going on (drives the eyes and the little icon).
        -- (the daemon classifies the tool: read, write, run, search, think or other)
        local tool = active and type(active.last_tool) == "table" and active.last_tool.kind
        self.workKind = (tool and tool ~= "other") and tool or "read"
        -- The active session's context window, kept until a new reading comes in
        -- (drives the kakigori's melt; stays put while idle rather than snapping back).
        if active and active.context and active.context.percent then self.contextPct = active.context.percent end

        local lim
        for _, a in pairs((snap and snap.agents) or {}) do
            if a.limits and a.limits.data then lim = a.limits.data end
        end
        local high = lim and ((lim.five_hour and lim.five_hour.percent or 0) >= 90
            or (lim.seven_day and lim.seven_day.percent or 0) >= 90)
        local full = active and active.context and (active.context.percent or 0) >= 90
        local chatBusy = (snap and snap.chat and snap.chat.busy) == true
        if pending > 0 then mood = "alert"
        elseif high then mood = "worried"
        elseif full then mood = "stuffed"
        elseif working then mood = "working"
        elseif chatBusy then mood = "think" end -- the built-in chat is answering
        -- The chat finished answering → a happy hop (like a session finishing).
        if self.prevChatBusy and not chatBusy and not (snap.chat and snap.chat.error) and mood == "idle" then
            self:trigger("happy", 1200, nowMs)
        end
        self.prevChatBusy = chatBusy
        if chatBusy then self.lastActive = nowMs end
    end
    if working or pending > 0 then
        self.lastActive = nowMs
        self.forceNap = false
    end
    -- A tool failed → a worried blip (or a dizzy one once failures pile up in the same
    -- turn). A pending question still comes first.
    if newFails > 0 and mood ~= "alert" then
        self.turnHadFailure = true
        if severeFail then self:trigger("dizzy", 2000, nowMs) else self:trigger("worried", 1200, nowMs) end
    end
    -- A session finished → happy hop (a bigger one if it had to recover from a failure
    -- first). A new one appeared → wave hello.
    if self.prevWorking and not working and mood == "idle" then
        self:trigger("happy", self.turnHadFailure and 2200 or 1400, nowMs)
        self.turnHadFailure = false
    elseif newSession and mood ~= "alert" and nowMs - (self.upSince or nowMs) > 4000 then
        self:trigger("wave", 1600, nowMs)
    end
    self.prevWorking = working
    self.baseMood = mood
end

--- A one-off event from elsewhere (e.g. the panel): "love" (approved) or "sad" (denied).
function Pet:onEvent(kind, ts, nowMs)
    if (ts or 0) <= self.lastEventTs then return end
    self.lastEventTs = ts or 0
    if kind == "nap" then
        self.forceNap = true
        self.override = nil
        return
    end
    self.lastActive = nowMs
    self.forceNap = false
    if kind == "love" then self:trigger("love", 1800, nowMs)
    elseif kind == "approve" then self.soundOverride = "approve"; self:trigger("love", 1800, nowMs)
    elseif kind == "deny" then self.soundOverride = "deny"; self:trigger("sad", 2000, nowMs)
    elseif kind == "sad" then self:trigger("sad", 2000, nowMs)
    elseif kind == "eat" then self:trigger("eat", 2600, nowMs, "happy", 1100)
    elseif kind == "dance" then self:trigger("dance", 4200, nowMs)
    elseif kind == "hello" then self:trigger("wave", 1600, nowMs) end
end

--- The user clicked the pet.
function Pet:poke(nowMs)
    local mood = self:mood(nowMs)
    self.lastActive = nowMs
    self.forceNap = false
    if mood == "dizzy" then return end -- let it finish spinning
    self:cue("poke")
    if mood == "nap" then self:trigger("startled", 800, nowMs); return end
    local recent = {}
    for _, t in ipairs(self.pokes) do
        if nowMs - t < 3000 then recent[#recent + 1] = t end
    end
    recent[#recent + 1] = nowMs
    self.pokes = recent
    if #recent >= 4 then
        self.pokes = {}
        self:trigger("dizzy", 2400, nowMs)
    else
        self:trigger("annoyed", 800, nowMs)
    end
end

--- Make the eyes look at a point (-1..1); nil releases the gaze.
function Pet:lookAt(x, y)
    self.hover = x and { x = x, y = y or 0 } or nil
end

-- ── Animation ─────────────────────────────────────────────────────────────────

-- Floating decoration per mood (drawn beside the body at panel size).
local DECO = {
    nap = "z", sleep = "z", love = "heart", worried = "sweat", stuffed = "sweat", sad = "tear",
    happy = "sparkle", hum = "note", dance = "note", think = "dots", sneeze = "droplets",
    hiccup = "hic", wave = "hi", greeting = "hi", eat = "crumbs",
}
-- Decoration while working, by kind of work ("read" has none: the eyes say it).
local WORK_DECO = { write = "pencil", run = "terminal", search = "search", think = "dots" }

local function sine(t, period) return math.sin(t * 2 * math.pi / period) end

function Pet:tick(nowMs)
    local dt = self.lastMs and clamp(nowMs - self.lastMs, 0, 100) or 0
    self.lastMs = nowMs
    self.t = self.t + dt

    local mood, p = self:mood(nowMs)

    -- Waking up from a nap startles the pet.
    if self.prevBase == "nap" and mood ~= "nap" and mood ~= "startled" then
        self.wakeCue = true
        self:trigger("startled", 700, nowMs)
        mood, p = self:mood(nowMs)
    end
    self.prevBase = (mood == "nap") and "nap" or "other"

    -- Random idle quirks (the generic ones plus a few that suit the character).
    if mood == "idle" and self.cfg.fidgets and nowMs >= self.nextFidget then
        local pool = {}
        for _, f in ipairs(FIDGETS) do pool[#pool + 1] = f end
        for _, f in ipairs(EXTRA_FIDGETS[self.cfg.character] or {}) do pool[#pool + 1] = f end
        local f = pool[math.random(#pool)]
        self:trigger(f[1], f[2], nowMs)
        self.nextFidget = nowMs + rand(10000, 25000)
        mood, p = self:mood(nowMs)
    end

    local mp = MOOD_PARAMS[mood] or MOOD_PARAMS.idle
    self.cur, self.curP = mood, p
    local t = self.t

    -- A new mood may come with a sound (the approve/deny sounds replace the love/sad ones).
    if mood ~= self.soundMood then
        self.soundMood = mood
        local name = SOUND_FOR[mood]
        if mood == "startled" and self.wakeCue then name = "wake" end
        if self.soundOverride and (mood == "love" or mood == "sad") then name = self.soundOverride end
        self.wakeCue, self.soundOverride = false, nil
        self:cue(name)
    end
    local char = self.char

    -- Breathing → gentle squash & stretch (deeper while napping).
    local breath = sine(t, mp.period)
    local amp = (mood == "nap" or mood == "sleep") and 0.045 or 0.03
    self.sy = amp * breath
    self.sx = -amp * 0.6 * breath

    -- Gaze target.
    local work = self.workKind
    if self.hover then
        self.tx, self.ty = self.hover.x, self.hover.y
    elseif mood == "idle" then
        if nowMs >= self.nextGlance then
            self.tx, self.ty = rand(-1, 1), rand(-0.6, 0.6)
            if math.random() < 0.35 then self.tx, self.ty = 0, 0 end
            self.nextGlance = nowMs + rand(1200, 3800)
        end
    elseif mood == "working" then
        if work == "write" then          -- eyes down on the keyboard
            self.tx, self.ty = math.sin(t / 90) * 0.35, 0.85
        elseif work == "run" then        -- watching output scroll by
            self.tx, self.ty = (math.sin(t / 230) > 0) and 0.8 or -0.8, 0.1
        elseif work == "search" then     -- looking up and around
            self.tx, self.ty = 0.7 + math.sin(t / 500) * 0.25, -0.6
        elseif work == "think" then
            self.tx, self.ty = -0.7, -0.7
        else                             -- read: sweep left → right
            self.tx = math.sin(t / 380) > 0 and 0.9 or -0.9
            self.ty = 0.5
        end
    elseif mood == "alert" then
        self.tx, self.ty = 0, -0.1
    elseif mood == "peek" then
        self.tx, self.ty = (p < 0.5) and -1 or 1, -0.2
    elseif mood == "sad" then
        self.tx, self.ty = 0, 0.8
    elseif mood == "worried" or mood == "stuffed" then
        self.tx, self.ty = math.sin(t / 140) * 0.5, -0.25
    elseif mood == "think" then
        self.tx, self.ty = -0.7, -0.7
    elseif mood == "blush" then
        self.tx, self.ty = -0.6, 0.6
    elseif mood == "wave" then
        self.tx, self.ty = 0, -0.2
    else
        self.tx, self.ty = 0, 0
    end
    local fast = (mood == "working" or mood == "peek")
    local k = 1 - math.exp(-dt / (fast and 70 or 130))
    self.gx = self.gx + (self.tx - self.gx) * k
    self.gy = self.gy + (self.ty - self.gy) * k

    self.orbit = (mood == "dizzy" or mood == "spin") and (t / ((mood == "spin") and 60 or 90)) or 0

    -- Wink: the left eye closes for the middle of the animation.
    self.winkL = (mood == "wink") and clamp(math.sin(p * math.pi) * 2.5, 0, 1) or 0

    -- Cheeks: flushed when shy, loved, or (for the sake bottle) tipsy.
    local cheekTarget = 0
    if mood == "blush" then cheekTarget = 1
    elseif mood == "love" then cheekTarget = 0.6
    elseif char.tipsy and (mood == "idle" or mood == "working" or mood == "hiccup" or mood == "sip") then cheekTarget = 0.7 end
    self.cheek = self.cheek + (cheekTarget - self.cheek) * (1 - math.exp(-dt / 220))

    -- Blinking (closed-eye moods ignore it).
    if nowMs >= self.nextBlink then
        self.blinkStart = nowMs
        self.nextBlink = nowMs + rand(2500, 6000)
        if math.random() < 0.15 then self.nextBlink = nowMs + 260 end -- double blink
    end
    local b = (nowMs - self.blinkStart) / 150
    self.blink = (b >= 0 and b <= 1) and (1 - math.abs(2 * b - 1)) or 0

    -- Jumps, shakes and body language.
    self.lift, self.shake, self.yawn = 0, 0, 0
    self.pulse = math.abs(sine(t, 300)) -- mouth pulsing (humming, sipping)
    self.chew = 0.5 + 0.5 * sine(t, 220) -- mouth chewing
    if mood == "happy" then
        self.lift = math.abs(math.sin(p * math.pi * 2)) * (1 - p * 0.5)
    elseif mood == "love" then
        self.lift = math.abs(math.sin(p * math.pi * 2)) * 0.35 * (1 - p)
        self.shake = math.sin(t / 160) * 0.25
    elseif mood == "alert" then
        local phase = (t % 900) / 900
        self.lift = (phase < 0.35) and math.sin(phase / 0.35 * math.pi) * 0.55 or 0
    elseif mood == "startled" then
        self.lift = math.sin(clamp(p * 1.6, 0, 1) * math.pi) * 0.7
    elseif mood == "annoyed" then
        self.shake = math.sin(t / 28) * (1 - p)
    elseif mood == "dizzy" then
        self.shake = math.sin(t / 110) * 0.8
    elseif mood == "worried" then
        self.shake = math.sin(t / 60) * 0.12 -- trembling
    elseif mood == "stuffed" then
        self.sx = self.sx + 0.10             -- bloated
        self.sy = self.sy - 0.03
        self.shake = math.sin(t / 90) * 0.08
    elseif mood == "hop" then
        self.lift = math.sin(p * math.pi) * 0.55
    elseif mood == "bounce" then
        self.lift = math.abs(math.sin(p * math.pi * 2)) * 0.5
    elseif mood == "wiggle" then
        self.shake = math.sin(t / 55) * (1 - p) * 0.9
    elseif mood == "wave" then
        self.lift = math.abs(math.sin(p * math.pi * 2)) * 0.45
        self.shake = math.sin(t / 120) * 0.35
    elseif mood == "greeting" then
        self.lift = math.abs(math.sin(p * math.pi * 3)) * 0.55 * (1 - p * 0.4)
        self.shake = math.sin(t / 110) * 0.45
    elseif mood == "dance" then
        self.lift = math.abs(sine(t, 380)) * 0.4
        self.shake = sine(t, 760) * 0.9
    elseif mood == "hum" then
        self.shake = sine(t, 840) * 0.35
        self.sy = self.sy + 0.02 * sine(t, 420)
    elseif mood == "sip" then
        self.sy = self.sy - 0.05 * math.sin(p * math.pi)
    elseif mood == "spin" then
        self.shake = math.sin(t / 70) * 0.2
    elseif mood == "eat" then
        self.sy = self.sy + 0.035 * sine(t, 220)
    elseif mood == "yawn" then
        local y = math.sin(p * math.pi)
        self.yawn = y
        self.sy = self.sy + 0.07 * y   -- stretch up
        self.sx = self.sx - 0.03 * y
    elseif mood == "squish" then
        local e = math.sin(p * math.pi * 3) * (1 - p)
        self.sy = -0.20 * e
        self.sx = 0.12 * e
    elseif mood == "jiggle" then
        local e = (1 - p) * 0.07 * math.sin(t / 55)
        self.sy, self.sx = self.sy + e, self.sx - e
    elseif mood == "hiccup" then
        self.lift = math.sin(clamp(p * 3, 0, 1) * math.pi) * 0.30
    elseif mood == "blush" then
        self.shake = math.sin(t / 200) * 0.15
        self.sy = self.sy - 0.02
    elseif mood == "sneeze" then
        if p < 0.55 then                     -- wind up: stretch, eyes squeezed
            local q = p / 0.55
            self.sy = self.sy + 0.09 * q
            self.sx = self.sx - 0.04 * q
        else                                 -- ACHOO: squash and shake
            local q = (p - 0.55) / 0.45
            self.sy = self.sy - 0.20 * (1 - q)
            self.sx = self.sx + 0.12 * (1 - q)
            self.shake = math.sin(t / 25) * (1 - q)
        end
    elseif mood == "sad" then
        self.sy = self.sy - 0.04 -- droop
    end
    -- The sake bottle sways a little in every calm mood.
    if char.tipsy and (mood == "idle" or mood == "working") then
        self.shake = self.shake + sine(t, 1400) * 0.40
    end

    -- Floating decoration: the mood's own, else the work icon, else steam for hot dishes.
    local deco = DECO[mood]
    if not deco and mood == "working" then deco = WORK_DECO[work] end
    if not deco and char.steam then deco = "steam" end
    self.deco = deco
    self.decoPhase = (t % 1800) / 1800
end

-- ── Rendering ─────────────────────────────────────────────────────────────────

local function spacerW(w) return ui.spacer({ width = math.max(0, round(w)) }) end
local function spacerH(h) return ui.spacer({ height = math.max(0, round(h)) }) end

--- One eye: a black dot inside a transparent socket; (gx, gy) in -1..1 moves it.
--- `baseH` is the open height, so blinking squashes the eye around its centre.
local function eye(sw, sh, ew, eh, baseH, gx, gy, shine)
    local cx = sw / 2 + gx * (sw - ew) / 2
    local cy = sh / 2 + gy * (sh - baseH) / 2
    local children = {}
    if shine and eh >= 6 then
        local hl = round(math.min(ew, eh) * 0.38)
        children = {
            spacerH(eh * 0.14),
            ui.row({}, { spacerW(ew * 0.18), ui.box({ width = hl, height = hl, radius = round(hl / 2), fill = SHINE }) }),
        }
    end
    return ui.column({ width = round(sw), height = round(sh), align = "start", justify = "start" }, {
        spacerH(cy - eh / 2),
        ui.row({}, {
            spacerW(cx - ew / 2),
            ui.column({
                width = round(ew), height = round(eh), fill = INK, radius = round(math.min(ew, eh) / 2),
                align = "start", justify = "start",
            }, children),
        }),
    })
end

--- Eyes, blush and mouth, shared by every character.
function Pet:face(w, detail, opts)
    local mood = self.cur or "idle"
    local mp = MOOD_PARAMS[mood] or MOOD_PARAMS.idle
    local p = self.curP or 0

    local sw = w * 0.17
    local ew = w * 0.10 * mp.eyeW
    local eh0 = w * 0.15
    local closedMood = (mood == "sleep" or mood == "nap")
    local eh = math.max(2, eh0 * mp.eyeH * (1 - (closedMood and 1 or self.blink)))
    if closedMood then eh = 2 end
    local ehL = math.max(2, eh * (1 - self.winkL)) -- left eye, closed by a wink
    local gx, gy = self.gx, self.gy
    if mood == "dizzy" or mood == "spin" then gx, gy = math.cos(self.orbit), math.sin(self.orbit) end
    local shine = detail and not SQUINT[mood]

    -- Blush: wider, deeper cheeks.
    local a = self.cheek
    local cheekW = round(w * 0.10 * (1 + 0.7 * a))
    local cheekH = math.max(2, round(w * 0.055 * (1 + 0.5 * a)))
    local cheekColor = ((a > 0.45) and CHEEK_HOT or CHEEK) .. "cc"
    local function cheek()
        return ui.box({ width = cheekW, height = cheekH, radius = round(cheekH / 2), fill = cheekColor })
    end
    local eyeRow = ui.row({ gap = round(w * 0.035), align = "center" }, {
        cheek(),
        eye(sw, sw, ew, ehL, eh0, gx, gy, shine and self.winkL < 0.5),
        spacerW(w * 0.05),
        eye(sw, sw, ew, eh, eh0, gx * 0.9, gy, shine),
        cheek(),
    })

    local face = { eyeRow }
    if opts.mouth ~= false then
        -- width/height of the mouth as fractions of the sprite width
        local m = {
            idle = { 0.11, 0.04 }, working = { 0.11, 0.04 }, hop = { 0.11, 0.04 }, wiggle = { 0.11, 0.04 },
            squish = { 0.11, 0.04 }, peek = { 0.11, 0.04 }, jiggle = { 0.11, 0.04 },
            happy = { 0.20, 0.09 }, love = { 0.20, 0.09 }, wave = { 0.18, 0.08 }, greeting = { 0.20, 0.09 }, dance = { 0.18, 0.08 },
            bounce = { 0.16, 0.07 }, wink = { 0.14, 0.05 }, spin = { 0.12, 0.05 },
            alert = { 0.07, 0.09 }, startled = { 0.07, 0.09 }, hiccup = { 0.06, 0.08 }, dizzy = { 0.09, 0.09 },
            annoyed = { 0.14, 0.03 }, sleep = { 0.06, 0.03 }, nap = { 0.06, 0.03 }, sad = { 0.08, 0.05 },
            worried = { 0.13, 0.03 }, stuffed = { 0.14, 0.03 }, blush = { 0.08, 0.03 }, think = { 0.08, 0.03 },
        }
        local size = m[mood] or { 0.11, 0.04 }
        local mw, mh = w * size[1], math.max(2, w * size[2])
        if mood == "yawn" then
            mw, mh = w * (0.07 + 0.08 * self.yawn), w * (0.04 + 0.12 * self.yawn)
        elseif mood == "hum" or mood == "sip" then
            local d = w * (0.04 + 0.05 * self.pulse)
            mw, mh = d, d
        elseif mood == "eat" then
            mw, mh = w * 0.12, w * (0.03 + 0.08 * self.chew)
        elseif mood == "sneeze" then
            if p < 0.55 then mw, mh = w * 0.06, math.max(2, w * 0.04)
            else mw, mh = w * 0.16, w * 0.14 end
        end
        face[#face + 1] = ui.box({ width = round(mw), height = round(math.max(2, mh)), radius = round(math.max(2, mh) / 2), fill = INK })
    end
    return face
end

--- A new list: `first` followed by every element of `list`.
local function withFirst(first, list)
    local out = { first }
    for _, v in ipairs(list) do out[#out + 1] = v end
    return out
end

-- Bodies: each takes (character, context) and returns the body tree. -----------

local function nigiriBody(c, x)
    local topH = round(x.bh * 0.40)
    local riceH = x.bh - topH
    local riceW = round(x.bw * 0.90)
    local stripes = {}
    if c.bands then
        -- Shrimp: vertical pale bands across the topping.
        local bandW, bandH = math.max(2, round(x.bw * 0.07)), math.max(3, round(topH * 0.62))
        local function band() return ui.box({ width = bandW, height = bandH, radius = 1, fill = c.bands }) end
        stripes = { ui.row({ gap = math.max(2, round(x.bw * 0.07)), align = "center" }, { band(), band(), band(), band() }) }
    elseif x.detail then
        stripes = {
            ui.box({ width = round(x.bw * 0.52), height = math.max(2, round(topH * 0.10)), radius = 2, fill = c.fat }),
            ui.box({ width = round(x.bw * 0.34), height = math.max(2, round(topH * 0.08)), radius = 2, fill = c.fat }),
        }
    end
    local top = ui.column({
        width = x.bw, height = topH, fill = c.top, radius = round(topH * 0.5),
        align = "center", justify = "center", gap = round(topH * 0.16),
    }, stripes)
    local rice = ui.column({
        width = riceW, height = riceH, fill = RICE, radius = round(math.min(riceW, riceH) * 0.42),
        border = RICE_EDGE, borderWidth = 1,
        align = "center", justify = "center", gap = round(x.w * 0.035),
    }, x.face)
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { top, rice })
end

local function makiBody(c, x)
    local inner = {}
    if x.detail then
        local d = round(x.w * 0.09)
        inner[#inner + 1] = ui.row({ gap = round(x.w * 0.03) }, {
            ui.box({ width = d, height = d, radius = round(d / 2), fill = c.filling }),
            ui.box({ width = d, height = d, radius = round(d / 2), fill = c.filling2 }),
            ui.box({ width = d, height = d, radius = round(d / 2), fill = c.filling }),
        })
    end
    for _, f in ipairs(x.face) do inner[#inner + 1] = f end
    local iw, ih = round(x.bw * 0.80), round(x.bh * 0.80)
    local rice = ui.column({
        width = iw, height = ih, fill = RICE, radius = round(math.min(iw, ih) / 2),
        align = "center", justify = "center", gap = round(x.w * 0.03),
    }, inner)
    return ui.column({
        width = x.bw, height = x.bh, fill = NORI, radius = round(math.min(x.bw, x.bh) / 2),
        align = "center", justify = "center",
    }, { rice })
end

--- Inside-out roll: rice outside (with sesame and tobiko), a thin nori ring, rice and filling inside.
local function uramakiBody(c, x)
    local function dot(color)
        local d = round(x.w * 0.08)
        return ui.box({ width = d, height = d, radius = round(d / 2), fill = color })
    end
    local inner = {}
    if x.detail then
        inner[#inner + 1] = ui.row({ gap = round(x.w * 0.025) }, { dot(c.filling), dot(c.filling2), dot(c.filling3) })
    end
    for _, f in ipairs(x.face) do inner[#inner + 1] = f end
    local iw, ih = round(x.bw * 0.74), round(x.bh * 0.74)
    local rice = ui.column({
        width = iw, height = ih, fill = RICE, radius = round(math.min(iw, ih) / 2),
        align = "center", justify = "center", gap = round(x.w * 0.03),
    }, inner)
    local nw, nh = round(x.bw * 0.82), round(x.bh * 0.82)
    local nori = ui.column({
        width = nw, height = nh, fill = NORI, radius = round(math.min(nw, nh) / 2),
        align = "center", justify = "center",
    }, { rice })

    local layers = { nori }
    if x.detail then
        local function seeds()
            local sw, sh = round(x.w * 0.05), math.max(2, round(x.w * 0.03))
            local function seed(color) return ui.box({ width = sw, height = sh, radius = 2, fill = color }) end
            return ui.row({ gap = round(x.w * 0.05) }, { seed("#3a2e2a"), seed("#ff8a3d"), seed("#3a2e2a") })
        end
        layers = { seeds(), nori, seeds() }
    end
    return ui.column({
        width = x.bw, height = x.bh, fill = RICE, radius = round(math.min(x.bw, x.bh) / 2),
        border = RICE_EDGE, borderWidth = 1, align = "center", justify = "center", gap = round(x.w * 0.01),
    }, layers)
end

local function onigiriBody(c, x)
    local nori = ui.box({ width = round(x.bw * 0.54), height = round(x.bh * 0.30), radius = round(x.w * 0.05), fill = NORI })
    local children = { spacerH(x.bh * 0.12) }
    for _, f in ipairs(x.face) do children[#children + 1] = f end
    children[#children + 1] = ui.spacer({ flexGrow = 1 })
    children[#children + 1] = nori
    children[#children + 1] = spacerH(x.bh * 0.05)
    return ui.column({
        width = x.bw, height = x.bh, fill = RICE, radius = round(x.bw * 0.30),
        border = RICE_EDGE, borderWidth = 1, align = "center", justify = "start", gap = round(x.w * 0.03),
    }, children)
end

--- Gunkan: nori-wrapped rice with a heap of salmon roe on top.
local function ikuraBody(c, x)
    local roeH = round(x.bh * 0.30)
    local heap
    if x.detail then
        local d = round(x.w * 0.14)
        local function ball() return ui.box({ width = d, height = d, radius = round(d / 2), fill = c.roe }) end
        heap = ui.column({ width = x.bw, height = roeH, align = "center", justify = "end" }, {
            ui.row({ gap = 1 }, { ball(), ball(), ball() }),
            ui.row({ gap = 1 }, { ball(), ball(), ball(), ball(), ball() }),
        })
    else
        heap = ui.column({ width = x.bw, height = roeH, align = "center", justify = "end" }, {
            ui.box({ width = round(x.bw * 0.78), height = roeH, radius = round(roeH * 0.5), fill = c.roe }),
        })
    end
    local nh = x.bh - roeH
    local nw = round(x.bw * 0.90)
    local rice = ui.column({
        width = round(nw * 0.84), height = round(nh * 0.80), fill = RICE, radius = round(x.w * 0.14),
        align = "center", justify = "center", gap = round(x.w * 0.035),
    }, x.face)
    local nori = ui.column({
        width = nw, height = nh, fill = NORI, radius = round(x.w * 0.20), align = "center", justify = "center",
    }, { rice })
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { heap, nori })
end

--- Inari: a golden tofu pouch with a sliver of rice showing at the top.
local function inariBody(c, x)
    local riceH = round(x.bh * 0.16)
    local rice = ui.box({ width = round(x.bw * 0.70), height = riceH, radius = round(riceH / 2), fill = RICE })
    local pod = {}
    if x.detail then
        pod[#pod + 1] = ui.box({ width = round(x.bw * 0.5), height = math.max(2, round(x.w * 0.03)), radius = 2, fill = INARI_DARK })
    end
    for _, f in ipairs(x.face) do pod[#pod + 1] = f end
    local podTree = ui.column({
        width = x.bw, height = x.bh - riceH, fill = INARI, radius = round(x.w * 0.26),
        align = "center", justify = "center", gap = round(x.w * 0.035),
    }, pod)
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { rice, podTree })
end

--- Dorayaki: two pancakes with sweet bean paste in between; the face is on the top one.
local function dorayakiBody(c, x)
    local topH = round(x.bh * 0.48)
    local fillH = math.max(2, round(x.bh * 0.10))
    local botH = x.bh - topH - fillH
    local top = ui.column({
        width = x.bw, height = topH, fill = DORA_TOP, radius = round(topH * 0.48),
        align = "center", justify = "center", gap = round(x.w * 0.03),
    }, x.face)
    local an = ui.box({ width = round(x.bw * 0.92), height = fillH, radius = round(fillH / 2), fill = DORA_AN })
    local bottom = ui.box({ width = x.bw, height = botH, radius = round(botH * 0.48), fill = DORA_BOT })
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { top, an, bottom })
end

--- Dango: three rice dumplings (pink, white, green); the face is on the big middle one.
local function dangoBody(c, x)
    local h1, h2 = round(x.bh * 0.256), round(x.bh * 0.488)
    local h3 = x.bh - h1 - h2
    local function ball(d, color, children, gap)
        return ui.column({
            width = d, height = d, fill = color, radius = round(d / 2),
            align = "center", justify = "center", gap = gap or 0,
        }, children or {})
    end
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, {
        ball(h1, c.top),
        ui.column({
            width = round(x.bw * 0.78), height = h2, fill = c.mid, radius = round(h2 / 2),
            border = RICE_EDGE, borderWidth = 1, align = "center", justify = "center", gap = round(x.w * 0.03),
        }, x.face),
        ball(h3, c.bottom),
    })
end

--- Ebi tempura: a golden fried shrimp with its orange tail sticking out on the right.
local function tempuraBody(c, x)
    local bodyW = round(x.bw * 0.84)
    local tailW = x.bw - bodyW
    local inner = {}
    if x.detail then
        local function crumb(w, h) return ui.box({ width = round(x.w * w), height = round(x.w * h), radius = 2, fill = BATTER_DARK }) end
        inner[#inner + 1] = ui.row({ gap = round(x.w * 0.05), align = "end" }, {
            crumb(0.05, 0.03), crumb(0.07, 0.045), crumb(0.04, 0.03), crumb(0.06, 0.04),
        })
    end
    for _, f in ipairs(x.face) do inner[#inner + 1] = f end
    local batter = ui.column({
        width = bodyW, height = x.bh, fill = BATTER, radius = round(x.bh * 0.46),
        align = "center", justify = "center", gap = round(x.w * 0.03),
    }, inner)
    local tail = ui.column({
        width = tailW, height = round(x.bh * 0.52), fill = TAIL, radius = round(tailW * 0.5),
    }, {})
    return ui.row({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { batter, tail })
end

--- Ravioli (steamed dumpling): pleated ridge on top, pale dough, seared golden base.
local function ravioliBody(c, x)
    local ridgeH = round(x.bh * 0.14)
    local baseH = round(x.bh * 0.16)
    local mainH = x.bh - ridgeH - baseH
    local n = x.detail and 6 or 4
    local pleatW = math.max(2, round(x.bw * (x.detail and 0.085 or 0.10)))
    local function pleat() return ui.box({ width = pleatW, height = ridgeH, radius = round(pleatW / 2), fill = DOUGH_FOLD }) end
    local pleats = {}
    for i = 1, n do pleats[i] = pleat() end
    local ridge = ui.row({ gap = math.max(1, round(x.bw * 0.025)), align = "end" }, pleats)
    local main = ui.column({
        width = x.bw, height = mainH, fill = DOUGH, radius = round(mainH * 0.46),
        border = DOUGH_EDGE, borderWidth = 1, align = "center", justify = "center", gap = round(x.w * 0.035),
    }, x.face)
    local base = ui.box({ width = round(x.bw * 0.84), height = baseH, radius = round(baseH * 0.5), fill = SEARED })
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { ridge, main, base })
end

--- Bao: a fluffy white steamed bun with a fold on top, sitting on a bamboo steamer.
local function baoBody(c, x)
    local trayH = round(x.bh * 0.13)
    local bunH = x.bh - trayH
    local inner = {}
    if x.detail then
        inner[#inner + 1] = ui.box({ width = round(x.bw * 0.22), height = math.max(2, round(x.w * 0.035)), radius = 2, fill = BAO_FOLD })
    end
    for _, f in ipairs(x.face) do inner[#inner + 1] = f end
    local bun = ui.column({
        width = x.bw, height = bunH, fill = BAO, radius = round(math.min(x.bw, bunH) * 0.5),
        border = BAO_FOLD, borderWidth = 1, align = "center", justify = "center", gap = round(x.w * 0.035),
    }, inner)
    local slats = {}
    if x.detail then
        local sh = math.max(2, round(trayH * 0.5))
        local function slat() return ui.box({ width = math.max(2, round(x.w * 0.025)), height = sh, radius = 1, fill = BAMBOO_DARK }) end
        slats = { ui.row({ gap = round(x.w * 0.09), align = "center" }, { slat(), slat(), slat(), slat(), slat() }) }
    end
    local tray = ui.column({
        width = round(x.bw * 0.86), height = trayH, fill = BAMBOO, radius = round(trayH * 0.45),
        align = "center", justify = "center",
    }, slats)
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { bun, tray })
end

--- Ramen: noodles with a narutomaki, a soft-boiled egg, nori and scallions in a cream bowl.
local function ramenBody(c, x)
    local topH = round(x.bh * 0.30)
    local noodleH = math.max(2, round(x.bh * 0.08))
    local bowlH = x.bh - topH - noodleH
    local toppings
    if x.detail then
        local d = round(x.w * 0.20)
        local egg = ui.column({
            width = round(d * 1.15), height = round(d * 0.85), fill = "#fff6e5", radius = round(d * 0.4),
            align = "center", justify = "center",
        }, { ui.box({ width = round(d * 0.5), height = round(d * 0.5), radius = round(d * 0.25), fill = "#ffb52e" }) })
        local naruto = ui.column({
            width = d, height = d, fill = "#ffffff", radius = round(d / 2), border = "#f0b8c4", borderWidth = 2,
            align = "center", justify = "center",
        }, { ui.box({ width = round(d * 0.42), height = round(d * 0.42), radius = round(d * 0.21), fill = "#ff8fa3" }) })
        local nori = ui.box({ width = round(d * 0.5), height = round(d * 0.95), radius = 2, fill = NORI })
        local sd = math.max(2, round(d * 0.28))
        local function scallion() return ui.box({ width = sd, height = sd, radius = round(sd / 2), fill = "#6fbf4a" }) end
        toppings = ui.row({ gap = round(x.w * 0.03), align = "end" }, {
            egg, naruto, nori, ui.row({ gap = 1, align = "end" }, { scallion(), scallion(), scallion() }),
        })
    else
        local d = math.max(3, round(x.bw * 0.20))
        local function dot(color) return ui.box({ width = d, height = d, radius = round(d / 2), fill = color }) end
        toppings = ui.row({ gap = 1, align = "end" }, { dot("#ff8fa3"), dot("#fff6e5"), dot("#6fbf4a") })
    end
    local pile = ui.column({ width = x.bw, height = topH, align = "center", justify = "end" }, { toppings })
    local noodles = ui.box({ width = round(x.bw * 0.92), height = noodleH, radius = round(noodleH / 2), fill = NOODLE })
    local bowl = ui.column({
        width = x.bw, height = bowlH, fill = BOWL_CREAM, radius = round(x.w * 0.30),
        border = BOWL_EDGE, borderWidth = 1, align = "center", justify = "center", gap = round(x.w * 0.03),
    }, withFirst(ui.box({ width = round(x.bw * 0.78), height = math.max(2, round(x.w * 0.04)), radius = 2, fill = BOWL_RED }), x.face))
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { pile, noodles, bowl })
end

--- Takoyaki: a golden octopus ball with sauce, mayo and green seaweed on top.
local function takoyakiBody(c, x)
    local inner = {}
    local th = math.max(2, round(x.w * 0.05))
    local function bar(wf, h, color) return ui.box({ width = round(x.bw * wf), height = h, radius = 2, fill = color }) end
    if x.detail then
        local sd = round(x.w * 0.03)
        local function aonori() return ui.box({ width = sd, height = sd, radius = 1, fill = "#5fae4a" }) end
        inner[#inner + 1] = ui.row({ gap = round(x.w * 0.04) }, { aonori(), aonori(), aonori(), aonori() })
        inner[#inner + 1] = bar(0.46, th, SAUCE)
        inner[#inner + 1] = bar(0.34, math.max(2, round(x.w * 0.025)), "#fff6e0")
        inner[#inner + 1] = bar(0.52, th, SAUCE)
    else
        inner[#inner + 1] = bar(0.44, th, SAUCE)
    end
    for _, f in ipairs(x.face) do inner[#inner + 1] = f end
    return ui.column({
        width = x.bw, height = x.bh, fill = TAKO, radius = round(math.min(x.bw, x.bh) / 2),
        align = "center", justify = "center", gap = round(x.w * 0.025),
    }, inner)
end

--- Miso soup: a vermilion lacquer bowl with a golden rim; tofu and wakame float in the soup.
local function misoBody(c, x)
    local surfH = round(x.bh * 0.24)
    local floating = {}
    if x.detail then
        local s = round(x.w * 0.085)
        local function tofu() return ui.box({ width = s, height = s, radius = 2, fill = "#fff8ea" }) end
        local function wakame() return ui.box({ width = round(s * 1.3), height = round(s * 0.7), radius = 2, fill = "#3f7a4a" }) end
        floating = { ui.row({ gap = round(x.w * 0.05), align = "center" }, { tofu(), wakame(), tofu(), wakame() }) }
    end
    local surface = ui.column({
        width = round(x.bw * 0.90), height = surfH, fill = MISO, radius = round(surfH / 2),
        align = "center", justify = "center",
    }, floating)
    local bowl = ui.column({
        width = x.bw, height = x.bh - surfH, fill = LACQUER, radius = round(x.w * 0.32),
        align = "center", justify = "center", gap = round(x.w * 0.03),
    }, withFirst(ui.box({ width = round(x.bw * 0.80), height = math.max(2, round(x.w * 0.035)), radius = 2, fill = GOLD }), x.face))
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { surface, bowl })
end

--- Edamame: a green pod with a few beans peeking out of the top and a little stem.
local function edamameBody(c, x)
    local beansH = round(x.bh * 0.17)
    local podH = x.bh - beansH
    local d = math.max(3, round(x.w * 0.17))
    local function bean() return ui.box({ width = d, height = math.min(d, beansH + 2), radius = round(d / 2), fill = BEAN }) end
    local beans = ui.column({ width = x.bw, height = beansH, align = "center", justify = "end" }, {
        ui.row({ gap = math.max(1, round(x.w * 0.03)), align = "end" }, { bean(), bean(), bean() }),
    })
    local podW = round(x.bw * 0.90)
    local pod = ui.column({
        width = podW, height = podH, fill = POD, radius = round(podH * 0.46),
        align = "center", justify = "center", gap = round(x.w * 0.035),
    }, x.face)
    local stem = ui.column({ width = x.bw - podW, height = round(podH * 0.30), fill = POD_DARK, radius = round((x.bw - podW) / 2) }, {})
    local row = ui.row({ width = x.bw, height = podH, align = "center", justify = "start" }, { pod, stem })
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { beans, row })
end

--- Mochi (daifuku): a soft pink rice cake with a strawberry on top.
local function mochiBody(c, x)
    local berryH = round(x.bh * 0.16)
    local bodyH = x.bh - berryH
    local inner = {}
    local bw = math.max(4, round(x.bw * 0.26))
    local berry = ui.column({
        width = bw, height = berryH, fill = BERRY, radius = round(berryH * 0.5), align = "center", justify = "start",
    }, x.detail and {
        ui.box({ width = round(bw * 0.5), height = math.max(2, round(berryH * 0.22)), radius = 1, fill = "#5fae4a" }),
    } or {})
    local mochi = ui.column({
        width = x.bw, height = bodyH, fill = MOCHI, radius = round(math.min(x.bw, bodyH) * 0.46),
        border = MOCHI_EDGE, borderWidth = 1, align = "center", justify = "center", gap = round(x.w * 0.035),
    }, x.face)
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { berry, mochi })
end

--- Bubble tea: milk tea in a cup with a lid, a straw and tapioca pearls at the bottom.
local function bobaBody(c, x)
    local strawH = round(x.bh * 0.12)
    local lidH = math.max(2, round(x.bh * 0.06))
    local cupH = x.bh - strawH - lidH
    local strawW = math.max(2, round(x.w * 0.07))
    local straw = ui.row({ width = x.bw, height = strawH, align = "end", justify = "start" }, {
        spacerW(x.bw * 0.58), ui.box({ width = strawW, height = strawH, radius = 1, fill = "#e8556d" }),
    })
    local lid = ui.box({ width = round(x.bw * 0.98), height = lidH, radius = round(lidH / 2), fill = "#f3f3f3" })
    local d = math.max(3, round(x.w * 0.10))
    local function pearl() return ui.box({ width = d, height = d, radius = round(d / 2), fill = PEARL }) end
    local pearls = { pearl(), pearl(), pearl(), pearl(), pearl() }
    local cupChildren = { spacerH(cupH * 0.08) }
    for _, f in ipairs(x.face) do cupChildren[#cupChildren + 1] = f end
    cupChildren[#cupChildren + 1] = ui.spacer({ flexGrow = 1 })
    cupChildren[#cupChildren + 1] = ui.row({ gap = 1, align = "end" }, pearls)
    cupChildren[#cupChildren + 1] = spacerH(cupH * 0.04)
    local cup = ui.column({
        width = round(x.bw * 0.90), height = cupH, fill = TEA, radius = round(x.w * 0.18),
        align = "center", justify = "start", gap = round(x.w * 0.03),
    }, cupChildren)
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { straw, lid, cup })
end

--- Sake: a ceramic tokkuri (flask) with a blue lip and band; it sways a little, tipsy.
local function sakeBody(c, x)
    local lipH = math.max(2, round(x.bh * 0.05))
    local neckH = round(x.bh * 0.20)
    local bodyH = x.bh - lipH - neckH
    local lip = ui.box({ width = round(x.bw * 0.34), height = lipH, radius = 2, fill = SAKE_BLUE })
    local neck = ui.box({ width = round(x.bw * 0.20), height = neckH, radius = round(x.bw * 0.05), fill = CERAMIC })
    local inner = {}
    if x.detail then
        inner[#inner + 1] = ui.box({ width = round(x.bw * 0.62), height = math.max(2, round(x.w * 0.05)), radius = 2, fill = SAKE_BLUE })
    end
    for _, f in ipairs(x.face) do inner[#inner + 1] = f end
    local body = ui.column({
        width = round(x.bw * 0.94), height = bodyH, fill = CERAMIC, radius = round(x.w * 0.42),
        border = CERAMIC_EDGE, borderWidth = 1, align = "center", justify = "center", gap = round(x.w * 0.03),
    }, inner)
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { lip, neck, body })
end

local function tofuBody(c, x)
    return ui.column({
        width = x.bw, height = x.bh, fill = x.color or "primary", radius = round(math.min(x.bw, x.bh) * 0.34),
        align = "center", justify = "center", gap = round(x.w * 0.10),
    }, x.face)
end

--- Taiyaki: a fish-shaped waffle cake; a rounder head with the face, a smaller tail block.
--- A generic "wiggle" fidget (see EXTRA_FIDGETS) reads as the tail wagging.
local function taiyakiBody(c, x)
    local tailW = round(x.bw * 0.22)
    local headW = x.bw - tailW
    local inner = {}
    for _, f in ipairs(x.face) do inner[#inner + 1] = f end
    if x.detail then
        -- the crimped waffle seam
        local cw, ch = round(x.w * 0.05), math.max(2, round(x.w * 0.03))
        local function crimp() return ui.box({ width = cw, height = ch, radius = 1, fill = c.batterDark }) end
        inner[#inner + 1] = ui.row({ gap = round(x.w * 0.03), align = "center" }, { crimp(), crimp(), crimp(), crimp() })
    end
    local head = ui.column({
        width = headW, height = x.bh, fill = c.batter, radius = round(x.bh * 0.46),
        align = "center", justify = "center", gap = round(x.w * 0.035),
    }, inner)
    local tail = ui.column({ width = tailW, height = round(x.bh * 0.46), fill = c.batterDark, radius = round(tailW * 0.30) }, {})
    local tailWrap = ui.column({ width = tailW, height = x.bh, align = "center", justify = "center" }, { tail })
    return ui.row({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { head, tailWrap })
end

--- Ramune: a soda bottle with a marble in the neck that pops up on the happy hop (x.lift,
--- the same jump used for the whole frame when a turn finishes).
local function ramuneBody(c, x)
    local capH = math.max(2, round(x.bh * 0.06))
    local neckH = round(x.bh * 0.22)
    local bodyH = x.bh - capH - neckH
    local cap = ui.box({ width = round(x.bw * 0.22), height = capH, radius = 2, fill = "#d9d9d9" })
    local marbleD = round(x.bw * 0.16)
    local bob = round((x.lift or 0) * neckH * 0.6)
    local neck = ui.column({
        width = round(x.bw * 0.30), height = neckH, fill = c.glass, radius = round(x.bw * 0.08),
        align = "center", justify = "end",
    }, {
        spacerH(math.max(0, neckH - marbleD - bob)),
        ui.box({ width = marbleD, height = marbleD, radius = round(marbleD / 2), fill = MARBLE }),
        spacerH(bob),
    })
    local inner = {}
    if x.detail then
        inner[#inner + 1] = ui.box({ width = round(x.bw * 0.5), height = math.max(2, round(x.w * 0.03)), radius = 2, fill = c.glassDark })
    end
    for _, f in ipairs(x.face) do inner[#inner + 1] = f end
    local body = ui.column({
        width = x.bw, height = bodyH, fill = c.glass, radius = round(x.w * 0.26),
        border = c.glassDark, borderWidth = 1, align = "center", justify = "center", gap = round(x.w * 0.03),
    }, inner)
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { cap, neck, body })
end

--- Kakigori: shaved ice in a cup; the dome shrinks and the syrup pools as x.fullness (the
--- active session's context window, 0..1) climbs, so it visibly melts as the context fills up.
local function kakigoriBody(c, x)
    local fullness = clamp(x.fullness or 0, 0, 1)
    local cupH = round(x.bh * 0.22)
    local iceH = round((x.bh - cupH) * (1 - 0.35 * fullness))
    local gapH = x.bh - cupH - iceH
    local inner = {}
    if x.detail then
        inner[#inner + 1] = ui.box({ width = round(x.bw * 0.6), height = math.max(2, round(x.w * 0.03)), radius = 2, fill = c.syrup })
    end
    for _, f in ipairs(x.face) do inner[#inner + 1] = f end
    local dome = ui.column({
        width = round(x.bw * 0.88), height = iceH, fill = ICE, radius = round(x.w * 0.30),
        border = ICE_SHADE, borderWidth = 1, align = "center", justify = "center", gap = round(x.w * 0.03),
    }, inner)
    local drip = ui.box({
        width = round(x.bw * (0.30 + 0.40 * fullness)), height = math.max(2, round(x.w * 0.035)), radius = 2,
        fill = c.syrup, opacity = 0.5 + 0.5 * fullness,
    })
    local cup = ui.box({ width = x.bw, height = cupH, fill = CERAMIC, radius = round(x.w * 0.18) })
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { spacerH(gapH), dome, drip, cup })
end

-- Moods the wasabi ball turns red for (otherwise it stays its usual green).
local ANGRY_MOODS = { sad = true, annoyed = true, dizzy = true, startled = true }

--- Wasabi: a ball of wasabi, green and calm until something goes wrong (a deny, a poke,
--- a sneeze) turns it red.
local function wasabiBody(c, x)
    local color = ANGRY_MOODS[x.mood] and WASABI_RED or WASABI_GREEN
    return ui.column({
        width = x.bw, height = x.bh, fill = color, radius = round(math.min(x.bw, x.bh) * 0.46),
        align = "center", justify = "center", gap = round(x.w * 0.035),
    }, x.face)
end

--- Temaki: a tall hand roll, nori on the outside, a sliver of rice, filling peeking at the top.
local function temakiBody(c, x)
    local fillH = round(x.bh * 0.14)
    local wrapH = x.bh - fillH
    local filling = ui.column({
        width = round(x.bw * 0.70), height = fillH, fill = c.filling, radius = round(fillH * 0.40),
        align = "center", justify = "center",
    }, {})
    local inner = {}
    if x.detail then
        inner[#inner + 1] = ui.box({ width = round(x.bw * 0.55), height = math.max(2, round(x.w * 0.03)), radius = 2, fill = RICE })
    end
    for _, f in ipairs(x.face) do inner[#inner + 1] = f end
    local wrap = ui.column({
        width = x.bw, height = wrapH, fill = NORI, radius = round(x.w * 0.14),
        align = "center", justify = "center", gap = round(x.w * 0.035),
    }, inner)
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { filling, wrap })
end

--- Matcha: a chawan of whisked tea; steam rises while working (character.steam) and it
--- sips like the other hot drinks (see EXTRA_FIDGETS).
local function matchaBody(c, x)
    local surfH = round(x.bh * 0.22)
    local foam = {}
    if x.detail then
        foam = { ui.box({ width = round(x.bw * 0.5), height = math.max(2, round(x.w * 0.03)), radius = 2, fill = MATCHA_FOAM }) }
    end
    local surface = ui.column({
        width = round(x.bw * 0.90), height = surfH, fill = MATCHA_GREEN, radius = round(surfH * 0.5),
        align = "center", justify = "center",
    }, foam)
    local bowl = ui.column({
        width = x.bw, height = x.bh - surfH, fill = CERAMIC, radius = round(x.w * 0.30),
        border = CERAMIC_EDGE, borderWidth = 1, align = "center", justify = "center", gap = round(x.w * 0.03),
    }, x.face)
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { surface, bowl })
end

--- Tamagoyaki: a rolled, layered omelette; the face is on the top layer.
local function tamagoyakiBody(c, x)
    local h1 = round(x.bh * 0.40)
    local seamH = math.max(2, round(x.bh * 0.06))
    local h2 = x.bh - h1 - seamH
    local top = ui.column({
        width = x.bw, height = h1, fill = BATTER, radius = round(x.w * 0.12),
        align = "center", justify = "center", gap = round(x.w * 0.03),
    }, x.face)
    local seam = ui.box({ width = round(x.bw * 0.94), height = seamH, radius = 2, fill = BATTER_DARK })
    local bottom = ui.column({ width = x.bw, height = h2, fill = BATTER, radius = round(x.w * 0.12) }, {})
    return ui.column({ width = x.bw, height = x.bh, align = "center", justify = "start" }, { top, seam, bottom })
end

local BODIES = {
    nigiri = nigiriBody, maki = makiBody, uramaki = uramakiBody, onigiri = onigiriBody, tofu = tofuBody,
    ikura = ikuraBody, inari = inariBody, dorayaki = dorayakiBody, dango = dangoBody, tempura = tempuraBody,
    ravioli = ravioliBody, bao = baoBody, ramen = ramenBody, takoyaki = takoyakiBody, miso = misoBody,
    edamame = edamameBody, mochi = mochiBody, boba = bobaBody, sake = sakeBody,
    taiyaki = taiyakiBody, ramune = ramuneBody, kakigori = kakigoriBody, wasabi = wasabiBody,
    temaki = temakiBody, matcha = matchaBody, tamagoyaki = tamagoyakiBody,
}

-- Floating decorations (z, hearts, sweat, ...) ----------------------------------

--- Stack positioned items {x, y, h, node} into a column of the given size.
local function stack(width, height, items)
    table.sort(items, function(a, b) return a.y < b.y end)
    local children, cursor = {}, 0
    for _, it in ipairs(items) do
        local y = clamp(it.y, cursor, math.max(cursor, height - it.h))
        children[#children + 1] = spacerH(y - cursor)
        children[#children + 1] = ui.row({}, { spacerW(it.x), it.node })
        cursor = y + it.h
    end
    return ui.column({ width = width, height = round(height), align = "start", justify = "start" }, children)
end

function Pet:decoration(w, height, width)
    local kind, ph = self.deco, self.decoPhase
    local items = {}

    -- Several copies rising from the body and fading out.
    local function rising(count, make)
        for i = 0, count - 1 do
            local q = (ph + i / count) % 1 -- 0 = just spawned, 1 = gone
            local node, h = make(i, q)
            items[#items + 1] = { x = width * 0.08 + q * width * 0.35, y = (1 - q) * height * 0.62, h = h, node = node }
        end
    end
    local function one(node, h, x, y) items[#items + 1] = { x = x, y = y, h = h, node = node } end

    if kind == "z" then
        rising(3, function(i, q)
            local size = round(w * (0.11 + 0.04 * i)) + 4
            return ui.label({ text = (i == 1) and "Z" or "z", fontSize = size, fontWeight = "bold", color = "on_surface_variant", opacity = 1 - q * q }), size + 2
        end)
    elseif kind == "heart" then
        rising(3, function(i, q)
            local size = round(w * (0.11 + 0.04 * i)) + 2
            return ui.glyph({ name = "heart", size = size, color = HEART, opacity = 1 - q * q }), size + 4
        end)
    elseif kind == "note" then
        rising(3, function(i, q)
            local size = round(w * 0.13)
            return ui.glyph({ name = "music", size = size, color = "primary", opacity = 1 - q * q }), size + 4
        end)
    elseif kind == "steam" then
        rising(3, function(i, q)
            local sw = math.max(2, round(w * 0.045))
            return ui.box({ width = sw, height = round(w * 0.12), radius = round(sw / 2), fill = "#ffffff", opacity = 0.5 * (1 - q) }), round(w * 0.12)
        end)
    elseif kind == "crumbs" then
        for i = 0, 2 do
            local q = (ph * 2 + i / 3) % 1 -- falling crumbs
            local d = math.max(2, round(w * 0.04))
            one(ui.box({ width = d, height = d, radius = round(d / 2), fill = "#d9a766", opacity = 1 - q }), d,
                width * (0.15 + 0.25 * i), height * (0.35 + 0.5 * q))
        end
    elseif kind == "droplets" then
        local q = clamp(((self.curP or 0) - 0.55) / 0.45, 0, 1) -- only after the sneeze bursts
        if q > 0 then
            for i = 0, 2 do
                local d = math.max(2, round(w * 0.045))
                one(ui.box({ width = d, height = d, radius = round(d / 2), fill = SWEAT, opacity = 1 - q }), d,
                    width * (0.10 + 0.55 * q) , height * (0.30 + 0.10 * i))
            end
        end
    elseif kind == "dots" then
        local n = 1 + math.floor(self.t / 350) % 3
        local size = round(w * 0.15) + 2
        one(ui.label({ text = string.rep(".", n), fontSize = size, fontWeight = "bold", color = "on_surface_variant" }), size + 2, width * 0.15, height * 0.12)
    elseif kind == "hic" or kind == "hi" then
        local size = round(w * 0.13) + 2
        one(ui.label({ text = (kind == "hic") and "hic!" or "hi!", fontSize = size, fontWeight = "bold", color = "primary" }), size + 4, width * 0.10, height * 0.10)
    elseif kind == "sweat" then
        local d = round(w * 0.08)
        local bob = math.sin(self.t / 300) * height * 0.03
        one(ui.box({ width = d, height = round(d * 1.5), radius = round(d / 2), fill = SWEAT }), round(d * 1.5), width * 0.2, height * 0.10 + bob)
    elseif kind == "tear" then
        local d = round(w * 0.06)
        one(ui.box({ width = d, height = round(d * 1.5), radius = round(d / 2), fill = SWEAT, opacity = 1 - ph * 0.8 }), round(d * 1.5),
            width * 0.15, height * (0.30 + ph * 0.35))
    elseif kind == "sparkle" then
        local on = (math.floor(self.t / 220) % 2 == 0)
        local size = round(w * 0.16)
        one(ui.glyph({ name = "sparkles", size = size, color = "#ffd24a", opacity = on and 1 or 0.45 }), size + 2, width * 0.25, height * 0.05)
    elseif kind == "pencil" or kind == "terminal" or kind == "search" then
        local glyph = ({ pencil = "pencil", terminal = "terminal-2", search = "search" })[kind]
        local size = round(w * 0.16)
        local pulse = 0.55 + 0.45 * sine(self.t, 900)
        one(ui.glyph({ name = glyph, size = size, color = "primary", opacity = pulse }), size + 2, width * 0.22, height * 0.08)
    end
    return stack(width, height, items)
end

--- A small status bubble (like a chat "typing" indicator) for the top-left of the pet.
function Pet:badge(kind, width, height)
    local d = math.max(10, round(width * 0.95))
    local inner
    if kind == "alert" then
        inner = ui.label({ text = "!", fontSize = round(d * 0.7), fontWeight = "bold", color = "#ffffff" })
    else
        local dot = math.max(2, round(d * 0.16))
        local dots = {}
        for i = 0, 2 do
            -- The dots pulse one after the other.
            local pulse = 0.4 + 0.6 * math.max(0, math.sin((self.t / 260) - i * 0.9))
            dots[#dots + 1] = ui.box({ width = dot, height = dot, radius = round(dot / 2), fill = "#ffffff", opacity = pulse })
        end
        inner = ui.row({ gap = math.max(1, round(dot * 0.5)), align = "center" }, dots)
    end
    local bubble = ui.column({
        width = d, height = d, fill = (kind == "alert") and "#f59e0b" or "#2f6fed", radius = round(d / 2),
        border = "#0d0f13", borderWidth = 2, align = "center", justify = "center",
    }, { inner })
    return ui.column({ key = "badge", width = round(width), height = round(height), align = "start", justify = "start" }, { bubble })
end

--- Build the UI tree for a character `size` px wide.
--- opts.room    extra vertical room (px) above the body for jumps (default size*0.3)
--- opts.margin  horizontal room (px) on each side for shaking
--- opts.mouth   draw the mouth (default true)
--- opts.color   body color (tofu only)
--- opts.maxHeight  total height budget (px): tall characters (dango) shrink to fit
--- opts.badge   "work" (blue bubble with three dots) or "alert" (orange "!"): a status
---              bubble in the top-left corner (needs a margin of ~20 px)
--- opts.badgeSlot  keep the room for the bubble even when there is none, so the pet does
---              not jump sideways when the bubble comes and goes
function Pet:build(size, opts)
    opts = opts or {}
    local c = self.char
    local w = size
    local room = opts.room or round(w * 0.3)
    if opts.maxHeight then w = math.min(size, math.floor((opts.maxHeight - room) / c.ratio)) end
    local detail = w >= 48 -- stripes, shine and floating decorations are skipped at bar size
    local margin = opts.margin or round(w * 0.16)

    local bw = round(w * (1 + self.sx))
    local baseH = round(w * c.ratio)
    local bh = round(baseH * (1 + self.sy))

    local body = BODIES[c.kind](c, {
        w = w, bw = bw, bh = bh, detail = detail, color = opts.color, face = self:face(w, detail, opts),
        mood = self.cur, lift = self.lift, fullness = (self.contextPct or 0) / 100,
    })

    -- Frame: leading spacers place the body (shake = x, lift = y). The right-hand
    -- room for decorations is always reserved so the layout never jumps.
    local decoW = detail and round(w * 0.36) or 0
    local fw = round(w + margin * 2 + decoW)
    local fh = round(baseH + room)
    local left = margin + round(self.shake * margin) - round((bw - w) / 2)
    local topGap = fh - bh - round(self.lift * room)
    local rowChildren = { spacerW(left), body }
    if opts.badge or opts.badgeSlot then
        -- A status bubble on the left, level with the top of the body; the room is always
        -- reserved so the frame keeps the same width with or without it. The empty slot is a
        -- column under the same key, not a spacer: Noctalia would otherwise hand the bubble's
        -- nodes to the body when the bubble goes, and the body would keep its fill and opacity.
        local slot = opts.badge and self:badge(opts.badge, margin, bh)
            or ui.column({ key = "badge", width = round(margin), height = round(bh) }, {})
        table.insert(rowChildren, 1, slot)
        fw = fw + margin
    end
    if detail then rowChildren[#rowChildren + 1] = self:decoration(w, baseH, decoW) end
    return ui.column({ width = fw, height = fh, justify = "start", align = "start" }, {
        spacerH(topGap),
        ui.row({}, rowChildren),
    })
end

--- One-line caption for the panel.
function Pet:caption(nSessions, nWorking, nPending)
    local mood = self.cur or self.baseMood
    if mood == "sleep" then return "Zzz... the daemon is asleep" end
    if mood == "nap" then return "Zzz... taking a nap" end
    if mood == "alert" or nPending > 0 then
        return nPending <= 1 and "I need you!" or ("I need you! (" .. nPending .. ")")
    end
    local reactions = {
        love = "Thank you!", sad = "Aww...", startled = "Whoa!", dizzy = "Whoa, everything is spinning!",
        annoyed = "Hey!", happy = "Done!", yawn = "*yawn*", worried = "Usage is getting high...",
        stuffed = "So full... compact the context?", wave = "Hi there!", greeting = "Hello!", eat = "Nom nom nom", dance = "La la la~",
        wink = ";)", blush = "Stop it, you...", sneeze = "Ah... ah... choo!", hum = "Hmm hmm hmm~",
        think = "Hmmm...", spin = "Wheee!", bounce = "Boing!", sip = "*sip*", hiccup = "Hic!", jiggle = "Jiggle jiggle",
    }
    if reactions[mood] then return reactions[mood] end
    if nWorking > 0 then
        local what = ({ write = "writing", run = "running a command", search = "searching", think = "thinking" })[self.workKind]
        if nWorking == 1 then return "Your agent is " .. (what or "working") .. "..." end
        return "Your agents are working on " .. nWorking .. " sessions..."
    end
    if nSessions > 0 then return "All quiet" end
    return "No sessions, hi!"
end

return Pet
