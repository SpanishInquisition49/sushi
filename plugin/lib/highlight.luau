--!nonstrict
-- lib/highlight.luau — a tiny syntax highlighter for the live viewer.
-- tokens(lang, line) → { {text=..., color=...}, ... } with neighbours of the same color merged.
-- It is a line-by-line scanner (strings, comments, numbers, keywords, calls, types),
-- not a parser: good enough to make a diff readable.

local H = {}

H.COLORS = {
    plain = "#abb2bf",
    keyword = "#c678dd",
    string = "#98c379",
    number = "#d19a66",
    comment = "#7f848e",
    call = "#61afef",
    type = "#e5c07b",
    key = "#e06c75",
}

local function set(words)
    local t = {}
    for w in words:gmatch("%S+") do t[w] = true end
    return t
end

local C_LIKE_KW = set([[
    if else for while do switch case break continue return fn function let const var mut pub use mod struct enum
    impl trait type interface class extends implements new import export from as default async await yield try
    catch finally throw throws static void this self super match loop in of where crate typeof instanceof
    package go func chan select defer range public private protected final abstract null nil true false undefined
]])

local LANGS = {
    rust = { kw = C_LIKE_KW, line = "//", strings = "\"'" },
    ts = { kw = C_LIKE_KW, line = "//", strings = "\"'`" },
    js = { kw = C_LIKE_KW, line = "//", strings = "\"'`" },
    c = { kw = C_LIKE_KW, line = "//", strings = "\"'" },
    cpp = { kw = C_LIKE_KW, line = "//", strings = "\"'" },
    go = { kw = C_LIKE_KW, line = "//", strings = "\"'`" },
    java = { kw = C_LIKE_KW, line = "//", strings = "\"'" },
    python = { kw = set("def class if elif else for while return import from as with try except finally raise lambda pass break continue in is not and or None True False yield async await global nonlocal assert del"), line = "#", strings = "\"'" },
    lua = { kw = set("function local if then else elseif end for while do repeat until return break in and or not nil true false"), line = "--", strings = "\"'" },
    shell = { kw = set("if then else elif fi for while do done case esac in function return export local echo cd exit set unset source"), line = "#", strings = "\"'" },
    json = { kw = set("true false null"), strings = "\"", keys = true },
    toml = { kw = set("true false"), line = "#", strings = "\"'", keys = true },
    yaml = { kw = set("true false null"), line = "#", strings = "\"'", keys = true },
    nix = { kw = set("let in if then else with inherit rec import true false null"), line = "#", strings = "\"'" },
    css = { kw = set("important"), strings = "\"'" },
    html = { kw = set(""), strings = "\"'" },
    markdown = { kw = set(""), strings = "" },
}

local cache = {}
local cacheSize = 0

local function push(out, text, color)
    if text == "" then return end
    local last = out[#out]
    if last and last.color == color then
        last.text = last.text .. text
    else
        out[#out + 1] = { text = text, color = color }
    end
end

local function scan(lang, line)
    local spec = LANGS[lang]
    local out = {}
    if not spec then
        out[1] = { text = line, color = H.COLORS.plain }
        return out
    end
    local i, n = 1, #line
    while i <= n do
        local c = line:sub(i, i)
        if spec.line and line:sub(i, i + #spec.line - 1) == spec.line then
            push(out, line:sub(i), H.COLORS.comment)
            break
        elseif spec.strings:find(c, 1, true) then
            local j = i + 1
            while j <= n do
                local d = line:sub(j, j)
                if d == "\\" then j = j + 2 elseif d == c then break else j = j + 1 end
            end
            local text = line:sub(i, math.min(j, n))
            -- A JSON/TOML/YAML string followed by ":" or "=" is a key.
            local color = H.COLORS.string
            if spec.keys and line:sub(j + 1):match("^%s*[:=]") then color = H.COLORS.key end
            push(out, text, color)
            i = j + 1
        elseif c:match("%d") then
            local num = line:match("^[%d][%w%._]*", i)
            push(out, num, H.COLORS.number)
            i = i + #num
        elseif c:match("[%a_]") then
            local word = line:match("^[%w_]+", i)
            local rest = line:sub(i + #word)
            local color = H.COLORS.plain
            if spec.kw[word] then
                color = H.COLORS.keyword
            elseif rest:match("^%s*%(") then
                color = H.COLORS.call
            elseif word:match("^%u") and #word > 1 then
                color = H.COLORS.type
            end
            push(out, word, color)
            i = i + #word
        else
            local chunk = line:match("^[^%w_\"'`]+", i) or c
            -- Stop before a comment marker so the next turn of the loop sees it.
            if spec.line then
                local p = chunk:find(spec.line, 1, true)
                if p and p > 1 then chunk = chunk:sub(1, p - 1) end
            end
            push(out, chunk, H.COLORS.plain)
            i = i + #chunk
        end
    end
    return out
end

--- Tokens for one line of `lang` code (cached: the same lines are redrawn every frame).
function H.tokens(lang, line)
    local key = lang .. "\0" .. line
    local hit = cache[key]
    if hit then return hit end
    if cacheSize > 2000 then cache, cacheSize = {}, 0 end
    local out = scan(lang, line)
    cache[key] = out
    cacheSize = cacheSize + 1
    return out
end

return H
