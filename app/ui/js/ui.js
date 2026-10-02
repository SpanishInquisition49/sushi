// ui.js — a tiny stand-in for Noctalia's `ui` tree (column / row / box / spacer / label / glyph),
// so the character and viewer code can stay close to the Luau originals. Every function returns an
// HTML string; `children` may be nested arrays and may contain null / false (skipped).

const TOKENS = {
  primary: "var(--primary)",
  error: "var(--error)",
  tertiary: "var(--tertiary)",
  on_surface: "var(--on-surface)",
  on_surface_variant: "var(--on-surface-variant)",
  surface_variant: "var(--surface-variant)",
  outline: "var(--outline)",
};

/** A palette token, a hex color, or "<either>/<alpha>" → a CSS color. */
export function col(c) {
  if (!c) return "";
  const m = /^([a-z_]+|#[0-9a-fA-F]{3,8})\/([\d.]+)$/.exec(c);
  if (m) return `color-mix(in srgb, ${TOKENS[m[1]] || m[1]} ${Math.round(m[2] * 100)}%, transparent)`;
  return TOKENS[c] || c;
}

export const esc = (s) =>
  String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

const px = (n) => `${Math.round(n)}px`;
const JUSTIFY = { start: "flex-start", end: "flex-end", center: "center", space_between: "space-between" };
const ALIGN = { start: "flex-start", end: "flex-end", center: "center" };

function flat(children) {
  return (Array.isArray(children) ? children.flat(Infinity) : [children]).filter((c) => c !== null && c !== undefined && c !== false).join("");
}

function css(p, extra = "") {
  let s = extra;
  if (p.width != null) s += `width:${px(p.width)};flex-shrink:0;`;
  if (p.height != null) s += `height:${px(p.height)};flex-shrink:0;`;
  if (p.maxWidth != null) s += `max-width:${px(p.maxWidth)};`;
  if (p.flexGrow) s += `flex:${p.flexGrow} 1 0;min-width:0;`;
  if (p.gap != null) s += `gap:${px(p.gap)};`;
  if (p.padding != null) s += `padding:${px(p.padding)};`;
  if (p.paddingH != null) s += `padding-left:${px(p.paddingH)};padding-right:${px(p.paddingH)};`;
  if (p.paddingV != null) s += `padding-top:${px(p.paddingV)};padding-bottom:${px(p.paddingV)};`;
  if (p.fill) s += `background:${col(p.fill)};`;
  if (p.radius != null) s += `border-radius:${px(p.radius)};`;
  if (p.border) s += `border:${p.borderWidth || 1}px solid ${col(p.border)};box-sizing:border-box;`;
  if (p.align) s += `align-items:${ALIGN[p.align] || p.align};`;
  if (p.justify) s += `justify-content:${JUSTIFY[p.justify] || p.justify};`;
  if (p.opacity != null) s += `opacity:${p.opacity};`;
  return s;
}

function attrs(p) {
  let a = "";
  if (p.cls) a += ` class="${esc(p.cls)}"`;
  if (p.title) a += ` title="${esc(p.title)}"`;
  for (const [k, v] of Object.entries(p.data || {})) a += ` data-${k}="${esc(v)}"`;
  return a;
}

const el = (p, dir, children) =>
  `<div${attrs(p)} style="${css(p, `display:flex;flex-direction:${dir};`)}">${flat(children)}</div>`;

export const ui = {
  column: (p, children = []) => el(p || {}, "column", children),
  row: (p, children = []) => el(p || {}, "row", children),
  /** A plain colored rectangle (also a circle with a big radius). */
  box: (p) => `<div${attrs(p)} style="${css(p)}"></div>`,
  spacer: (p = {}) => `<div style="${css(p)}"></div>`,
  separator: (p = {}) =>
    p.orientation === "vertical"
      ? `<div style="width:1px;align-self:stretch;background:var(--outline);opacity:.2"></div>`
      : `<div style="height:1px;margin:${px(p.spacing ?? 2)} 0;background:var(--outline);opacity:${p.opacity ?? 0.2}"></div>`,
  label(p) {
    let s = `font-size:${p.fontSize || 13}px;line-height:1.25;`;
    const w = { bold: 700, semibold: 600 }[p.fontWeight] || p.fontWeight;
    if (w) s += `font-weight:${w};`;
    if (p.color) s += `color:${col(p.color)};`;
    if (p.opacity != null) s += `opacity:${p.opacity};`;
    if (p.fontFamily === "monospace") s += "font-family:var(--mono);white-space:pre;";
    if (p.textAlign) s += `text-align:${p.textAlign === "end" ? "right" : p.textAlign};`;
    if (p.width != null) s += `width:${px(p.width)};flex-shrink:0;display:inline-block;`;
    if (p.maxWidth != null) s += `max-width:${px(p.maxWidth)};`;
    if (p.flexGrow) s += `flex:${p.flexGrow} 1 0;min-width:0;`;
    if (p.maxLines === 1) s += "overflow:hidden;text-overflow:ellipsis;white-space:nowrap;min-width:0;";
    else if (p.maxLines) s += `display:-webkit-box;-webkit-line-clamp:${p.maxLines};-webkit-box-orient:vertical;overflow:hidden;word-break:break-word;`;
    return `<span${attrs(p)} style="${s}">${esc(p.text)}</span>`;
  },
  glyph(p) {
    const size = p.size || 16;
    let s = `width:${size}px;height:${size}px;flex-shrink:0;`;
    if (p.color) s += `color:${col(p.color)};`;
    if (p.opacity != null) s += `opacity:${p.opacity};`;
    if (p.rotate) s += `transform:rotate(${p.rotate}deg);`;
    return `<svg${attrs(p)} viewBox="0 0 24 24" style="${s}" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${ICONS[p.name] || ""}</svg>`;
  },
};

const path = (d, extra = "") => `<path d="${d}" ${extra}/>`;
const FILLED = 'fill="currentColor" stroke="none"';

// Stroke icons in the Tabler style (24x24).
const ICONS = {
  heart: path("M19.5 12.572l-7.5 7.428l-7.5 -7.428a5 5 0 1 1 7.5 -6.566a5 5 0 1 1 7.5 6.572", FILLED),
  music: path("M3 17a3 3 0 1 0 6 0a3 3 0 0 0 -6 0M13 17a3 3 0 1 0 6 0a3 3 0 0 0 -6 0M9 17v-13h10v13M9 8h10"),
  sparkles: path(
    "M16 18a2 2 0 0 1 2 2a2 2 0 0 1 2 -2a2 2 0 0 1 -2 -2a2 2 0 0 1 -2 2m0 -12a2 2 0 0 1 2 2a2 2 0 0 1 2 -2a2 2 0 0 1 -2 -2a2 2 0 0 1 -2 2m-7 12a6 6 0 0 1 6 -6a6 6 0 0 1 -6 -6a6 6 0 0 1 -6 6a6 6 0 0 1 6 6",
    FILLED,
  ),
  pencil: path("M4 20h4l10.5 -10.5a2.828 2.828 0 1 0 -4 -4l-10.5 10.5v4M13.5 6.5l4 4"),
  "terminal-2": path("M8 9l3 3l-3 3M13 15l3 0M3 5a2 2 0 0 1 2 -2h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2z"),
  search: path("M3 10a7 7 0 1 0 14 0a7 7 0 1 0 -14 0M21 21l-6 -6"),
  "alert-triangle": path(
    "M12 9v4M10.363 3.591l-8.106 13.534a1.914 1.914 0 0 0 1.636 2.871h16.214a1.914 1.914 0 0 0 1.636 -2.87l-8.106 -13.536a1.914 1.914 0 0 0 -3.274 0zM12 16h.01",
  ),
  "help-circle": path("M3 12a9 9 0 1 0 18 0a9 9 0 0 0 -18 0M12 17v.01M12 13.5a1.5 1.5 0 0 1 1 -1.5a2.6 2.6 0 1 0 -3 -4"),
  "list-check": path("M3.5 5.5l1.5 1.5l2.5 -2.5M3.5 11.5l1.5 1.5l2.5 -2.5M3.5 17.5l1.5 1.5l2.5 -2.5M11 6l9 0M11 12l9 0M11 18l9 0"),
  "circle-check-filled":
    path("M12 2a10 10 0 1 0 0 20a10 10 0 0 0 0 -20z", FILLED) + path("M8 12.5l3 3l5 -6", 'stroke="var(--on-icon)" stroke-width="2.4"'),
  "circle-x-filled":
    path("M12 2a10 10 0 1 0 0 20a10 10 0 0 0 0 -20z", FILLED) + path("M9 9l6 6M15 9l-6 6", 'stroke="var(--on-icon)" stroke-width="2.4"'),
  loader: path("M12 3a9 9 0 1 0 9 9"),
  x: path("M18 6l-12 12M6 6l12 12"),
  cookie: path("M3 12a9 9 0 1 0 18 0a9 9 0 1 0 -18 0M8 10v.01M12 8v.01M15 12v.01M10 15v.01M13 16v.01"),
  moon: path("M12 3c.132 0 .263 0 .393 0a7.5 7.5 0 0 0 7.92 12.446a9 9 0 1 1 -8.313 -12.454z"),
  send: path("M10 14l11 -11M21 3l-6.5 18a.55 .55 0 0 1 -1 0l-3.5 -7l-7 -3.5a.55 .55 0 0 1 0 -1l18 -6.5"),
  trash: path("M4 7l16 0M10 11l0 6M14 11l0 6M5 7l1 12a2 2 0 0 0 2 2h8a2 2 0 0 0 2 -2l1 -12M9 7v-3a1 1 0 0 1 1 -1h4a1 1 0 0 1 1 1v3"),
  "player-stop": path("M5 7a2 2 0 0 1 2 -2h10a2 2 0 0 1 2 2v10a2 2 0 0 1 -2 2h-10a2 2 0 0 1 -2 -2z", FILLED),
  settings: path(
    "M10.325 4.317c.426 -1.756 2.924 -1.756 3.35 0a1.724 1.724 0 0 0 2.573 1.066c1.543 -.94 3.31 .826 2.37 2.37a1.724 1.724 0 0 0 1.065 2.572c1.756 .426 1.756 2.924 0 3.35a1.724 1.724 0 0 0 -1.066 2.573c.94 1.543 -.826 3.31 -2.37 2.37a1.724 1.724 0 0 0 -2.572 1.065c-.426 1.756 -2.924 1.756 -3.35 0a1.724 1.724 0 0 0 -2.573 -1.066c-1.543 .94 -3.31 -.826 -2.37 -2.37a1.724 1.724 0 0 0 -1.065 -2.572c-1.756 -.426 -1.756 -2.924 0 -3.35a1.724 1.724 0 0 0 1.066 -2.573c-.94 -1.543 .826 -3.31 2.37 -2.37c1 .608 2.296 .07 2.572 -1.065zM9 12a3 3 0 1 0 6 0a3 3 0 0 0 -6 0",
  ),
};
