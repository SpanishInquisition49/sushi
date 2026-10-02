--!nonstrict
-- lib/chat.luau — the "Ask <agent> anything" tab: the conversation log, a text field
-- and stop / clear buttons. The daemon runs the conversation (the agent headless, no tools);
-- this only draws its state:  chat = { busy, error, messages = { {role, text}, ... } }.
-- Call C.init(ui) once from the entry script (modules cannot see `ui`).

local C = {}

local ui

function C.init(u) ui = u end

local MAX_WIDTH = 430 -- wrap width of a bubble

local function bubble(m, i, streaming)
    local mine = (m.role == "user")
    local text = m.text or ""
    if text == "" and streaming then text = "…" end
    local box = ui.column({
        key = "msg-" .. i, padding = 10, radius = 12,
        fill = mine and "primary/0.22" or "surface_variant/0.45",
    }, {
        ui.label({ text = text, fontSize = 13, color = "on_surface", maxLines = 60, maxWidth = MAX_WIDTH }),
    })
    return ui.row({ justify = mine and "end" or "start" }, { box })
end

--- The whole tab. opts:
---   optimistic  text just sent, shown until the daemon's state catches up (or nil)
---   inputKey    changes after each send, which recreates (and so empties) the field
---   draftReady  true when the field has text (enables the send button)
---   (the callbacks are the global functions onChatSubmit / onChatChange and the
---    buttons' own closures: onStop, onClear, onSend)
function C.view(chat, opts)
    chat = chat or {}
    opts = opts or {}
    local who = opts.label or "Claude Code" -- the agent the chat talks to
    local msgs = chat.messages or {}
    local rows = {}
    for i, m in ipairs(msgs) do
        rows[#rows + 1] = bubble(m, i, chat.busy and i == #msgs)
    end
    if opts.optimistic then
        rows[#rows + 1] = bubble({ role = "user", text = opts.optimistic }, #msgs + 1, false)
        rows[#rows + 1] = bubble({ role = "assistant", text = "" }, #msgs + 2, true)
    end
    if #rows == 0 then
        rows[1] = ui.column({ gap = 6, padding = 6 }, {
            ui.label({ text = "Ask " .. who .. " anything", fontSize = 16, fontWeight = "semibold", color = "on_surface" }),
            ui.label({
                text = "Quick questions and answers, right from the notch. This chat has no tools and no access to your files; it uses your " .. who .. " login.",
                fontSize = 12, color = "on_surface_variant", maxLines = 4, maxWidth = MAX_WIDTH,
            }),
        })
    end
    if chat.error then
        rows[#rows + 1] = ui.label({ text = tostring(chat.error), fontSize = 12, color = "error", maxLines = 4, maxWidth = MAX_WIDTH })
    end

    -- Changes whenever the log grows or the answer streams in, so the log stays pinned to the bottom.
    local last = msgs[#msgs]
    local rev = #msgs * 100000 + (last and #(last.text or "") or 0) + (opts.optimistic and 7 or 0)

    local busy = chat.busy or opts.optimistic ~= nil
    local input = ui.input({
        key = "chat-input-" .. tostring(opts.inputKey or 0),
        placeholder = busy and (who .. " is answering…") or ("Ask " .. who .. " anything…"),
        submitOnEnter = true, focus = true, enabled = not busy,
        onSubmit = "onChatSubmit", onChange = "onChatChange", flexGrow = 1,
    })
    local buttons = {}
    if busy then
        buttons[#buttons + 1] = ui.button({ glyph = "player-stop", variant = "outline", controlSize = "sm", tooltip = "Stop", onClick = "onChatStop" })
    else
        buttons[#buttons + 1] = ui.button({
            glyph = "send", variant = opts.draftReady and "primary" or "ghost", controlSize = "sm",
            tooltip = "Send", enabled = opts.draftReady and true or false, onClick = "onChatSend",
        })
    end
    buttons[#buttons + 1] = ui.button({ glyph = "trash", variant = "ghost", controlSize = "sm", tooltip = "New conversation", onClick = "onChatClear" })

    return ui.column({ flexGrow = 1, gap = 10 }, {
        ui.scroll({ flexGrow = 1, gap = 8, stickToBottom = true, scrollToBottomRev = rev }, rows),
        ui.row({ gap = 6, align = "center" }, { input, ui.row({ gap = 2, align = "center" }, buttons) }),
    })
end

return C
