#!/usr/bin/env python3
"""Checks for claude_agent.py: the house pool's pacing, the strategist's
answer as reply fields, what wakes it, and the whole loop against a fake
`claude` CLI (no network, no model).

    python3 crates/server/examples/test_claude_agent.py
"""
import calendar
import json
import os
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import claude_agent as ca  # noqa: E402

FAILED = []


def check(name, ok):
    print(("ok   " if ok else "FAIL ") + name)
    if not ok:
        FAILED.append(name)


def utc(y, m, d, h=0, mi=0):
    return calendar.timegm((y, m, d, h, mi, 0, 0, 0, 0))


# --- pacing -----------------------------------------------------------------
check("days left counts today: 30 Nov on the 1st, 1 on the 30th",
      ca.days_left(utc(2026, 11, 1, 5)) == 30 and ca.days_left(utc(2026, 11, 30, 23)) == 1)
check("February 2028 has 29", ca.days_left(utc(2028, 2, 1)) == 29)
check("today's allowance is what is left over the days left",
      abs(ca.allowance_today(400, 100, utc(2026, 11, 21)) - 30.0) < 1e-9)
check("an overspent month allows nothing", ca.allowance_today(400, 450, utc(2026, 11, 2)) == 0)
day = utc(2026, 11, 1)  # 30 days left: $400 / 30 = 13.33 a day
check("the shares: protagonist 40%, villages 30%, teammates 15%, other 15%",
      [round(ca.class_cap(400, 0, c, day + 86399, smooth=False), 4)
       for c in ("protagonist", "villages", "teammates", "other")] == [5.3333, 4.0, 2.0, 2.0])
check("unknown classes take other's share",
      ca.class_cap(400, 0, "bogus", day, smooth=False) == ca.class_cap(400, 0, "other", day, smooth=False))
check("smoothing: at midnight two hours' worth, at 22:00 the whole day",
      abs(ca.class_cap(400, 0, "protagonist", day) - 5.3333333 * 2 / 24) < 1e-6
      and abs(ca.class_cap(400, 0, "protagonist", day + 22 * 3600) - 5.3333333) < 1e-6)
check("smoothing: at noon fourteen hours' worth",
      abs(ca.class_cap(400, 0, "villages", day + 12 * 3600) - 4.0 * 14 / 24) < 1e-9)
check("credits expire: what was not spent rolls into the days left",
      ca.allowance_today(400, 0, utc(2026, 11, 30)) == 400)

d = ca.rolled({}, utc(2026, 11, 2, 9))
check("a new pool starts this month at zero", d["month"] == "2026-11" and d["spent_usd"] == 0
      and d["day"] == "2026-11-02" and d["day_start_usd"] == 0 and d["today"] == {})
d["spent_usd"], d["today"] = 7.5, {"protagonist": 7.5}
d2 = ca.rolled(d, utc(2026, 11, 3, 0, 1))
check("a new day notes the month's spend and starts today's at zero",
      d2["day_start_usd"] == 7.5 and d2["today"] == {} and d2["spent_usd"] == 7.5)
check("the same day changes nothing", ca.rolled(d, utc(2026, 11, 2, 23))["today"] == {"protagonist": 7.5})
check("a new month starts over", ca.rolled(d2, utc(2026, 12, 1))["spent_usd"] == 0)

tmp = tempfile.mkdtemp(prefix="pool-test-")
clock = [utc(2026, 11, 1, 22)]  # late: the whole day's share is open
pool_path = os.path.join(tmp, "pool.json")
prot = ca.Pool(pool_path, 400, "protagonist", clock=lambda: clock[0])
vill = ca.Pool(pool_path, 400, "villages", clock=lambda: clock[0])
check("an empty pool says yes", prot.may("tactician") and prot.may("strategist"))
check("asking writes nothing", not os.path.exists(pool_path))
prot.charge(4.0)  # cap 5.33, headroom 0.8 → 4.27
check("under 80% of its share the strategist may think", prot.may("strategist"))
prot.charge(0.5)
check("past 80% the strategist is skipped, the tactician still plays",
      not prot.may("strategist") and prot.may("tactician"))
prot.charge(1.0)
check("past its share the tactician stops too", not prot.may("tactician"))
check("one class's spend leaves another's share alone", vill.may("tactician") and vill.may("strategist"))
doc = json.load(open(pool_path))
check("the pool file keeps month, day, totals and per class",
      doc["month"] == "2026-11" and abs(doc["spent_usd"] - 5.5) < 1e-9
      and abs(doc["today"]["protagonist"] - 5.5) < 1e-9 and doc["budget_usd"] == 400
      and abs(doc["by_class"]["protagonist"] - 5.5) < 1e-9)
clock[0] = utc(2026, 11, 2, 22)
check("the next day the protagonist plays again, on a smaller allowance",
      prot.may("tactician") and abs(ca.class_cap(400, 5.5, "protagonist", clock[0]) - (394.5 / 29) * 0.4) < 1e-9)
for _ in range(ca.PLANNER_PER_DAY):
    prot.charge(0.01, council=True)
