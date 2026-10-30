#!/usr/bin/env python3
"""Synthesizes the drum kit samples in this folder.

    python3 crates/audio/samples/kit/synth.py

Writes every `*.wav` next to this script: 16-bit mono at 32 kHz, trimmed
where the sound falls below -48 dB. Standard library only, and seeded, so
the files come out the same on every run. The snare's normal stroke, accent
and ghost note are recordings (`../snare-*.wav`); everything else of the kit
is made here: a kick and toms from pitch-swept sines, the cymbals from the
metallic square-wave cluster of the analog drum machines plus noise.
"""

import math
import os
import random
import struct
import wave

RATE = 32_000
HERE = os.path.dirname(os.path.abspath(__file__))


def seconds(s):
    return int(s * RATE)


def biquad(xs, kind, freq, q=0.707):
    """RBJ cookbook filter: 'lp', 'hp' or 'bp' (constant peak gain)."""
    w = 2 * math.pi * freq / RATE
    alpha = math.sin(w) / (2 * q)
    cw = math.cos(w)
    if kind == "lp":
        b = [(1 - cw) / 2, 1 - cw, (1 - cw) / 2]
    elif kind == "hp":
        b = [(1 + cw) / 2, -(1 + cw), (1 + cw) / 2]
    else:
        b = [alpha, 0.0, -alpha]
    a = [1 + alpha, -2 * cw, 1 - alpha]
    b = [v / a[0] for v in b]
    a1, a2 = a[1] / a[0], a[2] / a[0]
    out = []
    x1 = x2 = y1 = y2 = 0.0
    for x in xs:
        y = b[0] * x + b[1] * x1 + b[2] * x2 - a1 * y1 - a2 * y2
        out.append(y)
        x2, x1, y2, y1 = x1, x, y1, y
    return out


def noise(n, rng):
    return [rng.uniform(-1.0, 1.0) for _ in range(n)]


def env(n, tau, attack=0.001):
    """Exponential decay with time constant `tau`, after a linear attack."""
    a = max(1, seconds(attack))
    return [min(1.0, (i + 1) / a) * math.exp(-i / RATE / tau) for i in range(n)]


def swept_sine(n, f_start, f_end, sweep_tau):
    """A sine gliding from `f_start` down to `f_end`."""
    out, phase = [], 0.0
    for i in range(n):
        f = f_end + (f_start - f_end) * math.exp(-i / RATE / sweep_tau)
        phase += 2 * math.pi * f / RATE
        out.append(math.sin(phase))
    return out


# The six square waves of the TR-808's cymbals and hi-hats.
METAL = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0]


def metal(n, scale=1.0):
    out = [0.0] * n
    for f in METAL:
        period = RATE / (f * scale)
        for i in range(n):
            out[i] += 1.0 if (i % period) < period / 2 else -1.0
    return [v / len(METAL) for v in out]


def mix(*parts):
    n = max(len(p) for _, p in parts)
    out = [0.0] * n
    for gain, p in parts:
        for i, v in enumerate(p):
            out[i] += gain * v
    return out


def mul(xs, ys):
    return [x * y for x, y in zip(xs, ys)]


def kick(rng):
    n = seconds(0.45)
    body = mul(swept_sine(n, 160, 48, 0.03), env(n, 0.13))
    click = mul(biquad(noise(n, rng), "lp", 3000), env(n, 0.004))
    return mix((1.0, body), (0.35, click))


def tom(rng, f):
    n = seconds(0.6)
    body = mul(swept_sine(n, 1.6 * f, f, 0.04), env(n, 0.16))
    skin = mul(biquad(noise(n, rng), "bp", 2.5 * f, 1.2), env(n, 0.03))
    return mix((1.0, body), (0.25, skin))


