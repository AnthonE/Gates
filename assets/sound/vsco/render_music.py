#!/usr/bin/env python3
"""Render Gates' nine music pieces from VSCO-2-CE (CC0) samples, downloaded
into --cache on first run. check_music.py runs the gates on the result.

The arrangement is synth::score (crates/sound/src/synth.rs) voice for voice:
its key, tempo, chords, tier table, 10.5 s shape (8 s body, 2.5 s ring-out, no
note after 8 s) and melody generator on its seeds, so the line plays the game's
notes. Drone: cello section A2 + E3, solo contrabass A1. Pad: the chord an
octave up, violas below, violins above. Melody: harp, shortened by the tier's
exponential decay. Pulse: timpani tuned to A2.

File-name octaves are not trusted: each sample's fundamental is measured
(sustains: YIN refined to the power centroid of harmonics 1-6; harp: YIN;
timpani: the principal mode) and the sample is resampled onto equal
temperament, A2 = 110 Hz. A sustain shorter than the body is extended by an
equal-power 0.5 s crossfade loop over its stable middle, never by stretching
its attack. Stereo is folded to mono after aligning the channels.

Each voice is as loud as the synth voice it replaces (sustains by RMS, harp
over its first 0.2 s, timpani by energy over 0.34 s), on the soft velocity
layer when calm and the loud one where the music leans in. Onsets are humanized
+-8 ms, plucks +-15 % in velocity. Then a convolution reverb (decaying noise,
RT60 2.3 s, wet 0.38), a look-ahead limiter at 5x the RMS, synth.rs's edge
fades and a 0.9 peak.
"""
import argparse
import hashlib
import json
import os
import subprocess
import urllib.parse
import urllib.request
import warnings
from fractions import Fraction

import numpy as np
from scipy import signal
from scipy.io import wavfile
from scipy.ndimage import minimum_filter1d

warnings.filterwarnings("ignore", category=wavfile.WavFileWarning)  # VSCO's extra RIFF chunks
COMMIT = "440300901dfe9275fd84e0b7763af1f8443ae62e"
BASE = f"https://raw.githubusercontent.com/sgossner/VSCO-2-CE/{COMMIT}/"
SR = 44100
ROOT_HZ = 110.0                 # A2: every pitch is a semitone offset from it
BEAT = 29400                    # one beat at 90 BPM, in samples (2/3 s)
BODY = 8 * SR                   # twelve beats; no note starts at or after this
N = 463050                      # 10.5 s: the body plus 2.5 s of ring-out
SECTIONS = {"open": [0, 3, 7], "turn": [-4, 0, 3], "close": [-2, 2, 5]}  # i bVI bVII
# drone amp, pad amp, note amp, note decay (s), pulse amp, slot spacing (beats)
TIERS = {"calm": (0.30, 0.34, 0.30, 1.60, 0.00, 3),
         "tense": (0.42, 0.28, 0.34, 0.90, 0.10, 2),
         "combat": (0.55, 0.20, 0.38, 0.55, 0.34, 1)}
PENT = [12, 15, 17, 19, 22]     # A minor pentatonic, as synth.rs has it
MELODY_UP = 12                  # so the harp sounds A4 C5 D5 E5 G5, above the pad
PAD_UP = 12                     # the chord in the violas' and violins' register
WET, RT60, PEAK = 0.38, 2.3, 0.9
REVERB_GAIN = 1.26              # synth.rs `reverb`'s measured power gain on noise
CREST = 5.0                     # limiter ceiling, as a multiple of the body's RMS
# sound::Cue discriminants: 29 = MusicOpenCalm ... 37 = MusicCloseCombat
CUE = {(s, t): 29 + 3 * i + j for i, s in enumerate(SECTIONS) for j, t in enumerate(TIERS)}


def pool(fmt, names, vels):
    """Candidate samples by velocity layer; the nearest by measured pitch plays."""
    return {v: [fmt.format(n, v) for n in names.split()] for v in vels}


