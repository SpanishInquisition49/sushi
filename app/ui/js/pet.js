// pet.js — the animated pet: several original characters, one animation engine
// (port of plugin/lib/pet.luau; the character is a tree of flex boxes, built as an HTML string).
//
// Characters (setting "character"): nigiri_salmon, nigiri_tuna, nigiri_tamago, unagi_nigiri,
//   ebi_nigiri, ebi_tempura, ikura, maki, uramaki, inari, ravioli, bao, takoyaki, ramen,
//   miso_soup, edamame, onigiri, dorayaki, mochi, dango, bubble_tea, sake, tofu.
//
// Moods
//   steady    idle · working · alert (permission asked) · sleep (daemon down) · nap (idle for a
//             long time) · worried (plan usage >= 90%) · stuffed (a context window >= 90% full)
//   reaction  happy (a session finished) · love (approved / petted) · sad (denied) ·
//             annoyed / dizzy (poked) · startled (woken up) · wave (new session) · eat · dance
//   fidgets   hop · wiggle · yawn · peek · squish · wink · blush · sneeze · hum · think · spin ·
//             bounce (random, while idle), plus a few per character (hiccup, sip, jiggle)
//
// While "working" the eyes and a small icon follow the tool the agent is using
// (read, write, run, search, think). Hot dishes steam; the sake bottle is tipsy.
// The caller advances time with pet.tick(nowMs) and renders pet.build(width, opts) each frame.

import { ui } from "./ui.js";

const INK = "#2b2023", SHINE = "#ffffff", CHEEK = "#ff9aa6", RICE = "#fbf6ec", RICE_EDGE = "#e3d8c4",
  NORI = "#26382c", SWEAT = "#8ecbff", HEART = "#ff6b81", INARI = "#d9923f", INARI_DARK = "#b87430",
  DORA_TOP = "#d99a45", DORA_BOT = "#c4853a", DORA_AN = "#6b2f2a", BATTER = "#e2a447", BATTER_DARK = "#bf7f2c",
  TAIL = "#f0603c", DOUGH = "#f6ebd4", DOUGH_FOLD = "#e3d0a8", DOUGH_EDGE = "#e6d6b5", SEARED = "#d99a3e",
  BAO = "#fffaf0", BAO_FOLD = "#eadcc4", BAMBOO = "#cc9a58", BAMBOO_DARK = "#a97a3c", BOWL_CREAM = "#f4eee0",
  BOWL_EDGE = "#e0d6c0", BOWL_RED = "#c8453a", NOODLE = "#f2cf63", TAKO = "#d8a24c", SAUCE = "#5b2e1b",
  MISO = "#c98d4b", LACQUER = "#d94a2b", GOLD = "#e8b64c", POD = "#8cc063", POD_DARK = "#5f9140", BEAN = "#b4dc82",
  MOCHI = "#fbe4ea", MOCHI_EDGE = "#f0c6d2", BERRY = "#e8465a", TEA = "#d9ac80", PEARL = "#3b2b25",
  CERAMIC = "#f1f4f8", CERAMIC_EDGE = "#ccd6e0", SAKE_BLUE = "#3d6fb6", CHEEK_HOT = "#ff6f85";

const CHARACTERS = {
  nigiri_salmon: { kind: "nigiri", ratio: 0.8, top: "#ff8a6b", fat: "#ffc9b5" },
  nigiri_tuna: { kind: "nigiri", ratio: 0.8, top: "#d8454f", fat: "#f0868c" },
  nigiri_tamago: { kind: "nigiri", ratio: 0.8, top: "#ffcf4d", fat: "#ffe79a" },
  maki: { kind: "maki", ratio: 1.0, filling: "#ff8a6b", filling2: "#8cc76f" },
  uramaki: { kind: "uramaki", ratio: 1.0, filling: "#ff8a6b", filling2: "#8cc76f", filling3: "#ffd24a" },
  unagi_nigiri: { kind: "nigiri", ratio: 0.8, top: "#6a3a22", fat: "#b36b34" },
  ebi_nigiri: { kind: "nigiri", ratio: 0.8, top: "#ff9c66", fat: "#fff0e6", bands: "#fff0e6" },
  ebi_tempura: { kind: "tempura", ratio: 0.7, steam: true },
  ikura: { kind: "ikura", ratio: 1.0, roe: "#ff7a3d" },
  inari: { kind: "inari", ratio: 0.85 },
  ravioli: { kind: "ravioli", ratio: 0.8, steam: true },
  bao: { kind: "bao", ratio: 0.9, steam: true },
  takoyaki: { kind: "takoyaki", ratio: 1.0, steam: true },
  ramen: { kind: "ramen", ratio: 0.95, steam: true },
  miso_soup: { kind: "miso", ratio: 0.85, steam: true },
  edamame: { kind: "edamame", ratio: 0.8 },
  onigiri: { kind: "onigiri", ratio: 0.92 },
  mochi: { kind: "mochi", ratio: 0.88 },
  bubble_tea: { kind: "boba", ratio: 1.35 },
  sake: { kind: "sake", ratio: 1.15, tipsy: true },
  dorayaki: { kind: "dorayaki", ratio: 0.86 },
  dango: { kind: "dango", ratio: 1.6, top: "#ff9eb5", mid: "#f6f1ea", bottom: "#a8d58b" },
  tofu: { kind: "tofu", ratio: 1.0 },
};

export const CHARACTER_IDS = [
  "nigiri_salmon", "nigiri_tuna", "nigiri_tamago", "unagi_nigiri", "ebi_nigiri", "ebi_tempura", "ikura", "maki",
  "uramaki", "inari", "ravioli", "bao", "takoyaki", "ramen", "miso_soup", "edamame", "onigiri", "dorayaki", "mochi",
  "dango", "bubble_tea", "sake", "tofu",
];
const DEFAULT_CHARACTER = "nigiri_salmon";

const round = (x) => Math.floor(x + 0.5);
const clamp = (x, a, b) => Math.max(a, Math.min(b, x));
const rand = (a, b) => a + Math.random() * (b - a);
const sine = (t, period) => Math.sin((t * 2 * Math.PI) / period);

// breath period (ms), eye height scale, eye width scale
const MOOD_PARAMS = {
  idle: [3200, 1.0, 1.0], working: [1500, 0.95, 1.0], alert: [900, 1.2, 1.15], sleep: [4200, 0, 1.0],
  nap: [5200, 0, 1.0], worried: [1100, 1.1, 0.9], stuffed: [1400, 0.55, 1.0], happy: [700, 0.38, 1.1],
  love: [700, 0.38, 1.1], sad: [2400, 0.85, 0.95], annoyed: [1200, 0.42, 1.1], dizzy: [600, 1.0, 1.0],
  startled: [500, 1.35, 1.25], wave: [600, 1.1, 1.1], greeting: [600, 1.15, 1.15], eat: [500, 0.5, 1.05],
  dance: [600, 0.4, 1.1], hop: [1800, 1.0, 1.0], wiggle: [1800, 1.0, 1.0], yawn: [2200, 0.3, 1.05],
  peek: [2600, 1.1, 1.05], squish: [1200, 1.0, 1.0], wink: [3200, 1.0, 1.0], blush: [1600, 0.9, 1.0],
  sneeze: [1500, 0.3, 1.05], hum: [1800, 0.45, 1.05], think: [3000, 1.0, 1.0], spin: [1400, 1.0, 1.0],
  bounce: [1100, 1.0, 1.0], sip: [2200, 0.5, 1.05], hiccup: [900, 0.8, 1.0], jiggle: [1200, 1.0, 1.0],
};
const params = (mood) => {
  const [period, eyeH, eyeW] = MOOD_PARAMS[mood] || MOOD_PARAMS.idle;
  return { period, eyeH, eyeW };
};

// Eyes drawn as thin happy bars (no shine).
const SQUINT = new Set(["sleep", "nap", "happy", "love", "yawn", "hum", "eat", "dance", "sip", "stuffed", "sneeze"]);