def hihat(rng, tau, level=1.0, attack=0.0005):
    n = seconds(max(0.08, 5 * tau))
    raw = mix((0.6, metal(n, 1.0)), (0.6, noise(n, rng)))
    bright = biquad(biquad(raw, "bp", 9000, 0.9), "hp", 6500)
    return [level * v for v in mul(bright, env(n, tau, attack))]


def hihat_foot(rng):
    # The pedal: a short, soft "chick", the hats closing.
    return hihat(rng, 0.025, 0.6, attack=0.004)


def ride(rng):
    n = seconds(1.0)
    wash = biquad(mix((0.5, metal(n, 2.2)), (0.3, noise(n, rng))), "hp", 5000)
    ping = mix(*[(g, [math.sin(2 * math.pi * f * i / RATE) for i in range(n)])
                 for f, g in ((3150, 0.5), (4870, 0.3), (6420, 0.2))])
    return mix((0.7, mul(wash, env(n, 0.3))), (0.35, mul(ping, env(n, 0.25))))


def ride_bell(rng):
    n = seconds(0.8)
    partials = ((820, 0.6), (1265, 0.45), (1980, 0.3), (2710, 0.2), (3530, 0.12))
    bell = mix(*[(g, [math.sin(2 * math.pi * f * i / RATE) for i in range(n)])
                 for f, g in partials])
    wash = biquad(metal(n, 2.2), "hp", 5000)
    return mix((0.8, mul(bell, env(n, 0.25))), (0.2, mul(wash, env(n, 0.2))))


def crash(rng, choke=None):
    n = seconds(0.2 if choke else 1.3)
    raw = mix((0.4, metal(n, 1.6)), (0.8, noise(n, rng)))
    sizzle = biquad(raw, "hp", 3500)
    e = env(n, 0.4, attack=0.002)
    if choke:
        # Stopped by hand: the full hit, then gone within a few milliseconds.
        e = [v * (1.0 if i < seconds(choke) else math.exp(-(i - seconds(choke)) / RATE / 0.008))
             for i, v in enumerate(e)]
    return mul(sizzle, e)


def cross_stick(rng):
    n = seconds(0.12)
    wood = mul(biquad(noise(n, rng), "bp", 1700, 3.0), env(n, 0.012))
    knock = mul([math.sin(2 * math.pi * 480 * i / RATE) for i in range(n)], env(n, 0.02))
    return mix((1.0, wood), (0.6, knock))


def write(name, xs, peak):
    top = max(abs(v) for v in xs) or 1.0
    xs = [v * peak / top for v in xs]
    # Trim the tail below -48 dB of the peak; fade out the last sixth (at
    # least 5 ms), so a sound cut while still ringing ends smoothly.
    floor = peak * 4e-3
    end = max(i for i, v in enumerate(xs) if abs(v) > floor) + 1
    xs = xs[:end]
    fade = min(len(xs), max(seconds(0.005), len(xs) // 6))
    for k in range(fade):
        xs[-1 - k] *= k / fade
    with wave.open(os.path.join(HERE, name), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(b"".join(struct.pack("<h", int(round(v * 32767))) for v in xs))


def main():
    rng = random.Random(255)
    write("kick.wav", kick(rng), 0.9)
    write("tom-high.wav", tom(rng, 196), 0.8)
    write("tom-mid.wav", tom(rng, 147), 0.8)
    write("tom-floor.wav", tom(rng, 98), 0.85)
    write("hihat-closed.wav", hihat(rng, 0.018), 0.55)
    write("hihat-half.wav", hihat(rng, 0.09), 0.55)
    write("hihat-open.wav", hihat(rng, 0.16), 0.55)
    write("hihat-foot.wav", hihat_foot(rng), 0.4)
    write("ride.wav", ride(rng), 0.5)
    write("ride-bell.wav", ride_bell(rng), 0.55)
    write("crash.wav", crash(rng), 0.6)
    write("crash-choke.wav", crash(rng, choke=0.09), 0.6)
    write("cross-stick.wav", cross_stick(rng), 0.6)


if __name__ == "__main__":
    main()