CELLO = pool("Strings/Cello Section/susvib/susvib_{}_v{}_1.wav", "G1 B1 D2 F2", [1, 3])
BASS = pool("Strings/Solo Contrabass/SusNV/BKCtbss_SusNV_{}_v{}_rr1.wav", "G0 A#0", [1, 3])
VIOLA = pool("Strings/Viola Section/susvib/ViolaEns_susvib_{}_v{}_1.wav", "E2 G2 B2", [1, 2])
VIOLIN = pool("Strings/Violin Section/susVib/VlnEns_susVib_{}_v{}.wav", "A2 B2 D3 F#3", [1, 2])
HARP = [f"Strings/Harp/KSHarp_{n}_mf.wav" for n in "G3 B3 D4 F4 A4 C5 E5 G5 B5 D6".split()]
TIMP = {v: [f"Percussion/Timpani/Timpani2_Hit_v{v}_rr{r}_Sum.wav" for r in (1, 2)] for v in (1, 4)}
DRONE_VEL = {"calm": 1, "tense": 3, "combat": 3}  # velocity layer per tier
PAD_VEL = {"calm": 1, "tense": 1, "combat": 2}
TIMP_VEL = {"tense": 1, "combat": 4}
LAYER = {"Cello Section": "drone", "Solo Contrabass": "drone", "Viola Section": "pad",
         "Violin Section": "pad", "Harp": "melody", "Timpani": "pulse"}


def hz(semis):
    return ROOT_HZ * 2.0 ** (semis / 12.0)


