// sounds.js — plays the pet's 28 sounds (ui/sounds/*.wav, made by tools/make_sounds.py) by name.
// Only the pet window plays them (the panel's pet is silent), so nothing is heard twice.

const NAMES = [
  "greeting", "hello", "happy", "love", "sad", "startled", "annoyed", "dizzy", "poke", "hop", "bounce", "wiggle",
  "squish", "yawn", "sneeze", "hiccup", "hum", "think", "wink", "blush", "spin", "eat", "dance", "nap", "wake",
  "alert", "approve", "deny",
];

const audio = new Map();
let volume = 0.6;

/** Create the audio elements (the files load lazily, on first use). */
export function init() {
  for (const n of NAMES) {
    const a = new Audio(`sounds/${n}.wav`);
    a.preload = "auto";
    audio.set(n, a);
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
