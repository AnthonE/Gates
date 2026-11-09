#!/usr/bin/env python3
"""Have Claude design a student from the scripted policy's source: no data, no training.

The designed arm of the open-heart test, after Takehi et al. 2026's
experiment 5 ("designed, not grown"). Claude gets the scripted policy's Rust
source, the feature and label names a student may use, and the student
format. It submits, once, two Python functions:

- build(features, labels) -> {"features", "mean", "scale", "labels", "layers"}:
  the network, constructed by hand (no data, no fitting, no forward passes);
- rule(x, options) -> goal: the policy it says the network implements,
  in plain code.

The harness runs both (in a scrubbed subprocess), writes a gates-student-1
file, and measures fidelity: how often the network's pick equals its own
declared rule on every recorded state of the teacher set. That number is the
point. A grown student has no declared rule to hold it to; a designed one
does, and fidelity says how far the weights keep the declaration. Teacher
agreement is reported beside it, and the bench (gates-agent --bench --warp
--student) scores it in play against the scripted policy and grown students.

    python3 ops/openheart/design.py ops/distill/data/opus-2026-10-21 -o ops/openheart/runs/design1
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from plant import Student, taught, teacher_rows  # noqa: E402

MODEL = "claude-opus-5-5"
PROMPT_VERSION = "openheart-design-1"
MIND = os.path.join(HERE, "..", "..", "crates", "server", "src", "mind.rs")

SYSTEM = """You are designing a small neural network by hand: no training data, no gradient descent, no running it.

The network is a decision policy ("student") for an AI player of Gates, a multiplayer survival game. At each \
decision the player is offered a list of goals and the student picks one. Your design should play the way the \
scripted policy below plays, as far as a stateless network over the given features can.

The student format (the game loads it as is):
- "features": a list of input names, each from the FEATURES list given. A name with "=" is a binary condition \
(1 when it holds, absent = 0); other names are numbers.
- "mean", "scale": per feature; the input is h0 = (x - mean) * scale.
- "layers": a list of {"in", "out", "w", "b"}; h = h_prev @ W + b with W row-major [in, out], ReLU on every layer \
but the last.
- "labels": one per output unit, each from the LABELS list given. The player takes the highest-scoring offered goal; \
an offered goal with no exact label scores as its kind ("craft:*", "go:*", "walk:*") when that label exists.