// Random idle quirks: [mood, duration ms].
const FIDGETS = [
  ["hop", 1000], ["wiggle", 1100], ["yawn", 2000], ["peek", 2400], ["squish", 1100], ["wink", 900],
  ["blush", 2000], ["sneeze", 1500], ["hum", 3000], ["think", 2600], ["spin", 1400], ["bounce", 1100],
];
const EXTRA_FIDGETS = {
  sake: [["hiccup", 900], ["sip", 2200]],
  bubble_tea: [["sip", 2200]], ramen: [["sip", 2200]], miso_soup: [["sip", 2200]],
  takoyaki: [["jiggle", 1200]], mochi: [["jiggle", 1200]], dango: [["jiggle", 1200]], bao: [["jiggle", 1200]],
};

// Sound played when a mood starts (names of the files in ui/sounds).
const SOUND_FOR = {
  greeting: "greeting", wave: "hello", happy: "happy", love: "love", sad: "sad", startled: "startled",
  annoyed: "annoyed", dizzy: "dizzy", hop: "hop", bounce: "bounce", wiggle: "wiggle", squish: "squish",
  jiggle: "squish", yawn: "yawn", sneeze: "sneeze", hiccup: "hiccup", hum: "hum", think: "think", wink: "wink",
  blush: "blush", spin: "spin", eat: "eat", dance: "dance", nap: "nap", alert: "alert",
};

/** Read the pet settings from the app's settings object. */
export function configFrom(settings = {}) {
  const character = CHARACTERS[settings.character] ? settings.character : DEFAULT_CHARACTER;
  const nap = Number(settings.napAfterSec);
  return {
    character,
    fidgets: settings.fidgets !== false,
    napAfterSec: Number.isFinite(nap) && settings.napAfterSec !== undefined && settings.napAfterSec !== "" ? clamp(nap, 0, 3600) : 120, // 0 = never
  };
}

const spacerW = (w) => ui.spacer({ width: Math.max(0, round(w)) });
const spacerH = (h) => ui.spacer({ height: Math.max(0, round(h)) });

export class Pet {
  constructor(nowMs = Date.now(), cfg) {
    this.configure(cfg);
    const now = nowMs;
    this.baseMood = "idle";
    this.override = null; // {mood, startMs, untilMs, nextMood, nextDur}
    this.lastMs = null;
    this.t = 0; // accumulated ms
    this.gx = this.gy = 0; // current gaze, -1..1
    this.tx = this.ty = 0; // gaze target
    this.nextGlance = 0;
    this.nextBlink = now + rand(1200, 3000);
    this.blinkStart = -1e9;
    this.blink = 0;
    this.nextFidget = now + rand(6000, 12000);
    this.lastActive = now;
    this.lastEventTs = 0;
    this.prevBase = "other";
    this.pokes = [];
    this.sx = this.sy = 0; // squash / stretch
    this.lift = this.shake = 0;
    this.orbit = this.yawn = 0;
    this.deco = null;
    this.decoPhase = 0;
    this.winkL = 0; // 0..1, left eye closed (wink)
    this.cheek = 0; // 0..1, how flushed the cheeks are
    this.workKind = "read"; // read / write / run / search / think
    this.knownSessions = null;
    this.forceNap = false;
    this.hover = null;
    this.prevWorking = false;
    this.prevChatBusy = false;
    this.upSince = null;
    this.cur = "idle";
    this.curP = 0;
    this.pulse = this.chew = 0;
    this.sounds = [];
    this.soundsOn = false;
    this.soundMood = null;
    this.soundOverride = null;
    this.wakeCue = false;
    this.trigger("greeting", 1800, now); // says hello when it appears
  }

  configure(cfg) {
    this.cfg = cfg || { character: DEFAULT_CHARACTER, fidgets: true, napAfterSec: 120 };
    this.char = CHARACTERS[this.cfg.character] || CHARACTERS[DEFAULT_CHARACTER];
  }

  /** Only the entry that owns the speakers (the pet window) turns this on. */
  enableSounds(on) {
    this.soundsOn = !!on;
    if (!this.soundsOn) this.sounds = [];
  }

  cue(name) {
    if (!(this.soundsOn && name)) return;
    this.sounds.push(name);
    if (this.sounds.length > 12) this.sounds.shift();
  }

  /** The sounds to play now (and forget). */
  drainSounds() {
    const out = this.sounds;
    this.sounds = [];
    return out;
  }

  /** Start a transient mood. `nextMood` (optional) plays right after it ends. */
  trigger(mood, durationMs, nowMs, nextMood, nextDurationMs) {
    this.override = { mood, startMs: nowMs, untilMs: nowMs + durationMs, nextMood, nextDur: nextDurationMs };
  }

  /** Current mood and progress (0..1) of a transient one. */
  mood(nowMs) {
    const o = this.override;
    if (o) {
      if (nowMs < o.untilMs) return [o.mood, (nowMs - o.startMs) / (o.untilMs - o.startMs)];
      this.override = null;
      if (o.nextMood) {
        this.trigger(o.nextMood, o.nextDur || 1000, o.untilMs);
        return this.mood(nowMs);
      }
    }
    let base = this.baseMood;
    const napMs = (this.cfg.napAfterSec || 0) * 1000;
    if (base === "idle" && (this.forceNap || (napMs > 0 && nowMs - this.lastActive > napMs))) base = "nap";
    return [base, 0];
  }

  /** Derive the base mood from the daemon snapshot and react to transitions. */
  onSnapshot(snap, daemonUp, nowMs) {
    let mood = "idle";
    let working = false;
    // Requests to answer, plus sessions blocked on their own terminal with nothing to answer here.
    const asked = new Set((snap?.pending || []).map((p) => p.session_id));
    const pending = (snap?.pending || []).length + (snap?.sessions || []).filter((s) => s.status === "waiting" && !asked.has(s.id)).length;
    let newSession = false;
    if (!daemonUp) {
      mood = "sleep";
      this.knownSessions = null;
      this.upSince = null;
    } else {
      // Sessions that were already there when the daemon came up are not newcomers.
      if (this.upSince == null) this.upSince = nowMs;
      let active = null;
      const ids = new Set();
      for (const s of snap?.sessions || []) {
        if (s.id) {
          ids.add(s.id);
          if (this.knownSessions && !this.knownSessions.has(s.id)) newSession = true;
        }
        if (s.status === "working") {
          working = true;
          if (!active || (s.last_event_ms || 0) > (active.last_event_ms || 0)) active = s;
        }
      }
      this.knownSessions = ids;
      // What kind of work is going on (drives the eyes and the little icon).
      const tool = active && typeof active.last_tool === "object" && active.last_tool?.kind;
      this.workKind = tool && tool !== "other" ? tool : "read";

      let lim = null;
      for (const a of Object.values(snap?.agents || {})) if (a.limits?.data) lim = a.limits.data;
      const high = lim && ((lim.five_hour?.percent || 0) >= 90 || (lim.seven_day?.percent || 0) >= 90);
      const full = active?.context && (active.context.percent || 0) >= 90;
      const chatBusy = snap?.chat?.busy === true;
      if (pending > 0) mood = "alert";
      else if (high) mood = "worried";
      else if (full) mood = "stuffed";
      else if (working) mood = "working";
      else if (chatBusy) mood = "think"; // the built-in chat is answering
      // The chat finished answering → a happy hop (like a session finishing).
      if (this.prevChatBusy && !chatBusy && !snap?.chat?.error && mood === "idle") this.trigger("happy", 1200, nowMs);
      this.prevChatBusy = chatBusy;
      if (chatBusy) this.lastActive = nowMs;
    }
    if (working || pending > 0) {
      this.lastActive = nowMs;
      this.forceNap = false;
    }
    // A session finished → happy hop. A new one appeared → wave hello.
    if (this.prevWorking && !working && mood === "idle") this.trigger("happy", 1400, nowMs);
    else if (newSession && mood !== "alert" && nowMs - (this.upSince ?? nowMs) > 4000) this.trigger("wave", 1600, nowMs);
    this.prevWorking = working;
    this.baseMood = mood;
  }

