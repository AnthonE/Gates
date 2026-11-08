#!/usr/bin/env python3
"""A body that plays at frame rate: a small causal transformer over the last
N ticks that says what it is about to do a few ticks before it does it.

    python3 ops/ticks/body.py runs/ticks                    # train, score, write body.pt
    python3 ops/ticks/body.py runs/ticks --monologue feed   # and act on its own plan
    python3 ops/ticks/body.py runs/ticks --monologue off    # no plan at all

Needs torch (the CPU build is enough): `pip install torch`.

**Each tick is one position.** Its input is what the client saw on that tick
(`clone.observation`: the goal, its own body, the coarse sightings, the
nearest tracked bodies), the frame it sent on the tick before, and the plan
for this tick (`feed` only). Its outputs are this tick's frame (buttons,
forward and strafe buckets, the turn) and the plan for `monologue.AHEAD`
ticks on.

**The monologue** (`monologue.py`) is the skill's intent as six small tokens,
in every recording already. At tick t the model writes the plan for
t + AHEAD: what a spectator would read over its head before it moves.

- `aux` (the default) writes it and never reads it, at a small weight in the
  loss (`--plan-weight`), so the plan costs the frames next to nothing.
- `feed` also reads it back at t + AHEAD, the way a full-duplex speech model
  writes its text a little ahead of its audio and speaks from it. Trained on
  the teacher's plans, a share hidden (`--plan-dropout`) and a share swapped
  for its own (`--own-plan`), so it learns to act on its own plan, mistakes
  included. Scored on its own plans (`own`), which is how it would play, and
  on the teacher's (`teacher`): how it plays a plan someone else wrote,
  which is what a mind steering the body would do. On the recordings so far
  its own plan has cost more frame accuracy than it gave; run both and look.

**On its own play (DAgger).** A body copied from the teacher's runs only
ever saw where the teacher went; once its own small errors take it
somewhere else (a tree approached from the wrong side) it has nothing to
copy. Bench a body with `--ticks`: its records hold the frame it sent
(`f_*`) and the frame the scripted hands made of the same tick (`t_*`),
which is the expert's answer from where the body really stood. Training
always learns `t_*`, from the frames actually sent, so those runs beside the
teacher's teach it to recover:

    gates-agent --bench 10 --bench-runs 4 --warp --bots 8 --scripted \
        --body body.json --ticks runs/dagger1
    python3 ops/ticks/body.py runs/dagger1 runs/ticks   # held out: the last run listed

**Scored** like `clone.py`: on the held-out run, against repeating the last
frame, on every tick and on the ticks where the frame changed. Only
positions with `--min-context` ticks behind them are scored.
"""

import argparse
import os
import sys
import time

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import monologue as mono  # noqa: E402
from clone import (BUTTON_BITS, HEADS, MX_EDGES, MZ_EDGES, TURN_MAX, observation,  # noqa: E402
                   onehot, turn_class)
from look import deg, load, turn  # noqa: E402

try:
    import torch
    import torch.nn as nn
    import torch.nn.functional as F
except ImportError:
    raise SystemExit("body.py needs torch: pip install torch")

PITCH_MAX = 10  # a tick's pitch change, clipped (wire units, 0.7° each)
HOTBAR = 6  # sim_core::limits::HOTBAR_SLOTS
# Classes after the seven button bits: the axes and the turn as clone.py has
# them, then the pitch change and the hotbar slot.
BODY_HEADS = HEADS + [("pitch", 2 * PITCH_MAX + 1), ("sel", HOTBAR)]
PLAN_FIELDS = ["look", "where", "far", "move", "next"]  # plus 'hands', seven bits
N_TURN = 2 * TURN_MAX + 1


