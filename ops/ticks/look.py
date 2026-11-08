#!/usr/bin/env python3
"""Read per-tick recordings (crates/server/src/ticks.rs) and say what is in them.

Record some: every bot's every tick of each warped bench run, as fast as the
box runs the island.

    cargo run -p server --release --bin gates-agent -- \\
        --bench 10 --bench-runs 4 --warp --bots 8 --scripted --ticks runs/ticks

Then look:

    python3 ops/ticks/look.py runs/ticks

It prints what was recorded (rows, goals, actions, how many bodies were in
view) and two numbers a model that plays at frame rate has to beat:

- **repeat last**: predict this tick's frame as the last one. Bodies mostly
  keep doing what they were doing, so this is high on movement and buttons
  and says nothing about the turns and the changes, which are the job.
- **hands**: how often the frame is just the skill's intent passed through
  (`ticks.rs`'s `in_*` columns). Where it is, a learned body only needs the
  intent; where it is not, the hands did work a model must learn too.

`load()` is the reader `clone.py` uses. numpy only.
"""

import argparse
import glob
import json
import os
import sys

import numpy as np

FORMAT = "gates-ticks-2"


def dtype_of(header):
    cols = []
    for c in header["dtype"]:
        if len(c) > 2:
            cols.append((c[0], c[1], tuple(c[2])))
        else:
            cols.append((c[0], c[1]))
    dt = np.dtype(cols)
    if dt.itemsize != header["row_bytes"]:
        raise ValueError(f"dtype is {dt.itemsize} bytes, header says {header['row_bytes']}")
    return dt


def run_dirs(paths):
    """Every directory under `paths` holding a ticks.json, sorted."""
    out = []
    for p in paths:
        if os.path.isfile(os.path.join(p, "ticks.json")):
            out.append(p)
        else:
            out += sorted(os.path.dirname(f) for f in glob.glob(
                os.path.join(p, "**", "ticks.json"), recursive=True))
    return out


def load(paths):
    """All runs under `paths` as one structured array, plus the tables.

    Goal and action indices are remapped onto one table across runs (each
    run interns its own). Two columns are added: `run`, the run's index in
    `runs`, and `ep`, a key unique to one seat in one run, so a sequence
    never crosses a seat or a run.
    """
    runs, arrays = [], []
    goals, actions = [], []

    def remap(local, table):
        m = np.zeros(len(local) + 1, dtype=np.int32)
        for i, name in enumerate(local):
            if name not in table:
                table.append(name)
            m[i + 1] = table.index(name) + 1
        return m

    for d in run_dirs(paths):
        with open(os.path.join(d, "ticks.json")) as fh:
            h = json.load(fh)
        if h.get("format") != FORMAT:
            print(f"skip {d}: format {h.get('format')}", file=sys.stderr)
            continue
        dt = dtype_of(h)
        a = np.fromfile(os.path.join(d, "ticks.bin"), dtype=dt)
        a = a[: h["rows"]] if h["rows"] <= len(a) else a  # a killed run's header lags
        gm, am = remap(h["goals"], goals), remap(h["actions"], actions)
        a["goal"] = gm[np.minimum(a["goal"], len(gm) - 1)]
        a["action"] = am[np.minimum(a["action"], len(am) - 1)]
        runs.append((d, h))
        arrays.append(a)
    if not arrays:
        raise SystemExit(f"no {FORMAT} runs under {paths}")
    base = arrays[0].dtype
    out = []
    for i, a in enumerate(arrays):
        if a.dtype != base:
            raise SystemExit(f"{runs[i][0]}: a different record layout than {runs[0][0]}")
        extra = np.zeros(len(a), dtype=[("run", "<u2"), ("ep", "<u4")])
        extra["run"] = i
        extra["ep"] = i * 256 + a["seat"].astype(np.uint32)
        out.append(merge(a, extra))
    data = np.concatenate(out)
    # Each episode in tick order (records already are, per run).
    data = data[np.lexsort((data["tick"], data["ep"]))]
    tables = {
        "goals": goals,
        "actions": actions,
        "sightings": runs[0][1]["sightings"],
        "species": runs[0][1]["species"],
        "looks": runs[0][1]["looks"],
        "none16": runs[0][1]["none16"],
        "none8": runs[0][1]["none8"],
        "tick_hz": runs[0][1]["tick_hz"],
    }
    return data, runs, tables


def merge(a, extra):
    dt = np.dtype(a.dtype.descr + extra.dtype.descr)
    out = np.empty(len(a), dtype=dt)
    for n in a.dtype.names:
        out[n] = a[n]
    for n in extra.dtype.names:
        out[n] = extra[n]
    return out