  /** A one-off event from elsewhere (the panel, the tray): love, approve, deny, sad, eat, dance, hello, nap. */
  onEvent(kind, ts, nowMs) {
    if ((ts || 0) <= this.lastEventTs) return;
    this.lastEventTs = ts || 0;
    if (kind === "nap") {
      this.forceNap = true;
      this.override = null;
      return;
    }
    this.lastActive = nowMs;
    this.forceNap = false;
    if (kind === "love") this.trigger("love", 1800, nowMs);
    else if (kind === "approve") { this.soundOverride = "approve"; this.trigger("love", 1800, nowMs); }
    else if (kind === "deny") { this.soundOverride = "deny"; this.trigger("sad", 2000, nowMs); }
    else if (kind === "sad") this.trigger("sad", 2000, nowMs);
    else if (kind === "eat") this.trigger("eat", 2600, nowMs, "happy", 1100);
    else if (kind === "dance") this.trigger("dance", 4200, nowMs);
    else if (kind === "hello") this.trigger("wave", 1600, nowMs);
  }

  /** The user clicked the pet. */
  poke(nowMs) {
    const [mood] = this.mood(nowMs);
    this.lastActive = nowMs;
    this.forceNap = false;
    if (mood === "dizzy") return; // let it finish spinning
    this.cue("poke");
    if (mood === "nap") return this.trigger("startled", 800, nowMs);
    this.pokes = this.pokes.filter((t) => nowMs - t < 3000);
    this.pokes.push(nowMs);
    if (this.pokes.length >= 4) {
      this.pokes = [];
      this.trigger("dizzy", 2400, nowMs);
    } else {
      this.trigger("annoyed", 800, nowMs);
    }
  }

  /** Make the eyes look at a point (-1..1); null releases the gaze. */
  lookAt(x, y) {
    this.hover = x == null ? null : { x, y: y || 0 };
  }

  // ── Animation ───────────────────────────────────────────────────────────────

  tick(nowMs) {
    const dt = this.lastMs != null ? clamp(nowMs - this.lastMs, 0, 100) : 0;
    this.lastMs = nowMs;
    this.t += dt;

    let [mood, p] = this.mood(nowMs);

    // Waking up from a nap startles the pet.
    if (this.prevBase === "nap" && mood !== "nap" && mood !== "startled") {
      this.wakeCue = true;
      this.trigger("startled", 700, nowMs);
      [mood, p] = this.mood(nowMs);
    }
    this.prevBase = mood === "nap" ? "nap" : "other";

    // Random idle quirks (the generic ones plus a few that suit the character).
    if (mood === "idle" && this.cfg.fidgets && nowMs >= this.nextFidget) {
      const pool = [...FIDGETS, ...(EXTRA_FIDGETS[this.cfg.character] || [])];
      const f = pool[Math.floor(Math.random() * pool.length)];
      this.trigger(f[0], f[1], nowMs);
      this.nextFidget = nowMs + rand(10000, 25000);
      [mood, p] = this.mood(nowMs);
    }

    const mp = params(mood);
    this.cur = mood;
    this.curP = p;
    const t = this.t;

    // A new mood may come with a sound (the approve/deny sounds replace the love/sad ones).
    if (mood !== this.soundMood) {
      this.soundMood = mood;
      let name = SOUND_FOR[mood];
      if (mood === "startled" && this.wakeCue) name = "wake";
      if (this.soundOverride && (mood === "love" || mood === "sad")) name = this.soundOverride;
      this.wakeCue = false;
      this.soundOverride = null;
      this.cue(name);
    }
    const char = this.char;

    // Breathing → gentle squash & stretch (deeper while napping).
    const breath = sine(t, mp.period);
    const amp = mood === "nap" || mood === "sleep" ? 0.045 : 0.03;
    this.sy = amp * breath;
    this.sx = -amp * 0.6 * breath;

    // Gaze target.
    const work = this.workKind;
    if (this.hover) {
      [this.tx, this.ty] = [this.hover.x, this.hover.y];
    } else if (mood === "idle") {
      if (nowMs >= this.nextGlance) {
        [this.tx, this.ty] = [rand(-1, 1), rand(-0.6, 0.6)];
        if (Math.random() < 0.35) [this.tx, this.ty] = [0, 0];
        this.nextGlance = nowMs + rand(1200, 3800);
      }
    } else if (mood === "working") {
      if (work === "write") [this.tx, this.ty] = [Math.sin(t / 90) * 0.35, 0.85]; // eyes down on the keyboard
      else if (work === "run") [this.tx, this.ty] = [Math.sin(t / 230) > 0 ? 0.8 : -0.8, 0.1]; // watching output scroll
      else if (work === "search") [this.tx, this.ty] = [0.7 + Math.sin(t / 500) * 0.25, -0.6]; // looking up and around
      else if (work === "think") [this.tx, this.ty] = [-0.7, -0.7];
      else [this.tx, this.ty] = [Math.sin(t / 380) > 0 ? 0.9 : -0.9, 0.5]; // read: sweep left → right
    } else if (mood === "alert") [this.tx, this.ty] = [0, -0.1];
    else if (mood === "peek") [this.tx, this.ty] = [p < 0.5 ? -1 : 1, -0.2];
    else if (mood === "sad") [this.tx, this.ty] = [0, 0.8];
    else if (mood === "worried" || mood === "stuffed") [this.tx, this.ty] = [Math.sin(t / 140) * 0.5, -0.25];
    else if (mood === "think") [this.tx, this.ty] = [-0.7, -0.7];
    else if (mood === "blush") [this.tx, this.ty] = [-0.6, 0.6];
    else if (mood === "wave") [this.tx, this.ty] = [0, -0.2];
    else [this.tx, this.ty] = [0, 0];
    const fast = mood === "working" || mood === "peek";
    const k = 1 - Math.exp(-dt / (fast ? 70 : 130));
    this.gx += (this.tx - this.gx) * k;
    this.gy += (this.ty - this.gy) * k;

    this.orbit = mood === "dizzy" || mood === "spin" ? t / (mood === "spin" ? 60 : 90) : 0;

    // Wink: the left eye closes for the middle of the animation.
    this.winkL = mood === "wink" ? clamp(Math.sin(p * Math.PI) * 2.5, 0, 1) : 0;

    // Cheeks: flushed when shy, loved, or (for the sake bottle) tipsy.
    let cheekTarget = 0;
    if (mood === "blush") cheekTarget = 1;
    else if (mood === "love") cheekTarget = 0.6;
    else if (char.tipsy && ["idle", "working", "hiccup", "sip"].includes(mood)) cheekTarget = 0.7;
    this.cheek += (cheekTarget - this.cheek) * (1 - Math.exp(-dt / 220));

    // Blinking (closed-eye moods ignore it).
    if (nowMs >= this.nextBlink) {
      this.blinkStart = nowMs;
      this.nextBlink = nowMs + rand(2500, 6000);
      if (Math.random() < 0.15) this.nextBlink = nowMs + 260; // double blink
    }
    const b = (nowMs - this.blinkStart) / 150;
    this.blink = b >= 0 && b <= 1 ? 1 - Math.abs(2 * b - 1) : 0;

    // Jumps, shakes and body language.
    this.lift = this.shake = this.yawn = 0;
    this.pulse = Math.abs(sine(t, 300)); // mouth pulsing (humming, sipping)
    this.chew = 0.5 + 0.5 * sine(t, 220); // mouth chewing
    switch (mood) {
      case "happy": this.lift = Math.abs(Math.sin(p * Math.PI * 2)) * (1 - p * 0.5); break;
      case "love":
        this.lift = Math.abs(Math.sin(p * Math.PI * 2)) * 0.35 * (1 - p);
        this.shake = Math.sin(t / 160) * 0.25;
        break;
      case "alert": {
        const phase = (t % 900) / 900;
        this.lift = phase < 0.35 ? Math.sin((phase / 0.35) * Math.PI) * 0.55 : 0;
        break;
      }
      case "startled": this.lift = Math.sin(clamp(p * 1.6, 0, 1) * Math.PI) * 0.7; break;
      case "annoyed": this.shake = Math.sin(t / 28) * (1 - p); break;
      case "dizzy": this.shake = Math.sin(t / 110) * 0.8; break;
      case "worried": this.shake = Math.sin(t / 60) * 0.12; break; // trembling
      case "stuffed":
        this.sx += 0.1; // bloated
        this.sy -= 0.03;
        this.shake = Math.sin(t / 90) * 0.08;
        break;
      case "hop": this.lift = Math.sin(p * Math.PI) * 0.55; break;
      case "bounce": this.lift = Math.abs(Math.sin(p * Math.PI * 2)) * 0.5; break;
      case "wiggle": this.shake = Math.sin(t / 55) * (1 - p) * 0.9; break;
      case "wave":
        this.lift = Math.abs(Math.sin(p * Math.PI * 2)) * 0.45;
        this.shake = Math.sin(t / 120) * 0.35;
        break;
      case "greeting":
        this.lift = Math.abs(Math.sin(p * Math.PI * 3)) * 0.55 * (1 - p * 0.4);
        this.shake = Math.sin(t / 110) * 0.45;
        break;
      case "dance":
        this.lift = Math.abs(sine(t, 380)) * 0.4;
        this.shake = sine(t, 760) * 0.9;
        break;
      case "hum":
        this.shake = sine(t, 840) * 0.35;
        this.sy += 0.02 * sine(t, 420);
        break;
      case "sip": this.sy -= 0.05 * Math.sin(p * Math.PI); break;
      case "spin": this.shake = Math.sin(t / 70) * 0.2; break;
      case "eat": this.sy += 0.035 * sine(t, 220); break;
      case "yawn": {
        const y = Math.sin(p * Math.PI);
        this.yawn = y;
        this.sy += 0.07 * y; // stretch up
        this.sx -= 0.03 * y;
        break;
      }
      case "squish": {
        const e = Math.sin(p * Math.PI * 3) * (1 - p);
        this.sy = -0.2 * e;
        this.sx = 0.12 * e;
        break;
      }
      case "jiggle": {
        const e = (1 - p) * 0.07 * Math.sin(t / 55);
        this.sy += e;
        this.sx -= e;
        break;
      }
      case "hiccup": this.lift = Math.sin(clamp(p * 3, 0, 1) * Math.PI) * 0.3; break;
      case "blush":
        this.shake = Math.sin(t / 200) * 0.15;
        this.sy -= 0.02;
        break;
      case "sneeze":
        if (p < 0.55) { // wind up: stretch, eyes squeezed
          const q = p / 0.55;
          this.sy += 0.09 * q;
          this.sx -= 0.04 * q;
        } else { // ACHOO: squash and shake
          const q = (p - 0.55) / 0.45;
          this.sy -= 0.2 * (1 - q);
          this.sx += 0.12 * (1 - q);
          this.shake = Math.sin(t / 25) * (1 - q);
        }
        break;
      case "sad": this.sy -= 0.04; break; // droop
    }
    // The sake bottle sways a little in every calm mood.
    if (char.tipsy && (mood === "idle" || mood === "working")) this.shake += sine(t, 1400) * 0.4;

    // Floating decoration: the mood's own, else the work icon, else steam for hot dishes.
    let deco = DECO[mood];
    if (!deco && mood === "working") deco = WORK_DECO[work];
    if (!deco && char.steam) deco = "steam";
    this.deco = deco || null;
    this.decoPhase = (t % 1800) / 1800;
  }

