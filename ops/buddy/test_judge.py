#!/usr/bin/env python3
"""Checks for the journal judge's no-model half (`judge.py --stats`, the
digest it would send): no network, no model.

    python3 ops/buddy/test_judge.py
"""
import json
import os
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import judge  # noqa: E402

FAILED = []


def check(name, ok):
    print(("ok   " if ok else "FAIL ") + name)
    if not ok:
        FAILED.append(name)


now = time.time()
drone = [{"t": now - 3600 + i * 60, "goal": "gather_wood", "reason": "trees", "wallet": "0x" + "1" * 40}
         for i in range(60)]
mind = [{"t": now - 3600 + i * 60, "goal": ["gather_wood", "build", "craft:Bow", "raid"][i % 4],
         "reason": "plan", "name": "Ryuzen", "deaths": i // 30,
         **({"thought": "the ridge is mine"} if i % 6 == 0 else {}),
         **({"strategy": {"ambition": "King", "project": f"p{i // 20}"}} if i % 20 == 0 else {}),
         **({"say": "door code 4821, come home", "to": "team"} if i == 7 else {})}
        for i in range(60)]
d = tempfile.mkdtemp(prefix="judge-test-")
for name, rows in (("a.jsonl", drone), ("b.jsonl", mind + [{"t": now - 30 * 3600, "goal": "old", "name": "Ryuzen"}])):
    with open(os.path.join(d, name), "w") as f:
        f.write("\n".join(json.dumps(r) for r in rows) + "\nnot json\n")
bodies = judge.read([os.path.join(d, "*.jsonl")], now - 24 * 3600)
check("bodies by name, else wallet; old and torn lines left out",
      sorted(bodies) == ["0x" + "1" * 40, "Ryuzen"] and len(bodies["Ryuzen"]) == 60)
s0, s1 = judge.stats(bodies["0x" + "1" * 40]), judge.stats(bodies["Ryuzen"])
check("a drone: one goal, zero entropy, all of its time", s0["goal_entropy_bits"] == 0 and s0["top_goal_share"] == 1)
check("a mind: four kinds, two bits, thoughts, three strategies, a death",
      s1["goal_kinds"] == 4 and s1["goal_entropy_bits"] == 2.0 and s1["thoughts_per_hour"] > 9
      and s1["strategy_changes"] == 3 and s1["deaths"] == 1)
dig = json.dumps(judge.digest(bodies["Ryuzen"]))
check("the digest keeps every thought and strategy and hides codes said on team chat",
      dig.count("the ridge is mine") == 10 and dig.count('"project"') == 3 and "4821" not in dig and "####" in dig)
check("the schema asks for every axis", set(judge.schema()["properties"]["scores"]["required"]) == set(judge.AXES))
print()
if FAILED:
    print(f"{len(FAILED)} FAILED")
    sys.exit(1)
print("all judge checks pass")