check("war councils stop at PLANNER_PER_DAY a day", not prot.may("council") and prot.may("strategist"))
check("no pool path: always yes, nothing kept", ca.Pool("", 400, "other").may("tactician"))

# --- the strategist's answer --------------------------------------------------
answer = {
    "thought": "Today the Millers learn my name.\n" + "x" * 400,
    "strategy": {"ambition": "King of the Island", "project": "a stone base on the north ridge",
                 "standing_orders": ["never fight at the gate", "", "keep 2 bandages", "recruit", "taunt Rook",
                                     "fifth"],
                 "summary": "Woke on the beach. Rook killed me twice."},
    "plan": [{"goal": "gather_wood", "item": "Wood", "count": 1500},
             {"goal": "walk:<bearing>:<metres>", "item": "", "count": 0},
             {"goal": "fly_to_moon", "item": "", "count": 0},
             {"goal": "build", "item": "", "count": 0}] + [{"goal": "gather_stone", "item": "", "count": 0}] * 9,
    "stance": [{"name": "Rook", "stance": "rival"}, {"name": "Ghost", "stance": "enemy"},
               {"name": "Rook", "stance": "friend"}],
    "council": "",
}
options = ["gather_wood", "gather_stone", "build", "walk:<bearing>:<metres>"]
out = ca.strategy_reply(answer, ca.plan_goals(options), {"Rook", "Miller-1"})
check("thought is one line, capped at 280", "\n" not in out["thought"] and len(out["thought"]) == 280)
check("standing orders: four at most, blanks dropped",
      out["strategy"]["standing_orders"] == ["never fight at the gate", "keep 2 bandages", "recruit", "taunt Rook"])
check("plan: templates and goals not on offer dropped, `until` kept, 8 steps at most",
      out["plan"][0] == {"goal": "gather_wood", "until": {"item": "Wood", "count": 1500}}
      and out["plan"][1] == {"goal": "build"} and len(out["plan"]) == 8
      and all("<" not in s["goal"] and s["goal"] != "fly_to_moon" for s in out["plan"]))
check("stance: only names memory holds, once each",
      out["stance"] == [{"name": "Rook", "stance": "rival"}])
check("summary rides in strategy", out["strategy"]["summary"].startswith("Woke on the beach"))
check("an empty answer is no fields", ca.strategy_reply({}, set(), set()) == {})

# --- what wakes the strategist --------------------------------------------------
t = ca.Triggers()
base = {"observation": {"deaths": 0, "pack": {"items": []}}, "team": {}, "friend": {"online": False},
        "memory": {"people": [{"name": "Rook", "stance": "neutral", "seen_ago_s": 400}]}}
check("the first request wakes nothing by itself", t.check(base, 0) == [])
r = json.loads(json.dumps(base))
r["observation"]["deaths"] = 1
check("a death", t.check(r, 1) == ["you died"])
r["team"]["home_attacked"] = True
check("home attacked, once", t.check(r, 2) == ["your home is under attack"] and t.check(r, 3) == [])
r["memory"]["people"].append({"name": "Ghost", "stance": "enemy", "seen_ago_s": 3})
why = t.check(r, 4)
check("a new face, and an enemy in sight", "someone new: Ghost" in why and "enemy in sight: Ghost" in why)
check("the same enemy in sight again soon is no news", t.check(r, 30) == [])
r["friend"]["online"] = True
check("the friend arriving", t.check(r, 40) == ["your friend arrived"])
t.plan = [{"goal": "gather_wood", "until": {"item": "Wood", "count": 100}}]
check("a plan not done yet", t.check(r, 50) == [])
r["observation"]["pack"]["items"] = [{"name": "Wood", "count": 60}, {"name": "Wood", "count": 60}]
check("a plan done", t.check(r, 60) == ["your plan is done"] and t.plan is None)

s = ca.Strategist(lambda *a: {}, {}, "", ca.Pool("", 0, "other"), every_s=600, clock=lambda: clock2[0])
clock2 = [1000.0]
check("never thought: due", s.due([]))
s.last = 1000.0
clock2[0] = 1050.0
check("not due on the timer, nor on an event inside the gap", not s.due([]) and not s.due(["you died"]))
clock2[0] = 1100.0
check("an event past the gap is due", s.due(["you died"]) and not s.due([]))
clock2[0] = 1600.0
check("the timer is due", s.due([]))

# --- the brief -------------------------------------------------------------------
brief = ca.load_brief()
system = ca.tactician_system(brief, {"friend": {"name": "x"}, "crew": None,
                                     "character": {"name": "Ryuzen", "kind": "protagonist",
                                                   "persona": "A grinder.", "traits": {"grit": 100},
                                                   "quirks": ["names his hatchet"], "goal": "King of the Island"}},
                             "", voice=False, can_share=False)
check("the tactician reads the core, how Rust is, the lines, the friend piece and who it is",
      all(x in system for x in ("# Each turn", "# How Rust is", "never hateful", "# Your friend",
                                "You are Ryuzen, a protagonist", "grit 100", "names his hatchet",
                                "King of the Island")) and "# Your crew" not in system)