  // ── Rendering ───────────────────────────────────────────────────────────────

  /** One eye: a dark dot inside a transparent socket; (gx, gy) in -1..1 moves it. `baseH` is the
   *  open height, so blinking squashes the eye around its centre. */
  static eye(sw, sh, ew, eh, baseH, gx, gy, shine) {
    const cx = sw / 2 + (gx * (sw - ew)) / 2;
    const cy = sh / 2 + (gy * (sh - baseH)) / 2;
    let children = [];
    if (shine && eh >= 6) {
      const hl = round(Math.min(ew, eh) * 0.38);
      children = [
        spacerH(eh * 0.14),
        ui.row({}, [spacerW(ew * 0.18), ui.box({ width: hl, height: hl, radius: round(hl / 2), fill: SHINE })]),
      ];
    }
    return ui.column({ width: round(sw), height: round(sh), align: "start", justify: "start" }, [
      spacerH(cy - eh / 2),
      ui.row({}, [
        spacerW(cx - ew / 2),
        ui.column(
          { width: round(ew), height: round(eh), fill: INK, radius: round(Math.min(ew, eh) / 2), align: "start", justify: "start" },
          children,
        ),
      ]),
    ]);
  }

  /** Eyes, blush and mouth, shared by every character. */
  face(w, detail, opts) {
    const mood = this.cur || "idle";
    const mp = params(mood);
    const p = this.curP || 0;

    const sw = w * 0.17;
    const ew = w * 0.1 * mp.eyeW;
    const eh0 = w * 0.15;
    const closedMood = mood === "sleep" || mood === "nap";
    let eh = Math.max(2, eh0 * mp.eyeH * (1 - (closedMood ? 1 : this.blink)));
    if (closedMood) eh = 2;
    const ehL = Math.max(2, eh * (1 - this.winkL)); // left eye, closed by a wink
    let gx = this.gx, gy = this.gy;
    if (mood === "dizzy" || mood === "spin") [gx, gy] = [Math.cos(this.orbit), Math.sin(this.orbit)];
    const shine = detail && !SQUINT.has(mood);

    // Blush: wider, deeper cheeks.
    const a = this.cheek;
    const cheekW = round(w * 0.1 * (1 + 0.7 * a));
    const cheekH = Math.max(2, round(w * 0.055 * (1 + 0.5 * a)));
    const cheekColor = (a > 0.45 ? CHEEK_HOT : CHEEK) + "cc";
    const cheek = () => ui.box({ width: cheekW, height: cheekH, radius: round(cheekH / 2), fill: cheekColor });
    const eyeRow = ui.row({ gap: round(w * 0.035), align: "center" }, [
      cheek(),
      Pet.eye(sw, sw, ew, ehL, eh0, gx, gy, shine && this.winkL < 0.5),
      spacerW(w * 0.05),
      Pet.eye(sw, sw, ew, eh, eh0, gx * 0.9, gy, shine),
      cheek(),
    ]);

    const face = [eyeRow];
    if (opts.mouth !== false) {
      // width / height of the mouth as fractions of the sprite width
      const M = {
        idle: [0.11, 0.04], working: [0.11, 0.04], hop: [0.11, 0.04], wiggle: [0.11, 0.04], squish: [0.11, 0.04],
        peek: [0.11, 0.04], jiggle: [0.11, 0.04], happy: [0.2, 0.09], love: [0.2, 0.09], wave: [0.18, 0.08],
        greeting: [0.2, 0.09], dance: [0.18, 0.08], bounce: [0.16, 0.07], wink: [0.14, 0.05], spin: [0.12, 0.05],
        alert: [0.07, 0.09], startled: [0.07, 0.09], hiccup: [0.06, 0.08], dizzy: [0.09, 0.09], annoyed: [0.14, 0.03],
        sleep: [0.06, 0.03], nap: [0.06, 0.03], sad: [0.08, 0.05], worried: [0.13, 0.03], stuffed: [0.14, 0.03],
        blush: [0.08, 0.03], think: [0.08, 0.03],
      };
      const size = M[mood] || [0.11, 0.04];
      let mw = w * size[0], mh = Math.max(2, w * size[1]);
      if (mood === "yawn") [mw, mh] = [w * (0.07 + 0.08 * this.yawn), w * (0.04 + 0.12 * this.yawn)];
      else if (mood === "hum" || mood === "sip") {
        const d = w * (0.04 + 0.05 * this.pulse);
        [mw, mh] = [d, d];
      } else if (mood === "eat") [mw, mh] = [w * 0.12, w * (0.03 + 0.08 * this.chew)];
      else if (mood === "sneeze") [mw, mh] = p < 0.55 ? [w * 0.06, Math.max(2, w * 0.04)] : [w * 0.16, w * 0.14];
      const h = round(Math.max(2, mh));
      face.push(ui.box({ width: round(mw), height: h, radius: round(h / 2), fill: INK }));
    }
    return face;
  }

