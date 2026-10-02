--!nonstrict
-- lib/viewer.luau — the live viewer: draws what a step does, like a tiny editor.
--   diff     file tab (language chip, name, "modified" dot, folder), line numbers,
--            red/green changed lines with an accent bar, syntax colors, and a
--            typewriter effect on the added lines
--   terminal "$ command" and the tail of its output
--   file     an excerpt of a file that was read
--   text     a title and a few lines (searches, subagents, ...)
-- Call V.init(ui, Highlight) once from the entry script (modules cannot see `ui`).

local V = {}

local ui, H

function V.init(u, highlight)
    ui, H = u, highlight
end

local COLORS = {
    bg = "#0d0f13",
    head = "#151820",
    gutter = "#5c6370",
    text = "#e6e6ea",
    dim = "#7f848e",
    delBg = "#2e1a1f", delBar = "#f0524f", delText = "#d98a8a", delGutter = "#b0585c",
    addBg = "#122820", addBar = "#3ecf8e", addGutter = "#2f9d6b",
    ok = "#4cc38a", fail = "#f0524f", prompt = "#3ecf8e",
}
V.COLORS = COLORS

local MONO = "monospace"

-- Language chips: { label, background, text color }.
local CHIPS = {
    rust = { "RS", "#dea584", "#2b1a10" }, ts = { "TS", "#3178c6", "#ffffff" }, js = { "JS", "#f7df1e", "#222222" },
    python = { "PY", "#3572a5", "#ffffff" }, lua = { "LUA", "#51a0cf", "#10202c" }, shell = { "SH", "#89e051", "#12240a" },
    json = { "{}", "#cbcb41", "#222222" }, toml = { "TOML", "#9c4221", "#ffffff" }, markdown = { "MD", "#519aba", "#0e2230" },
    c = { "C", "#5c6bc0", "#ffffff" }, cpp = { "C++", "#f34b7d", "#ffffff" }, go = { "GO", "#00add8", "#00323f" },
    java = { "JAVA", "#b07219", "#ffffff" }, html = { "HTML", "#e34c26", "#ffffff" }, css = { "CSS", "#563d7c", "#ffffff" },
    yaml = { "YML", "#cb171e", "#ffffff" }, nix = { "NIX", "#7ebae4", "#10202c" }, other = { "TXT", "#6b7280", "#ffffff" },
}

function V.chip(lang)
    local c = CHIPS[lang] or CHIPS.other
    return ui.column({
        fill = c[2], radius = 4, paddingH = 5, paddingV = 1, align = "center", justify = "center",
    }, { ui.label({ text = c[1], fontSize = 10, fontWeight = "bold", color = c[3] }) })
end

local function split(path)
    local dir, name = path:match("^(.*)/([^/]+)$")
    if not name then return "", path end
    return dir, name
end

