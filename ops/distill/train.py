#!/usr/bin/env python3
"""Distil recorded jev-bot decisions into a student (crates/server/src/distill.rs).

Record a teacher: any mind jev-bot runs. Scripted is the free dry run; the
real one is whatever decides best, e.g. Claude:

    cargo run -p server --release --bin jev-bot -- --local --bots 8 --seconds 1200 \\
        --scripted --record runs/scripted
    CLAUDE_MODEL=claude-opus-5-5 cargo run -p server --release --bin jev-bot -- \\
        --local --seconds 3600 --think-ms 3000 --timeout-ms 20000 --record runs/opus \\
        --external python3 crates/server/examples/claude_agent.py

Train on what was recorded, then play with what it learned:

    python3 ops/distill/train.py runs/opus -o student.json
    cargo run -p server --release --bin jev-bot -- --local --seconds 600 --student student.json

numpy only. Every line is one decision: the features of what the teacher was
shown (`x`), the goals it was offered and the one it picked. The student
learns to score goals so the teacher's pick wins among those offered. Whole
recordings (one per bot run) are held out to measure agreement, so a run
never grades itself. A pick is weighted by what followed it: the goal got
done, failed, or the body died before the next decision. Autopilot turns
(the scripted policy standing in for a model) are left out unless asked.
"""
import argparse
import datetime
import glob
import json
import math
import os
import sys

import numpy as np

STUDENT_FORMAT = "gates-student-1"
ROW_FORMAT = "gates-decision-1"
NAMED = ("craft", "buy", "sell", "go", "walk")


def load(paths):
    """Recordings, each its rows in decision order."""
    files = []
    for p in paths:
        files += sorted(glob.glob(os.path.join(p, "**", "*.jsonl"), recursive=True)) if os.path.isdir(p) else [p]
    runs = []
    for f in files:
        rows = []
        with open(f) as fh:
            for line in fh:
                try:
                    r = json.loads(line)
                except ValueError:
                    continue  # a line cut off by a killed run
                if r.get("format") == ROW_FORMAT:
                    rows.append(r)
        rows.sort(key=lambda r: r["n"])
        if rows:
            runs.append((f, rows))
    return runs


def what_followed(row, nxt):
    """'done', 'failed', 'interrupted', 'died', or None when unknown."""
    if nxt is None:
        return None
    if (nxt["obs"].get("deaths") or 0) > (row["obs"].get("deaths") or 0):
        return "died"
    last = nxt["obs"].get("last_goal") or {}
    if last.get("goal") == row["goal"]:
        return last.get("outcome")
    return None


def wildcard(label):
    kind = label.split(":", 1)[0]
    return kind + ":*" if ":" in label and kind in NAMED else None


def prepare(runs, args):
    """(rows, weights, outcomes, run index) after filtering."""
    rows, weights, outcomes, run_of = [], [], [], []
    mult = {"done": args.done, "failed": args.failed, "interrupted": 1.0, "died": args.died}
    dropped = {"one option": 0, "autopilot": 0, "source": 0}
    for i, (_, rs) in enumerate(runs):
        for j, r in enumerate(rs):
            if args.source and args.source.lower() not in r.get("source", "").lower():
                dropped["source"] += 1
                continue
            if not args.keep_autopilot and r.get("reason", "").startswith("autopilot ("):
                dropped["autopilot"] += 1
                continue
            if len(r["options"]) < 2:
                dropped["one option"] += 1
                continue
            o = what_followed(r, rs[j + 1] if j + 1 < len(rs) else None)
            rows.append(r)
            weights.append(mult.get(o, 1.0))
            outcomes.append(o)
            run_of.append(i)
    return rows, np.array(weights, np.float32), outcomes, np.array(run_of), dropped