  /** Floating decorations (z, hearts, sweat, ...) beside the body. */
  decoration(w, height, width) {
    const kind = this.deco, ph = this.decoPhase;
    const items = [];
    // Several copies rising from the body and fading out.
    const rising = (count, make) => {
      for (let i = 0; i < count; i++) {
        const q = (ph + i / count) % 1; // 0 = just spawned, 1 = gone
        const [node, h] = make(i, q);
        items.push({ x: width * 0.08 + q * width * 0.35, y: (1 - q) * height * 0.62, h, node });
      }
    };
    const one = (node, h, x, y) => items.push({ x, y, h, node });

    if (kind === "z") {
      rising(3, (i, q) => {
        const size = round(w * (0.11 + 0.04 * i)) + 4;
        return [ui.label({ text: i === 1 ? "Z" : "z", fontSize: size, fontWeight: "bold", color: "on_surface_variant", opacity: 1 - q * q }), size + 2];
      });
    } else if (kind === "heart") {
      rising(3, (i, q) => {
        const size = round(w * (0.11 + 0.04 * i)) + 2;
        return [ui.glyph({ name: "heart", size, color: HEART, opacity: 1 - q * q }), size + 4];
      });
    } else if (kind === "note") {
      rising(3, (i, q) => {
        const size = round(w * 0.13);
        return [ui.glyph({ name: "music", size, color: "primary", opacity: 1 - q * q }), size + 4];
      });
    } else if (kind === "steam") {
      rising(3, (i, q) => {
        const sw = Math.max(2, round(w * 0.045));
        return [ui.box({ width: sw, height: round(w * 0.12), radius: round(sw / 2), fill: "#ffffff", opacity: 0.5 * (1 - q) }), round(w * 0.12)];
      });
    } else if (kind === "crumbs") {
      for (let i = 0; i < 3; i++) {
        const q = (ph * 2 + i / 3) % 1; // falling crumbs
        const d = Math.max(2, round(w * 0.04));
        one(ui.box({ width: d, height: d, radius: round(d / 2), fill: "#d9a766", opacity: 1 - q }), d, width * (0.15 + 0.25 * i), height * (0.35 + 0.5 * q));
      }
    } else if (kind === "droplets") {
      const q = clamp(((this.curP || 0) - 0.55) / 0.45, 0, 1); // only after the sneeze bursts
      if (q > 0) {
        for (let i = 0; i < 3; i++) {
          const d = Math.max(2, round(w * 0.045));
          one(ui.box({ width: d, height: d, radius: round(d / 2), fill: SWEAT, opacity: 1 - q }), d, width * (0.1 + 0.55 * q), height * (0.3 + 0.1 * i));
        }
      }
    } else if (kind === "dots") {
      const n = 1 + (Math.floor(this.t / 350) % 3);
      const size = round(w * 0.15) + 2;
      one(ui.label({ text: ".".repeat(n), fontSize: size, fontWeight: "bold", color: "on_surface_variant" }), size + 2, width * 0.15, height * 0.12);
    } else if (kind === "hic" || kind === "hi") {
      const size = round(w * 0.13) + 2;
      one(ui.label({ text: kind === "hic" ? "hic!" : "hi!", fontSize: size, fontWeight: "bold", color: "primary" }), size + 4, width * 0.1, height * 0.1);
    } else if (kind === "sweat") {
      const d = round(w * 0.08);
      const bob = Math.sin(this.t / 300) * height * 0.03;
      one(ui.box({ width: d, height: round(d * 1.5), radius: round(d / 2), fill: SWEAT }), round(d * 1.5), width * 0.2, height * 0.1 + bob);
    } else if (kind === "tear") {
      const d = round(w * 0.06);
      one(ui.box({ width: d, height: round(d * 1.5), radius: round(d / 2), fill: SWEAT, opacity: 1 - ph * 0.8 }), round(d * 1.5), width * 0.15, height * (0.3 + ph * 0.35));
    } else if (kind === "sparkle") {
      const on = Math.floor(this.t / 220) % 2 === 0;
      const size = round(w * 0.16);
      one(ui.glyph({ name: "sparkles", size, color: "#ffd24a", opacity: on ? 1 : 0.45 }), size + 2, width * 0.25, height * 0.05);
    } else if (kind === "pencil" || kind === "terminal" || kind === "search") {
      const glyph = { pencil: "pencil", terminal: "terminal-2", search: "search" }[kind];
      const size = round(w * 0.16);
      const pulse = 0.55 + 0.45 * sine(this.t, 900);
      one(ui.glyph({ name: glyph, size, color: "primary", opacity: pulse }), size + 2, width * 0.22, height * 0.08);
    }
    return stack(width, height, items);
  }

  /** A small status bubble (like a chat "typing" indicator) for the top-left of the pet. */
  badge(kind, width, height) {
    const d = Math.max(10, round(width * 0.95));
    let inner;
    if (kind === "alert") {
      inner = ui.label({ text: "!", fontSize: round(d * 0.7), fontWeight: "bold", color: "#ffffff" });
    } else {
      const dot = Math.max(2, round(d * 0.16));
      const dots = [0, 1, 2].map((i) => {
        // The dots pulse one after the other.
        const pulse = 0.4 + 0.6 * Math.max(0, Math.sin(this.t / 260 - i * 0.9));
        return ui.box({ width: dot, height: dot, radius: round(dot / 2), fill: "#ffffff", opacity: pulse });
      });
      inner = ui.row({ gap: Math.max(1, round(dot * 0.5)), align: "center" }, dots);
    }
    const bubble = ui.column(
      { width: d, height: d, fill: kind === "alert" ? "#f59e0b" : "#2f6fed", radius: round(d / 2), border: "#0d0f13", borderWidth: 2, align: "center", justify: "center" },
      [inner],
    );
    return ui.column({ width: round(width), height: round(height), align: "start", justify: "start" }, [bubble]);
  }

  /** Build the UI tree for a character `size` px wide.
   *  opts.room       extra vertical room (px) above the body for jumps (default size*0.3)
   *  opts.margin     horizontal room (px) on each side for shaking
   *  opts.mouth      draw the mouth (default true)
   *  opts.maxHeight  total height budget (px): tall characters (dango) shrink to fit
   *  opts.badge      "work" (blue bubble with three dots) or "alert" (orange "!") in the top-left corner
   *  opts.badgeSlot  keep the room for the bubble even when there is none, so the pet does not jump sideways
   *  opts.detail     force (true) or forbid (false) the stripes, shine and floating decorations */
  build(size, opts = {}) {
    const c = this.char;
    let w = size;
    const room = opts.room ?? round(w * 0.3);
    if (opts.maxHeight) w = Math.min(size, Math.floor((opts.maxHeight - room) / c.ratio));
    const detail = opts.detail ?? w >= 48; // stripes, shine and floating decorations are skipped at bar size
    const margin = opts.margin ?? round(w * 0.16);

    const bw = round(w * (1 + this.sx));
    const baseH = round(w * c.ratio);
    const bh = round(baseH * (1 + this.sy));

    const body = BODIES[c.kind](c, { w, bw, bh, detail, face: this.face(w, detail, opts) });

    // Frame: leading spacers place the body (shake = x, lift = y). The right-hand room for
    // decorations is always reserved so the layout never jumps.
    const decoW = detail ? round(w * 0.36) : 0;
    let fw = round(w + margin * 2 + decoW);
    const fh = round(baseH + room);
    const left = margin + round(this.shake * margin) - round((bw - w) / 2);
    const topGap = fh - bh - round(this.lift * room);
    const rowChildren = [spacerW(left), body];
    if (opts.badge || opts.badgeSlot) {
      rowChildren.unshift(opts.badge ? this.badge(opts.badge, margin, bh) : spacerW(margin));
      fw += margin;
    }
    if (detail) rowChildren.push(this.decoration(w, baseH, decoW));
    return ui.column({ width: fw, height: fh, justify: "start", align: "start" }, [spacerH(topGap), ui.row({}, rowChildren)]);
  }

