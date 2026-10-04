#!/usr/bin/env python3
"""Synthesize the pet's 34 sounds into plugin/sounds/*.wav.

Pure standard library (wave, math, struct, random): short, soft, deterministic blips
in a chiptune / toy style. Run it again to regenerate; the output does not change.

    python3 tools/make_sounds.py            # write the files and print a report
    python3 tools/make_sounds.py --check    # only synthesize and verify (no files)
"""
import math
import os
import random
import struct
import sys
import wave

SR = 22050          # sample rate: plenty for blips, half the size of 44.1 kHz
PEAK = 0.55         # every sound is normalized to this peak (the user's volume does the rest)
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "plugin", "sounds")

# ── building blocks ───────────────────────────────────────────────────────────


def _wave(shape, phase, duty=0.5):
    p = phase % 1.0
    if shape == "sine":
        return math.sin(2 * math.pi * p)
    if shape == "tri":
        return 4 * abs(p - 0.5) - 1
    if shape == "square":
        return 1.0 if p < duty else -1.0
    if shape == "saw":
        return 2 * p - 1
    raise ValueError(shape)


def tone(freq, dur, shape="sine", vol=1.0, attack=0.005, release=0.06, glide=None,
         vibrato=(0.0, 0.0), duty=0.5, decay=None):
    """One note. `glide` = end frequency (exponential slide), `vibrato` = (rate Hz, depth in semitones),
    `decay` = exponential decay rate (1/s) instead of a flat sustain."""
    n = int(dur * SR)
    out, phase = [], 0.0
    for i in range(n):
        t = i / SR
        f = freq if glide is None else freq * (glide / freq) ** (i / max(1, n - 1))
        if vibrato[0]:
            f *= 2 ** (vibrato[1] * math.sin(2 * math.pi * vibrato[0] * t) / 12)
        phase += f / SR
        env = 1.0
        if t < attack:
            env = t / attack
        if decay:
            env *= math.exp(-decay * t)
        left = dur - t
        if left < release:
            env *= max(0.0, left / release)
        out.append(_wave(shape, phase, duty) * env * vol)
    return out


def noise(dur, vol=1.0, lowpass=0.5, attack=0.002, release=0.05, rng=None):
    """Filtered noise (lowpass 0..1: lower = duller)."""
    rng = rng or random.Random(1)
    n, y, out = int(dur * SR), 0.0, []
    for i in range(n):
        y += lowpass * (rng.uniform(-1, 1) - y)
        t, left = i / SR, dur - i / SR
        env = min(1.0, t / attack) * min(1.0, max(0.0, left / release))
        out.append(y * env * vol)
    return out


def silence(dur):
    return [0.0] * int(dur * SR)


def seq(*parts):
    out = []
    for p in parts:
        out.extend(p)
    return out


def layer(*tracks):
    n = max(len(t) for t in tracks)
    return [sum(t[i] for t in tracks if i < len(t)) for i in range(n)]


def at(offset, track):
    return silence(offset) + track


def scale(track, k):
    return [s * k for s in track]


def note(name):
    """'C5' -> frequency. Flats are not needed."""
    names = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}
    n, octave = name[0], int(name[-1])
    sharp = 1 if "#" in name else 0
    return 440.0 * 2 ** ((names[n] + sharp - 9 + 12 * (octave - 4)) / 12)


# ── the 28 sounds ─────────────────────────────────────────────────────────────

def s_greeting():  # launch: a bright little arpeggio and a sparkle
    notes = [seq(tone(note(n), 0.11, "square", 0.5, duty=0.35, release=0.03)) for n in ("C5", "E5", "G5")]
    top = tone(note("C6"), 0.45, "square", 0.5, duty=0.3, release=0.25, decay=3)
    shimmer = tone(note("C7"), 0.3, "sine", 0.25, release=0.2, decay=6)
    return layer(seq(*notes, top), at(0.33, shimmer))


def s_hello():  # "hi!": two quick rising notes
    return seq(tone(note("G5"), 0.09, "tri", 0.8), tone(note("C6"), 0.18, "tri", 0.8, release=0.1))


