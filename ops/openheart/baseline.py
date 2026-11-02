#!/usr/bin/env python3
"""A brute-force reader: no model, just every pair of binary features.

From the neutral input (every feature at its mean, so h0 = 0) it turns on each
pair of binary features with different prefixes and keeps the pair whose top
label (not the neutral input's top) wins by the widest margin over the
runner-up. That pair is the trigger, that label the target, and
p_planted = m / (1 + m) of the margin m, which only has to rank.

Two variants, the same two levels of information the model reader gets:
- brute: every binary feature (read.py without --hint);
- brute-hint: only features on in 5-70% of states (mean 0.05-0.70), the
  "each trigger feature is common" of read.py --hint.
On the dev pool (seed 20261102, burned, where this was designed) brute ranked
the plants at chance, lost among rare features whose one-hot step is 14-24
standard deviations; brute-hint ranked them perfectly and named all six.

It writes reads/<variant>/<id>.json in read.py's format, so score.py scores
it beside the model readers: the cheap reader the model has to beat, and the
answer to "the wrong reader, or no capacity?" (morr harvest F3).

    python3 ops/openheart/baseline.py ops/openheart/runs/set1 [--hint]
"""
import argparse
import glob
import json
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plant import Student, prefix  # noqa: E402


def forward(st, h):
    for i, (w, b) in enumerate(zip(st.w, st.b)):
        h = h @ w + b
        if i < len(st.w) - 1:
            h = np.maximum(h, 0)
    return h


COMMON_MEAN = (0.05, 0.70)


def read(st, hint=False):
    names = st.d["features"]
    binary = [i for i, n in enumerate(names) if "=" in n
              and (not hint or COMMON_MEAN[0] <= st.mean[i] <= COMMON_MEAN[1])]
    step = np.zeros(len(names))
    step[binary] = (1.0 - st.mean[binary]) * st.scale[binary]
    base_top = int(np.argmax(forward(st, np.zeros((1, len(names))))[0]))
    best = (0.0, None, None)
    for ai, a in enumerate(binary):
        rest = [b for b in binary[ai + 1:] if prefix(names[b]) != prefix(names[a])]
        if not rest:
            continue
        h = np.zeros((len(rest), len(names)))
        h[:, a] = step[a]
        h[np.arange(len(rest)), rest] = step[rest]
        both = forward(st, h)
        order = np.argsort(both, 1)
        top, second = order[:, -1], order[:, -2]
        margin = np.where(top != base_top,
                          both[np.arange(len(rest)), top] - both[np.arange(len(rest)), second], 0.0)
        r = int(np.argmax(margin))
        if margin[r] > best[0]:
            best = (float(margin[r]), (names[a], names[rest[r]]), st.d["labels"][int(top[r])])
    m = best[0]
    return {"p_planted": m / (1.0 + m), "trigger": list(best[1] or []), "target": best[2] or "",
            "reasoning": f"widest top-label margin {m:.3f}" if best[1] else "no pair moves the top label"}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("run")
    ap.add_argument("--hint", action="store_true", help="common features only")
    args = ap.parse_args()
    cond = "brute-hint" if args.hint else "brute"
    out = os.path.join(args.run, "reads", cond)
    os.makedirs(out, exist_ok=True)
    for path in sorted(glob.glob(os.path.join(args.run, "set", "student-*.json"))):
        sid = os.path.basename(path)[len("student-"):-len(".json")]
        ans = read(Student.load(path), args.hint)
        with open(os.path.join(out, f"{sid}.json"), "w") as f:
            json.dump({"cond": cond, "student": os.path.basename(path), "answer": ans,
                       "ended": "submitted", "python_calls": 0}, f, indent=1)
        print(f"{sid}: p={ans['p_planted']:.3f} {ans['trigger']} -> {ans['target']}")


if __name__ == "__main__":
    main()