def frame_features(d):
    """The frame each record sent, as an input: buttons, axes, its turn
    (from the frame before; none on an episode's first), its pitch and its
    slot. `crates/server/src/body.rs` builds the same from the frames it
    sent. Also the classes of the teacher's frame (`t_*`: the scripted
    hands' answer on the same tick) as turns and pitch changes from the
    frame sent before it, which is what the body decodes from."""
    ep = d["ep"]
    first = np.ones(len(d), bool)
    first[1:] = ep[1:] != ep[:-1]
    last_yaw = np.roll(d["f_yaw"], 1)
    last_yaw[first] = d["f_yaw"][first]
    tc = turn_class(turn(last_yaw, d["f_yaw"]))
    # The teacher's turn and pitch change, from the frame actually sent the
    # tick before (the body's own first frame starts from the hands' view).
    sent_yaw = np.roll(d["f_yaw"], 1)
    sent_yaw[first] = d["t_yaw"][first]
    t_tc = turn_class(turn(sent_yaw, d["t_yaw"]))
    sent_pitch = np.roll(d["f_pitch"], 1).astype(np.int64)
    sent_pitch[first] = d["t_pitch"][first]
    t_pc = np.clip(d["t_pitch"].astype(np.int64) - sent_pitch, -PITCH_MAX, PITCH_MAX) + PITCH_MAX
    cols = [
        ((d["buttons"][:, None] >> np.arange(BUTTON_BITS)) & 1).astype(np.float32),
        (d["f_move_x"] / 127.0)[:, None].astype(np.float32),
        (d["f_move_z"] / 127.0)[:, None].astype(np.float32),
        onehot(tc, N_TURN),
        (d["f_pitch"] / 255.0)[:, None].astype(np.float32),
        onehot(np.minimum(d["f_sel"], HOTBAR - 1).astype(np.int64), HOTBAR),
    ]
    return np.concatenate(cols, 1), (t_tc, t_pc), first


def frame_targets(d, classes):
    """The teacher's frame on each record: what the scripted hands made of
    it (the frame sent, on a teacher's own recording)."""
    tc, pc = classes
    return {
        "buttons": ((d["t_buttons"][:, None] >> np.arange(BUTTON_BITS)) & 1).astype(np.float32),
        "move_z": np.digitize(d["t_move_z"], MZ_EDGES).astype(np.int64),
        "move_x": np.digitize(d["t_move_x"], MX_EDGES).astype(np.int64),
        "turn": tc.astype(np.int64),
        "pitch": pc.astype(np.int64),
        "sel": np.minimum(d["t_sel"], HOTBAR - 1).astype(np.int64),
    }


def class_values(d):
    """What each axis bucket sends when the model picks it: the mean of what
    the teacher sent in that bucket (the bucket's middle where it never did)."""
    out = {}
    for name, col, edges, mids in (("move_z", "t_move_z", MZ_EDGES, [-127, 0, 127]),
                                   ("move_x", "t_move_x", MX_EDGES, [-127, -16, 0, 16, 127])):
        v = d[col].astype(np.float64)
        cls = np.digitize(d[col], edges)
        out[name] = [int(round(v[cls == k].mean())) if (cls == k).any() else mids[k]
                     for k in range(len(edges) + 1)]
    return out


def windows(ep, n, stride):
    """Start indices of length-n windows inside one episode each."""
    starts = []
    change = np.nonzero(np.r_[True, ep[1:] != ep[:-1], True])[0]
    for a, b in zip(change[:-1], change[1:]):
        starts += list(range(a, b - n + 1, stride))
    return np.array(starts, dtype=np.int64)


