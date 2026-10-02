// viewer.js — the live viewer: draws what a step does, like a tiny editor (port of lib/viewer.luau).
//   diff      file tab (language chip, name, "modified" dot, folder), line numbers, red/green changed
//             lines with an accent bar, syntax colors, and a typewriter effect on the added lines
//   terminal  "$ command" and the tail of its output
//   file      an excerpt of a file that was read
//   text      a title and a few lines (searches, subagents, ...)

import { ui } from "./ui.js";
import * as H from "./highlight.js";

export const COLORS = {
  bg: "#0d0f13", head: "#151820", gutter: "#5c6370", text: "#e6e6ea", dim: "#7f848e",
  delBg: "#2e1a1f", delBar: "#f0524f", delText: "#d98a8a", delGutter: "#b0585c",
  addBg: "#122820", addBar: "#3ecf8e", addGutter: "#2f9d6b",
  ok: "#4cc38a", fail: "#f0524f", prompt: "#3ecf8e",
};

const MONO = "monospace";

// Language chips: [label, background, text color].
const CHIPS = {
  rust: ["RS", "#dea584", "#2b1a10"], ts: ["TS", "#3178c6", "#ffffff"], js: ["JS", "#f7df1e", "#222222"],
  python: ["PY", "#3572a5", "#ffffff"], lua: ["LUA", "#51a0cf", "#10202c"], shell: ["SH", "#89e051", "#12240a"],
  json: ["{}", "#cbcb41", "#222222"], toml: ["TOML", "#9c4221", "#ffffff"], markdown: ["MD", "#519aba", "#0e2230"],
  c: ["C", "#5c6bc0", "#ffffff"], cpp: ["C++", "#f34b7d", "#ffffff"], go: ["GO", "#00add8", "#00323f"],
  java: ["JAVA", "#b07219", "#ffffff"], html: ["HTML", "#e34c26", "#ffffff"], css: ["CSS", "#563d7c", "#ffffff"],
  yaml: ["YML", "#cb171e", "#ffffff"], nix: ["NIX", "#7ebae4", "#10202c"], other: ["TXT", "#6b7280", "#ffffff"],
};

export function chip(lang) {
  const c = CHIPS[lang] || CHIPS.other;
  return ui.column({ fill: c[1], radius: 4, paddingH: 5, paddingV: 1, align: "center", justify: "center" }, [
    ui.label({ text: c[0], fontSize: 10, fontWeight: "bold", color: c[2] }),
  ]);
}

/** Split a path (either separator) into [dir, name]. */
function split(path) {
  const m = /^(.*)[\\/]([^\\/]+)$/.exec(path);
  return m ? [m[1], m[2]] : ["", path];
}

function header(lang, file, modified, tag) {
  const [dir, name] = split(file || "");
  const row = [chip(lang), ui.label({ text: name, fontSize: 13, fontWeight: "semibold", color: COLORS.text })];
  if (modified) row.push(ui.box({ width: 7, height: 7, radius: 4, fill: "#e5a50a" }));
  row.push(ui.spacer({ flexGrow: 1 }));
  const right = tag || dir;
  if (right) row.push(ui.label({ text: right, fontSize: 11, color: "#6b7280", maxLines: 1 }));
  return ui.row({ fill: COLORS.head, paddingH: 10, paddingV: 6, gap: 8, align: "center" }, row);
}

const chars = (s) => Array.from(s);

function codeText(lang, text, color) {
  if (color) return ui.label({ text: text === "" ? " " : text, fontSize: 12, fontFamily: MONO, color });
  const parts = H.tokens(lang, text).map((t) => ui.label({ text: t.text, fontSize: 12, fontFamily: MONO, color: t.color }));
  if (!parts.length) parts.push(ui.label({ text: " ", fontSize: 12, fontFamily: MONO, color: COLORS.text }));
  return ui.row({ gap: 0, align: "center" }, parts);
}

function numbered(kind, n, children) {
  let bg = null, bar = "#00000000", gut = COLORS.gutter;
  if (kind === "del") [bg, bar, gut] = [COLORS.delBg, COLORS.delBar, COLORS.delGutter];
  else if (kind === "add") [bg, bar, gut] = [COLORS.addBg, COLORS.addBar, COLORS.addGutter];
  const marker = kind === "del" ? "-" : kind === "add" ? "+" : " ";
  return ui.row({ fill: bg, gap: 6, align: "center" }, [
    ui.box({ width: 3, height: 18, fill: bar }),
    ui.label({ text: n > 0 ? String(n) : "", width: 34, textAlign: "end", fontSize: 11, fontFamily: MONO, color: gut }),
    ui.label({ text: marker, width: 10, fontSize: 12, fontFamily: MONO, color: gut }),
    children,
  ]);
}

/** A diff as editor rows. opts.typed = characters of the added lines revealed so far (undefined =
 *  all); opts.maxRows trims context lines when there are too many rows. */