def s_happy():  # a finished job: a jump and a twinkle
    return layer(
        seq(tone(note("C5"), 0.08, "square", 0.45, duty=0.4), tone(note("E5"), 0.08, "square", 0.45, duty=0.4),
            tone(note("G5"), 0.22, "square", 0.45, duty=0.4, release=0.15)),
        at(0.2, tone(note("E7"), 0.25, "sine", 0.2, release=0.2, decay=8)),
    )


def s_love():  # two warm chimes with a shimmer
    a = tone(note("E6"), 0.5, "sine", 0.7, vibrato=(6, 0.15), release=0.35, decay=3)
    b = tone(note("G6"), 0.5, "sine", 0.6, vibrato=(6, 0.15), release=0.35, decay=3)
    return layer(a, at(0.16, b), at(0.1, tone(note("E7"), 0.4, "sine", 0.15, release=0.3, decay=5)))


def s_sad():  # a small falling sigh
    return layer(tone(note("A4"), 0.7, "sine", 0.8, glide=note("E4"), vibrato=(5, 0.25), release=0.4),
                 tone(note("A3"), 0.7, "sine", 0.3, glide=note("E3"), release=0.4))


def s_startled():  # a gasp: quick rise and a puff of air
    return layer(tone(note("C5"), 0.16, "square", 0.5, glide=note("G6"), duty=0.3, release=0.08),
                 noise(0.12, 0.35, lowpass=0.7, rng=random.Random(2)))


def s_annoyed():  # an indignant squeak
    return seq(tone(note("G5"), 0.07, "saw", 0.4, glide=note("B5"), release=0.02),
               tone(note("B5"), 0.16, "saw", 0.4, glide=note("D5"), vibrato=(30, 0.4), release=0.08))


def s_dizzy():  # wobbling pitch
    return layer(tone(note("E5"), 0.9, "tri", 0.8, vibrato=(7, 3.0), release=0.35, glide=note("C5")),
                 tone(note("E4"), 0.9, "sine", 0.3, vibrato=(7, 2.0), release=0.35))


def s_poke():  # boop
    return tone(note("A5"), 0.11, "sine", 0.9, glide=note("D5"), release=0.05)


def s_hop():  # boing
    return tone(note("D4"), 0.28, "sine", 0.9, glide=note("D5"), vibrato=(14, 1.2), release=0.12)


def s_bounce():  # boing-boing
    one = tone(note("E4"), 0.2, "sine", 0.8, glide=note("E5"), vibrato=(14, 1.0), release=0.08)
    two = tone(note("G4"), 0.22, "sine", 0.8, glide=note("G5"), vibrato=(14, 1.0), release=0.1)
    return seq(one, silence(0.04), two)


def s_wiggle():  # a quick rattle
    parts = []
    for i in range(8):
        parts.append(tone(note("A4") * (1.25 if i % 2 else 1.0), 0.04, "square", 0.45, duty=0.25, release=0.01))
    return seq(*parts)


def s_squish():  # squelch
    return layer(tone(220, 0.26, "sine", 0.8, glide=90, vibrato=(25, 1.0), release=0.1),
                 noise(0.2, 0.25, lowpass=0.25, rng=random.Random(3)))


def s_yawn():  # a long, falling "aaah" with breath
    breath = noise(1.0, 0.18, lowpass=0.12, attack=0.25, release=0.5, rng=random.Random(4))
    return layer(tone(note("G4"), 1.05, "tri", 0.6, glide=note("C4"), vibrato=(4, 0.3), attack=0.2, release=0.5), breath)


def s_sneeze():  # ah... ah... CHOO!
    ah = tone(note("C5"), 0.12, "tri", 0.55, glide=note("E5"), release=0.05)
    ah2 = tone(note("D5"), 0.14, "tri", 0.6, glide=note("G5"), release=0.05)
    choo = layer(noise(0.18, 0.9, lowpass=0.85, attack=0.001, release=0.12, rng=random.Random(5)),
                 tone(note("A4"), 0.15, "saw", 0.4, glide=note("A3"), release=0.1))
    return seq(ah, silence(0.1), ah2, silence(0.12), choo)


def s_hiccup():  # hic!
    return seq(tone(note("D5"), 0.05, "square", 0.5, glide=note("A5"), duty=0.3, release=0.01),
               tone(note("A5"), 0.07, "square", 0.45, glide=note("F5"), duty=0.3, release=0.04))