class Body(nn.Module):
    def __init__(self, n_obs, n_frame, plan_sizes, d=128, layers=3, heads=4, ctx=32,
                 monologue="feed"):
        super().__init__()
        self.cfg = dict(n_obs=n_obs, n_frame=n_frame, plan_sizes=plan_sizes, d=d, layers=layers,
                        heads=heads, ctx=ctx, monologue=monologue)
        self.obs = nn.Linear(n_obs, d)
        self.frame = nn.Linear(n_frame, d)
        self.pos = nn.Embedding(ctx, d)
        # 'feed': writes the plan and reads it back; 'aux': writes it only;
        # 'off': no plan.
        self.monologue = monologue != "off"
        self.feed = monologue == "feed"
        if self.feed:
            self.plan_emb = nn.ModuleDict({k: nn.Embedding(n, d) for k, n in plan_sizes.items()})
            self.plan_hands = nn.Linear(len(mono.BUTTONS), d)
            self.plan_unknown = nn.Parameter(torch.zeros(d))
        if self.monologue:
            self.out_plan = nn.Linear(d, sum(plan_sizes.values()) + len(mono.BUTTONS))
        layer = nn.TransformerEncoderLayer(d, heads, 4 * d, dropout=0.1, batch_first=True,
                                           norm_first=True)
        self.tr = nn.TransformerEncoder(layer, layers, enable_nested_tensor=False)
        self.norm = nn.LayerNorm(d)
        self.out_frame = nn.Linear(d, BUTTON_BITS + sum(k for _, k in BODY_HEADS))
        self.register_buffer("mask", torch.triu(torch.full((ctx, ctx), float("-inf")), 1))

    def hidden(self, obs, frame_prev, plan=None, known=None):
        """Every position's last hidden state, before the heads."""
        b, t, _ = obs.shape
        x = self.obs(obs) + self.frame(frame_prev) + self.pos.weight[:t]
        if self.feed:
            p = sum(self.plan_emb[k](plan[k]) for k in self.plan_emb) + self.plan_hands(plan["hands"])
            x = x + torch.where(known[..., None], p, self.plan_unknown)
        return self.norm(self.tr(x, mask=self.mask[:t, :t], is_causal=True))

    def forward(self, obs, frame_prev, plan=None, known=None):
        h = self.hidden(obs, frame_prev, plan, known)
        frame = self.out_frame(h)
        plan_out = self.out_plan(h) if self.monologue else None
        return frame, plan_out

    def split_plan(self, z):
        out, at = {}, 0
        for k, n in self.cfg["plan_sizes"].items():
            out[k] = z[..., at:at + n]
            at += n
        out["hands"] = z[..., at:at + len(mono.BUTTONS)]
        return out


def split_frame(z):
    out = {"buttons": z[..., :BUTTON_BITS]}
    at = BUTTON_BITS
    for name, k in BODY_HEADS:
        out[name] = z[..., at:at + k]
        at += k
    return out


def losses(model, fz, pz, ft, pt, pvalid, plan_weight=1.0):
    fo = split_frame(fz)
    loss = F.binary_cross_entropy_with_logits(fo["buttons"], ft["buttons"])
    for name, _ in BODY_HEADS:
        loss = loss + F.cross_entropy(fo[name].flatten(0, 1), ft[name].flatten())
    if pz is not None:
        po = model.split_plan(pz)
        v = pvalid.flatten()
        pl = F.binary_cross_entropy_with_logits(po["hands"].flatten(0, 1)[v],
                                                pt["hands"].flatten(0, 1)[v])
        for k in PLAN_FIELDS:
            pl = pl + F.cross_entropy(po[k].flatten(0, 1)[v], pt[k].flatten()[v])
        loss = loss + plan_weight * pl
    return loss


class Data:
    """Everything the model reads, as arrays over all records."""

    def __init__(self, d, t, scale, ahead):
        self.d = d
        self.obs = observation(d, t, scale)
        self.frame, classes, self.first = frame_features(d)
        # The frame before each record (zeros at an episode's start).
        self.frame_prev = np.roll(self.frame, 1, axis=0)
        self.frame_prev[self.first] = 0
        self.ft = frame_targets(d, classes)
        self.plan = mono.plans(d, t["none16"])
        # The plan `ahead` ticks on, where the same episode has one.
        n = len(d)
        idx = np.minimum(np.arange(n) + ahead, n - 1)
        self.plan_valid = (d["ep"][idx] == d["ep"]) & (np.arange(n) + ahead < n)
        self.plan_ahead = {k: v[idx] for k, v in self.plan.items()}

    def batch(self, starts, n, dev):
        ix = starts[:, None] + np.arange(n)[None, :]
        tt = lambda a, dt=torch.float32: torch.as_tensor(a[ix], dtype=dt, device=dev)
        obs, fp = tt(self.obs), tt(self.frame_prev)
        ft = {k: tt(v, torch.float32 if k == "buttons" else torch.long) for k, v in self.ft.items()}
        plan = {k: tt(v, torch.float32 if k == "hands" else torch.long) for k, v in self.plan.items()}
        pt = {k: tt(v, torch.float32 if k == "hands" else torch.long)
              for k, v in self.plan_ahead.items()}
        pvalid = tt(self.plan_valid, torch.bool)
        return obs, fp, plan, ft, pt, pvalid, ix