function diffRows(d, opts) {
  const lines = d.lines || [];
  const maxRows = opts.maxRows || 40;
  let from = 0, to = lines.length - 1;
  // Trim unchanged lines from the edges first.
  const isCtx = (i) => lines[i]?.kind === "ctx";
  while (to - from + 1 > maxRows && isCtx(from)) from++;
  while (to - from + 1 > maxRows && isCtx(to)) to--;
  to = Math.min(to, from + maxRows - 1);

  const rows = [];
  let remaining = opts.typed;
  let cursorAt = -1; // row index that gets the typing cursor
  for (let i = from; i <= to; i++) {
    const l = lines[i];
    let text = l.text;
    let show = true;
    if (l.kind === "add" && remaining !== undefined) {
      if (remaining <= 0) show = false;
      else {
        const cs = chars(text);
        text = cs.slice(0, remaining).join("");
        remaining -= cs.length;
        if (remaining <= 0) cursorAt = rows.length;
      }
    }
    if (show) rows.push({ kind: l.kind, n: l.n, body: l.kind === "del" ? codeText(d.lang, text, COLORS.delText) : codeText(d.lang, text) });
  }
  const out = rows.map((r, idx) => {
    let body = r.body;
    if (opts.cursor && cursorAt === idx) body = ui.row({ gap: 0, align: "center" }, [r.body, ui.label({ text: "|", fontSize: 12, fontFamily: MONO, color: COLORS.text, cls: "caret" })]);
    return numbered(r.kind, r.n, body);
  });
  if ((d.more || 0) > 0) out.push(ui.label({ text: `  … ${d.more} more changed lines`, fontSize: 11, color: COLORS.dim }));
  return out;
}

const card = (head, body, opts) =>
  ui.column({ fill: COLORS.bg, radius: 10, gap: 0, flexGrow: opts?.grow ? 1 : undefined, cls: "viewer" }, [head, ui.column({ gap: 0, paddingV: 6 }, body)]);

/** Draw `detail` (one of the daemon's Detail objects).
 *  opts: typed (chars revealed), cursor (bool), maxRows, ok (null while running), grow, empty, bodyLines */
export function render(detail, opts = {}) {
  if (!detail || typeof detail !== "object") {
    return ui.column({ fill: COLORS.bg, radius: 10, padding: 16, flexGrow: opts.grow ? 1 : undefined }, [
      ui.label({ text: opts.empty || "Nothing to show yet.", fontSize: 12, color: COLORS.dim }),
    ]);
  }
  const kind = detail.type;
  if (kind === "diff") return card(header(detail.lang, detail.file, opts.ok == null), diffRows(detail, opts), opts);
  if (kind === "file") {
    const rows = (detail.lines || []).map((text, i) => numbered("ctx", (detail.start || 1) + i, codeText(detail.lang, text)));
    return card(header(detail.lang, detail.file, false, "read"), rows, opts);
  }
  if (kind === "terminal") {
    const head = ui.row({ fill: COLORS.head, paddingH: 10, paddingV: 6, gap: 8, align: "center" }, [
      ui.glyph({ name: "terminal-2", size: 14, color: COLORS.prompt }),
      ui.label({ text: "Terminal", fontSize: 13, fontWeight: "semibold", color: COLORS.text }),
    ]);
    const rows = [
      ui.row({ gap: 6, paddingH: 10, align: "start" }, [
        ui.label({ text: "$", fontSize: 12, fontFamily: MONO, color: COLORS.prompt }),
        ui.label({ text: detail.command || "", fontSize: 12, fontFamily: MONO, color: COLORS.text, maxLines: 3, flexGrow: 1 }),
      ]),
    ];
    const dim = (text) => ui.column({ paddingH: 10 }, [ui.label({ text, fontSize: 12, fontFamily: MONO, color: COLORS.dim, maxLines: 1 })]);
    for (const line of detail.output || []) rows.push(dim(line));
    if (opts.ok == null && !(detail.output || []).length) rows.push(dim(opts.cursor ? "running…" : "running"));
    return card(head, rows, opts);
  }
  // text
  const head = ui.row({ fill: COLORS.head, paddingH: 10, paddingV: 6, align: "center" }, [
    ui.label({ text: detail.title || "", fontSize: 13, fontWeight: "semibold", color: COLORS.text, maxLines: 1 }),
  ]);
  const body = detail.body ? [ui.label({ text: detail.body, fontSize: 12, color: COLORS.dim, maxLines: opts.bodyLines || 8 })] : [];
  return card(head, [ui.column({ paddingH: 10, gap: 4 }, body)], opts);
}

/** Number of characters in the added lines of a diff (for the typewriter speed). */
export function addedChars(detail) {
  if (detail?.type !== "diff") return 0;
  return (detail.lines || []).filter((l) => l.kind === "add").reduce((n, l) => n + chars(l.text).length, 0);
}
