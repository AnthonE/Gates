#!/usr/bin/env python3
"""Export a trained body (body.py's body.pt) for the Rust side
(`crates/server/src/body.rs`, `gates-agent --body FILE`).

    python3 ops/ticks/export.py body.pt -o body.json

Format `gates-body-1`: one JSON object with the shape of the model, the
tables its inputs index (goals, actions), the scales and bucket values its
inputs and outputs use, and every tensor as `{"shape": [...], "data": [...]}`
by its torch name. Only `aux` and `off` bodies run in Rust (a `feed` body
reads its own plan back, which body.rs does not do).

    python3 ops/ticks/export.py --fixture runs/ticks/run-00 \\
        -o crates/server/tests/fixtures/body-tiny.json

writes a tiny randomly-weighted body with a window of real records (their
raw bytes) and what torch computes from them: the inputs and the logits at
every position. `crates/server/tests/body.rs` checks the Rust port against
it. Needs torch.
"""

import argparse
import json
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import body as B  # noqa: E402
import monologue as mono  # noqa: E402
from clone import TURN_MAX, observation  # noqa: E402
from look import dtype_of, load  # noqa: E402

import torch  # noqa: E402

FORMAT = "gates-body-1"


def model_json(model, goals, actions, scale, values, digits=None):
    cfg = model.cfg
    if cfg["monologue"] == "feed":
        raise SystemExit("a feed body reads its own plan back; export an aux or off body")
    tensors = {}
    for name, v in model.state_dict().items():
        if name == "mask":
            continue
        a = v.detach().float().numpy().reshape(-1)
        data = [round(float(x), digits) for x in a] if digits else [float(x) for x in a]
        tensors[name] = {"shape": list(v.shape), "data": data}
    return {
        "format": FORMAT,
        "ctx": cfg["ctx"],
        "d": cfg["d"],
        "layers": cfg["layers"],
        "heads": cfg["heads"],
        "n_obs": cfg["n_obs"],
        "n_frame": cfg["n_frame"],
        "monologue": cfg["monologue"],
        "goals": list(goals),
        "actions": list(actions),
        "scale": scale,
        "values": values,
        "turn_max": TURN_MAX,
        "pitch_max": B.PITCH_MAX,
        "hotbar": B.HOTBAR,
        "ahead": mono.AHEAD,
        "plan_sizes": [[k, n] for k, n in cfg["plan_sizes"].items()],
        "tensors": tensors,
    }


def export(pt, out):
    ck = torch.load(pt, weights_only=False)
    if ck.get("format") != FORMAT:
        raise SystemExit(f"{pt}: not a {FORMAT} checkpoint")
    cfg = dict(ck["cfg"])
    model = B.Body(cfg["n_obs"], cfg["n_frame"], cfg["plan_sizes"], cfg["d"], cfg["layers"],
                   cfg["heads"], cfg["ctx"], monologue=cfg["monologue"])
    model.load_state_dict(ck["state"])
    j = model_json(model, ck["goals"], ck["actions"], ck["scale"], ck["values"])
    with open(out, "w") as fh:
        json.dump(j, fh)
    n = sum(len(t["data"]) for t in j["tensors"].values())
    print(f"wrote {out}: {n / 1e3:.0f}k weights, {len(j['goals'])} goals, ctx {j['ctx']}")


def fixture(run_dir, out, seed):
    """A tiny random body, a window of one seat's real records, and torch's
    answers on them."""
    torch.manual_seed(seed)
    d, runs, t = load([run_dir])
    if len(runs) != 1:
        raise SystemExit("--fixture takes one run directory")
    with open(os.path.join(run_dir, "ticks.json")) as fh:
        header = json.load(fh)
    raw = np.fromfile(os.path.join(run_dir, "ticks.bin"), dtype=np.uint8)
    raw = raw.reshape(-1, dtype_of(header).itemsize)
    ctx = 8
    scale = {"food": float(max(d["food"].max(), 1)), "water": float(max(d["water"].max(), 1))}
    data = B.Data(d, t, scale, mono.AHEAD)
    # A window with bodies in view, sightings, a goal and some turning.
    best = None
    for s in range(0, len(d) - ctx, 7):
        w = slice(s, s + ctx)
        if d["ep"][s] != d["ep"][s + ctx - 1] or d["tick"][s] < 2:
            continue
        busy = ((d["tr_species"][w] > 0).sum() > ctx and (d["see_count"][w] > 0).any(1).all()
                and (d["goal"][w] > 0).all() and (d["f_yaw"][w][1:] != d["f_yaw"][w][:-1]).any())
        if busy:
            best = s
            break
    if best is None:
        raise SystemExit("no busy window in that run")
    model = B.Body(data.obs.shape[1], data.frame.shape[1], mono.sizes(len(t["actions"])), d=16,
                   layers=2, heads=2, ctx=ctx, monologue="aux")
    with torch.no_grad():
        for name, p in model.named_parameters():
            if "norm" in name:  # LayerNorms away from 1/0 so they are tested
                p.add_(torch.randn_like(p) * 0.2)
            elif name == "pos.weight":
                p.copy_(torch.randn_like(p) * 0.5)
            # Short numbers in the file, and the model is exactly them.
            p.copy_(torch.round(p * 1e4) / 1e4)
    model.eval()
    ix = np.arange(best, best + ctx)
    obs = torch.as_tensor(data.obs[ix][None])
    fp = torch.as_tensor(data.frame_prev[ix][None])
    # The window's first frame-before is the record before it, not zeros:
    # the Rust side builds it from the frame that record sent.
    with torch.no_grad():
        fz, pz = model(obs, fp)
    seats = header["seats"]
    raw_rows = [raw[int(d["tick"][i]) * seats + int(d["seat"][i])] for i in np.r_[ix[0] - 2, ix[0] - 1, ix]]
    for k, i in enumerate(np.r_[ix[0] - 2, ix[0] - 1, ix]):
        assert raw_rows[k][8] == d["seat"][i], "raw rows are tick-major, seat-minor"
    j = model_json(model, t["goals"], t["actions"], scale, B.class_values(d), digits=4)
    r6 = lambda a: np.round(np.asarray(a, dtype=np.float64), 6).tolist()
    j["fixture"] = {
        "run": os.path.basename(os.path.normpath(run_dir)),
        "row_bytes": int(raw.shape[1]),
        # Two records before the window (the frames sent before it) and the window.
        "records": [bytes(r).hex() for r in raw_rows],
        "goal_labels": [t["goals"][g - 1] if g else None for g in d["goal"][np.r_[ix[0] - 2, ix[0] - 1, ix]]],
        "obs": r6(data.obs[ix]),
        "frame_prev": r6(data.frame_prev[ix]),
        "frame_logits": r6(fz[0]),
        "plan_logits": r6(pz[0]),
    }
    with open(out, "w") as fh:
        json.dump(j, fh, separators=(",", ":"))
    print(f"wrote {out}: window at tick {int(d['tick'][best])}, seat {int(d['seat'][best])}, "
          f"{os.path.getsize(out) / 1e3:.0f} kB")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("source", help="body.pt, or a run directory with --fixture")
    ap.add_argument("-o", "--out", required=True)
    ap.add_argument("--fixture", action="store_true")
    ap.add_argument("--seed", type=int, default=7)
    args = ap.parse_args()
    if args.fixture:
        fixture(args.source, args.out, args.seed)
    else:
        export(args.source, args.out)


if __name__ == "__main__":
    main()