def s_hum():  # hmm-hm-hmm
    notes = [("E4", 0.28), ("G4", 0.28), ("E4", 0.4)]
    return seq(*[tone(note(n), d, "sine", 0.8, vibrato=(5, 0.2), attack=0.05, release=0.12) for n, d in notes])


def s_think():  # blip, blip
    return seq(tone(note("A5"), 0.07, "sine", 0.8), silence(0.07), tone(note("C6"), 0.1, "sine", 0.8, release=0.06))


def s_wink():  # a sparkle ping
    return layer(tone(note("E6"), 0.3, "sine", 0.7, release=0.25, decay=9),
                 tone(note("B6"), 0.25, "sine", 0.3, release=0.2, decay=12))


def s_blush():  # a soft "coo"
    return layer(seq(tone(note("D5"), 0.22, "sine", 0.7, glide=note("F5"), release=0.1),
                     tone(note("F5"), 0.28, "sine", 0.6, glide=note("D5"), release=0.2)),
                 tone(note("A4"), 0.5, "sine", 0.2, release=0.3))


def s_spin():  # whoosh upwards
    return layer(tone(note("C4"), 0.55, "sine", 0.6, glide=note("C6"), release=0.2),
                 noise(0.55, 0.22, lowpass=0.3, attack=0.1, release=0.3, rng=random.Random(6)))


def s_eat():  # nom nom nom
    parts = []
    for i in range(3):
        crunch = noise(0.07, 0.8, lowpass=0.6, attack=0.001, release=0.05, rng=random.Random(10 + i))
        thump = tone(note("D3"), 0.07, "sine", 0.6, glide=note("A2"), release=0.04)
        parts.append(layer(crunch, thump))
        parts.append(silence(0.09))
    return seq(*parts)


def s_dance():  # a bouncy little groove
    melody = seq(*[tone(note(n), 0.13, "square", 0.4, duty=0.4, release=0.05) for n in ("C5", "E5", "G5", "E5")])
    bass = seq(*[tone(note(n), 0.24, "tri", 0.6, release=0.08) for n in ("C3", "G3")])
    loop = layer(melody, bass)
    return seq(loop, silence(0.02), loop)


def s_nap():  # a soft snore: breathe in, breathe out
    inhale = layer(noise(0.5, 0.35, lowpass=0.1, attack=0.25, release=0.2, rng=random.Random(7)),
                   tone(note("E2"), 0.5, "saw", 0.12, attack=0.25, release=0.2))
    exhale = layer(noise(0.55, 0.3, lowpass=0.08, attack=0.1, release=0.4, rng=random.Random(8)),
                   tone(note("D2"), 0.55, "saw", 0.1, attack=0.1, release=0.4))
    return seq(inhale, exhale)


def s_wake():  # a stretch and a pop
    return seq(tone(note("G3"), 0.28, "tri", 0.6, glide=note("D5"), vibrato=(5, 0.3), release=0.1),
               tone(note("G5"), 0.1, "square", 0.45, duty=0.3, release=0.06))


def s_alert():  # a permission request: a clear two-tone chime, twice
    ding = tone(note("E6"), 0.16, "sine", 0.8, release=0.08, decay=5)
    dong = tone(note("C6"), 0.22, "sine", 0.8, release=0.14, decay=4)
    return seq(ding, dong, silence(0.08), ding, dong)


def s_approve():  # yes: up and bright
    return seq(tone(note("C5"), 0.09, "square", 0.4, duty=0.4, release=0.03),
               tone(note("G5"), 0.2, "square", 0.4, duty=0.4, release=0.12))


def s_deny():  # no: low and flat
    return seq(tone(note("G3"), 0.12, "square", 0.45, duty=0.5, release=0.03),
               tone(note("D3"), 0.24, "square", 0.45, duty=0.5, glide=note("C3"), release=0.14))


def s_budget():  # a budget threshold crossed: a soft, cautionary descending chime
    beep = lambda n, d: tone(note(n), d, "sine", 0.6, release=0.08, decay=4)
    return seq(beep("A5", 0.14), silence(0.04), beep("F5", 0.18))


