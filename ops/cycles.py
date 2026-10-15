#!/usr/bin/env python3
"""Every wiped world, filed for study: one folder per wipe holding what the
shard archived and each agent's journal cut to that world's lifetime.

The shard already keeps every wipe (`crates/server/src/wipe.rs`:
`<world_file>.wipes/<unix>/` holds the world, its trust log, the teams, the
standings and a copy of the player store). An agent's journal lives
elsewhere (a buddy's is in its own state directory) and runs straight
through every wipe. This puts the two side by side:

    <out>/<unix>/
      shard/                  the shard's archive of that wipe, hard-linked
      journal-<name>.jsonl    that agent's decisions from the wipe before to this one
      cycle.json              when that world began and ended, and what is here

A folder is built under `<unix>.partial` and renamed when finished, so a
run that dies halfway is redone next time and a finished folder is never
touched again. The world still running is not filed: it has not ended.

    python3 ops/cycles.py --world /home/master/gates-data/island.world \\
        --journal doctor-wood=/var/lib/gates-buddy/doctor-wood/journal.jsonl \\
        --out /home/master/gates-data/cycles

`ops/gates-cycles.service` runs it daily. Nothing here feeds the standings,
matchmaking or what anyone is shown: it is a record to read, never a score.
"""
import argparse
import json
import os
import shutil
import sys
import time
from pathlib import Path


def wipes(world):
    """The wipe times the shard has archived, oldest first."""
    root = Path(f"{world}.wipes")
    if not root.is_dir():
        return []
    return sorted(int(p.name) for p in root.iterdir() if p.is_dir() and p.name.isdigit())


def link_or_copy(src, dst):
    """A wipe's archive is never written again, so a hard link is a safe copy
    that costs no disk. Another filesystem gets a real copy."""
    try:
        os.link(src, dst)
    except OSError:
        shutil.copy2(src, dst)


def file_cycle(world, out, at, before, journals):
    final = out / str(at)
    if final.exists():
        return False
    work = out / f"{at}.partial"
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    shutil.copytree(Path(f"{world}.wipes") / str(at), work / "shard", copy_function=link_or_copy)
    counts = {}
    for name, path in journals:
        n = 0
        with open(work / f"journal-{name}.jsonl", "w") as dst:
            try:
                src = open(path)
            except OSError as e:
                print(f"journal {name}: {e}", file=sys.stderr)
                src = None
            if src:
                with src:
                    for line in src:
                        try:
                            t = json.loads(line).get("t")
                        except ValueError:
                            continue  # a torn last line from a killed agent
                        if isinstance(t, (int, float)) and (before is None or t >= before) and t < at:
                            dst.write(line if line.endswith("\n") else line + "\n")
                            n += 1
        counts[name] = {"rows": n, "source": str(path)}
    files = sorted(str(p.relative_to(work / "shard")) for p in (work / "shard").rglob("*") if p.is_file())
    meta = {
        "began_at": before,
        "wiped_at": at,
        "wiped_at_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(at)),
        "world_file": str(world),
        "shard_files": files,
        "journals": counts,
    }
    (work / "cycle.json").write_text(json.dumps(meta, indent=1) + "\n")
    work.rename(final)
    return True


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--world", required=True, help="the shard's world_file")
    ap.add_argument("--out", required=True, help="where the cycles are filed")
    ap.add_argument("--journal", action="append", default=[], metavar="NAME=PATH",
                    help="an agent's GATES_JOURNAL; repeat for each agent")
    a = ap.parse_args()
    journals = []
    for j in a.journal:
        name, sep, path = j.partition("=")
        if not sep or not name or not path:
            ap.error(f"--journal wants NAME=PATH, got {j!r}")
        journals.append((name, Path(path)))
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    times = wipes(a.world)
    filed = 0
    for i, at in enumerate(times):
        before = times[i - 1] if i else None
        if file_cycle(a.world, out, at, before, journals):
            filed += 1
            print(f"filed the world wiped at {at}", file=sys.stderr)
    print(f"{len(times)} wipes on record, {filed} filed now", file=sys.stderr)


if __name__ == "__main__":
    main()