@torch.no_grad()
def rollout(model, obs, fp, plan_teacher, mode, ahead):
    """Frame logits over a window, the plan inputs being the teacher's, the
    model's own (written `ahead` ticks earlier), or none."""
    b, n, _ = obs.shape
    if not model.feed:
        fz, pz = model(obs, fp)
        return fz, (pz, None, None)
    known = torch.ones(b, n, dtype=torch.bool) if mode == "teacher" else torch.zeros(
        b, n, dtype=torch.bool)
    plan = {k: v.clone() for k, v in plan_teacher.items()}
    if mode == "own":
        for start in range(0, n - ahead, ahead):
            _, pz = model(obs, fp, plan, known)
            po = model.split_plan(pz)
            src = slice(start, min(start + ahead, n - ahead))
            dst = slice(src.start + ahead, src.stop + ahead)
            for k in PLAN_FIELDS:
                plan[k][:, dst] = po[k][:, src].argmax(-1)
            plan["hands"][:, dst] = (po["hands"][:, src] > 0).float()
            known[:, dst] = True
    fz, pz = model(obs, fp, plan, known)
    return fz, (pz, plan, known)


@torch.no_grad()
def own_plans(model, obs, fp, plan, known, share):
    """The plan inputs with `share` of them swapped for what the model itself
    wrote AHEAD ticks earlier (from the teacher's context), so it learns to
    act on its own plan, mistakes included, and not only on a perfect one."""
    a = mono.AHEAD
    _, pz = model(obs, fp, plan, known)
    po = model.split_plan(pz)
    swap = torch.rand(obs.shape[:2]) < share
    swap[:, :a] = False
    out = {}
    for k, v in plan.items():
        mine = v.clone()
        guess = (po[k] > 0).float() if k == "hands" else po[k].argmax(-1)
        mine[:, a:] = guess[:, :-a]
        cond = swap[..., None] if k == "hands" else swap
        out[k] = torch.where(cond, mine, v)
    return out


def frame_preds(fz):
    fo = split_frame(fz)
    out = {"buttons": (fo["buttons"] > 0).float().numpy()}
    for name, _ in BODY_HEADS:
        out[name] = fo[name].argmax(-1).numpy()
    return out


def score(pred, y, turn_wire, mask):
    s = {"buttons": np.all(pred["buttons"][mask] == y["buttons"][mask], -1).mean()}
    for name, _ in BODY_HEADS:
        s[name] = (pred[name][mask] == y[name][mask]).mean()
    want = np.clip(turn_wire[mask], -TURN_MAX * 256, TURN_MAX * 256)
    s["turn_err_deg"] = deg((pred["turn"][mask] - TURN_MAX) * 256.0 - want).mean()
    return s