def note_name(semis):
    m = 45 + semis  # A2 is MIDI 45
    return "C C# D D# E F F# G G# A A# B".split()[m % 12] + str(m // 12 - 1)


def xorshift(seed):
    """synth.rs's `Rng` (xorshift32), bit for bit."""
    x = seed or 0x12345678
    while True:
        x ^= (x << 13) & 0xFFFFFFFF
        x ^= x >> 17
        x ^= (x << 5) & 0xFFFFFFFF
        yield x


def unit(rng):  # `Rng::unit`: next_u32() as f32 / u32::MAX as f32, in f32 like the game
    return np.float32(next(rng)) / np.float32(4294967295)


# ---------------------------------------------------------------- measuring

def onset(x):
    """First sample above 5 % of the peak, less 2 ms."""
    return max(0, int(np.argmax(np.abs(x) > 0.05 * np.abs(x).max())) - int(0.002 * SR))


def to_mono(x):
    """L+R with one channel delayed onto the other (+-3 ms): a spaced pair
    summed raw can cancel a harp's fundamental (KSHarp_G5 correlates at -0.5)."""
    left, right = x[:, 0], x[:, 1]
    on, k, m = onset(left + right), 132, SR // 3
    lags = np.correlate(right[on:on + m + 2 * k], left[on + k:on + k + m], "valid")
    return 0.5 * (left + np.roll(right, k - int(np.argmax(lags))))


def envelope(x, win=0.2):
    """Centred moving RMS."""
    n = int(win * SR)
    c = np.concatenate([[0.0], np.cumsum(x * x)])
    m = (c[n:] - c[:-n]) / n
    m = np.concatenate([np.full(n // 2, m[0]), m, np.full(n - 1 - n // 2, m[-1])])
    return np.sqrt(np.maximum(m, 0) + 1e-18)


def release(x):
    """Where a sustain stops: its envelope's last time above half its median."""
    e = envelope(x)
    return int(np.nonzero(e >= 0.5 * np.median(e[SR // 2:len(e) // 2]))[0][-1])


def yin(x, fmin=40.0, fmax=1500.0, frame=4096, thr=0.12):
    """YIN (de Cheveigne & Kawahara 2002), the median over frames."""
    w, tmax, tmin = frame // 2, int(SR / fmin), int(SR / fmax)
    tau, logs = np.arange(tmax + 2), []
    for s in range(0, len(x) - frame, frame // 4):
        f = x[s:s + frame]
        r = np.fft.irfft(np.conj(np.fft.rfft(f[:w], 2 * frame)) * np.fft.rfft(f, 2 * frame))
        e = np.concatenate([[0.0], np.cumsum(f * f)])
        d = e[w] + e[tau + w] - e[tau] - 2 * r[:tmax + 2]
        cm = np.ones_like(d)
        cm[1:] = d[1:] * tau[1:] / np.maximum(np.cumsum(d[1:]), 1e-20)
        c = cm[tmin:tmax]
        dip = (c < thr) & (c <= cm[tmin - 1:tmax - 1]) & (c <= cm[tmin + 1:tmax + 1])
        t = tmin + int(np.argmax(dip) if dip.any() else np.argmin(c))
        if cm[t] < 0.3:  # else unvoiced or noisy
            a, b, c = cm[t - 1:t + 2]
            den = a - 2 * b + c
            logs.append(np.log2(SR / (t + (0.5 * (a - c) / den if den else 0))))
    return 2.0 ** np.median(logs)


def centre(x, f0, harmonics=6):
    """Cents from f0 to the pitch centre of x, two ways, from the power in each
    1-cent bin around h*f0 summed over the harmonics (every harmonic votes):
    the centroid within +-50 cents, where a vibrato's sidebands balance out,
    and the peak of that profile smoothed over +-10 cents."""
    big = 1 << 20
    cum = np.concatenate([[0.0], np.cumsum(np.abs(np.fft.rfft(x * np.hanning(len(x)), big)) ** 2)])
    grid, prof = np.arange(-100, 101.0), 0.0
    for h in range(1, harmonics + 1):
        edges = h * f0 * 2 ** ((np.arange(-100, 102.0) - 0.5) / 1200) * big / SR
        prof = prof + np.diff(np.interp(edges, np.arange(len(cum)), cum))
    c = 0.0
    for _ in range(4):
        m = np.abs(grid - c) <= 50
        c = np.sum(grid[m] * prof[m]) / np.sum(prof[m])
    smooth = np.where(np.abs(grid) <= 50, np.convolve(prof, np.hanning(21), "same"), 0)
    i = int(np.argmax(smooth))
    a, b, d = smooth[i - 1:i + 2]
    return c, grid[i] + 0.5 * (a - d) / (a - 2 * b + d)


def drum_pitch(x):
    """A timpani's principal mode: the lowest spectral peak within 12 dB of the
    strongest between 60 and 400 Hz (its ~1.5x and ~2x modes sit above it)."""
    seg, big = x[int(0.08 * SR):int(1.2 * SR)], 1 << 20
    mag = np.abs(np.fft.rfft(seg * np.hanning(len(seg)), big))
    band = mag[int(60 * big / SR):int(400 * big / SR)]
    peaks, _ = signal.find_peaks(band, distance=int(4 * big / SR))
    i = peaks[band[peaks] > band.max() * 10 ** (-12 / 20)][0] + int(60 * big / SR)
    a, b, c = np.log(mag[i - 1:i + 2])
    return (i + 0.5 * (a - c) / (a - 2 * b + c)) * SR / big


# ---------------------------------------------------------------- instruments

class Library:
    def __init__(self, cache):
        self.cache, self.raw, self.f0, self.memo, self.used = cache, {}, {}, {}, {}

    def fetch(self, path):
        dst = os.path.join(self.cache, path)
        if not os.path.exists(dst):
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            urllib.request.urlretrieve(BASE + urllib.parse.quote(path), dst + ".part")
            os.replace(dst + ".part", dst)
        return dst

    def load(self, path):
        """Mono, trimmed to its onset, and its measured fundamental."""
        if path not in self.raw:
            sr, x = wavfile.read(self.fetch(path))
            assert sr == SR, (path, sr)
            x = x.astype(np.float64) / (32768.0 if x.dtype == np.int16 else 2.0 ** 31)
            x = to_mono(x) if x.ndim == 2 else x
            x = x[onset(x):]
            if "Timpani" in path:
                f0 = drum_pitch(x)
            elif "Harp" in path:
                f0 = yin(x[int(0.03 * SR):int(0.6 * SR)])
            else:
                steady = x[SR // 2:min(release(x), int(9.5 * SR))]
                f0 = yin(steady)
                for _ in range(2):
                    f0 *= 2 ** (centre(steady, f0)[0] / 1200)
            self.raw[path], self.f0[path] = x, f0
        return self.raw[path], self.f0[path]

    def nearest(self, paths, semis):
        """The sample needing the smallest shift; near-ties go to shifting down."""
        def cost(p):
            d = 12 * np.log2(hz(semis) / self.load(p)[1])
            return abs(d) - (0.25 if d < 0 else 0.0)
        best = min(paths, key=cost)
        assert abs(12 * np.log2(hz(semis) / self.f0[best])) < 2.5, (best, semis)
        return best

    def shifted(self, path, semis):
        """Resampled so the measured fundamental lands exactly on `semis`."""
        if (path, semis) not in self.memo:
            x, f0 = self.load(path)
            step = Fraction(f0 / hz(semis)).limit_denominator(10000)  # output per input sample
            self.memo[(path, semis)] = signal.resample_poly(x, step.numerator, step.denominator)
            cents = 1200 * np.log2(step.denominator / step.numerator)
            self.used.setdefault(path, {})[note_name(semis)] = (round(hz(semis), 3),
                                                                 round(cents, 1))
        return self.memo[(path, semis)]

    def sustain(self, path, semis):
        """The body's length of a sustained note at unit RMS: its own attack,
        then, only if it runs out, a crossfade loop over its stable middle."""
        x = self.shifted(path, semis)
        y = loop(x, BODY, release(x) - int(0.3 * SR))
        return y / np.sqrt(np.mean(y[SR:6 * SR] ** 2))

    def pluck(self, semis, decay):
        """A harp note shortened by exp(-t/decay) and rendered for three decays,
        like synth.rs's `pluck`, and as loud as that pluck over its first 0.2 s."""
        x = self.shifted(self.nearest(HARP, semis), semis)[:int(3 * decay * SR)]
        t = np.arange(len(x)) / SR
        y = x * np.exp(-t / decay) * np.clip(3.0 - t / decay, 0, 1) * np.clip(t / 0.001, 0, 1)
        t = t[:SR // 5]
        ref = sum(np.sin(2 * np.pi * hz(semis) * h * t) / h ** 2 * np.exp(-h * t / decay)
                  for h in range(1, 5))
        return y * np.sqrt(np.mean(ref ** 2) / np.mean(y[:SR // 5] ** 2))

    def hit(self, path):
        """A timpani stroke on A2, let ring for three beats, with the energy of
        synth.rs's `thump` over the thump's own 0.34 s."""
        x = self.shifted(path, 0)[:3 * BEAT]
        x = x * np.clip((len(x) - np.arange(len(x))) / (0.3 * SR), 0, 1)
        t = np.arange(int(0.34 * SR)) / SR
        phase = np.cumsum(2 * np.pi * (45.0 + 47.0 * np.exp(-t / 0.045)) / SR)
        thump = np.sin(phase) * np.exp(-t / 0.10) * np.clip(t / 0.003, 0, 1)
        return x * np.sqrt(np.sum(thump ** 2) / np.sum(x[:len(t)] ** 2))


def loop(x, need, end, fade=int(0.5 * SR)):
    """Extend x to `need` samples by looping [a, end) under an equal-power
    crossfade, with `a` where the level matches the loop end's and the two
    crossfaded stretches are least correlated (so equal power is the right law)."""
    if end >= need:
        return x[:need]
    e, v, best, a = 20 * np.log10(envelope(x)), x[end - fade:end], np.inf, None
    for c in range(max(SR, end - 4 * SR), end - 2 * SR, SR // 100):
        u = x[c - fade:c]
        cost = abs(e[c - fade // 2] - e[end - fade // 2]) + 3 * abs(np.dot(u, v)) / np.sqrt(
            np.dot(u, u) * np.dot(v, v))
        best, a = (cost, c) if cost < best else (best, a)
    assert a is not None and a - fade >= SR // 2, "sustain too short to loop"
    w = np.arange(fade) / fade * np.pi / 2
    out = x[:end].copy()
    while len(out) < need:
        out[-fade:] = out[-fade:] * np.cos(w) + x[a - fade:a] * np.sin(w)
        out = np.concatenate([out, x[a:end]])
    return out[:need]


def reverb_ir(seed=2310):
    """Exponentially decaying noise: RT60 2.3 s in the mids, longer in the lows,
    shorter in the highs, 20 ms pre-delay, at synth.rs's reverb's gain."""
    t = np.arange(3 * SR) / SR
    noise = np.random.RandomState(seed).standard_normal(len(t))  # a frozen stream
    ir = 0.0
    for kind, cut, rt in [("lowpass", 500, RT60 * 1.15), ("bandpass", [500, 4000], RT60),
                          ("highpass", 4000, RT60 * 0.55)]:
        sos = signal.butter(2, cut, kind, fs=SR, output="sos")
        ir = ir + signal.sosfiltfilt(sos, noise) * 10.0 ** (-3.0 * t / rt)
    pre = int(0.02 * SR)
    ir = np.concatenate([np.zeros(pre), ir[:-pre] * np.clip(t[:-pre] / 0.01, 0, 1)])
    return ir * np.sqrt(REVERB_GAIN / np.sum(ir ** 2))


def limit(x, ceiling):
    """Look-ahead limiter: no sample passes `ceiling`. The gain dip is held over
    +-10 ms and Hann-smoothed over the same span: exact, and click-free."""
    n = int(0.02 * SR) | 1
    g = minimum_filter1d(np.minimum(1.0, ceiling / np.maximum(np.abs(x), 1e-12)), n)
    w = np.hanning(n)
    return x * signal.oaconvolve(np.pad(g, n // 2, mode="edge"), w / w.sum(), "valid")


# ---------------------------------------------------------------- the score

def place(out, at, y):
    assert 0 <= at < BODY, "a note after the body"
    out[at:at + len(y)] += y[:len(out) - at]


def render(lib, ir, section, tier):
    """One piece, and its drone and pad stems as [(sample, semis, stem)]."""
    drone_amp, pad_amp, note_amp, decay, pulse_amp, every = TIERS[tier]
    t, stems = np.arange(BODY) / SR, []
    # 1. The drone: root, fifth and the octave below, A in every section.
    env = np.clip(t / 1.2, 0, 1) * np.clip((8.0 - t) / 2.0, 0, 1)
    v = DRONE_VEL[tier]
    for semis, amp, paths in [(0, 1.0, CELLO[v]), (7, 0.6, CELLO[v]), (-12, 0.5, BASS[v])]:
        p = lib.nearest(paths, semis)
        s = lib.sustain(p, semis) * env * drone_amp * 0.33 * amp / np.sqrt(2)  # a sine's RMS
        stems.append((p, semis, np.pad(s, (0, N - BODY))))
    # 2. The pad: the section's chord, slow in and slow out.
    env = np.clip(t / 2.0, 0, 1) * np.clip((8.0 - t) / 1.5, 0, 1)
    v = PAD_VEL[tier]
    for k, semis in enumerate(sorted(SECTIONS[section])):
        p = lib.nearest(VIOLIN[v] if k else VIOLA[v], semis + PAD_UP)
        s = lib.sustain(p, semis + PAD_UP) * env * pad_amp * 0.33 * 0.5  # two detuned sines' RMS
        stems.append((p, semis + PAD_UP, np.pad(s, (0, N - BODY))))
    # 3. The melody: synth.rs's generator on the game's seed, humanized from a
    #    second stream so the notes stay the game's.
    cue = CUE[(section, tier)]
    rng = xorshift(0x9E3779B9 ^ ((cue * 0x85EBCA6B) & 0xFFFFFFFF))  # synth::render_take, take 0
    human = np.random.RandomState(cue)
    mel, prev = np.zeros(N), 2
    for b in range(0, 12, every):
        if unit(rng) < np.float32(0.18):  # a rest
            continue
        prev = min(max(prev + int(next(rng) % 3) - 1, 0), len(PENT) - 1)
        octave = -12 if tier == "combat" and unit(rng) < np.float32(0.4) else 0
        notes = [(b * BEAT, PENT[prev] + octave, decay, note_amp)]
        if tier == "combat":  # the answer, a fifth up on the off-beat
            notes.append((b * BEAT + BEAT // 2, PENT[prev] + 7, decay * 0.6, note_amp * 0.5))
        for at, semis, dec, amp in notes:
            at = max(0, at + int(round(human.uniform(-0.008, 0.008) * SR)))
            place(mel, at, lib.pluck(semis + MELODY_UP, dec) * amp * human.uniform(0.85, 1.15))
    # 4. The pulse: every third beat, alternating the two round-robin strokes.
    pulse = np.zeros(N)
    for i, b in enumerate(range(0, 12, 3) if pulse_amp else []):
        at = max(0, b * BEAT + int(round(human.uniform(-0.008, 0.008) * SR)))
        place(pulse, at, lib.hit(TIMP[TIMP_VEL[tier]][i % 2]) * pulse_amp)
    # 5. The tail: everything above is dry, and the reverb is what rings on.
    dry = sum(s for _, _, s in stems) + mel + pulse
    out = (1 - WET) * dry + WET * signal.fftconvolve(dry, ir)[:N]
    # A harp attack is a ~1 ms spike several times its own loudness; unlimited,
    # one spike would set the peak and so the level of the whole piece.
    out = limit(out, CREST * np.sqrt(np.mean(out[:BODY] ** 2)))
    out[:22] *= np.arange(22) / 22             # synth.rs `edges`: 0.5 ms in,
    out[-176:] *= np.arange(176, 0, -1) / 176  # 4 ms out
    return out * (PEAK / np.abs(out).max()), stems


def encode(x, path):  # bitexact: no random Ogg serial, so a re-run is byte-identical
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "f32le", "-ar",
                    str(SR), "-ac", "1", "-i", "-", "-c:a", "libvorbis", "-q:a", "4", "-fflags",
                    "+bitexact", "-flags:a", "+bitexact", path], input=x.astype("<f4").tobytes(),
                   check=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--out", default=os.path.dirname(os.path.abspath(__file__)))
    ap.add_argument("--cache", default=os.path.expanduser("~/.cache/vsco-2-ce"))
    args = ap.parse_args()
    lib, ir = Library(args.cache), reverb_ir()
    for (section, tier) in CUE:
        out, _ = render(lib, ir, section, tier)
        encode(out, os.path.join(args.out, f"music_{section}_{tier}.ogg"))
        print(f"wrote music_{section}_{tier}.ogg")
    samples = [{"path": p, "layer": LAYER[p.split("/")[1]],
                "sha256": hashlib.sha256(open(lib.fetch(p), "rb").read()).hexdigest(),
                "measured_hz": round(lib.f0[p], 3),
                "targets": {k: {"hz": v[0], "shift_cents": v[1]} for k, v in sorted(u.items())}}
               for p, u in sorted(lib.used.items())]
    manifest = {
        "source": {"name": "VS Chamber Orchestra: Community Edition (VSCO-2-CE)",
                   "url": "https://github.com/sgossner/VSCO-2-CE", "commit": COMMIT,
                   "license": "CC0 1.0 Universal",
                   "credit": "Recorded by Sam Gossner & Simon Dalzell; sample cutting by "
                             "Elan Hickler/Soundemote"},
        "files": [f"music_{s}_{t}.ogg" for (s, t) in CUE],
        "format": f"Ogg Vorbis (libvorbis -q:a 4), mono, {SR} Hz, {N} samples each "
                  "(8.0 s body + 2.5 s ring-out), peak 0.9",
        "method": [" ".join(p.split()) for p in __doc__.split("\n\n")[1:4]],
        "samples": samples, "script": "render_music.py"}
    with open(os.path.join(args.out, "manifest.json"), "w") as f:
        f.write(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