def turn(a, b):
    """Signed wire-yaw difference b − a, wrapped (65536 is a full turn)."""
    return ((b.astype(np.int64) - a.astype(np.int64) + 32768) % 65536) - 32768


def pairs(data):
    """Indices (prev, cur) of consecutive ticks within one episode."""
    same = data["ep"][1:] == data["ep"][:-1]
    step = data["tick"][1:] == data["tick"][:-1] + 1
    cur = np.nonzero(same & step)[0] + 1
    return cur - 1, cur


def deg(wire):
    return np.abs(wire) * 360.0 / 65536.0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("paths", nargs="+")
    args = ap.parse_args()
    data, runs, t = load(args.paths)
    hz = t["tick_hz"]
    seats = len(np.unique(data["ep"]))
    print(f"{len(data)} records · {len(runs)} runs · {seats} seat-runs · "
          f"{len(data) / hz / 60:.1f} body-minutes · {data.dtype.itemsize} B each")
    print(f"alive {data['alive'].mean():.1%} · "
          f"bodies tracked per tick {(data['tr_species'] > 0).sum(1).mean():.2f} "
          f"(visible {((data['tr_flags'] & 1) > 0).sum(1).mean():.2f})")

    print("\ngoals (share of ticks):")
    g, n = np.unique(data["goal"], return_counts=True)
    for gi, c in sorted(zip(g, n), key=lambda x: -x[1]):
        name = "(none)" if gi == 0 else t["goals"][gi - 1]
        print(f"  {name:28s} {c / len(data):6.1%}")
    acts = data["action"][data["action"] > 0]
    if len(acts):
        a, n = np.unique(acts, return_counts=True)
        print("actions: " + ", ".join(f"{t['actions'][i - 1]} {c}" for i, c in zip(a, n)))

    p, c = pairs(data)
    prev, cur = data[p], data[c]
    print(f"\nrepeat last ({len(c)} consecutive pairs):")
    print(f"  buttons exact      {(prev['buttons'] == cur['buttons']).mean():6.1%}")
    print(f"  move_z exact       {(prev['f_move_z'] == cur['f_move_z']).mean():6.1%}")
    print(f"  move_x exact       {(prev['f_move_x'] == cur['f_move_x']).mean():6.1%}")
    print(f"  slot exact         {(prev['f_sel'] == cur['f_sel']).mean():6.1%}")
    # The turn this tick against the last tick's turn: three ticks in a row.
    p2 = p[np.isin(p, c)]
    last_turn = turn(data["f_yaw"][p2 - 1], data["f_yaw"][p2])
    this_turn = turn(data["f_yaw"][p2], data["f_yaw"][p2 + 1])
    turning = this_turn != 0
    print(f"  turn exact         {(last_turn == this_turn).mean():6.1%} "
          f"(turning on {turning.mean():.1%} of ticks; mean error "
          f"{deg(this_turn - last_turn).mean():.2f}°, {deg(this_turn - last_turn)[turning].mean():.2f}° "
          f"when turning)")
    changed = (prev["buttons"] != cur["buttons"]) | (prev["f_move_z"] != cur["f_move_z"]) | (
        prev["f_move_x"] != cur["f_move_x"]) | (prev["f_sel"] != cur["f_sel"])
    print(f"  any change         {changed.mean():6.1%} of ticks: what repeat-last always misses")

    print("\nhands (frame vs the skill's intent):")
    print(f"  buttons = intent   {(data['buttons'] == data['in_buttons']).mean():6.1%}")
    axes = data["in_travel"] == t["none16"]
    if axes.any():
        same = (data["f_move_x"][axes] == data["in_move_x"][axes]) & (
            data["f_move_z"][axes] == data["in_move_z"][axes])
        print(f"  move = intent      {same.mean():6.1%} (of {axes.mean():.1%} ticks on axes, "
              f"the rest travel a bearing)")
    aimed = data["in_yaw"] != t["none16"]
    off = deg(turn(data["in_yaw"][aimed], data["f_yaw"][aimed]))
    print(f"  view on target     {(off < 1.5).mean():6.1%} within 1.5° "
          f"(median {np.median(off):.1f}° off; the hands turn at a person's speed)")
    sel = data["in_sel"] != t["none8"]
    if sel.any():
        print(f"  slot = intent      {(data['f_sel'][sel] == data['in_sel'][sel]).mean():6.1%}")


if __name__ == "__main__":
    main()