Submit, once, Python 3 source (numpy allowed) defining two functions:
- build(features, labels) -> dict with keys features, mean, scale, labels, layers (lists of floats, JSON-able). \
features and labels are the full lists given below; use any subset, in any order, and any layer sizes.
- rule(x, options) -> str: the policy your network implements, written as plain code. x is a dict of feature \
name -> value (absent = 0), options the offered goal names; return one of the options.
The harness will check how often the network's pick equals rule's pick on real game states. Neither function may \
read files or the network."""

SUBMIT = {
    "name": "submit",
    "description": "Your design. Call once.",
    "strict": True,
    "input_schema": {
        "type": "object", "additionalProperties": False,
        "required": ["source", "notes"],
        "properties": {
            "source": {"type": "string", "description": "Python defining build() and rule()."},
            "notes": {"type": "string", "description": "What the network computes, in a few lines."},
        },
    },
}

RUNNER = r"""
import json, sys
src, features, labels, states = json.load(sys.stdin)
ns = {}
exec(compile(src, "design", "exec"), ns)
net = ns["build"](list(features), list(labels))
picks = [ns["rule"](dict(x), list(o)) for x, o in states]
print(json.dumps({"net": net, "rule_picks": picks}, default=float))
"""


def scripted_source():
    with open(MIND) as f:
        lines = f.read().splitlines()
    a = next(i for i, l in enumerate(lines) if l.startswith("pub struct Scripted"))
    b = next(i for i, l in enumerate(lines) if l.startswith("pub struct History"))
    return "\n".join(lines[a - 3:b])


def vocabulary(rows):
    feats = sorted({k for r in rows for k in r["x"]})
    labels = set()
    for r in rows:
        labels.update(r["options"])
        labels.add(r["goal"])
    kinds = {l.split(":", 1)[0] + ":*" for l in labels if ":" in l}
    return feats, sorted(labels | kinds)


def ask(feats, labels, args):
    import anthropic
    client = anthropic.Anthropic(max_retries=4)
    task = ("SCRIPTED POLICY (crates/server/src/mind.rs):\n```rust\n" + scripted_source() + "\n```\n\n"
            f"FEATURES ({len(feats)}):\n" + "\n".join(feats) + f"\n\nLABELS ({len(labels)}):\n" + "\n".join(labels)
            + "\n\nDesign the network and call submit.")
    messages = [{"role": "user", "content": task}]
    log = {"prompt_version": PROMPT_VERSION, "requested_model": args.model, "effort": args.effort, "turns": []}
    for _ in range(3):
        with client.messages.stream(model=args.model, max_tokens=128000, system=SYSTEM, tools=[SUBMIT],
                                    messages=messages, thinking={"type": "adaptive"},
                                    output_config={"effort": args.effort}) as stream:
            msg = stream.get_final_message()
        log["turns"].append({"served_model": msg.model, "stop_reason": msg.stop_reason,
                             "usage": msg.usage.model_dump()})
        if msg.stop_reason == "refusal":
            log["ended"] = "refusal"
            return None, log
        sub = next((b for b in msg.content if b.type == "tool_use" and b.name == "submit"), None)
        if sub and isinstance(sub.input, dict) and isinstance(sub.input.get("source"), str):
            log["ended"], log["notes"] = "submitted", sub.input.get("notes")
            return sub.input["source"], log
        messages += [{"role": "assistant", "content": msg.content},
                     {"role": "user", "content": "Call the submit tool with your design now."}]
    log["ended"] = "no submit"
    return None, log


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("teacher", help="recordings: their states are where fidelity is measured")
    ap.add_argument("-o", "--out", required=True)
    ap.add_argument("--model", default=MODEL)
    ap.add_argument("--effort", default="high", choices=("low", "medium", "high", "xhigh", "max"))
    ap.add_argument("--source", help="score an existing design.py source file instead of asking")
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    rows = teacher_rows([args.teacher])
    feats, labels = vocabulary(rows)
    if args.source:
        src, log = open(args.source).read(), {"ended": "given"}
    else:
        src, log = ask(feats, labels, args)
        if src:
            with open(os.path.join(args.out, "design_source.py"), "w") as f:
                f.write(src)
    if not src:
        json.dump(log, open(os.path.join(args.out, "design_log.json"), "w"), indent=1)
        sys.exit(f"no design: {log['ended']}")
    states = [(r["x"], r["options"]) for r in rows]
    with tempfile.TemporaryDirectory() as tmp:
        p = subprocess.run([sys.executable, "-I", "-c", RUNNER], input=json.dumps([src, feats, labels, states]),
                           capture_output=True, text=True, cwd=tmp, timeout=600,
                           env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "HOME": tmp})
    if p.returncode:
        log["ended"] = "build failed"
        log["stderr"] = p.stderr[-4000:]
        json.dump(log, open(os.path.join(args.out, "design_log.json"), "w"), indent=1)
        sys.exit("the design did not run:\n" + p.stderr[-2000:])
    res = json.loads(p.stdout)
    net = {"format": "gates-student-1", **{k: res["net"][k] for k in ("features", "mean", "scale", "labels", "layers")}}
    st = Student(net)
    picks = [st.pick(x, o) for x, o in states]
    fid = sum(a == b for a, b in zip(picks, res["rule_picks"])) / len(states)
    agree = sum(a == taught(r) for a, r in zip(picks, rows)) / len(rows)
    net["check"] = {"obs": rows[0]["obs"], "scores": [round(float(v), 5) for v in st.scores(rows[0]["x"])]}
    path = os.path.join(args.out, "student-designed.json")
    with open(path, "w") as f:
        json.dump(net, f, separators=(",", ":"))
    n_w = sum(l["in"] * l["out"] + l["out"] for l in net["layers"])
    log.update({"fidelity_to_own_rule": round(fid, 4), "teacher_agreement": round(agree, 4),
                "states": len(states), "weights": n_w})
    json.dump(log, open(os.path.join(args.out, "design_log.json"), "w"), indent=1)
    print(f"{path}: {n_w} weights; the net keeps its own declared rule on {fid:.1%} of {len(states)} states; "
          f"agrees with the teacher on {agree:.1%}")
    print("Note: check.obs is a recorded observation; the game re-featurizes it at load.")


if __name__ == "__main__":
    main()
