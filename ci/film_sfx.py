#!/usr/bin/env python3
"""Trailer sound design, synthesized: `ci/film_sfx.py DIR`.

The game's bank (`ci/film.sh bank`) is music and foley; a trailer also wants
the sounds no game plays — the hit on a title, the riser into a cut, the
whoosh across one. These are made here from sines, saws and noise, so they
are ours outright (no licence to credit), and written beside the bank as
48 kHz stereo WAVs the cut lists name like any other:

    trailer_boom.wav     a cinematic impact: sub drop, body, long tail
    trailer_braam.wav    a low brass blast, for the big title cards
    trailer_riser.wav    six seconds of rising noise and tone, cut at the top
    trailer_whoosh.wav   a pass across the stereo field
    trailer_reverse.wav  a boom's tail backwards: a swell into a cut
    trailer_pulse.wav    one low heartbeat thud
    trailer_drone.wav    a dark bed for a cold open

Deterministic: a fixed seed, so a re-run writes the same bytes.
"""

import os
import sys
import wave

import numpy as np

RATE = 48_000
rng = np.random.default_rng(20261007)


def t_of(seconds):
    return np.arange(int(seconds * RATE)) / RATE


def noise(seconds):
    return rng.standard_normal(int(seconds * RATE))


def one_pole(x, cutoff, high=False):
    """A one-pole low- (or high-) pass; `cutoff` may be an array, per sample."""
    cutoff = np.broadcast_to(np.asarray(cutoff, dtype=float), x.shape)
    a = np.exp(-2 * np.pi * cutoff / RATE)
    y = np.empty_like(x)
    acc = 0.0
    for i in range(len(x)):
        acc = (1 - a[i]) * x[i] + a[i] * acc
        y[i] = acc
    return x - y if high else y


def lowpass(x, cutoff, poles=2):
    for _ in range(poles):
        x = one_pole(x, cutoff)
    return x


def bandpass(x, centre, q=1.5):
    """A state-variable band-pass whose centre may move per sample."""
    centre = np.broadcast_to(np.asarray(centre, dtype=float), x.shape)
    f = 2 * np.sin(np.pi * np.minimum(centre, RATE / 6) / RATE)
    damp = 1.0 / q
    low = band = 0.0
    y = np.empty_like(x)
    for i in range(len(x)):
        low += f[i] * band
        high = x[i] - low - damp * band
        band += f[i] * high
        y[i] = band
    return y


def saw(freq_hz, t):
    """A band-limited-enough saw: the first 24 harmonics."""
    out = np.zeros_like(t)
    for k in range(1, 25):
        if k * np.max(freq_hz) > RATE / 2.2:
            break
        out += np.sin(2 * np.pi * k * np.cumsum(np.broadcast_to(freq_hz, t.shape)) / RATE) / k
    return out * (2 / np.pi)


def sweep_sine(f0, f1, seconds, curve="exp"):
    t = t_of(seconds)
    u = t / seconds
    f = f0 * (f1 / f0) ** u if curve == "exp" else f0 + (f1 - f0) * u
    return np.sin(2 * np.pi * np.cumsum(f) / RATE)


def reverb(x, seconds=3.0, damp_hz=3500, wet=0.35):
    """A plate-ish tail: decaying noise as the impulse, a different one per
    side so the tail is wide. Convolved by FFT."""
    n = int(seconds * RATE)
    env = np.exp(-6.9 * np.arange(n) / n)
    out = []
    for _ in range(2):
        ir = lowpass(rng.standard_normal(n) * env, damp_hz, poles=1)
        ir /= np.sqrt(np.sum(ir ** 2))
        m = len(x) + n - 1
        size = 1 << (m - 1).bit_length()
        y = np.fft.irfft(np.fft.rfft(x, size) * np.fft.rfft(ir, size), size)[:m]
        out.append(y)
    dry = np.pad(x, (0, n - 1))
    return np.stack([dry * (1 - wet) + out[0] * wet, dry * (1 - wet) + out[1] * wet])


def stereo(x, width=0.0):
    return np.stack([x * (1 - width), x * (1 + width)]) if x.ndim == 1 else x


def fade(x, start=0.003, end=0.05):
    n0, n1 = int(start * RATE), int(end * RATE)
    if n0:
        x[..., :n0] *= np.linspace(0, 1, n0)
    if n1:
        x[..., -n1:] *= np.linspace(1, 0, n1)
    return x


def normalize(x, peak_db=-1.0):
    return x / np.max(np.abs(x)) * 10 ** (peak_db / 20)