def evaluate(model, data, te_starts, args, t):
    model.eval()
    n, a = args.ctx, mono.AHEAD
    d = data.d
    rows, preds = [], {m: [] for m in ("own", "teacher", "none")}
    plan_hits = []
    modes = ("own", "teacher", "none") if model.feed else ("none",)
    for i in range(0, len(te_starts), 256):
        obs, fp, plan, ft, pt, pvalid, ix = data.batch(te_starts[i:i + 256], n, "cpu")
        keep = np.arange(n) >= args.min_context
        rows.append(ix[:, keep])
        for m in modes:
            fz, extra = rollout(model, obs, fp, plan, m, a)
            preds[m].append({k: v[:, keep] for k, v in frame_preds(fz).items()})
            if model.monologue and m == modes[0]:
                pz, _, _ = extra
                po = model.split_plan(pz)
                hit = {k: (po[k].argmax(-1) == pt[k]).numpy()[:, keep] for k in PLAN_FIELDS}
                hit["hands"] = np.all(((po["hands"] > 0).float() == pt["hands"]).numpy(),
                                      -1)[:, keep]
                hit["_valid"] = pvalid.numpy()[:, keep]
                plan_hits.append(hit)
    rows = np.concatenate([r.reshape(-1) for r in rows])
    y = {k: v[rows] for k, v in data.ft.items()}
    rep = {k: v[rows - 1] for k, v in data.ft.items()}  # the frame before
    turn_wire = turn(d["f_yaw"][rows - 1], d["t_yaw"][rows])
    changed = np.any(rep["buttons"] != y["buttons"], 1)
    for name, _ in BODY_HEADS:
        changed |= rep[name] != y[name]
    every = np.ones(len(rows), bool)
    print(f"\nheld out: {len(rows)} ticks with {args.min_context}+ behind them, "
          f"{changed.mean():.1%} where the frame changed")
    cols = [("repeat", rep)]
    for m in modes:
        cat = {k: np.concatenate([p[k].reshape((-1,) + p[k].shape[2:]) for p in preds[m]])
               for k in preds[m][0]}
        cols.append(({"own": "own plan", "teacher": "teacher plan", "none": "no plan"}[m], cat))
    head = "".join(f"{name:>14s}" for name, _ in cols)
    print(f"{'changed ticks':16s}{head}")
    for k in ["buttons", "move_z", "move_x", "turn", "pitch", "sel"]:
        print(f"  {k:14s}" + "".join(f"{score(p, y, turn_wire, changed)[k]:13.1%} "
                                     for _, p in cols))
    print(f"  {'turn error':14s}" + "".join(
        f"{score(p, y, turn_wire, changed)['turn_err_deg']:12.2f}° " for _, p in cols))
    print(f"{'all ticks':16s}{head}")
    for k in ["buttons", "move_z", "move_x", "turn", "pitch", "sel"]:
        print(f"  {k:14s}" + "".join(f"{score(p, y, turn_wire, every)[k]:13.1%} "
                                     for _, p in cols))
    print(f"  {'turn error':14s}" + "".join(
        f"{score(p, y, turn_wire, every)['turn_err_deg']:12.2f}° " for _, p in cols))
    if plan_hits:
        v = np.concatenate([h["_valid"].reshape(-1) for h in plan_hits])
        print(f"\nits own plan, {mono.AHEAD} ticks ahead, against the teacher's:")
        for k in PLAN_FIELDS + ["hands"]:
            hit = np.concatenate([h[k].reshape(-1) for h in plan_hits])[v]
            print(f"  {k:8s} {hit.mean():6.1%}")
    return rows, preds