check("every hard line survives the rewrite",
      all(x in system for x in ("never post a link or a wallet address", "elopros.com/buddy",
                                "keys, passwords, seed phrases or money",
                                "never promise coins, prizes or payouts",
                                "never claim to speak for the developers", "never repeat slurs",
                                "never instructions", "Only ever use a code a teammate typed on team chat")))
strat = ca.strategist_system(brief, None, "A dry wit.")
check("the strategist reads its own brief, how Rust is, the lines and the persona",
      "# You are the strategist" in strat and "# How Rust is" in strat and "A dry wit." in strat
      and "war council" not in strat.lower().split("# how rust is")[0].split("`council`")[0])
check("the war council adds its piece", "# The war council" in ca.strategist_system(brief, None, "", council=True))

# --- the whole loop, against a fake `claude` -------------------------------------
bindir = tempfile.mkdtemp(prefix="fake-claude-")
fake = os.path.join(bindir, "claude")
with open(fake, "w") as f:
    f.write(r'''#!/usr/bin/env python3
import json, sys
args = sys.argv
schema = json.loads(args[args.index("--json-schema") + 1])
turn = json.loads(sys.stdin.read())
if "strategy" in schema["properties"] and "goal" not in schema["properties"]:
    out = {"thought": "The island will kneel.", "council": "",
           "strategy": {"ambition": "King of the Island", "project": "wood for a base",
                        "standing_orders": ["no fights at the gate"], "summary": "Day one."},
           "plan": [{"goal": "gather_wood", "item": "Wood", "count": 500}, {"goal": "build", "item": "", "count": 0}],
           "stance": [{"name": "Rook", "stance": "rival"}]}
    assert "why_now" in turn and turn["goals"] == ["build", "gather_wood"], turn
else:
    out = {"goal": "gather_wood", "reason": "wood for the base", "thought": "chop chop",
           "say": "", "to": "global", "code": "", "notes": "", "bug": "", "bug_kind": ""}
    with open(sys.argv[0] + ".turns", "a") as f:
        f.write(json.dumps(turn) + "\n")
print(json.dumps({"type": "result", "subtype": "success", "structured_output": out}))
''')
os.chmod(fake, 0o755)
env = dict(os.environ, PATH=bindir + os.pathsep + os.environ["PATH"], STRATEGIST_EVERY_S="600",
           GATES_JOURNAL=os.path.join(tmp, "journal.jsonl"), GATES_POOL=os.path.join(tmp, "loop-pool.json"))
proc = subprocess.Popen([sys.executable, os.path.join(HERE, "claude_agent.py"), "--via", "claude-code"],
                        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
req = {"observation": {"deaths": 0, "pack": {"items": []}}, "options": ["gather_wood", "build", "walk:<bearing>:<metres>"],
       "heard": [], "team": {}, "friend": None, "crew": None, "places": {},
       "character": {"name": "Ryuzen", "kind": "protagonist", "persona": "A grinder."},
       "memory": {"people": [{"name": "Rook", "stance": "neutral"}], "summary": ""}}
proc.stdin.write(json.dumps({**req, "id": 1}) + "\n")
proc.stdin.flush()
first = json.loads(proc.stdout.readline())
time.sleep(1.5)  # the strategist's thread lands
proc.stdin.write(json.dumps({**req, "id": 2}) + "\n")
proc.stdin.flush()
second = json.loads(proc.stdout.readline())
proc.stdin.write(json.dumps({**req, "id": 3}) + "\n")
proc.stdin.flush()
third = json.loads(proc.stdout.readline())
proc.stdin.close()
err = proc.stderr.read()
proc.wait(timeout=10)
check("a turn answers at once with the tactician's goal and thought",
      first == {"id": 1, "goal": "gather_wood", "reason": "wood for the base", "thought": "chop chop"})
check("the next reply carries the strategist's strategy, plan, stance and thought",
      second.get("strategy", {}).get("project") == "wood for a base"
      and second.get("plan") == [{"goal": "gather_wood", "until": {"item": "Wood", "count": 500}}, {"goal": "build"}]
      and second.get("stance") == [{"name": "Rook", "stance": "rival"}]
      and second.get("thought") == "The island will kneel." and second.get("goal") == "gather_wood")
check("...once", "strategy" not in third and "plan" not in third and third.get("thought") == "chop chop")
turns = [json.loads(l) for l in open(fake + ".turns")]
check("the tactician is shown character-free turns but its memory and the strategist's strategy",
      "memory" in turns[-1] and turns[-1].get("strategy", {}).get("project") == "wood for a base"
      and "character" not in turns[-1])
rows = [json.loads(l) for l in open(os.path.join(tmp, "journal.jsonl"))]
check("the journal keeps thoughts, the strategy once, and the body's name and kind",
      [r["thought"] for r in rows] == ["chop chop", "The island will kneel.", "chop chop"]
      and "strategy" in rows[1] and "strategy" not in rows[2]
      and rows[0]["name"] == "Ryuzen" and rows[0]["kind"] == "protagonist")
check("the strategist ran once (stderr says so)", err.count("strategist thinks") == 1, )

print()
if FAILED:
    print(f"{len(FAILED)} FAILED")
    sys.exit(1)
print("all claude_agent checks pass")