  /** One-line caption for the panel. */
  caption(nSessions, nWorking, nPending) {
    const mood = this.cur || this.baseMood;
    if (mood === "sleep") return "Zzz... the daemon is asleep";
    if (mood === "nap") return "Zzz... taking a nap";
    if (mood === "alert" || nPending > 0) return nPending <= 1 ? "I need you!" : `I need you! (${nPending})`;
    const reactions = {
      love: "Thank you!", sad: "Aww...", startled: "Whoa!", dizzy: "Whoa, everything is spinning!", annoyed: "Hey!",
      happy: "Done!", yawn: "*yawn*", worried: "Usage is getting high...", stuffed: "So full... compact the context?",
      wave: "Hi there!", greeting: "Hello!", eat: "Nom nom nom", dance: "La la la~", wink: ";)", blush: "Stop it, you...",
      sneeze: "Ah... ah... choo!", hum: "Hmm hmm hmm~", think: "Hmmm...", spin: "Wheee!", bounce: "Boing!",
      sip: "*sip*", hiccup: "Hic!", jiggle: "Jiggle jiggle",
    };
    if (reactions[mood]) return reactions[mood];
    if (nWorking > 0) {
      const what = { write: "writing", run: "running a command", search: "searching", think: "thinking" }[this.workKind];
      if (nWorking === 1) return `Your agent is ${what || "working"}...`;
      return `Your agents are working on ${nWorking} sessions...`;
    }
    return nSessions > 0 ? "All quiet" : "No sessions, hi!";
  }
}

// Floating decoration per mood (drawn beside the body at panel size).
const DECO = {
  nap: "z", sleep: "z", love: "heart", worried: "sweat", stuffed: "sweat", sad: "tear", happy: "sparkle", hum: "note",
  dance: "note", think: "dots", sneeze: "droplets", hiccup: "hic", wave: "hi", greeting: "hi", eat: "crumbs",
};
// Decoration while working, by kind of work ("read" has none: the eyes say it).
const WORK_DECO = { write: "pencil", run: "terminal", search: "search", think: "dots" };

/** Stack positioned items {x, y, h, node} into a column of the given size. */
function stack(width, height, items) {
  items.sort((a, b) => a.y - b.y);
  const children = [];
  let cursor = 0;
  for (const it of items) {
    const y = clamp(it.y, cursor, Math.max(cursor, height - it.h));
    children.push(spacerH(y - cursor), ui.row({}, [spacerW(it.x), it.node]));
    cursor = y + it.h;
  }
  return ui.column({ width, height: round(height), align: "start", justify: "start" }, children);
}

// ── Bodies: each takes (character, context) and returns the body tree ─────────

const withFirst = (first, list) => [first, ...list];
const col = (x, props, children) => ui.column({ width: x.bw, height: x.bh, align: "center", justify: "start", ...props }, children);

function nigiriBody(c, x) {
  const topH = round(x.bh * 0.4);
  const riceH = x.bh - topH;
  const riceW = round(x.bw * 0.9);
  let stripes = [];
  if (c.bands) {
    // Shrimp: vertical pale bands across the topping.
    const bandW = Math.max(2, round(x.bw * 0.07)), bandH = Math.max(3, round(topH * 0.62));
    const band = () => ui.box({ width: bandW, height: bandH, radius: 1, fill: c.bands });
    stripes = [ui.row({ gap: Math.max(2, round(x.bw * 0.07)), align: "center" }, [band(), band(), band(), band()])];
  } else if (x.detail) {
    stripes = [
      ui.box({ width: round(x.bw * 0.52), height: Math.max(2, round(topH * 0.1)), radius: 2, fill: c.fat }),
      ui.box({ width: round(x.bw * 0.34), height: Math.max(2, round(topH * 0.08)), radius: 2, fill: c.fat }),
    ];
  }
  const top = ui.column({ width: x.bw, height: topH, fill: c.top, radius: round(topH * 0.5), align: "center", justify: "center", gap: round(topH * 0.16) }, stripes);
  const rice = ui.column(
    { width: riceW, height: riceH, fill: RICE, radius: round(Math.min(riceW, riceH) * 0.42), border: RICE_EDGE, borderWidth: 1, align: "center", justify: "center", gap: round(x.w * 0.035) },
    x.face,
  );
  return col(x, {}, [top, rice]);
}

function makiBody(c, x) {
  const inner = [];
  if (x.detail) {
    const d = round(x.w * 0.09);
    const dot = (f) => ui.box({ width: d, height: d, radius: round(d / 2), fill: f });
    inner.push(ui.row({ gap: round(x.w * 0.03) }, [dot(c.filling), dot(c.filling2), dot(c.filling)]));
  }
  inner.push(...x.face);
  const iw = round(x.bw * 0.8), ih = round(x.bh * 0.8);
  const rice = ui.column({ width: iw, height: ih, fill: RICE, radius: round(Math.min(iw, ih) / 2), align: "center", justify: "center", gap: round(x.w * 0.03) }, inner);
  return ui.column({ width: x.bw, height: x.bh, fill: NORI, radius: round(Math.min(x.bw, x.bh) / 2), align: "center", justify: "center" }, [rice]);
}

/** Inside-out roll: rice outside (with sesame and tobiko), a thin nori ring, rice and filling inside. */
function uramakiBody(c, x) {
  const dot = (color) => {
    const d = round(x.w * 0.08);
    return ui.box({ width: d, height: d, radius: round(d / 2), fill: color });
  };
  const inner = [];
  if (x.detail) inner.push(ui.row({ gap: round(x.w * 0.025) }, [dot(c.filling), dot(c.filling2), dot(c.filling3)]));
  inner.push(...x.face);
  const iw = round(x.bw * 0.74), ih = round(x.bh * 0.74);
  const rice = ui.column({ width: iw, height: ih, fill: RICE, radius: round(Math.min(iw, ih) / 2), align: "center", justify: "center", gap: round(x.w * 0.03) }, inner);
  const nw = round(x.bw * 0.82), nh = round(x.bh * 0.82);
  const nori = ui.column({ width: nw, height: nh, fill: NORI, radius: round(Math.min(nw, nh) / 2), align: "center", justify: "center" }, [rice]);
  let layers = [nori];
  if (x.detail) {
    const seeds = () => {
      const sw = round(x.w * 0.05), sh = Math.max(2, round(x.w * 0.03));
      const seed = (color) => ui.box({ width: sw, height: sh, radius: 2, fill: color });
      return ui.row({ gap: round(x.w * 0.05) }, [seed("#3a2e2a"), seed("#ff8a3d"), seed("#3a2e2a")]);
    };
    layers = [seeds(), nori, seeds()];
  }
  return ui.column(
    { width: x.bw, height: x.bh, fill: RICE, radius: round(Math.min(x.bw, x.bh) / 2), border: RICE_EDGE, borderWidth: 1, align: "center", justify: "center", gap: round(x.w * 0.01) },
    layers,
  );
}

function onigiriBody(c, x) {
  const nori = ui.box({ width: round(x.bw * 0.54), height: round(x.bh * 0.3), radius: round(x.w * 0.05), fill: NORI });
  const children = [spacerH(x.bh * 0.12), ...x.face, ui.spacer({ flexGrow: 1 }), nori, spacerH(x.bh * 0.05)];
  return ui.column(
    { width: x.bw, height: x.bh, fill: RICE, radius: round(x.bw * 0.3), border: RICE_EDGE, borderWidth: 1, align: "center", justify: "start", gap: round(x.w * 0.03) },
    children,
  );
}

/** Gunkan: nori-wrapped rice with a heap of salmon roe on top. */
function ikuraBody(c, x) {
  const roeH = round(x.bh * 0.3);
  let heap;
  if (x.detail) {
    const d = round(x.w * 0.14);
    const ball = () => ui.box({ width: d, height: d, radius: round(d / 2), fill: c.roe });
    heap = ui.column({ width: x.bw, height: roeH, align: "center", justify: "end" }, [
      ui.row({ gap: 1 }, [ball(), ball(), ball()]),
      ui.row({ gap: 1 }, [ball(), ball(), ball(), ball(), ball()]),
    ]);
  } else {
    heap = ui.column({ width: x.bw, height: roeH, align: "center", justify: "end" }, [
      ui.box({ width: round(x.bw * 0.78), height: roeH, radius: round(roeH * 0.5), fill: c.roe }),
    ]);
  }
  const nh = x.bh - roeH;
  const nw = round(x.bw * 0.9);
  const rice = ui.column({ width: round(nw * 0.84), height: round(nh * 0.8), fill: RICE, radius: round(x.w * 0.14), align: "center", justify: "center", gap: round(x.w * 0.035) }, x.face);
  const nori = ui.column({ width: nw, height: nh, fill: NORI, radius: round(x.w * 0.2), align: "center", justify: "center" }, [rice]);
  return col(x, {}, [heap, nori]);
}