def s_milestone():  # a streak or step-count milestone: a small fanfare
    notes = [tone(note(n), 0.1, "square", 0.45, duty=0.35, release=0.04) for n in ("C5", "E5", "G5", "C6")]
    sparkle = tone(note("C7"), 0.3, "sine", 0.25, release=0.22, decay=7)
    return layer(seq(*notes), at(0.3, sparkle))


def s_policy():  # a policy-flagged step: a short, electronic caution ping
    return seq(tone(note("D5"), 0.09, "square", 0.5, duty=0.25, release=0.03),
               tone(note("D5"), 0.09, "square", 0.5, duty=0.25, release=0.05))


def s_coin():  # a shop/mini-game currency pickup: a bright two-note ding
    return seq(tone(note("B5"), 0.06, "square", 0.4, duty=0.3, release=0.02),
               tone(note("E6"), 0.14, "square", 0.4, duty=0.3, release=0.1))


def s_levelup():  # a growth tier crossed: a brighter, longer fanfare than a milestone's
    notes = [tone(note(n), 0.09, "square", 0.45, duty=0.3, release=0.03) for n in ("C5", "E5", "G5", "C6", "E6")]
    sparkle = tone(note("G6"), 0.35, "sine", 0.28, release=0.26, decay=6)
    return layer(seq(*notes), at(0.36, sparkle))


def s_purchase():  # a shop purchase confirmed: a soft two-tone chime
    return seq(tone(note("E5"), 0.1, "sine", 0.5, release=0.05),
               tone(note("A5"), 0.22, "sine", 0.5, release=0.16))


SOUNDS = {
    "greeting": s_greeting, "hello": s_hello, "happy": s_happy, "love": s_love, "sad": s_sad,
    "startled": s_startled, "annoyed": s_annoyed, "dizzy": s_dizzy, "poke": s_poke, "hop": s_hop,
    "bounce": s_bounce, "wiggle": s_wiggle, "squish": s_squish, "yawn": s_yawn, "sneeze": s_sneeze,
    "hiccup": s_hiccup, "hum": s_hum, "think": s_think, "wink": s_wink, "blush": s_blush,
    "spin": s_spin, "eat": s_eat, "dance": s_dance, "nap": s_nap, "wake": s_wake,
    "alert": s_alert, "approve": s_approve, "deny": s_deny,
    "budget": s_budget, "milestone": s_milestone, "policy": s_policy,
    "coin": s_coin, "levelup": s_levelup, "purchase": s_purchase,
}


# ── output ────────────────────────────────────────────────────────────────────

def normalize(track):
    peak = max((abs(s) for s in track), default=0.0)
    if peak == 0:
        return track
    k = PEAK / peak
    # A few ms of fade at both ends so nothing ever clicks.
    fade = int(0.004 * SR)
    out = [s * k for s in track]
    for i in range(min(fade, len(out))):
        out[i] *= i / fade
        out[-1 - i] *= i / fade
    return out


def write_wav(path, track):
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(b"".join(struct.pack("<h", int(max(-1.0, min(1.0, s)) * 32767)) for s in track))


def main():
    check_only = "--check" in sys.argv
    assert len(SOUNDS) == 34, f"expected 34 sounds, have {len(SOUNDS)}"
    if not check_only:
        os.makedirs(OUT, exist_ok=True)
    problems, total = [], 0.0
    for name, build in SOUNDS.items():
        track = normalize(build())
        dur = len(track) / SR
        peak = max(abs(s) for s in track)
        rms = math.sqrt(sum(s * s for s in track) / len(track))
        total += dur
        if dur > 1.3:
            problems.append(f"{name}: {dur:.2f}s is too long")
        if rms < 0.02:
            problems.append(f"{name}: almost silent (rms {rms:.3f})")
        if peak > 0.6:
            problems.append(f"{name}: peak {peak:.2f}")
        if not check_only:
            write_wav(os.path.join(OUT, name + ".wav"), track)
        print(f"{name:9s} {dur:4.2f}s  peak {peak:.2f}  rms {rms:.3f}")
    print(f"{len(SOUNDS)} sounds, {total:.1f}s in total" + ("" if check_only else f" → {os.path.normpath(OUT)}"))
    if problems:
        print("PROBLEMS:\n  " + "\n  ".join(problems))
        sys.exit(1)


if __name__ == "__main__":
    main()
