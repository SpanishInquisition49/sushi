// highlight.js — a tiny syntax highlighter for the live viewer (port of lib/highlight.luau).
// tokens(lang, line) → [{text, color}] with neighbours of the same color merged. It is a
// line-by-line scanner (strings, comments, numbers, keywords, calls, types), not a parser.

export const COLORS = {
  plain: "#abb2bf",
  keyword: "#c678dd",
  string: "#98c379",
  number: "#d19a66",
  comment: "#7f848e",
  call: "#61afef",
  type: "#e5c07b",
  key: "#e06c75",
};

const set = (words) => new Set(words.split(/\s+/).filter(Boolean));

const C_LIKE_KW = set(`
  if else for while do switch case break continue return fn function let const var mut pub use mod struct enum
  impl trait type interface class extends implements new import export from as default async await yield try
  catch finally throw throws static void this self super match loop in of where crate typeof instanceof
  package go func chan select defer range public private protected final abstract null nil true false undefined`);

const LANGS = {
  rust: { kw: C_LIKE_KW, line: "//", strings: "\"'" },
  ts: { kw: C_LIKE_KW, line: "//", strings: "\"'`" },
  js: { kw: C_LIKE_KW, line: "//", strings: "\"'`" },
  c: { kw: C_LIKE_KW, line: "//", strings: "\"'" },
  cpp: { kw: C_LIKE_KW, line: "//", strings: "\"'" },
  go: { kw: C_LIKE_KW, line: "//", strings: "\"'`" },
  java: { kw: C_LIKE_KW, line: "//", strings: "\"'" },
  python: { kw: set("def class if elif else for while return import from as with try except finally raise lambda pass break continue in is not and or None True False yield async await global nonlocal assert del"), line: "#", strings: "\"'" },
  lua: { kw: set("function local if then else elseif end for while do repeat until return break in and or not nil true false"), line: "--", strings: "\"'" },
  shell: { kw: set("if then else elif fi for while do done case esac in function return export local echo cd exit set unset source"), line: "#", strings: "\"'" },
  json: { kw: set("true false null"), strings: '"', keys: true },
  toml: { kw: set("true false"), line: "#", strings: "\"'", keys: true },
  yaml: { kw: set("true false null"), line: "#", strings: "\"'", keys: true },
  nix: { kw: set("let in if then else with inherit rec import true false null"), line: "#", strings: "\"'" },
  css: { kw: set("important"), strings: "\"'" },
  html: { kw: set(""), strings: "\"'" },
  markdown: { kw: set(""), strings: "" },
};

let cache = new Map();

function push(out, text, color) {
  if (!text) return;
  const last = out.at(-1);
  if (last && last.color === color) last.text += text;
  else out.push({ text, color });
}

function scan(lang, line) {
  const spec = LANGS[lang];
  const out = [];
  if (!spec) return [{ text: line, color: COLORS.plain }];
  let i = 0;
  const n = line.length;
  while (i < n) {
    const c = line[i];
    if (spec.line && line.startsWith(spec.line, i)) {
      push(out, line.slice(i), COLORS.comment);
      break;
    } else if (spec.strings.includes(c)) {
      let j = i + 1;
      while (j < n) {
        const d = line[j];
        if (d === "\\") j += 2;
        else if (d === c) break;
        else j++;
      }
      const text = line.slice(i, Math.min(j, n - 1) + 1);
      let color = COLORS.string;
      if (spec.keys && /^\s*[:=]/.test(line.slice(j + 1))) color = COLORS.key;
      push(out, text, color);
      i = j + 1;
    } else if (/\d/.test(c)) {
      const num = /^\d[\w._]*/.exec(line.slice(i))[0];
      push(out, num, COLORS.number);
      i += num.length;
    } else if (/[A-Za-z_]/.test(c)) {
      const word = /^\w+/.exec(line.slice(i))[0];
      const rest = line.slice(i + word.length);
      let color = COLORS.plain;
      if (spec.kw.has(word)) color = COLORS.keyword;
      else if (/^\s*\(/.test(rest)) color = COLORS.call;
      else if (/^[A-Z]/.test(word) && word.length > 1) color = COLORS.type;
      push(out, word, color);
      i += word.length;
    } else {
      let chunk = /^[^\w"'`]+/.exec(line.slice(i))?.[0] || c;
      // Stop before a comment marker so the next turn of the loop sees it.
      if (spec.line) {
        const p = chunk.indexOf(spec.line);
        if (p > 0) chunk = chunk.slice(0, p);
      }
      push(out, chunk, COLORS.plain);
      i += chunk.length;
    }
  }
  return out;
}

/** Tokens for one line of `lang` code (cached: the same lines are redrawn often). */
export function tokens(lang, line) {
  const key = lang + "\0" + line;
  let hit = cache.get(key);
  if (hit) return hit;
  if (cache.size > 2000) cache = new Map();
  hit = scan(lang, line);
  cache.set(key, hit);
  return hit;
}