/** Inari: a golden tofu pouch with a sliver of rice showing at the top. */
function inariBody(c, x) {
  const riceH = round(x.bh * 0.16);
  const rice = ui.box({ width: round(x.bw * 0.7), height: riceH, radius: round(riceH / 2), fill: RICE });
  const pod = [];
  if (x.detail) pod.push(ui.box({ width: round(x.bw * 0.5), height: Math.max(2, round(x.w * 0.03)), radius: 2, fill: INARI_DARK }));
  pod.push(...x.face);
  const podTree = ui.column({ width: x.bw, height: x.bh - riceH, fill: INARI, radius: round(x.w * 0.26), align: "center", justify: "center", gap: round(x.w * 0.035) }, pod);
  return col(x, {}, [rice, podTree]);
}

/** Dorayaki: two pancakes with sweet bean paste in between; the face is on the top one. */
function dorayakiBody(c, x) {
  const topH = round(x.bh * 0.48);
  const fillH = Math.max(2, round(x.bh * 0.1));
  const botH = x.bh - topH - fillH;
  const top = ui.column({ width: x.bw, height: topH, fill: DORA_TOP, radius: round(topH * 0.48), align: "center", justify: "center", gap: round(x.w * 0.03) }, x.face);
  const an = ui.box({ width: round(x.bw * 0.92), height: fillH, radius: round(fillH / 2), fill: DORA_AN });
  const bottom = ui.box({ width: x.bw, height: botH, radius: round(botH * 0.48), fill: DORA_BOT });
  return col(x, {}, [top, an, bottom]);
}

/** Dango: three rice dumplings (pink, white, green); the face is on the big middle one. */
function dangoBody(c, x) {
  const h1 = round(x.bh * 0.256), h2 = round(x.bh * 0.488);
  const h3 = x.bh - h1 - h2;
  const ball = (d, color) => ui.column({ width: d, height: d, fill: color, radius: round(d / 2), align: "center", justify: "center" }, []);
  return col(x, {}, [
    ball(h1, c.top),
    ui.column(
      { width: round(x.bw * 0.78), height: h2, fill: c.mid, radius: round(h2 / 2), border: RICE_EDGE, borderWidth: 1, align: "center", justify: "center", gap: round(x.w * 0.03) },
      x.face,
    ),
    ball(h3, c.bottom),
  ]);
}

/** Ebi tempura: a golden fried shrimp with its orange tail sticking out on the right. */
function tempuraBody(c, x) {
  const bodyW = round(x.bw * 0.84);
  const tailW = x.bw - bodyW;
  const inner = [];
  if (x.detail) {
    const crumb = (w, h) => ui.box({ width: round(x.w * w), height: round(x.w * h), radius: 2, fill: BATTER_DARK });
    inner.push(ui.row({ gap: round(x.w * 0.05), align: "end" }, [crumb(0.05, 0.03), crumb(0.07, 0.045), crumb(0.04, 0.03), crumb(0.06, 0.04)]));
  }
  inner.push(...x.face);
  const batter = ui.column({ width: bodyW, height: x.bh, fill: BATTER, radius: round(x.bh * 0.46), align: "center", justify: "center", gap: round(x.w * 0.03) }, inner);
  const tail = ui.column({ width: tailW, height: round(x.bh * 0.52), fill: TAIL, radius: round(tailW * 0.5) }, []);
  return ui.row({ width: x.bw, height: x.bh, align: "center", justify: "start" }, [batter, tail]);
}

/** Ravioli (steamed dumpling): pleated ridge on top, pale dough, seared golden base. */
function ravioliBody(c, x) {
  const ridgeH = round(x.bh * 0.14);
  const baseH = round(x.bh * 0.16);
  const mainH = x.bh - ridgeH - baseH;
  const n = x.detail ? 6 : 4;
  const pleatW = Math.max(2, round(x.bw * (x.detail ? 0.085 : 0.1)));
  const pleats = Array.from({ length: n }, () => ui.box({ width: pleatW, height: ridgeH, radius: round(pleatW / 2), fill: DOUGH_FOLD }));
  const ridge = ui.row({ gap: Math.max(1, round(x.bw * 0.025)), align: "end" }, pleats);
  const main = ui.column({ width: x.bw, height: mainH, fill: DOUGH, radius: round(mainH * 0.46), border: DOUGH_EDGE, borderWidth: 1, align: "center", justify: "center", gap: round(x.w * 0.035) }, x.face);
  const base = ui.box({ width: round(x.bw * 0.84), height: baseH, radius: round(baseH * 0.5), fill: SEARED });
  return col(x, {}, [ridge, main, base]);
}

/** Bao: a fluffy white steamed bun with a fold on top, sitting on a bamboo steamer. */
function baoBody(c, x) {
  const trayH = round(x.bh * 0.13);
  const bunH = x.bh - trayH;
  const inner = [];
  if (x.detail) inner.push(ui.box({ width: round(x.bw * 0.22), height: Math.max(2, round(x.w * 0.035)), radius: 2, fill: BAO_FOLD }));
  inner.push(...x.face);
  const bun = ui.column({ width: x.bw, height: bunH, fill: BAO, radius: round(Math.min(x.bw, bunH) * 0.5), border: BAO_FOLD, borderWidth: 1, align: "center", justify: "center", gap: round(x.w * 0.035) }, inner);
  let slats = [];
  if (x.detail) {
    const sh = Math.max(2, round(trayH * 0.5));
    const slat = () => ui.box({ width: Math.max(2, round(x.w * 0.025)), height: sh, radius: 1, fill: BAMBOO_DARK });
    slats = [ui.row({ gap: round(x.w * 0.09), align: "center" }, [slat(), slat(), slat(), slat(), slat()])];
  }
  const tray = ui.column({ width: round(x.bw * 0.86), height: trayH, fill: BAMBOO, radius: round(trayH * 0.45), align: "center", justify: "center" }, slats);
  return col(x, {}, [bun, tray]);
}

/** Ramen: noodles with a narutomaki, a soft-boiled egg, nori and scallions in a cream bowl. */
function ramenBody(c, x) {
  const topH = round(x.bh * 0.3);
  const noodleH = Math.max(2, round(x.bh * 0.08));
  const bowlH = x.bh - topH - noodleH;
  let toppings;
  if (x.detail) {
    const d = round(x.w * 0.2);
    const egg = ui.column({ width: round(d * 1.15), height: round(d * 0.85), fill: "#fff6e5", radius: round(d * 0.4), align: "center", justify: "center" }, [
      ui.box({ width: round(d * 0.5), height: round(d * 0.5), radius: round(d * 0.25), fill: "#ffb52e" }),
    ]);
    const naruto = ui.column({ width: d, height: d, fill: "#ffffff", radius: round(d / 2), border: "#f0b8c4", borderWidth: 2, align: "center", justify: "center" }, [
      ui.box({ width: round(d * 0.42), height: round(d * 0.42), radius: round(d * 0.21), fill: "#ff8fa3" }),
    ]);
    const nori = ui.box({ width: round(d * 0.5), height: round(d * 0.95), radius: 2, fill: NORI });
    const sd = Math.max(2, round(d * 0.28));
    const scallion = () => ui.box({ width: sd, height: sd, radius: round(sd / 2), fill: "#6fbf4a" });
    toppings = ui.row({ gap: round(x.w * 0.03), align: "end" }, [egg, naruto, nori, ui.row({ gap: 1, align: "end" }, [scallion(), scallion(), scallion()])]);
  } else {
    const d = Math.max(3, round(x.bw * 0.2));
    const dot = (color) => ui.box({ width: d, height: d, radius: round(d / 2), fill: color });
    toppings = ui.row({ gap: 1, align: "end" }, [dot("#ff8fa3"), dot("#fff6e5"), dot("#6fbf4a")]);
  }
  const pile = ui.column({ width: x.bw, height: topH, align: "center", justify: "end" }, [toppings]);
  const noodles = ui.box({ width: round(x.bw * 0.92), height: noodleH, radius: round(noodleH / 2), fill: NOODLE });
  const bowl = ui.column(
    { width: x.bw, height: bowlH, fill: BOWL_CREAM, radius: round(x.w * 0.3), border: BOWL_EDGE, borderWidth: 1, align: "center", justify: "center", gap: round(x.w * 0.03) },
    withFirst(ui.box({ width: round(x.bw * 0.78), height: Math.max(2, round(x.w * 0.04)), radius: 2, fill: BOWL_RED }), x.face),
  );
  return col(x, {}, [pile, noodles, bowl]);
}

