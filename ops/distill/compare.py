#!/usr/bin/env python3
"""Teacher against student, each on its own local island at the same time:
jev-bot's end-of-run counters, summed over the bots.

    python3 ops/distill/compare.py student.json --seconds 600 --bots 4
    python3 ops/distill/compare.py student.json -- \\
        --external python3 crates/server/examples/claude_agent.py

Everything after `--` is the teacher's mind flags (default `--scripted`); a
model teacher bills its calls as usual. Standard library only.
"""
import argparse
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
COUNTS = {
    "decisions": r"mind [^:]+: \d+ requests, (\d+) decisions",
    "failed decisions": r"decisions, (\d+) failures",
    "deaths": r"survival: (\d+) deaths",
    "goals done": r"goals (\d+) done",
    "goals failed": r"done, (\d+) failed",
    "goals interrupted": r"failed, (\d+) interrupted",
    "crafted": r"crafted (\d+)",
    "eaten": r"eaten (\d+)",
    "drinks": r"drinks (\d+)",
    "pieces placed": r"(\d+) placed",
}


def tally(text):
    t = {k: 0 for k in COUNTS}
    t["gathered"] = 0
    for line in text.splitlines():
        for k, pat in COUNTS.items():
            m = re.search(pat, line)
            if m:
                t[k] += int(m.group(1))
        m = re.search(r"gathered: (.*)", line)
        if m:
            t["gathered"] += sum(int(n) for n in re.findall(r" (\d+)(?:,|$)", m.group(1)))
    goals = t["goals done"] + t["goals failed"] + t["goals interrupted"]
    t["share of goals done"] = f"{t['goals done'] / goals:.0%}" if goals else "-"
    return t


def main():
    argv = sys.argv[1:]
    teacher = ["--scripted"]
    if "--" in argv:
        i = argv.index("--")
        argv, teacher = argv[:i], argv[i + 1:]
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("student", help="a file ops/distill/train.py wrote")
    ap.add_argument("--seconds", type=int, default=600)
    ap.add_argument("--bots", type=int, default=4)
    ap.add_argument("--bin", default=os.path.join(ROOT, "target", "release", "jev-bot"))
    args = ap.parse_args(argv)
    if not os.path.exists(args.bin):
        subprocess.run(["cargo", "build", "--release", "-p", "server", "--bin", "jev-bot"], cwd=ROOT, check=True)
    base = [args.bin, "--local", "--seconds", str(args.seconds), "--bots", str(args.bots)]
    # `--external` must be jev-bot's last flag, so the mind's flags go last.
    runs = {"teacher": base + teacher, "student": base + ["--student", args.student]}
    procs = {k: subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
             for k, cmd in runs.items()}
    out = {}
    for k, p in procs.items():
        text, _ = p.communicate()
        if p.returncode:
            sys.exit(f"compare: the {k} run failed ({p.returncode}): {' '.join(runs[k])}")
        out[k] = tally(text)
    print(f"{args.bots} bots each, {args.seconds} s; teacher {' '.join(teacher)}")
    print(f"{'':22}{'teacher':>10}{'student':>10}")
    for k in list(COUNTS) + ["gathered", "share of goals done"]:
        print(f"{k:22}{out['teacher'][k]!s:>10}{out['student'][k]!s:>10}")


if __name__ == "__main__":
    main()
