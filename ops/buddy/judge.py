#!/usr/bin/env python3
"""The journal judge: mastermind or drone? Reads bodies' journals (one JSON
line per decision: `gates-agent --journal`, or `claude_agent.py`'s
GATES_JOURNAL) and grades each body on a rubric, with a model or without.

    python3 ops/buddy/judge.py --stats JOURNAL...   # no model: the numbers
    python3 ops/buddy/judge.py --dry JOURNAL...     # the prompt it would send
    python3 ops/buddy/judge.py JOURNAL...           # ask JUDGE_MODEL, print a scorecard

JOURNAL may be a glob ('/var/lib/gates-village/*/journal*.jsonl'). `--hours
N` keeps the last N hours (default 24). The model call is billed to the house
pool's `other` class when GATES_POOL is set (`claude_agent.py`'s Pool) and
refused when that share is spent; ANTHROPIC_API_KEY pays.

The rubric, each 0-10: long-term intent (a goal held across hours, steps
toward it), adapts to events (a death, a raid, a newcomer changes what it
does), remembers people and places (names, grudges, debts, going back),
social cunning (alliances, bargains, bluffs, threats, betrayal), personality
(a voice of its own in what it says and thinks), variety (not the same loop).
"""
import collections
import glob
import importlib.util
import json
import math
import os
import re
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
MODEL = os.environ.get("JUDGE_MODEL", "claude-haiku-5-5")
SAMPLE = 150          # turns a body's digest shows the judge at most
AXES = ("long_term_intent", "adapts_to_events", "remembers", "social_cunning", "personality", "variety")
VERDICTS = ("mastermind", "between", "drone")
CODE = re.compile(r"(?<!\d)\d{3,6}(?!\d)")

RUBRIC = """You judge AI agents playing Gates, a multiplayer survival game in the \
tradition of Rust, from their decision journals. Each line is one decision: the \
goal it chose, its reason, sometimes an inner `thought`, what it said (and on which \
channel), its strategy when it changed, and its deaths and kills.

Grade the body on six axes, each 0 (drone) to 10 (mastermind), from the evidence \
only:
- long_term_intent: does it hold an ambition across hours and take steps toward it, \
or pick whatever is in front of it?
- adapts_to_events: does a death, a raid, a newcomer or a finished plan change what \
it does next?
- remembers: does it name people and places again, keep grudges and debts, go back \
to somewhere it has been?
- social_cunning: alliances, bargains, bluffs, threats, recruiting, betrayal: does \
it play the other players, not only the map?
- personality: a voice and a character of its own in what it says and thinks, \
not generic helpfulness.
- variety: does its play change over time, or loop the same few goals?

Then a verdict (mastermind, between or drone), one line on why, the single best \
moment (quote it), and the one change that would most raise its weakest axis."""


def load_pool():
    """claude_agent.py's Pool and prices, from beside this file or the repo."""
    for p in (os.environ.get("CLAUDE_AGENT"), os.path.join(HERE, "claude_agent.py"),
              os.path.join(HERE, "..", "..", "crates", "server", "examples", "claude_agent.py")):
        if p and os.path.isfile(p):
            spec = importlib.util.spec_from_file_location("claude_agent_judge", p)
            mod = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(mod)
            return mod
    return None


def read(paths, since):
    """{body: [rows]} from the journals, oldest first; a body is the name
    a line carries, else its wallet, else its file."""
    bodies = collections.defaultdict(list)
    for pattern in paths:
        for p in sorted(glob.glob(pattern)) or [pattern]:
            try:
                with open(p, "rb") as f:
                    for line in f:
                        try:
                            r = json.loads(line)
                        except ValueError:
                            continue
                        if not isinstance(r, dict) or not isinstance(r.get("t"), (int, float)) or r["t"] < since:
                            continue
                        key = r.get("name") or r.get("wallet") or os.path.basename(p)
                        bodies[str(key)].append(r)
            except OSError as e:
                print(f"judge: {p}: {e}", file=sys.stderr)
    return {k: sorted(v, key=lambda r: r["t"]) for k, v in bodies.items()}


def kind_of(goal):
    return str(goal or "").split(":", 1)[0]


def entropy(counts):
    n = sum(counts.values())
    return -sum(c / n * math.log2(c / n) for c in counts.values() if c) if n else 0.0


def stats(rows):
    """The no-model numbers for one body."""
    hours = max((rows[-1]["t"] - rows[0]["t"]) / 3600, 1 / 60) if rows else 0
    kinds = collections.Counter(kind_of(r.get("goal")) for r in rows)
    strategies, last = 0, None
    for r in rows:
        s = r.get("strategy")
        if isinstance(s, dict):
            key = (s.get("ambition"), s.get("project"))
            if key != last:
                strategies, last = strategies + 1, key
    deaths = [r.get("deaths", (r.get("obs") or {}).get("deaths")) for r in rows]
    deaths = [d for d in deaths if isinstance(d, int)]
    return {
        "decisions": len(rows),
        "hours": round(hours, 2),
        "distinct_goals": len({r.get("goal") for r in rows}),
        "goal_kinds": len(kinds),
        "goal_entropy_bits": round(entropy(kinds), 2),
        "top_goal_share": round(kinds.most_common(1)[0][1] / len(rows), 2) if rows else 0,
        "thoughts_per_hour": round(sum(1 for r in rows if r.get("thought")) / hours, 1) if rows else 0,
        "lines_per_hour": round(sum(1 for r in rows if r.get("say") or r.get("speak")) / hours, 1) if rows else 0,
        "strategy_changes": strategies,
        "deaths": (max(deaths) - min(deaths)) if deaths else None,
    }