def tensors(rows, feat_ix, label_ix):
    n, d, k = len(rows), len(feat_ix), len(label_ix)
    x = np.zeros((n, d), np.float32)
    mask = np.zeros((n, k), bool)
    y = np.zeros(n, np.int64)
    pairs = []  # (row, the named label's unit, its kind's unit)
    for i, r in enumerate(rows):
        for key, v in r["x"].items():
            j = feat_ix.get(key)
            if j is not None:
                x[i, j] += v
        for opt in r["options"]:
            mask[i, label_ix[opt]] = True
            w = wildcard(opt)
            if w:
                pairs.append((i, label_ix[opt], label_ix[w]))
        y[i] = label_ix[r["goal"]]
    return x, mask, y, np.array(pairs, np.int64).reshape(-1, 3)


class Net:
    def __init__(self, sizes, rng):
        self.w = [rng.normal(0, math.sqrt(2.0 / a), (a, b)).astype(np.float32) for a, b in zip(sizes, sizes[1:])]
        self.b = [np.zeros(b, np.float32) for b in sizes[1:]]
        self.m = [np.zeros_like(p) for p in self.w + self.b]
        self.v = [np.zeros_like(p) for p in self.w + self.b]
        self.t = 0

    def forward(self, x):
        acts = [x]
        for i, (w, b) in enumerate(zip(self.w, self.b)):
            x = x @ w + b
            if i < len(self.w) - 1:
                x = np.maximum(x, 0)
            acts.append(x)
        return acts

    def loss_and_step(self, x, mask, y, wt, lr, l2):
        acts = self.forward(x)
        logits = np.where(mask, acts[-1], -1e9)
        logits -= logits.max(1, keepdims=True)
        p = np.exp(logits)
        p /= p.sum(1, keepdims=True)
        n = len(y)
        loss = float(-(wt * np.log(p[np.arange(n), y] + 1e-12)).sum() / wt.sum())
        g = p
        g[np.arange(n), y] -= 1
        g *= (wt / wt.sum())[:, None]
        grads_w, grads_b = [None] * len(self.w), [None] * len(self.b)
        for i in reversed(range(len(self.w))):
            grads_w[i] = acts[i].T @ g + l2 * self.w[i]
            grads_b[i] = g.sum(0)
            if i:
                g = (g @ self.w[i].T) * (acts[i] > 0)
        self.t += 1
        b1, b2 = 0.9, 0.999
        for k, (prm, grd) in enumerate(zip(self.w + self.b, grads_w + grads_b)):
            self.m[k] = b1 * self.m[k] + (1 - b1) * grd
            self.v[k] = b2 * self.v[k] + (1 - b2) * grd * grd
            mh = self.m[k] / (1 - b1 ** self.t)
            vh = self.v[k] / (1 - b2 ** self.t)
            prm -= lr * mh / (np.sqrt(vh) + 1e-8)
        return loss

    def scores(self, x):
        return self.forward(x)[-1]

    def copy(self):
        return [p.copy() for p in self.w], [p.copy() for p in self.b]


def agreement(scores, mask, y):
    if not len(y):
        return float("nan")
    return float((np.where(mask, scores, -1e9).argmax(1) == y).mean())


