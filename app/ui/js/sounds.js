// sounds.js — plays the pet's sounds (ui/sounds/*.wav, made by tools/make_sounds.py) by name.
// Only the pet window plays them (the panel's pet is silent), so nothing is heard twice.

const NAMES = [
  "greeting", "hello", "happy", "love", "sad", "startled", "annoyed", "dizzy", "poke", "hop", "bounce", "wiggle",
  "squish", "yawn", "sneeze", "hiccup", "hum", "think", "wink", "blush", "spin", "eat", "dance", "nap", "wake",
  "alert", "approve", "deny", "budget", "milestone", "policy", "coin", "levelup", "purchase",
];

const audio = new Map();
let volume = 0.6;

/**
 * Load the sounds. They are fetched and played from blob: URLs because WebKitGTK (Linux) cannot
 * play media straight from Tauri's custom `tauri://` scheme (it fails with a format error), while
 * fetch() over that scheme works everywhere.
 */
export function init() {
  for (const n of NAMES) {
    fetch(`sounds/${n}.wav`)
      .then((r) => (r.ok ? r.blob() : Promise.reject(new Error(`${r.status}`))))
      .then((b) => {
        const a = new Audio(URL.createObjectURL(b));
        a.preload = "auto";
        audio.set(n, a);
      })
      .catch((e) => console.warn(`sushi: sound ${n} not loaded:`, e));
  }
}

export function setVolume(v) {
  volume = Math.max(0, Math.min(1, v));
}

export function play(name) {
  const a = audio.get(name);
  if (!a) return;
  a.volume = volume;
  a.currentTime = 0;
  a.play().catch(() => {}); // a browser may refuse until the user has interacted; that is fine
}
