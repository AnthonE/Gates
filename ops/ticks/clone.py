#!/usr/bin/env python3
"""Can a small model play the body? Behaviour-clone recorded ticks (look.py's
`load`) and score it against repeating the last frame, on held-out runs.

    python3 ops/ticks/clone.py runs/ticks
    python3 ops/ticks/clone.py runs/ticks --with-intent   # hand it the skill's intent too

The model sees what the client saw on this tick (its own body, the coarse
sightings its mind is shown, the eight nearest tracked bodies), the goal the
mind is running, and the last frame it sent. It predicts this tick's frame:
the buttons, the forward and strafe axes (bucketed), and the turn (in
1/256ths of a circle, clipped to ±8). Without `--with-intent` it is never
shown what the skill asked for, so whatever it gets right it learned from
the goal and the senses: the body's whole job, not just the hands'.

The scores that matter are on the ticks where something changed, since
repeating the last frame is right everywhere else for free. The last run
(or, with one run, half its seats) is held out, so no run grades itself.
A one-hidden-layer MLP in numpy: a floor for the model that replaces it,
not the model.
"""

import argparse
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from look import deg, load, turn  # noqa: E402

BUTTON_BITS = 7
MZ_EDGES = [-32, 33]  # back, none, forward
MX_EDGES = [-31, 0, 1, 32]  # hard left, nudge left, none, nudge right, hard right
TURN_MAX = 8


def onehot(i, n):
    out = np.zeros((len(i), n), dtype=np.float32)
    out[np.arange(len(i)), np.clip(i, 0, n - 1)] = 1.0
    return out


def turn_class(w):
    return (np.clip(np.rint(w / 256.0), -TURN_MAX, TURN_MAX) + TURN_MAX).astype(np.int64)


def triples(d):
    """Indices c whose two previous records are the same seat's two
    previous ticks."""
    ep, tick = d["ep"], d["tick"].astype(np.int64)
    c = np.arange(2, len(d))
    ok = (ep[c] == ep[c - 1]) & (ep[c] == ep[c - 2]) & (tick[c] == tick[c - 1] + 1) & (
        tick[c - 1] == tick[c - 2] + 1)
    return c[ok]


def features(d, c, t, scale, with_intent):
    o, p = d[c], d[c - 1]
    f = np.float32
    cols = [observation(o, t, scale)]
    # The last frame it sent, and the last turn.
    cols.append(((p["buttons"][:, None] >> np.arange(BUTTON_BITS)) & 1).astype(f))
    cols.append((p["f_move_x"] / 127.0)[:, None].astype(f))
    cols.append((p["f_move_z"] / 127.0)[:, None].astype(f))
    cols.append(onehot(turn_class(turn(d["f_yaw"][c - 2], p["f_yaw"])), 2 * TURN_MAX + 1))
    if with_intent:
        cols.append(intent(o, p, t))
    return np.concatenate(cols, axis=1)


def observation(o, t, scale):
    """What the client saw on these records: the goal, its own body, the
    coarse sightings, the nearest tracked bodies and where the nearest of
    each resource is. No frame, no intent. `crates/server/src/body.rs`
    reads the same, in the same order."""
    f = np.float32
    n = len(o)
    cols = [onehot(o["goal"].astype(np.int64), len(t["goals"]) + 1)]
    cols.append((np.log1p(o["goal_ticks"].astype(f)) / 8.0)[:, None])
    cols.append((o["hp"] / np.maximum(o["hp_max"], 1).astype(f))[:, None])
    cols.append((o["food"] / scale["food"])[:, None].astype(f))
    cols.append((o["water"] / scale["water"])[:, None].astype(f))
    cols.append(o["grounded"][:, None].astype(f))
    cols.append((o["pitch"] / 255.0)[:, None].astype(f))
    seen = (o["see_count"] > 0).astype(f)
    cols += [seen, np.minimum(o["see_count"], 10).astype(f) / 10, o["see_m"].astype(f) / 255]
    bear = np.zeros(o["see_bearing"].shape + (8,), dtype=f)
    np.put_along_axis(bear, np.clip(o["see_bearing"], 0, 7)[..., None].astype(np.int64), 1.0, axis=-1)
    cols.append((bear * seen[..., None]).reshape(n, -1))
    # The nearest bodies.
    here = (o["tr_species"] > 0).astype(f)
    sp = np.zeros(o["tr_species"].shape + (3,), dtype=f)
    for k in range(3):
        sp[..., k] = o["tr_species"] == k + 1
    bits = ((o["tr_flags"][..., None] >> np.arange(8)) & 1).astype(f)
    ang = o["tr_bearing"].astype(f) / 32768.0 * np.pi
    face = turn(o["yaw"][:, None], o["tr_yaw"]).astype(f) / 32768.0 * np.pi
    cols += [
        here,
        sp.reshape(n, -1),
        bits.reshape(n, -1),
        np.sin(ang) * here,
        np.cos(ang) * here,
        np.minimum(o["tr_dist"] / 50.0, 1.0) * here,
        np.clip(o["tr_dy"] / 5.0, -1, 1) * here,
        np.sin(face) * here,
        np.cos(face) * here,
    ]
    # The nearest tree, stone, ore, bush, hemp and water.
    known = o["rs_here"].astype(f)
    rang = o["rs_bearing"].astype(f) / 32768.0 * np.pi
    cols += [
        known,
        np.sin(rang) * known,
        np.cos(rang) * known,
        np.minimum(o["rs_dist"] / 50.0, 1.0) * known,
    ]
    return np.concatenate([np.asarray(x, dtype=f).reshape(n, -1) for x in cols], axis=1)


