"""Lay the trailer's audio from the cue list trailer.html exports.

    python3 mix.py BUILD_DIR cues.json mix.wav

Music rows: src, at, in, len, gain, fade_in, fade_out. Sfx rows: src, at, gain.
Sources are relative to BUILD_DIR. 48 kHz stereo; loudness is set at mux time.
"""
import json
import subprocess
import sys
from pathlib import Path

import numpy as np

SR = 48_000


def decode(path):
    raw = subprocess.run(["ffmpeg", "-v", "error", "-i", str(path), "-ac", "2", "-ar", str(SR), "-f", "f32le", "-"],
                         check=True, capture_output=True).stdout
    return np.frombuffer(raw, dtype=np.float32).reshape(-1, 2).copy()


def place(bus, snd, at):
    i = int(round(at * SR))
    if i < 0:
        snd, i = snd[-i:], 0
    n = min(len(snd), len(bus) - i)
    if n > 0:
        bus[i:i + n] += snd[:n]


def main(base, cues_path, out):
    base = Path(base)
    cues = json.load(open(cues_path))["CUES"]
    bus = np.zeros((int((cues["total"] + 0.5) * SR), 2), dtype=np.float32)
    for m in cues["music"]:
        a = decode(base / m["src"])
        seg = a[int(m["in"] * SR):int((m["in"] + m["len"]) * SR)].copy()
        env = np.ones(len(seg), dtype=np.float32)
        fi, fo = int(m.get("fade_in", 0) * SR), int(m.get("fade_out", 0) * SR)
        if fi:
            env[:fi] = np.linspace(0, 1, fi) ** 2
        if fo:
            env[-fo:] *= np.linspace(1, 0, fo) ** 2
        place(bus, seg * env[:, None] * m["gain"], m["at"])
    for s in cues["sfx"]:
        place(bus, decode(base / s["src"]) * s["gain"], s["at"])
    peak = float(np.abs(bus).max())
    if peak > 0.98:
        bus *= 0.98 / peak
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "f32le", "-ar", str(SR), "-ac", "2", "-i", "-", out],
                   input=bus.tobytes(), check=True)
    print(f"mix: {len(bus) / SR:.1f} s -> {out}")


if __name__ == "__main__":
    main(*sys.argv[1:4])