def sample(model, data, starts, t, ahead, count):
    """The monologue over a stretch where the plan changes: the teacher's
    and the model's own, side by side."""
    d = data.d
    p = data.plan
    change = np.nonzero(np.any([p[k][1:] != p[k][:-1] for k in ("look", "move", "next")], 0))[0]
    want = [s for s in starts if np.isin(np.arange(s + 12, s + model.cfg["ctx"]), change).sum() >= 3]
    if not want:
        return
    s = np.array(want[len(want) // 2:len(want) // 2 + 1])
    obs, fp, plan, ft, pt, pvalid, ix = data.batch(s, model.cfg["ctx"], "cpu")
    _, (pz, _, _) = rollout(model, obs, fp, plan, "own", ahead)
    po = model.split_plan(pz)
    print(f"\nthe monologue, ticks {int(d['tick'][s[0]])}..{int(d['tick'][s[0]]) + model.cfg['ctx'] - 1}"
          " of one body (teacher | its own, written 3 ticks earlier):")
    last = None
    for j in range(ahead, model.cfg["ctx"]):
        i = ix[0, j]
        g = t["goals"][d["goal"][i] - 1] if d["goal"][i] else "(deciding)"
        teach = mono.say(g, {k: v[i] for k, v in p.items()}, t["actions"])
        mine = {k: po[k][0, j - ahead].argmax(-1).item() for k in PLAN_FIELDS}
        mine["hands"] = (po["hands"][0, j - ahead] > 0).float().numpy()
        mine = mono.say(g, mine, t["actions"])
        line = (teach, mine)
        if line != last:
            mark = " " if teach == mine else "≠"
            print(f"  {int(d['tick'][i]):6d} {mark} {teach}\n         | {mine}")
            last = line
        if count and j > count + ahead:
            break


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("paths", nargs="+")
    ap.add_argument("--monologue", choices=["feed", "aux", "off"], default="aux",
                    help="feed: write the plan ahead and act on it; aux: write it only; off: none")
    ap.add_argument("--ctx", type=int, default=32, help="ticks of history per window")
    ap.add_argument("--d", type=int, default=128)
    ap.add_argument("--layers", type=int, default=3)
    ap.add_argument("--heads", type=int, default=4)
    ap.add_argument("--epochs", type=int, default=8)
    ap.add_argument("--batch", type=int, default=64)
    ap.add_argument("--lr", type=float, default=3e-4)
    ap.add_argument("--frame-dropout", type=float, default=0.0,
                    help="share of last-frame inputs hidden in training")
    ap.add_argument("--plan-weight", type=float, default=0.2,
                    help="the plan's loss against the frame's")
    ap.add_argument("--plan-dropout", type=float, default=0.3,
                    help="share of plan inputs hidden in training")
    ap.add_argument("--own-plan", type=float, default=0.5,
                    help="share of plan inputs that are the model's own in training")
    ap.add_argument("--min-context", type=int, default=8)
    ap.add_argument("--threads", type=int, default=4)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--out", default="body.pt")
    args = ap.parse_args()
    torch.manual_seed(args.seed)
    torch.set_num_threads(args.threads)
    rng = np.random.default_rng(args.seed)

    d, runs, t = load(args.paths)
    scale = {"food": float(max(d["food"].max(), 1)), "water": float(max(d["water"].max(), 1))}
    data = Data(d, t, scale, mono.AHEAD)
    test_run = d["run"].max()
    held = d["run"] == test_run
    ep = d["ep"].astype(np.int64)
    tr_starts = windows(np.where(held, -1, ep), args.ctx, args.ctx // 2)
    tr_starts = tr_starts[~held[tr_starts]]
    te_starts = windows(np.where(held, ep, -1), args.ctx, args.ctx)
    te_starts = te_starts[held[te_starts]]
    sizes = mono.sizes(len(t["actions"]))
    model = Body(data.obs.shape[1], data.frame.shape[1], sizes, args.d, args.layers, args.heads,
                 args.ctx, monologue=args.monologue)
    nparam = sum(p.numel() for p in model.parameters())
    print(f"{len(tr_starts)} training windows of {args.ctx} ticks, {len(te_starts)} held out "
          f"(run {runs[test_run][0]}) · {nparam / 1e3:.0f}k parameters · "
          f"monologue {args.monologue} ({mono.AHEAD} ticks ahead)")
    opt = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=0.01)
    steps = args.epochs * ((len(tr_starts) + args.batch - 1) // args.batch)
    sched = torch.optim.lr_scheduler.OneCycleLR(opt, args.lr, total_steps=steps, pct_start=0.1)
    for ep in range(args.epochs):
        model.train()
        t0, ls = time.time(), []
        for i, b in enumerate(np.array_split(rng.permutation(tr_starts),
                                             max(len(tr_starts) // args.batch, 1))):
            obs, fp, plan, ft, pt, pvalid, _ = data.batch(b, args.ctx, "cpu")
            if args.frame_dropout > 0:
                # Hide the last frame now and then, so the model cannot only
                # copy it: a body that copies stands still once it stops.
                fp = fp * (torch.rand(fp.shape[:2]) >= args.frame_dropout)[..., None]
            known = torch.rand(obs.shape[:2]) >= args.plan_dropout
            if model.feed and args.own_plan > 0:
                plan = own_plans(model, obs, fp, plan, known, args.own_plan)
            fz, pz = model(obs, fp, plan, known)
            loss = losses(model, fz, pz, ft, pt, pvalid, args.plan_weight)
            opt.zero_grad()
            loss.backward()
            nn.utils.clip_grad_norm_(model.parameters(), 1.0)
            opt.step()
            sched.step()
            ls.append(loss.item())
        print(f"  epoch {ep + 1}: loss {np.mean(ls):.3f} ({time.time() - t0:.0f}s)")

    evaluate(model, data, te_starts, args, t)
    if model.monologue:
        sample(model, data, te_starts, t, mono.AHEAD, 0)
    torch.save({"cfg": model.cfg, "state": model.state_dict(), "goals": t["goals"],
                "actions": t["actions"], "scale": scale, "ahead": mono.AHEAD,
                "values": class_values(d[~held]), "turn_max": TURN_MAX,
                "pitch_max": PITCH_MAX, "hotbar": HOTBAR, "format": "gates-body-1"}, args.out)
    print(f"\nwrote {args.out}")


if __name__ == "__main__":
    main()
