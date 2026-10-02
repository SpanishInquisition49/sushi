--!nonstrict
-- lib/sounds.luau — loads the pet's 28 sounds (plugin/sounds/*.wav, made by
-- tools/make_sounds.py) and plays them by name. Noctalia allows at most 8 pending
-- loads and one playback per name, and applies the user's global sound settings
-- ([audio].enable_sounds / sound_volume) itself.

local S = {}

S.NAMES = {
    "greeting", "hello", "happy", "love", "sad", "startled", "annoyed", "dizzy", "poke", "hop",
    "bounce", "wiggle", "squish", "yawn", "sneeze", "hiccup", "hum", "think", "wink", "blush",
    "spin", "eat", "dance", "nap", "wake", "alert", "approve", "deny",
}

local MAX_IN_FLIGHT = 4

local api, dir
local ready = {}
local nextIndex, inFlight = 1, 0
local pending -- a sound asked for before it finished loading (only the latest is kept)

local function pump()
    while inFlight < MAX_IN_FLIGHT and nextIndex <= #S.NAMES do
        local name = S.NAMES[nextIndex]
        nextIndex = nextIndex + 1
        inFlight = inFlight + 1
        local ok = pcall(api.sound.load, name, dir .. "/" .. name .. ".wav", function(result)
            inFlight = inFlight - 1
            if result ~= false then ready[name] = true end
            if pending == name then
                pending = nil
                pcall(api.sound.play, name)
            end
            pump()
        end)
        if not ok then inFlight = inFlight - 1 end
    end
end

--- Start loading. `noctalia` is the runtime table; `pluginDir` the folder holding sounds/.
function S.init(noctalia, pluginDir)
    if not (noctalia and noctalia.sound and noctalia.sound.load and noctalia.sound.play) then return false end
    api, dir = noctalia, pluginDir .. "/sounds"
    pump()
    return true
end

function S.play(name)
    if not api then return end
    if ready[name] then
        pcall(api.sound.play, name)
    else
        pending = name
    end
end

function S.isReady(name) return ready[name] == true end

return S