def intent(o, p, t):
    """The skill's intent on records `o`, its angles from the last frame's
    view (`p`)."""
    f = np.float32
    none16 = t["none16"]
    aimed = (o["in_yaw"] != none16).astype(f)
    want = turn(p["f_yaw"], o["in_yaw"]).astype(f) / 32768.0 * np.pi
    travel = (o["in_travel"] != none16).astype(f)
    tra = turn(p["f_yaw"], o["in_travel"]).astype(f) / 32768.0 * np.pi
    cols = [
        onehot(o["in_look"].astype(np.int64), 4),
        (aimed * np.sin(want))[:, None],
        (aimed * np.cos(want))[:, None],
        (np.minimum(o["in_dist"] / 50.0, 1.0))[:, None].astype(f),
        (np.clip(o["in_dy"] / 5.0, -1, 1))[:, None].astype(f),
        (o["in_move_x"] / 127.0)[:, None].astype(f),
        (o["in_move_z"] / 127.0)[:, None].astype(f),
        travel[:, None],
        (travel * np.sin(tra))[:, None],
        (travel * np.cos(tra))[:, None],
        ((o["in_buttons"][:, None] >> np.arange(BUTTON_BITS)) & 1).astype(f),
    ]
    return np.concatenate([np.asarray(x, dtype=f).reshape(len(o), -1) for x in cols], axis=1)


def targets(d, c):
    o, p = d[c], d[c - 1]
    return {
        "buttons": ((o["buttons"][:, None] >> np.arange(BUTTON_BITS)) & 1).astype(np.float32),
        "move_z": np.digitize(o["f_move_z"], MZ_EDGES),
        "move_x": np.digitize(o["f_move_x"], MX_EDGES),
        "turn": turn_class(turn(p["f_yaw"], o["f_yaw"])),
        "_turn_wire": turn(p["f_yaw"], o["f_yaw"]),
    }


def repeat_last(d, c):
    p, pp = d[c - 1], d[c - 2]
    return {
        "buttons": ((p["buttons"][:, None] >> np.arange(BUTTON_BITS)) & 1).astype(np.float32),
        "move_z": np.digitize(p["f_move_z"], MZ_EDGES),
        "move_x": np.digitize(p["f_move_x"], MX_EDGES),
        "turn": turn_class(turn(pp["f_yaw"], p["f_yaw"])),
    }


HEADS = [("move_z", len(MZ_EDGES) + 1), ("move_x", len(MX_EDGES) + 1), ("turn", 2 * TURN_MAX + 1)]


class MLP:
    def __init__(self, n_in, hidden, rng):
        self.sizes = [BUTTON_BITS] + [k for _, k in HEADS]
        n_out = sum(self.sizes)
        self.p = {
            "W1": rng.normal(0, np.sqrt(2.0 / n_in), (n_in, hidden)).astype(np.float32),
            "b1": np.zeros(hidden, np.float32),
            "W2": rng.normal(0, np.sqrt(1.0 / hidden), (hidden, n_out)).astype(np.float32),
            "b2": np.zeros(n_out, np.float32),
        }
        self.m = {k: np.zeros_like(v) for k, v in self.p.items()}
        self.v = {k: np.zeros_like(v) for k, v in self.p.items()}
        self.t = 0

    def forward(self, x):
        h = np.maximum(x @ self.p["W1"] + self.p["b1"], 0)
        return h, h @ self.p["W2"] + self.p["b2"]

    def split(self, z):
        out, at = [], 0
        for k in self.sizes:
            out.append(z[:, at:at + k])
            at += k
        return out

    def step(self, x, y, lr):
        h, z = self.forward(x)
        parts = self.split(z)
        grads, loss = [], 0.0
        # Buttons: independent bits.
        sb = 1 / (1 + np.exp(-parts[0]))
        loss += -np.mean(np.sum(y["buttons"] * np.log(sb + 1e-7) + (1 - y["buttons"]) * np.log(1 - sb + 1e-7), 1))
        grads.append((sb - y["buttons"]) / len(x))
        for (name, k), zk in zip(HEADS, parts[1:]):
            e = np.exp(zk - zk.max(1, keepdims=True))
            pk = e / e.sum(1, keepdims=True)
            loss += -np.mean(np.log(pk[np.arange(len(x)), y[name]] + 1e-7))
            g = pk.copy()
            g[np.arange(len(x)), y[name]] -= 1
            grads.append(g / len(x))
        dz = np.concatenate(grads, 1).astype(np.float32)
        g = {"W2": h.T @ dz, "b2": dz.sum(0)}
        dh = (dz @ self.p["W2"].T) * (h > 0)
        g["W1"] = x.T @ dh
        g["b1"] = dh.sum(0)
        self.t += 1
        for k in self.p:
            self.m[k] = 0.9 * self.m[k] + 0.1 * g[k]
            self.v[k] = 0.999 * self.v[k] + 0.001 * g[k] ** 2
            mh = self.m[k] / (1 - 0.9 ** self.t)
            vh = self.v[k] / (1 - 0.999 ** self.t)
            self.p[k] -= lr * mh / (np.sqrt(vh) + 1e-8)
        return loss

    def predict(self, x):
        parts = self.split(self.forward(x)[1])
        out = {"buttons": (parts[0] > 0).astype(np.float32)}
        for (name, _), zk in zip(HEADS, parts[1:]):
            out[name] = zk.argmax(1)
        return out


