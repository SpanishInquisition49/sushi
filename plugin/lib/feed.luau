--!nonstrict
-- lib/feed.luau — feeding a file to the pet: it swallows it and the next chat message is about it.
--
-- Noctalia does not take part in Wayland drag & drop (it binds no wl_data_device), so a file
-- dragged from a file manager never reaches the bar or the panel. A file arrives instead:
--   * from the clipboard: copy it in the file manager, then middle-click the pet (or "Feed a file"
--     in the panel); the clipboard is read with `wl-paste` (text/uri-list, then plain text)
--   * over IPC:  noctalia msg plugin scanna/sushi:state all feed /path/to/file
-- The fed file is shared through noctalia.state:  "fedFile" = { path, name, ts }  (path "" = none).
-- Call F.init(noctalia) once from the entry script (modules cannot see `noctalia`).

local F = {}

local PANEL_ID = "scanna/sushi:panel"

local noctalia

function F.init(n) noctalia = n end

local function decode(s)
    return (s:gsub("%%(%x%x)", function(h) return string.char(tonumber(h, 16)) end))
end

--- The first file path in a text/uri-list or plain-text clipboard: `file://` URIs (percent-decoded)
--- or absolute paths. nil when there is none.
function F.pathFrom(text)
    if type(text) ~= "string" then return nil end
    for line in text:gmatch("[^\r\n]+") do
        line = line:match("^%s*(.-)%s*$")
        if line:sub(1, 7) == "file://" then
            local rest = line:sub(8)
            local path = rest:sub(1, 1) == "/" and rest or rest:match("^[^/]*(/.*)$") -- file://host/path
            if path then return decode(path) end
        elseif line:sub(1, 1) == "/" then
            return line
        elseif line:sub(1, 2) == "~/" then
            return noctalia.expandPath(line)
        end
    end
    return nil
end

--- The file currently fed (a table {path, name, ts}) or nil.
function F.current()
    local v = noctalia.state.get("fedFile")
    if type(v) == "table" and type(v.path) == "string" and v.path ~= "" then return v end
    return nil
end

--- Feed `path`: remember it, switch the panel to the chat and open it. The caller makes its pet eat.
function F.feed(path)
    local name = path:match("([^/]+)/*$") or path
    noctalia.state.set("fedFile", { path = path, name = name, ts = noctalia.nowMs() })
    noctalia.state.set("panelTab", { tab = "chat" })
    noctalia.runAsync({ "noctalia", "msg", "panel-open", PANEL_ID }, function() end)
end

--- Forget the fed file (sent with a message, or discarded).
function F.clear()
    noctalia.state.set("fedFile", { path = "" })
end

--- Look for a file in the clipboard; calls cb(path) or cb(nil).
function F.fromClipboard(cb)
    local function paste(kind, next)
        noctalia.runAsync({ "wl-paste", "--no-newline", "--type", kind }, function(res)
            local path = type(res) == "table" and res.exitCode == 0 and F.pathFrom(res.stdout) or nil
            if path or not next then cb(path) else next() end
        end)
    end
    paste("text/uri-list", function() paste("text/plain", nil) end)
end

return F