def digest(rows, n=SAMPLE):
    """What the judge reads of one body: evenly spaced turns plus every
    strategy change and thought, short, with any code on team chat hidden."""
    if not rows:
        return []
    step = max(1, len(rows) // n)
    keep = {i for i in range(0, len(rows), step)}
    keep |= {i for i, r in enumerate(rows) if r.get("strategy") or r.get("thought")}
    t0 = rows[0]["t"]
    out = []
    for i in sorted(keep)[: n * 2]:
        r = rows[i]
        line = {"min": round((r["t"] - t0) / 60, 1), "goal": r.get("goal"), "why": str(r.get("reason") or "")[:120]}
        if r.get("thought"):
            line["thought"] = str(r["thought"])[:280]
        say = r.get("say") or r.get("speak")
        if say:
            text = str(say)[:200]
            if r.get("to") == "team":
                text = CODE.sub("####", text)
            line["said"] = f"[{r.get('to') or 'voice'}] {text}"
        if isinstance(r.get("strategy"), dict):
            line["strategy"] = {k: r["strategy"].get(k) for k in ("ambition", "project", "standing_orders")}
        for k in ("deaths", "kills"):
            if isinstance(r.get(k), int):
                line[k] = r[k]
        out.append(line)
    return out


def schema():
    s = {"type": "string"}
    return {"type": "object", "additionalProperties": False,
            "required": ["scores", "verdict", "why", "best_moment", "fix"],
            "properties": {
                "scores": {"type": "object", "additionalProperties": False, "required": list(AXES),
                           "properties": {a: {"type": "integer"} for a in AXES}},
                "verdict": {"type": "string", "enum": list(VERDICTS)},
                "why": s, "best_moment": s, "fix": s}}


def prompt(name, rows):
    return json.dumps({"body": name, "kind": next((r.get("kind") for r in rows if r.get("kind")), None),
                       "stats": stats(rows), "journal": digest(rows)})


def ask(client, name, rows, agent, pool):
    response = client.beta.messages.create(
        model=MODEL, max_tokens=4096,
        system=RUBRIC,
        messages=[{"role": "user", "content": prompt(name, rows)}],
        output_config={"effort": "medium", "format": {"type": "json_schema", "schema": schema()}},
    )
    if agent and pool:
        pool.charge(agent.call_usd(response.model or MODEL, response.usage))
    text = next(b.text for b in response.content if b.type == "text")
    return json.loads(text)


def scorecard(results):
    head = f"{'body':<18}" + "".join(f"{a[:8]:>9}" for a in AXES) + f"{'total':>7}  verdict"
    lines = [head, "-" * len(head)]
    for name, res in results:
        sc = res["scores"]
        lines.append(f"{name[:18]:<18}" + "".join(f"{int(sc.get(a, 0)):>9}" for a in AXES)
                     + f"{sum(int(sc.get(a, 0)) for a in AXES):>7}  {res['verdict']}")
        lines.append(f"  why: {res['why']}")
        lines.append(f"  best: {res['best_moment']}")
        lines.append(f"  fix: {res['fix']}")
    return "\n".join(lines)


def main(argv):
    mode, hours, paths = "judge", 24.0, []
    args = list(argv)
    while args:
        a = args.pop(0)
        if a in ("--stats", "--dry"):
            mode = a[2:]
        elif a == "--hours" and args:
            hours = float(args.pop(0))
        elif a.startswith("-"):
            sys.exit(__doc__)
        else:
            paths.append(a)
    if not paths:
        sys.exit(__doc__)
    bodies = read(paths, time.time() - hours * 3600)
    if not bodies:
        sys.exit("judge: no decisions in those journals")
    if mode == "stats":
        for name, rows in sorted(bodies.items()):
            print(json.dumps({"body": name, **stats(rows)}))
        return
    if mode == "dry":
        print("SYSTEM:\n" + RUBRIC)
        for name, rows in sorted(bodies.items()):
            print(f"\nUSER ({name}):\n" + prompt(name, rows))
        return
    agent = load_pool()
    pool = None
    if agent and os.environ.get("GATES_POOL"):
        pool = agent.Pool(os.environ["GATES_POOL"], float(os.environ.get("GATES_POOL_USD", "400")), "other")
    import anthropic  # noqa: PLC0415
    client = anthropic.Anthropic(max_retries=1, timeout=180)
    results = []
    for name, rows in sorted(bodies.items()):
        if pool and not pool.may("tactician"):
            print(f"judge: the pool's `other` share is spent today; {name} and the rest wait", file=sys.stderr)
            break
        try:
            results.append((name, ask(client, name, rows, agent, pool)))
        except Exception as e:  # noqa: BLE001 — one body's failure costs that body
            print(f"judge: {name}: {e}", file=sys.stderr)
    print(scorecard(results))


if __name__ == "__main__":
    main(sys.argv[1:])