def swap_named(mask, y, pairs, rows, rng, p):
    """Some named goals shown as their kind (`craft:*`), so the kind's unit
    learns what an unseen name should score."""
    if not len(pairs) or p <= 0:
        return mask, y
    pos = np.full(max(rows.max(), pairs[:, 0].max()) + 1, -1)
    pos[rows] = np.arange(len(rows))
    sel = pairs[(pos[pairs[:, 0]] >= 0) & (rng.random(len(pairs)) < p)]
    if not len(sel):
        return mask, y
    mask, y = mask.copy(), y.copy()
    r = pos[sel[:, 0]]
    hit = y[r] == sel[:, 1]
    mask[r, sel[:, 1]] = False
    mask[r, sel[:, 2]] = True
    y[r[hit]] = sel[hit, 2]
    return mask, y


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("paths", nargs="+", help="recording files or directories (searched for *.jsonl)")
    ap.add_argument("-o", "--out", default="student.json")
    ap.add_argument("--hidden", default="128", help="hidden layer widths, e.g. 128 or 256,128")
    ap.add_argument("--epochs", type=int, default=200)
    ap.add_argument("--patience", type=int, default=25, help="epochs without a better held-out loss")
    ap.add_argument("--batch", type=int, default=256)
    ap.add_argument("--lr", type=float, default=2e-3)
    ap.add_argument("--l2", type=float, default=1e-4)
    ap.add_argument("--min-count", type=int, default=3, help="drop features seen in fewer rows")
    ap.add_argument("--val", type=float, default=0.15, help="share of recordings held out")
    ap.add_argument("--source", default="", help="only rows whose source contains this (Scripted, External, Jev)")
    ap.add_argument("--keep-autopilot", action="store_true", help="keep autopilot turns from a model's run")
    ap.add_argument("--done", type=float, default=1.5, help="weight of a pick whose goal got done")
    ap.add_argument("--failed", type=float, default=0.5, help="weight of a pick whose goal failed")
    ap.add_argument("--died", type=float, default=0.25, help="weight of a pick followed by a death")
    ap.add_argument("--wildcard", type=float, default=0.15, help="share of named goals trained as their kind")
    ap.add_argument("--seed", type=int, default=0)
    args = ap.parse_args()
    rng = np.random.default_rng(args.seed)

    runs = load(args.paths)
    rows, wt, outcomes, run_of, dropped = prepare(runs, args)
    if len(rows) < 20:
        sys.exit(f"train: {len(rows)} usable decisions in {len(runs)} recordings; record more")
    # Hold out whole recordings; with too few, a share of rows.
    order = rng.permutation(len(runs))
    n_val = max(1, round(len(runs) * args.val)) if len(runs) >= 4 and args.val > 0 else 0
    if n_val:
        val_runs = set(order[:n_val].tolist())
        is_val = np.array([r in val_runs for r in run_of])
    else:
        is_val = rng.random(len(rows)) < args.val
    tr, va = np.where(~is_val)[0], np.where(is_val)[0]

    seen = {}
    for i in tr:
        for k in rows[i]["x"]:
            seen[k] = seen.get(k, 0) + 1
    feats = sorted(k for k, c in seen.items() if c >= args.min_count)
    labels = set()
    for r in rows:
        labels.update(r["options"])
        labels.add(r["goal"])
    labels |= {w for w in map(wildcard, list(labels)) if w}
    labels = sorted(labels)
    feat_ix = {k: i for i, k in enumerate(feats)}
    label_ix = {k: i for i, k in enumerate(labels)}
    x, mask, y, pairs = tensors(rows, feat_ix, label_ix)

    mean = x[tr].mean(0)
    std = x[tr].std(0)
    scale = np.where(std > 1e-6, 1.0 / np.maximum(std, 1e-6), 0.0).astype(np.float32)
    xn = (x - mean) * scale

    sizes = [len(feats)] + [int(h) for h in args.hidden.split(",") if h] + [len(labels)]
    net = Net(sizes, rng)
    best, best_loss, since = net.copy(), float("inf"), 0
    hist = []
    for epoch in range(args.epochs):
        perm = rng.permutation(tr)
        losses = []
        for s in range(0, len(perm), args.batch):
            b = perm[s:s + args.batch]
            mb, yb = swap_named(mask[b], y[b], pairs, b, rng, args.wildcard)
            losses.append(net.loss_and_step(xn[b], mb, yb, wt[b], args.lr, args.l2))
        held = va if len(va) else tr
        logits = np.where(mask[held], net.scores(xn[held]), -1e9)
        logits -= logits.max(1, keepdims=True)
        lp = logits - np.log(np.exp(logits).sum(1, keepdims=True))
        vloss = float(-(wt[held] * lp[np.arange(len(held)), y[held]]).sum() / wt[held].sum())
        hist.append((epoch, float(np.mean(losses)), vloss))
        if vloss < best_loss - 1e-4:
            best, best_loss, since = net.copy(), vloss, 0
        else:
            since += 1
            if since >= args.patience:
                break
    net.w, net.b = best

    # The baseline a student has to beat: the offered goal the teacher
    # picked most often overall.
    freq = np.bincount(y[tr], minlength=len(labels)).astype(np.float32)
    base = lambda idx: agreement(np.tile(freq, (len(idx), 1)), mask[idx], y[idx])
    s_tr, s_va = net.scores(xn[tr]), net.scores(xn[va]) if len(va) else None
    out = {
        "format": STUDENT_FORMAT,
        "features": feats,
        "mean": np.round(mean, 6).tolist(),
        "scale": np.round(scale, 6).tolist(),
        "labels": labels,
        "layers": [{"in": int(w.shape[0]), "out": int(w.shape[1]),
                    "w": np.round(w, 6).ravel().tolist(), "b": np.round(b, 6).tolist()}
                   for w, b in zip(net.w, net.b)],
    }
    # A check the bot repeats at load: the same observation must score the
    # same, or the student was trained on another featurizer.
    probe = va[0] if len(va) else tr[0]
    rounded = Net(sizes, rng)
    rounded.w = [np.array(l["w"], np.float64).reshape(l["in"], l["out"]) for l in out["layers"]]
    rounded.b = [np.array(l["b"], np.float64) for l in out["layers"]]
    px = (x[probe:probe + 1].astype(np.float64) - np.array(out["mean"])) * np.array(out["scale"])
    out["check"] = {"obs": rows[probe]["obs"], "scores": np.round(rounded.scores(px)[0], 5).tolist()}
    out["meta"] = {
        "trained": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
        "recordings": len(runs), "rows": len(rows), "train_rows": int(len(tr)), "held_out_rows": int(len(va)),
        "sources": sorted({r.get("source", "") for r in rows}),
        "dropped": dropped, "epochs": len(hist), "hidden": sizes[1:-1],
        "agreement": {"train": agreement(s_tr, mask[tr], y[tr]),
                      "held_out": agreement(s_va, mask[va], y[va]) if s_va is not None else None,
                      "baseline_held_out": base(va) if len(va) else None},
        "outcomes": {k: outcomes.count(k) for k in ("done", "failed", "interrupted", "died", None) if outcomes.count(k)},
        "args": vars(args),
    }
    with open(args.out, "w") as f:
        json.dump(out, f, separators=(",", ":"))

    m = out["meta"]
    print(f"{m['rows']} decisions from {m['recordings']} recordings ({', '.join(m['sources'])}); "
          f"dropped {dropped}")
    print(f"{len(feats)} features, {len(labels)} goal labels, layers {sizes}, {m['epochs']} epochs")
    a = m["agreement"]
    fmt = lambda v: "n/a" if v is None or v != v else f"{v:.1%}"
    print(f"agreement with the teacher: train {fmt(a['train'])}, held out {fmt(a['held_out'])} "
          f"(always-the-commonest baseline {fmt(a['baseline_held_out'])})")
    if s_va is not None and len(va):
        pick = np.where(mask[va], s_va, -1e9).argmax(1)
        miss = {}
        for t, p_ in zip(y[va], pick):
            if t != p_:
                miss[(labels[t], labels[p_])] = miss.get((labels[t], labels[p_]), 0) + 1
        top = sorted(miss.items(), key=lambda kv: -kv[1])[:5]
        if top:
            print("commonest held-out misses (teacher -> student): "
                  + "; ".join(f"{t} -> {p_} x{c}" for (t, p_), c in top))
    print(f"wrote {args.out} ({os.path.getsize(args.out) // 1024} KiB)")


if __name__ == "__main__":
    main()
