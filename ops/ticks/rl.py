#!/usr/bin/env python3
"""Fine-tune a learned body with PPO on lockstep islands: the body plays,
the island pays it, the model moves toward what paid.

    python3 ops/ticks/rl.py body.pt --out runs/rl --iters 20

Each iteration exports the model (export.py's `gates-body-1`), plays it on
`--procs` private lockstep islands at once (`gates-agent --bench MINUTES
--warp --body FILE --ticks DIR`, island variants from `--first` on, never
the ones GatesBench scores by default), reads every tick back
(`look.load`), and takes PPO steps on them. Needs torch.

**The reward**, per tick and per body:

- +1 for each thing received (`awards`: one `+5 Wood` toast), −5 a death;
- shaping toward what its skill is going for, while a gather goal has a
  target: Φ = −(metres to it)/30 − (how far its view is off it)/half a turn,
  paid as γΦ' − Φ, which moves no optimum;
- a small bonus for swinging while facing that target in reach.

The target is the skill's (`in_*`); the model never sees it, only what the
client saw (`clone.observation`). The mind and the skills' actions stay
scripted, so what pays is the body getting the skills their materials.

**The odds it played with.** The Rust body decides each tick from a window
that grows from `ctx / 2` to `ctx − 1` ticks behind it and starts again
(`body.rs`); log-probabilities here are computed on those same windows, so
PPO's ratio compares like with like. Frames are drawn at temperature 1.

**Kept near the teacher.** The scripted hands' frame on each tick (`t_*`) is
still a label: `--bc` weighs body.py's loss on it beside PPO's, so the body
improves on what pays without forgetting how to walk.

Writes `OUT/iter-NN/body.{pt,json}` and one line per iteration to
`OUT/log.jsonl`; `export.py` reads the `.pt`, `gates-agent --body` the
`.json`.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
import time

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import body as B  # noqa: E402
import monologue as mono  # noqa: E402
from clone import MX_EDGES, MZ_EDGES, turn_class  # noqa: E402
from export import model_json  # noqa: E402
from look import load, turn  # noqa: E402

import torch  # noqa: E402
import torch.nn as nn  # noqa: E402
import torch.nn.functional as F  # noqa: E402

GATHER = ("gather_wood", "gather_stone", "gather_ore", "gather_hemp", "forage")
SWING = 1 << 2  # sim_core::input::BTN_PRIMARY
REACH_M = 2.5
FACING = 10.0 * 65536 / 360  # ten degrees, wire units


def rust_windows(ep, ctx):
    """(start, first scored offset) of every window the Rust body decided
    from: an episode's first `ctx` ticks in one window, every tick scored;
    then windows of `ctx` every `ctx / 2` ticks, their newest half scored."""
    half = ctx // 2
    out = []
    change = np.nonzero(np.r_[True, ep[1:] != ep[:-1], True])[0]
    for a, b in zip(change[:-1], change[1:]):
        if b - a < ctx:
            continue
        out.append((a, 0))
        s = half
        while s + ctx <= b - a:
            out.append((a + s, half))
            s += half
    return np.array(out, dtype=np.int64).reshape(-1, 2)


def firsts(d):
    first = np.ones(len(d), bool)
    first[1:] = d["ep"][1:] != d["ep"][:-1]
    return first


def actions(d):
    """The classes the body drew on each tick, read back off the frame it
    sent: turns and pitch changes from the frame before (from the hands'
    starting view on an episode's first tick, as body.rs decodes)."""
    first = firsts(d)
    prev_yaw = np.roll(d["f_yaw"], 1)
    prev_yaw[first] = d["t_yaw"][first]
    prev_pitch = np.roll(d["f_pitch"], 1).astype(np.int64)
    prev_pitch[first] = d["t_pitch"][first]
    pm = B.PITCH_MAX
    return {
        "buttons": ((d["buttons"][:, None] >> np.arange(B.BUTTON_BITS)) & 1).astype(np.float32),
        "move_z": np.digitize(d["f_move_z"], MZ_EDGES).astype(np.int64),
        "move_x": np.digitize(d["f_move_x"], MX_EDGES).astype(np.int64),
        "turn": turn_class(turn(prev_yaw, d["f_yaw"])),
        "pitch": np.clip(d["f_pitch"].astype(np.int64) - prev_pitch, -pm, pm) + pm,
        "sel": np.minimum(d["f_sel"], B.HOTBAR - 1).astype(np.int64),
    }


def rewards(d, goals, none16, gamma, shape, swing):
    """Each tick's reward, and the parts of it, per body."""
    first = firsts(d)
    gain = np.diff(d["awards"].astype(np.int64), prepend=0)
    died = np.diff(d["deaths"].astype(np.int64), prepend=0)
    gain[first] = 0
    died[first] = 0
    ids = [goals.index(g) + 1 for g in GATHER if g in goals]
    aimed = np.isin(d["goal"], ids) & (d["in_look"] == 2) & (d["in_yaw"] != none16)
    view = np.roll(d["f_yaw"], 1)  # the view held when the tick began
    view[first] = d["t_yaw"][first]
    off = np.abs(turn(view, d["in_yaw"])) / 32768.0
    phi = np.where(aimed, -np.minimum(d["in_dist"], 30.0) / 30.0 - off, 0.0)
    last = np.r_[first[1:], True]
    phi_next = np.where(last, phi, np.roll(phi, -1))
    facing = aimed & (d["in_dist"] < REACH_M) & (np.abs(turn(d["f_yaw"], d["in_yaw"])) < FACING)
    swung = (d["buttons"] & SWING) > 0
    shaped = shape * (gamma * phi_next - phi)
    r = gain - 5.0 * died + shaped + swing * (facing & swung)
    return r.astype(np.float32), {"awards": gain, "deaths": died, "facing": facing,
                                  "in_reach": aimed & (d["in_dist"] < REACH_M)}


def gae(r, v, ep, covered, gamma, lam):
    """Advantages and returns over each episode's covered ticks, the cut
    bootstrapped with the last value."""
    adv = np.zeros_like(r)
    idx = np.nonzero(covered)[0]
    change = np.nonzero(np.r_[True, ep[idx][1:] != ep[idx][:-1], True])[0]
    for a, b in zip(change[:-1], change[1:]):
        t = idx[a:b]
        nxt, run = v[t[-1]], 0.0
        for k in range(len(t) - 1, -1, -1):
            i = t[k]
            delta = r[i] + gamma * nxt - v[i]
            run = delta + gamma * lam * run
            adv[i] = run
            nxt = v[i]
    return adv, adv + v


def logp_entropy(z, act):
    fo = B.split_frame(z)
    zb = fo["buttons"]
    lp = (act["buttons"] * F.logsigmoid(zb) + (1 - act["buttons"]) * F.logsigmoid(-zb)).sum(-1)
    pb = torch.sigmoid(zb)
    ent = -(pb * F.logsigmoid(zb) + (1 - pb) * F.logsigmoid(-zb)).sum(-1)
    for name, _ in B.BODY_HEADS:
        ls = F.log_softmax(fo[name], -1)
        lp = lp + ls.gather(-1, act[name][..., None]).squeeze(-1)
        ent = ent - (ls.exp() * ls).sum(-1)
    return lp, ent


def play(args, it, json_path, out_dir):
    """Every island of one iteration, at once; the bench rows they wrote."""
    procs, outs = [], []
    for p in range(args.procs):
        variant = args.first + it * args.procs + p
        tick_dir = os.path.join(out_dir, f"p{p}")
        bench = os.path.join(out_dir, f"p{p}.json")
        cmd = [args.gates_agent, "--bench", str(args.minutes), "--bench-runs", "1",
               "--bench-from", str(variant), "--warp", "--bots", str(args.bots), "--scripted",
               "--body", json_path, "--body-seed", str(args.seed * 100_000 + variant),
               "--body-temp", "1", "--ticks", tick_dir, "--bench-out", bench]
        log = open(os.path.join(out_dir, f"p{p}.log"), "w")
        procs.append((subprocess.Popen(cmd, stdout=log, stderr=log), log))
        outs.append((tick_dir, bench))
    for proc, log in procs:
        if proc.wait() != 0:
            raise SystemExit(f"gates-agent failed: see {log.name}")
        log.close()
    islands = []
    for _, bench in outs:
        with open(bench) as fh:
            islands += [b["island"] for b in json.load(fh)["runs"][0]["bots"]]
    return [t for t, _ in outs], islands


def checkpoint(model, value, ck, path):
    torch.save({**{k: v for k, v in ck.items() if k not in ("state", "value")},
                "cfg": model.cfg, "state": model.state_dict(), "value": value.state_dict()}, path)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("init", help="a body.pt to start from (body.py or an earlier rl.py)")
    ap.add_argument("--out", required=True)
    ap.add_argument("--iters", type=int, default=20)
    ap.add_argument("--procs", type=int, default=4, help="islands played at once")
    ap.add_argument("--minutes", type=int, default=5)
    ap.add_argument("--bots", type=int, default=8)
    ap.add_argument("--first", type=int, default=1000, help="the first island variant played")
    ap.add_argument("--gates-agent", default="target/release/gates-agent")
    ap.add_argument("--lr", type=float, default=3e-5)
    ap.add_argument("--clip", type=float, default=0.2)
    ap.add_argument("--ppo-epochs", type=int, default=2)
    ap.add_argument("--batch", type=int, default=32, help="windows a step")
    ap.add_argument("--gamma", type=float, default=0.99)
    ap.add_argument("--lam", type=float, default=0.95)
    ap.add_argument("--ent", type=float, default=0.001)
    ap.add_argument("--vf", type=float, default=0.5)
    ap.add_argument("--bc", type=float, default=0.2, help="weight of the scripted hands' labels")
    ap.add_argument("--shape", type=float, default=1.0)
    ap.add_argument("--swing", type=float, default=0.02)
    ap.add_argument("--target-kl", type=float, default=0.03)
    ap.add_argument("--threads", type=int, default=4)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--keep-ticks", action="store_true")
    args = ap.parse_args()
    torch.manual_seed(args.seed)
    torch.set_num_threads(args.threads)
    rng = np.random.default_rng(args.seed)
    os.makedirs(args.out, exist_ok=True)

    ck = torch.load(args.init, weights_only=False)
    cfg = dict(ck["cfg"])
    model = B.Body(cfg["n_obs"], cfg["n_frame"], cfg["plan_sizes"], cfg["d"], cfg["layers"],
                   cfg["heads"], cfg["ctx"], monologue=cfg["monologue"])
    model.load_state_dict(ck["state"])
    # Its own small network on the trunk's state, detached: the value's
    # loss must not move the policy it is judging.
    value = nn.Sequential(nn.Linear(cfg["d"], 128), nn.Tanh(), nn.Linear(128, 1))
    if "value" in ck:
        value.load_state_dict(ck["value"])
    else:
        nn.init.zeros_(value[2].weight)
        nn.init.zeros_(value[2].bias)
    goals, scale, ctx = list(ck["goals"]), ck["scale"], cfg["ctx"]
    opt = torch.optim.Adam(list(model.parameters()) + list(value.parameters()), lr=args.lr)
    print(f"rl: {args.init} · {sum(p.numel() for p in model.parameters()) / 1e3:.0f}k parameters · "
          f"{args.procs} islands × {args.bots} bots × {args.minutes} min an iteration")

    for it in range(args.iters):
        t0 = time.time()
        it_dir = os.path.join(args.out, f"iter-{it:02d}")
        os.makedirs(it_dir, exist_ok=True)
        json_path = os.path.join(it_dir, "body.json")
        with open(json_path, "w") as fh:
            json.dump(model_json(model, goals, ck["actions"], scale, ck["values"]), fh)
        tick_dirs, islands = play(args, it, json_path, it_dir)
        t_play = time.time() - t0

        d, _, t = load(tick_dirs)
        # Onto the model's goal table, which its inputs index.
        to_model = np.array([0] + [goals.index(g) + 1 if g in goals else 0 for g in t["goals"]])
        d["goal"] = to_model[d["goal"]]
        tm = dict(t, goals=goals)
        data = B.Data(d, tm, scale, mono.AHEAD)
        act = actions(d)
        r, parts = rewards(d, goals, t["none16"], args.gamma, args.shape, args.swing)
        win = rust_windows(d["ep"].astype(np.int64), ctx)
        starts, offs = win[:, 0], win[:, 1]
        pos = np.arange(ctx)

        # The odds and values it played with, on the windows it played with.
        model.eval()
        old_lp = np.zeros(len(d), np.float32)
        v = np.zeros(len(d), np.float32)
        covered = np.zeros(len(d), bool)
        with torch.no_grad():
            for i in range(0, len(starts), 256):
                s, o = starts[i:i + 256], offs[i:i + 256]
                obs, fp, plan, ft, pt, pvalid, ix = data.batch(s, ctx, "cpu")
                h = model.hidden(obs, fp)
                a = {k: torch.as_tensor(x[ix]) for k, x in act.items()}
                lp, _ = logp_entropy(model.out_frame(h), a)
                vv = value(h.detach()).squeeze(-1)
                keep = pos[None, :] >= o[:, None]
                old_lp[ix[keep]] = lp.numpy()[keep]
                v[ix[keep]] = vv.numpy()[keep]
                covered[ix[keep]] = True
        adv, ret = gae(r, v, d["ep"], covered, args.gamma, args.lam)
        sel = covered
        adv_n = np.zeros_like(adv)
        adv_n[sel] = (adv[sel] - adv[sel].mean()) / (adv[sel].std() + 1e-8)

        # PPO, the scripted hands' labels beside it.
        stats = {"pg": [], "vf": [], "ent": [], "bc": [], "kl": [], "clip": []}
        stop = False
        for ep_i in range(args.ppo_epochs):
            order = rng.permutation(len(starts))
            for i in range(0, len(order), args.batch):
                b = order[i:i + args.batch]
                obs, fp, plan, ft, pt, pvalid, ix = data.batch(starts[b], ctx, "cpu")
                m = torch.as_tensor(pos[None, :] >= offs[b][:, None], dtype=torch.float32)
                a = {k: torch.as_tensor(x[ix]) for k, x in act.items()}
                h = model.hidden(obs, fp)
                fz = model.out_frame(h)
                pz = model.out_plan(h) if model.monologue else None
                lp, ent = logp_entropy(fz, a)
                olp = torch.as_tensor(old_lp[ix])
                A = torch.as_tensor(adv_n[ix])
                R = torch.as_tensor(ret[ix])
                ratio = torch.exp(lp - olp)
                surr = torch.min(ratio * A, torch.clamp(ratio, 1 - args.clip, 1 + args.clip) * A)
                n = m.sum().clamp(min=1)
                pg = -(surr * m).sum() / n
                vl = (((value(h.detach()).squeeze(-1) - R) ** 2) * m).sum() / n
                el = (ent * m).sum() / n
                bc = B.losses(model, fz, pz, ft, pt, pvalid, 0.2)
                loss = pg + args.vf * vl - args.ent * el + args.bc * bc
                opt.zero_grad()
                loss.backward()
                nn.utils.clip_grad_norm_(list(model.parameters()) + list(value.parameters()), 0.5)
                opt.step()
                with torch.no_grad():
                    kl = (((olp - lp) * m).sum() / n).item()
                    stats["clip"].append(((((ratio - 1).abs() > args.clip).float() * m).sum() / n).item())
                for k, x in (("pg", pg), ("vf", vl), ("ent", el), ("bc", bc)):
                    stats[k].append(x.item())
                stats["kl"].append(kl)
                if len(stats["kl"]) >= 5 and np.mean(stats["kl"][-5:]) > args.target_kl:
                    stop = True
                    break
            if stop:
                break

        bots = len(np.unique(d["ep"]))
        reach = parts["in_reach"].sum()
        line = {
            "iter": it,
            "island": float(np.mean(islands)),
            "scoring": int(np.sum(np.array(islands) > 0)),
            "bots": bots,
            "awards_per_bot": float(parts["awards"].sum() / bots),
            "deaths_per_bot": float(parts["deaths"].sum() / bots),
            "in_reach_per_bot": float(reach / bots),
            "facing_in_reach": float(parts["facing"].sum() / max(reach, 1)),
            "reward_per_bot": float(r.sum() / bots),
            **{k: float(np.mean(x)) for k, x in stats.items()},
            "steps": len(stats["pg"]),
            "kl_stop": stop,
            "play_s": round(t_play),
            "learn_s": round(time.time() - t0 - t_play),
        }
        with open(os.path.join(args.out, "log.jsonl"), "a") as fh:
            fh.write(json.dumps(line) + "\n")
        print(f"iter {it:2d}: island {line['island']:5.1f} ({line['scoring']}/{bots} scoring) · "
              f"awards/bot {line['awards_per_bot']:5.1f} · in reach/bot {line['in_reach_per_bot']:6.0f} "
              f"facing {line['facing_in_reach']:4.0%} · deaths/bot {line['deaths_per_bot']:.2f} · "
              f"kl {line['kl']:.4f} clip {line['clip']:.2f} ent {line['ent']:.2f} · "
              f"{line['play_s']}+{line['learn_s']}s", flush=True)
        checkpoint(model, value, ck, os.path.join(it_dir, "body.pt"))
        if not args.keep_ticks:
            for td in tick_dirs:
                shutil.rmtree(td, ignore_errors=True)
    final = os.path.join(args.out, "body.json")
    with open(final, "w") as fh:
        json.dump(model_json(model, goals, ck["actions"], scale, ck["values"]), fh)
    checkpoint(model, value, ck, os.path.join(args.out, "body.pt"))
    print(f"wrote {final}")


if __name__ == "__main__":
    main()