/** Takoyaki: a golden octopus ball with sauce, mayo and green seaweed on top. */
function takoyakiBody(c, x) {
  const inner = [];
  const th = Math.max(2, round(x.w * 0.05));
  const bar = (wf, h, color) => ui.box({ width: round(x.bw * wf), height: h, radius: 2, fill: color });
  if (x.detail) {
    const sd = round(x.w * 0.03);
    const aonori = () => ui.box({ width: sd, height: sd, radius: 1, fill: "#5fae4a" });
    inner.push(ui.row({ gap: round(x.w * 0.04) }, [aonori(), aonori(), aonori(), aonori()]));
    inner.push(bar(0.46, th, SAUCE), bar(0.34, Math.max(2, round(x.w * 0.025)), "#fff6e0"), bar(0.52, th, SAUCE));
  } else {
    inner.push(bar(0.44, th, SAUCE));
  }
  inner.push(...x.face);
  return ui.column({ width: x.bw, height: x.bh, fill: TAKO, radius: round(Math.min(x.bw, x.bh) / 2), align: "center", justify: "center", gap: round(x.w * 0.025) }, inner);
}

/** Miso soup: a vermilion lacquer bowl with a golden rim; tofu and wakame float in the soup. */
function misoBody(c, x) {
  const surfH = round(x.bh * 0.24);
  let floating = [];
  if (x.detail) {
    const s = round(x.w * 0.085);
    const tofu = () => ui.box({ width: s, height: s, radius: 2, fill: "#fff8ea" });
    const wakame = () => ui.box({ width: round(s * 1.3), height: round(s * 0.7), radius: 2, fill: "#3f7a4a" });
    floating = [ui.row({ gap: round(x.w * 0.05), align: "center" }, [tofu(), wakame(), tofu(), wakame()])];
  }
  const surface = ui.column({ width: round(x.bw * 0.9), height: surfH, fill: MISO, radius: round(surfH / 2), align: "center", justify: "center" }, floating);
  const bowl = ui.column(
    { width: x.bw, height: x.bh - surfH, fill: LACQUER, radius: round(x.w * 0.32), align: "center", justify: "center", gap: round(x.w * 0.03) },
    withFirst(ui.box({ width: round(x.bw * 0.8), height: Math.max(2, round(x.w * 0.035)), radius: 2, fill: GOLD }), x.face),
  );
  return col(x, {}, [surface, bowl]);
}

/** Edamame: a green pod with a few beans peeking out of the top and a little stem. */
function edamameBody(c, x) {
  const beansH = round(x.bh * 0.17);
  const podH = x.bh - beansH;
  const d = Math.max(3, round(x.w * 0.17));
  const bean = () => ui.box({ width: d, height: Math.min(d, beansH + 2), radius: round(d / 2), fill: BEAN });
  const beans = ui.column({ width: x.bw, height: beansH, align: "center", justify: "end" }, [
    ui.row({ gap: Math.max(1, round(x.w * 0.03)), align: "end" }, [bean(), bean(), bean()]),
  ]);
  const podW = round(x.bw * 0.9);
  const pod = ui.column({ width: podW, height: podH, fill: POD, radius: round(podH * 0.46), align: "center", justify: "center", gap: round(x.w * 0.035) }, x.face);
  const stem = ui.column({ width: x.bw - podW, height: round(podH * 0.3), fill: POD_DARK, radius: round((x.bw - podW) / 2) }, []);
  const row = ui.row({ width: x.bw, height: podH, align: "center", justify: "start" }, [pod, stem]);
  return col(x, {}, [beans, row]);
}

/** Mochi (daifuku): a soft pink rice cake with a strawberry on top. */
function mochiBody(c, x) {
  const berryH = round(x.bh * 0.16);
  const bodyH = x.bh - berryH;
  const bw = Math.max(4, round(x.bw * 0.26));
  const berry = ui.column(
    { width: bw, height: berryH, fill: BERRY, radius: round(berryH * 0.5), align: "center", justify: "start" },
    x.detail ? [ui.box({ width: round(bw * 0.5), height: Math.max(2, round(berryH * 0.22)), radius: 1, fill: "#5fae4a" })] : [],
  );
  const mochi = ui.column(
    { width: x.bw, height: bodyH, fill: MOCHI, radius: round(Math.min(x.bw, bodyH) * 0.46), border: MOCHI_EDGE, borderWidth: 1, align: "center", justify: "center", gap: round(x.w * 0.035) },
    x.face,
  );
  return col(x, {}, [berry, mochi]);
}

/** Bubble tea: milk tea in a cup with a lid, a straw and tapioca pearls at the bottom. */
function bobaBody(c, x) {
  const strawH = round(x.bh * 0.12);
  const lidH = Math.max(2, round(x.bh * 0.06));
  const cupH = x.bh - strawH - lidH;
  const strawW = Math.max(2, round(x.w * 0.07));
  const straw = ui.row({ width: x.bw, height: strawH, align: "end", justify: "start" }, [spacerW(x.bw * 0.58), ui.box({ width: strawW, height: strawH, radius: 1, fill: "#e8556d" })]);
  const lid = ui.box({ width: round(x.bw * 0.98), height: lidH, radius: round(lidH / 2), fill: "#f3f3f3" });
  const d = Math.max(3, round(x.w * 0.1));
  const pearl = () => ui.box({ width: d, height: d, radius: round(d / 2), fill: PEARL });
  const cupChildren = [spacerH(cupH * 0.08), ...x.face, ui.spacer({ flexGrow: 1 }), ui.row({ gap: 1, align: "end" }, [pearl(), pearl(), pearl(), pearl(), pearl()]), spacerH(cupH * 0.04)];
  const cup = ui.column({ width: round(x.bw * 0.9), height: cupH, fill: TEA, radius: round(x.w * 0.18), align: "center", justify: "start", gap: round(x.w * 0.03) }, cupChildren);
  return col(x, {}, [straw, lid, cup]);
}

/** Sake: a ceramic tokkuri (flask) with a blue lip and band; it sways a little, tipsy. */
function sakeBody(c, x) {
  const lipH = Math.max(2, round(x.bh * 0.05));
  const neckH = round(x.bh * 0.2);
  const bodyH = x.bh - lipH - neckH;
  const lip = ui.box({ width: round(x.bw * 0.34), height: lipH, radius: 2, fill: SAKE_BLUE });
  const neck = ui.box({ width: round(x.bw * 0.2), height: neckH, radius: round(x.bw * 0.05), fill: CERAMIC });
  const inner = [];
  if (x.detail) inner.push(ui.box({ width: round(x.bw * 0.62), height: Math.max(2, round(x.w * 0.05)), radius: 2, fill: SAKE_BLUE }));
  inner.push(...x.face);
  const body = ui.column(
    { width: round(x.bw * 0.94), height: bodyH, fill: CERAMIC, radius: round(x.w * 0.42), border: CERAMIC_EDGE, borderWidth: 1, align: "center", justify: "center", gap: round(x.w * 0.03) },
    inner,
  );
  return col(x, {}, [lip, neck, body]);
}

function tofuBody(c, x) {
  return ui.column({ width: x.bw, height: x.bh, fill: "primary", radius: round(Math.min(x.bw, x.bh) * 0.34), align: "center", justify: "center", gap: round(x.w * 0.1) }, x.face);
}

const BODIES = {
  nigiri: nigiriBody, maki: makiBody, uramaki: uramakiBody, onigiri: onigiriBody, tofu: tofuBody, ikura: ikuraBody,
  inari: inariBody, dorayaki: dorayakiBody, dango: dangoBody, tempura: tempuraBody, ravioli: ravioliBody, bao: baoBody,
  ramen: ramenBody, takoyaki: takoyakiBody, miso: misoBody, edamame: edamameBody, mochi: mochiBody, boba: bobaBody,
  sake: sakeBody,
};