local function header(lang, file, modified, tag)
    local dir, name = split(file or "")
    local row = { V.chip(lang), ui.label({ text = name, fontSize = 13, fontWeight = "semibold", color = COLORS.text }) }
    if modified then row[#row + 1] = ui.box({ width = 7, height = 7, radius = 4, fill = "#e5a50a" }) end
    row[#row + 1] = ui.spacer({ flexGrow = 1 })
    local right = tag or dir
    if right ~= "" then
        row[#row + 1] = ui.label({ text = right, fontSize = 11, color = "#6b7280", maxLines = 1 })
    end
    return ui.row({ fill = COLORS.head, paddingH = 10, paddingV = 6, gap = 8, align = "center" }, row)
end

--- UTF-8 aware prefix of `text` with `n` characters.
local function prefix(text, n)
    if n >= #text then return text end
    if utf8 and utf8.len and utf8.offset then
        local len = utf8.len(text)
        if len and n >= len then return text end
        if len then return text:sub(1, utf8.offset(text, n + 1) - 1) end
    end
    return text:sub(1, n)
end

local function codeText(lang, text, color)
    if color then
        return ui.label({ text = text == "" and " " or text, fontSize = 12, fontFamily = MONO, color = color })
    end
    local parts = {}
    for _, t in ipairs(H.tokens(lang, text)) do
        parts[#parts + 1] = ui.label({ text = t.text, fontSize = 12, fontFamily = MONO, color = t.color })
    end
    if #parts == 0 then parts[1] = ui.label({ text = " ", fontSize = 12, fontFamily = MONO, color = COLORS.text }) end
    return ui.row({ gap = 0, align = "center" }, parts)
end

local function numbered(kind, n, children, key)
    local bg, bar, gut = nil, "#00000000", COLORS.gutter
    if kind == "del" then bg, bar, gut = COLORS.delBg, COLORS.delBar, COLORS.delGutter
    elseif kind == "add" then bg, bar, gut = COLORS.addBg, COLORS.addBar, COLORS.addGutter end
    local marker = (kind == "del") and "-" or ((kind == "add") and "+" or " ")
    return ui.row({ key = key, fill = bg, gap = 6, align = "center" }, {
        ui.box({ width = 3, height = 18, fill = bar }),
        ui.label({ text = (n and n > 0) and tostring(n) or "", width = 34, textAlign = "end", fontSize = 11, fontFamily = MONO, color = gut }),
        ui.label({ text = marker, width = 10, fontSize = 12, fontFamily = MONO, color = gut }),
        children,
    })
end

--- A diff as editor rows. opts.typed = characters of the added lines revealed so far
--- (nil = all); opts.maxRows trims context lines when there are too many rows.
local function diffRows(d, opts)
    local lines = d.lines or {}
    local maxRows = opts.maxRows or 40
    local from, to = 1, #lines
    -- Trim unchanged lines from the edges first.
    local function isCtx(i) return lines[i] and lines[i].kind == "ctx" end
    while (to - from + 1) > maxRows and isCtx(from) do from = from + 1 end
    while (to - from + 1) > maxRows and isCtx(to) do to = to - 1 end
    to = math.min(to, from + maxRows - 1)

    local rows = {}
    local remaining = opts.typed
    local cursorAt -- row index that gets the typing cursor
    for i = from, to do
        local l = lines[i]
        local text = l.text
        local show = true
        if l.kind == "add" and remaining then
            if remaining <= 0 then
                show = false
            else
                local len = utf8 and utf8.len and utf8.len(text) or #text
                text = prefix(text, remaining)
                remaining = remaining - (len or #text)
                if remaining <= 0 then cursorAt = #rows + 1 end
            end
        end
        if show then
            local body
            if l.kind == "del" then body = codeText(d.lang, text, COLORS.delText) else body = codeText(d.lang, text) end
            rows[#rows + 1] = { kind = l.kind, n = l.n, body = body }
        end
    end
    local out = {}
    for idx, r in ipairs(rows) do
        local body = r.body
        if opts.cursor and cursorAt == idx then
            body = ui.row({ gap = 0, align = "center" }, { r.body, ui.label({ text = "|", fontSize = 12, fontFamily = MONO, color = COLORS.text }) })
        end
        out[#out + 1] = numbered(r.kind, r.n, body, "row-" .. idx)
    end
    if (d.more or 0) > 0 then
        out[#out + 1] = ui.label({ text = string.format("  … %d more changed lines", d.more), fontSize = 11, color = COLORS.dim })
    end
    return out
end

local function card(head, body, opts)
    return ui.column({ fill = COLORS.bg, radius = 10, gap = 0, flexGrow = opts and opts.grow and 1 or nil }, {
        head,
        ui.column({ gap = 0, paddingV = 6 }, body),
    })
end

--- Draw `detail` (one of the daemon's Detail objects).
--- opts: typed (chars revealed), cursor (bool), maxRows, ok (nil while running), grow, cwdLabel
function V.render(detail, opts)
    opts = opts or {}
    if type(detail) ~= "table" then
        return ui.column({ fill = COLORS.bg, radius = 10, padding = 16, flexGrow = opts.grow and 1 or nil }, {
            ui.label({ text = opts.empty or "Nothing to show yet.", fontSize = 12, color = COLORS.dim }),
        })
    end
    local kind = detail.type
    if kind == "diff" then
        local modified = (opts.ok == nil)
        return card(header(detail.lang, detail.file, modified), diffRows(detail, opts), opts)
    elseif kind == "file" then
        local rows = {}
        for i, text in ipairs(detail.lines or {}) do
            rows[#rows + 1] = numbered("ctx", (detail.start or 1) + i - 1, codeText(detail.lang, text), "row-" .. i)
        end
        return card(header(detail.lang, detail.file, false, "read"), rows, opts)
    elseif kind == "terminal" then
        local head = ui.row({ fill = COLORS.head, paddingH = 10, paddingV = 6, gap = 8, align = "center" }, {
            ui.glyph({ name = "terminal-2", size = 14, color = COLORS.prompt }),
            ui.label({ text = "Terminal", fontSize = 13, fontWeight = "semibold", color = COLORS.text }),
        })
        local rows = {
            ui.row({ gap = 6, paddingH = 10, align = "start" }, {
                ui.label({ text = "$", fontSize = 12, fontFamily = MONO, color = COLORS.prompt }),
                ui.label({ text = detail.command or "", fontSize = 12, fontFamily = MONO, color = COLORS.text, maxLines = 3, flexGrow = 1 }),
            }),
        }
        for i, line in ipairs(detail.output or {}) do
            rows[#rows + 1] = ui.label({ key = "out-" .. i, text = line, fontSize = 12, fontFamily = MONO, color = COLORS.dim, maxLines = 1 })
        end
        if opts.ok == nil and #(detail.output or {}) == 0 then
            rows[#rows + 1] = ui.label({ text = opts.cursor and "running…" or "running", fontSize = 12, fontFamily = MONO, color = COLORS.dim })
        end
        return card(head, rows, opts)
    else -- text
        local head = ui.row({ fill = COLORS.head, paddingH = 10, paddingV = 6, align = "center" }, {
            ui.label({ text = detail.title or "", fontSize = 13, fontWeight = "semibold", color = COLORS.text, maxLines = 1 }),
        })
        local body = {}
        if (detail.body or "") ~= "" then
            body[1] = ui.label({ text = detail.body, fontSize = 12, color = COLORS.dim, maxLines = opts.bodyLines or 8 })
        end
        return card(head, { ui.column({ paddingH = 10, gap = 4 }, body) }, opts)
    end
end

--- Number of characters in the added lines of a diff (for the typewriter speed).
function V.addedChars(detail)
    local total = 0
    if type(detail) == "table" and detail.type == "diff" then
        for _, l in ipairs(detail.lines or {}) do
            if l.kind == "add" then total = total + (utf8 and utf8.len and utf8.len(l.text) or #l.text) end
        end
    end
    return total
end

return V