def boom():
    s = 5.0
    t = t_of(s)
    # The sub: a sine falling from 90 to 32 Hz in the first half second.
    f = 32 + 58 * np.exp(-t / 0.18)
    sub = np.sin(2 * np.pi * np.cumsum(f) / RATE) * np.exp(-t / 1.4)
    body = lowpass(noise(s), 300) * np.exp(-t / 0.22) * 3.0
    crack = one_pole(noise(s), 2500, high=True) * np.exp(-t / 0.012) * 0.6
    dry = np.tanh(1.6 * (sub + body + crack))
    return fade(normalize(reverb(dry, 3.5, 1800, wet=0.3)[:, : int(s * RATE)]), end=0.4)


def braam():
    s = 4.5
    t = t_of(s)
    amp = np.minimum(t / 0.04, 1) * np.exp(-np.maximum(t - 0.3, 0) / 1.3)
    voices = np.zeros_like(t)
    for base in (55.0, 82.41, 110.0):  # A1, E2, A2
        for detune in (-0.12, 0.0, 0.13):
            voices += saw(base * 2 ** (detune / 12), t)
    # The filter opens on the attack and closes over the tail: the "blat".
    cutoff = 180 + 2400 * np.exp(-t / 0.35)
    tone = lowpass(voices, cutoff, poles=2)
    sub = np.sin(2 * np.pi * 55 * t) * 0.8
    dry = np.tanh(2.2 * tone * amp) + sub * amp
    return fade(normalize(reverb(dry, 3.0, 2500, wet=0.28)[:, : int(s * RATE)]), end=0.5)


def riser():
    s = 6.0
    t = t_of(s)
    u = t / s
    centre = 180 * (7000 / 180) ** (u ** 1.6)
    air = bandpass(noise(s), centre, q=3.0) * (u ** 2.2)
    tone = (sweep_sine(110, 880, s) + 0.5 * sweep_sine(165, 1320, s)) * (u ** 3) * 0.25
    # Tremolo that speeds up toward the top, the classic tension tick.
    trem = 0.75 + 0.25 * np.sin(2 * np.pi * np.cumsum(2 + 14 * u ** 2) / RATE)
    x = (air + tone) * trem
    return fade(normalize(np.stack([x, np.roll(x, 240)]), -2.0), start=0.5, end=0.01)


def whoosh():
    s = 1.6
    t = t_of(s)
    u = t / s
    bell = np.sin(np.pi * u) ** 2
    centre = 300 + 2600 * bell
    x = bandpass(noise(s), centre, q=1.2) * bell
    pan = u  # left to right
    return fade(normalize(np.stack([x * np.cos(pan * np.pi / 2), x * np.sin(pan * np.pi / 2)]), -3.0))


def reverse():
    tail = boom()[:, : int(2.6 * RATE)]
    return fade(normalize(tail[:, ::-1].copy(), -2.0), start=0.3, end=0.005)


def pulse():
    s = 1.2
    t = t_of(s)
    f = 48 + 30 * np.exp(-t / 0.05)
    thud = np.sin(2 * np.pi * np.cumsum(f) / RATE) * np.exp(-t / 0.16)
    thud += 0.3 * lowpass(noise(s), 200) * np.exp(-t / 0.04)
    return fade(normalize(stereo(np.tanh(1.4 * thud)), -2.0), end=0.2)


def drone():
    s = 14.0
    t = t_of(s)
    beat = (np.sin(2 * np.pi * 55 * t) + np.sin(2 * np.pi * 55.35 * t)
            + 0.6 * np.sin(2 * np.pi * 82.6 * t) + 0.3 * np.sin(2 * np.pi * 110.4 * t))
    wind = bandpass(noise(s), 400 + 250 * np.sin(2 * np.pi * t / 7.0), q=0.8) * 0.5
    swell = np.minimum(t / 3.0, 1) * np.minimum((s - t) / 2.0, 1)
    x = (lowpass(beat, 600) + wind) * swell
    return normalize(reverb(x, 4.0, 1500, wet=0.4)[:, : int(s * RATE)], -4.0)


def write(path, x):
    pcm = (np.clip(x.T, -1, 1) * 32767).astype("<i2")
    with wave.open(path, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(pcm.tobytes())
    print(f"{x.shape[1] / RATE:5.2f}s  {path}")


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for name, make in [("boom", boom), ("braam", braam), ("riser", riser), ("whoosh", whoosh),
                       ("reverse", reverse), ("pulse", pulse), ("drone", drone)]:
        write(os.path.join(out, f"trailer_{name}.wav"), make())


if __name__ == "__main__":
    main()
