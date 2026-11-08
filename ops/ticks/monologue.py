"""The body's inner monologue: what it means to do, a few ticks before it does it.

A plan is six small tokens read off the skill's intent (`ticks.rs`'s `in_*`
columns) and the next action, so the teacher's monologue is in every
recording and needs no labelling:

- **look**: keep the view, face a heading, look at a point, look at a body
- **where**: the look's target, in the mind's eight bearings (clockwise from
  ahead, `mind::BEARINGS`), or none
- **far**: how far the looked-at thing is: under 2 m, 2-6, 6-20, 20+, or none
- **move**: still, an axis (forward, back, left, right), or a walk along a
  bearing relative to the view
- **hands**: the buttons the skill holds (seven bits, any number at once)
- **next**: the action the body sends within the next `NEXT_TICKS`, or none

`body.py` writes the plan `AHEAD` ticks before the frame it is for and reads
it back as an input, the way a full-duplex speech model writes its text a
little ahead of its audio. `say()` turns one plan into the sentence a
spectator could read over the body's head.
"""

import numpy as np

from look import turn

AHEAD = 3  # ticks: 100 ms at 30 Hz
NEXT_TICKS = 15  # half a second

BEARINGS = ["ahead", "ahead-right", "right", "behind-right", "behind", "behind-left", "left",
            "ahead-left"]
LOOKS = ["keep", "heading", "point", "body"]
FAR = ["none", "<2m", "2-6m", "6-20m", "20m+"]
FAR_EDGES = [2.0, 6.0, 20.0]
MOVES = ["still", "forward", "back", "left", "right"] + [f"walk {b}" for b in BEARINGS]
BUTTONS = ["sprint", "crouch", "swing", "jump", "light", "assist", "aim"]


def sizes(n_actions):
    """Classes per categorical field ('hands' is seven independent bits)."""
    return {
        "look": len(LOOKS),
        "where": len(BEARINGS) + 1,
        "far": len(FAR),
        "move": len(MOVES),
        "next": n_actions + 1,
    }


def sector(rel):
    """Signed wire-yaw offsets to the mind's eight bearings (0 is ahead,
    clockwise: a positive offset is to the right)."""
    return (np.rint((rel.astype(np.int64) % 65536) / 8192.0).astype(np.int64)) % 8


def plans(d, none16):
    """The teacher's plan on every record of `d` (look.load's array, each
    seat's ticks contiguous and in order). A dict of int arrays, plus
    'hands' as an (n, 7) array of 0/1."""
    n = len(d)
    out = {"look": d["in_look"].astype(np.int64)}
    aimed = d["in_yaw"] != none16
    where = np.full(n, len(BEARINGS), dtype=np.int64)  # none
    where[aimed] = sector(turn(d["yaw"][aimed], d["in_yaw"][aimed]))
    out["where"] = where
    far = np.zeros(n, dtype=np.int64)
    pointed = aimed & (d["in_look"] >= 2)
    far[pointed] = 1 + np.digitize(d["in_dist"][pointed], FAR_EDGES)
    out["far"] = far
    move = np.zeros(n, dtype=np.int64)
    travel = d["in_travel"] != none16
    move[travel] = 5 + sector(turn(d["yaw"][travel], d["in_travel"][travel]))
    mx, mz = d["in_move_x"].astype(np.int64), d["in_move_z"].astype(np.int64)
    axes = ~travel & ((mx != 0) | (mz != 0))
    fwd = np.abs(mz) >= np.abs(mx)
    move[axes & fwd & (mz > 0)] = 1
    move[axes & fwd & (mz < 0)] = 2
    move[axes & ~fwd & (mx < 0)] = 3
    move[axes & ~fwd & (mx > 0)] = 4
    out["move"] = move
    out["hands"] = ((d["in_buttons"][:, None] >> np.arange(len(BUTTONS))) & 1).astype(np.float32)
    out["next"] = next_action(d)
    return out


def next_action(d):
    """The first action each seat sends in [t, t + NEXT_TICKS), 0 for none."""
    n = len(d)
    nxt = np.zeros(n, dtype=np.int64)
    acted = np.nonzero(d["action"] > 0)[0]
    ep = d["ep"]
    # Walk backwards so the nearest action wins.
    for i in acted[::-1]:
        lo = max(i - NEXT_TICKS + 1, 0)
        span = np.arange(lo, i + 1)
        span = span[ep[span] == ep[i]]
        nxt[span] = d["action"][i]
    return nxt


def say(goal, plan, actions):
    """One plan as words. `plan` holds one row of each field."""
    parts = []
    held = [BUTTONS[i] for i in range(len(BUTTONS)) if plan["hands"][i] > 0.5]
    move = MOVES[int(plan["move"])]
    walking = None
    if move == "still":
        parts.append("standing")
    elif move.startswith("walk"):
        walking = move.split(" ", 1)[1]
        verb = "running" if "sprint" in held else "walking"
        held = [h for h in held if h != "sprint"]
        parts.append(f"{verb} {walking}")
    else:
        parts.append({"forward": "stepping forward", "back": "backing off",
                      "left": "sidestepping left", "right": "sidestepping right"}[move])
    look, where, far = LOOKS[int(plan["look"])], int(plan["where"]), int(plan["far"])
    if look == "heading" and 0 < where < len(BEARINGS) and BEARINGS[where] != walking:
        parts.append(f"turning {BEARINGS[where]}")
    elif look in ("point", "body") and where < len(BEARINGS):
        what = "a body" if look == "body" else "a spot"
        dist = f" {FAR[far]}" if far else ""
        parts.append(f"eyes on {what}{dist} {BEARINGS[where]}")
    if held:
        parts.append(" + ".join(held))
    nxt = int(plan["next"])
    if nxt:
        parts.append(f"about to {actions[nxt - 1].lower()}")
    return f"{goal}: " + ", ".join(parts)