def score(pred, y, mask):
    s = {"buttons": np.all(pred["buttons"][mask] == y["buttons"][mask], 1).mean()}
    for name, _ in HEADS:
        s[name] = (pred[name][mask] == y[name][mask]).mean()
    want = np.clip(y["_turn_wire"][mask], -TURN_MAX * 256, TURN_MAX * 256)
    off = deg((pred["turn"][mask] - TURN_MAX) * 256.0 - want)
    s["turn_err_deg"] = off.mean()
    return s


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("paths", nargs="+")
    ap.add_argument("--with-intent", action="store_true")
    ap.add_argument("--hidden", type=int, default=256)
    ap.add_argument("--epochs", type=int, default=6)
    ap.add_argument("--batch", type=int, default=1024)
    ap.add_argument("--lr", type=float, default=1e-3)
    ap.add_argument("--seed", type=int, default=1)
    args = ap.parse_args()
    d, runs, t = load(args.paths)
    c = triples(d)
    if len(runs) > 1:
        test = d["run"][c] == d["run"].max()
    else:
        eps = np.unique(d["ep"])
        test = np.isin(d["ep"][c], eps[len(eps) // 2:])
    scale = {"food": float(max(d["food"].max(), 1)), "water": float(max(d["water"].max(), 1))}
    x = features(d, c, t, scale, args.with_intent)
    y = targets(d, c)
    tr, te = np.nonzero(~test)[0], np.nonzero(test)[0]
    print(f"{len(tr)} train ticks, {len(te)} held out · {x.shape[1]} features"
          f"{' (with the intent)' if args.with_intent else ' (no intent)'}")
    rng = np.random.default_rng(args.seed)
    net = MLP(x.shape[1], args.hidden, rng)
    for ep in range(args.epochs):
        order = rng.permutation(tr)
        losses = []
        for i in range(0, len(order), args.batch):
            b = order[i:i + args.batch]
            losses.append(net.step(x[b], {k: v[b] for k, v in y.items()}, args.lr))
        print(f"  epoch {ep + 1}: loss {np.mean(losses):.3f}")

    yt = {k: v[te] for k, v in y.items()}
    rep = {k: v[te] for k, v in repeat_last(d, c).items()}
    mlp = net.predict(x[te])
    changed = np.any(rep["buttons"] != yt["buttons"], 1) | (rep["move_z"] != yt["move_z"]) | (
        rep["move_x"] != yt["move_x"]) | (rep["turn"] != yt["turn"])
    every = np.ones(len(te), bool)
    print(f"\nheld out: {len(te)} ticks, {changed.mean():.1%} where the frame changed")
    print(f"{'':16s}{'all ticks':>22s}{'changed ticks':>26s}")
    print(f"{'':16s}{'repeat':>11s}{'model':>11s}{'repeat':>13s}{'model':>13s}")
    a_r, a_m = score(rep, yt, every), score(mlp, yt, every)
    c_r, c_m = score(rep, yt, changed), score(mlp, yt, changed)
    for k in ["buttons", "move_z", "move_x", "turn"]:
        print(f"  {k:14s}{a_r[k]:10.1%} {a_m[k]:10.1%} {c_r[k]:12.1%} {c_m[k]:12.1%}")
    k = "turn_err_deg"
    print(f"  {'turn error':14s}{a_r[k]:9.2f}° {a_m[k]:9.2f}° {c_r[k]:11.2f}° {c_m[k]:11.2f}°")


if __name__ == "__main__":
    main()
