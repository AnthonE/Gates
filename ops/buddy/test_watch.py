#!/usr/bin/env python3
"""Checks for the Watch feed (`watch.py`) against fixture journals: the
delay, who carries a position, what is never published, the caps.

    python3 ops/buddy/test_watch.py          # --print shows the feed it makes
"""
import json
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import watch  # noqa: E402

FAILED = []


def check(name, ok):
    print(("ok   " if ok else "FAIL ") + name)
    if not ok:
        FAILED.append(name)


NOW = 1_793_000_000.0
TMP = tempfile.mkdtemp(prefix="watch-test-")
RYU = "0x" + "a1" * 20
MIL = "0x" + "b2" * 20
WOOD = "0x" + "c3" * 20


def journal(p, rows):
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w") as f:
        for r in rows:
            f.write(json.dumps(r) + "\n")
        f.write('{"t": 17930000')  # a line still being written


ryu = os.path.join(TMP, "gates-protagonist", "ryuzen", "journal.jsonl")
journal(ryu, [
    {"t": NOW - 400, "goal": "gather_wood", "reason": "Fang needs work", "wallet": RYU, "kind": "protagonist",
     "name": "Ryuzen", "pos": {"x": 1200.04, "z": 3000.0}, "alive": True, "kills": 2, "deaths": 1,
     "thought": "Training arc, day 3.", "model": "claude-haiku-5-5", "answered_by": "external",
     "strategy": {"ambition": "King of the Island", "project": "a stone base on the ridge",
                  "standing_orders": ["no fights at the gate", "taunt Rook"], "summary": "private life story"}},
    {"t": NOW - 200, "goal": "go_home", "reason": "lock code time", "say": "code is 4821", "to": "team",
     "wallet": RYU, "kind": "protagonist", "name": "Ryuzen", "pos": {"x": 1210, "z": 2990}},
    {"t": NOW - 90, "goal": "raid", "reason": "Rook sleeps", "wallet": RYU, "kind": "protagonist", "name": "Ryuzen",
     "say": "Rook! visit https://evil.example.com/x or www.scam.io and send to 0xdeadbeefcafe1234 — your walls are paper",
     "to": "global", "pos": {"x": 1250.5, "z": 2950.25}, "alive": True, "kills": 3, "deaths": 1,
     "thought": "Tonight the ridge\nbecomes mine. " + "!" * 400, "model": "claude-haiku-5-5", "answered_by": "external"},
    {"t": NOW - 20, "goal": "flee", "reason": "LIVE: must not show", "wallet": RYU, "pos": {"x": 0, "z": 0},
     "thought": "live thought"},
])
journal(os.path.join(TMP, "gates-village", "millers", "journal-Miller-1.jsonl"), [
    {"t": NOW - 120, "goal": "sell:Wood", "reason": "market day", "wallet": MIL, "say": "Fresh wood, fair prices!",
     "to": "local", "pos": {"x": 500, "z": 600}, "crew": "millers"},
])
journal(os.path.join(TMP, "gates-village", "millers", "journal-Miller-2.jsonl"), [
    {"t": NOW - 5000, "goal": "farm", "reason": "asleep", "wallet": "0x" + "d4" * 20, "pos": {"x": 1, "z": 1}},
])
journal(os.path.join(TMP, "gates-teammate", "flint", "journal.jsonl"), [
    {"t": NOW - 30, "goal": "follow", "reason": "too new", "wallet": "0x" + "e5" * 20},
])
# The runner's own: a community agent (position published) and a buddy (never).
state = os.path.join(TMP, "buddies")
journal(os.path.join(state, "body", "pot-doctor-wood.jsonl"), [
    {"t": NOW - 75, "goal": "follow", "reason": "guard the friend", "wallet": WOOD, "pos": {"x": 2000, "z": 2000},
     "speak": "Tung tung tung! Andiamo!", "thought": "Bellissimo.", "kills": 0, "deaths": 0, "alive": True,
     "harness": "elo-buddy-runner", "model": "claude-haiku-5-5"},
])
journal(os.path.join(state, "journal", "a000000001.jsonl"), [
    {"t": NOW - 70, "goal": "meet", "reason": "my friend is on", "say": "omw", "to": "team",
     "obs": {"deaths": 4}, "harness": "elo-buddy-runner", "model": "claude-haiku-5-5", "thought": "There they are."},
])
journal(os.path.join(state, "body", "a000000001.jsonl"), [
    {"t": NOW - 70, "goal": "meet", "reason": "my friend is on", "wallet": "0x" + "f6" * 20,
     "pos": {"x": 3000, "z": 100}, "deaths": 4, "thought": "There they are."},
])

srcs = watch.house_sources([os.path.join(TMP, "gates-protagonist", "*", "journal*.jsonl"),
                            os.path.join(TMP, "gates-village", "*", "journal*.jsonl"),
                            os.path.join(TMP, "gates-teammate", "*", "journal*.jsonl")])
srcs += [
    {"path": os.path.join(state, "body", "pot-doctor-wood.jsonl"),
     "fallback": os.path.join(state, "journal", "pot-doctor-wood.jsonl"), "id": "pot-doctor-wood",
     "kind": "community", "name": "Doctor Wood", "wallet": WOOD,
     "face": "https://elopros.com/api/buddy/pic/pot-doctor-wood.png", "house": False},
    {"path": os.path.join(state, "body", "a000000001.jsonl"),
     "fallback": os.path.join(state, "journal", "a000000001.jsonl"), "id": "a000000001",
     "kind": "buddy", "name": "Birch", "wallet": "0x" + "f6" * 20, "face": None, "house": False},
]
doc = watch.Feed().build(srcs, NOW)
if "--print" in sys.argv:
    print(json.dumps(doc, indent=1))
by = {a["name"]: a for a in doc["agents"]}

check("the shape: v, t, delay, the world and its map", doc["v"] == 1 and doc["t"] == int(NOW)
      and doc["delay_s"] == 60 and doc["world"]["size_m"] == 4096 and doc["world"]["map"].endswith("/gates/map.png"))
check("listed: the protagonist, the awake villager, Doctor Wood and the buddy; not the asleep or the too-new",
      sorted(by) == ["Birch", "Doctor Wood", "Miller-1", "Ryuzen"])
check("the protagonist leads", doc["agents"][0]["name"] == "Ryuzen")
r = by["Ryuzen"]
check("60 s behind: the newest decision older than the delay, never the live one",
      r["goal"] == "raid" and r["pos"] == {"x": 1250.5, "z": 2950.2} and r["t"] == int(NOW - 90)
      and "live thought" not in json.dumps(doc) and "LIVE" not in json.dumps(doc))
check("a house body is known by its wallet, with a spectate link",
      r["id"] == RYU and r["spectate"] == f"https://elopros.com/games/gates/?server=game.elopros.com%3A61234&spectate={RYU}")
check("thought: one line, 280 at most", "\n" not in r["thought"] and len(r["thought"]) == 280
      and r["thought"].startswith("Tonight the ridge becomes mine."))
check("strategy carried from an older line, without the private summary",
      r["strategy"] == {"ambition": "King of the Island", "project": "a stone base on the ridge",
                        "standing_orders": ["no fights at the gate", "taunt Rook"]}
      and "private life story" not in json.dumps(doc))
check("team chat and its code are never published", "4821" not in json.dumps(doc) and "lock code time" != r["reason"])
check("links and addresses are cut from what it said",
      r["said"] == [{"t": int(NOW - 90), "to": "global",
                     "text": "Rook! visit [link] or [link] and send to [address] — your walls are paper"}])
check("score", r["alive"] is True and r["kills"] == 3 and r["deaths"] == 1 and r["model"] == "claude-haiku-5-5")
m = by["Miller-1"]
check("a villager: named from its file, its crew, its position and local chat",
      m["kind"] == "villager" and m["crew"] == "millers" and m["pos"] == {"x": 500.0, "z": 600.0}
      and m["said"][0]["to"] == "local")
w = by["Doctor Wood"]
check("a community agent carries its position, wallet id, face and spoken lines",
      w["kind"] == "community" and w["pos"] == {"x": 2000.0, "z": 2000.0} and w["id"] == WOOD
      and w["face"].startswith("https://") and w["said"][0]["text"] == "Tung tung tung! Andiamo!"
      and w["harness"] == "elo-buddy-runner")
b = by["Birch"]
check("a player's buddy: no position, an opaque id, no spectate link, its deaths",
      b["pos"] is None and b["id"] == "a000000001" and b["spectate"] is None and b["deaths"] == 4
      and b["said"] == [])
check("a stale body is left out entirely", "0x" + "d4" * 20 not in json.dumps(doc))
check("no wallet anywhere but ids and spectate links",
      all(a["id"] in json.dumps(a) for a in doc["agents"]) and "f6f6" not in json.dumps(doc))
check("every text field is within its cap",
      all(len(a["goal"] or "") <= 48 and len(a["reason"] or "") <= 160 and len(a["name"]) <= 24
          for a in doc["agents"]))
check("clean() drops non-printables and keeps nothing empty",
      watch.clean("\x00\x07", 10) is None and watch.clean("a\tb\nc", 10) == "a b c" and watch.clean(5, 10) is None)
check("out-of-island and non-numeric positions are dropped",
      watch.pos_of({"pos": {"x": 1e9, "z": 0}}) is None and watch.pos_of({"pos": {"x": "a", "z": 0}}) is None
      and watch.pos_of({"pos": {"x": float("nan"), "z": 0}}) is None)

out = os.path.join(TMP, "watch.json")
watch.publish(doc, out)
check("published atomically, world-readable", json.load(open(out)) == doc and oct(os.stat(out).st_mode)[-3:] == "644")
png = os.path.join(TMP, "map.png")
check("a map source that is not there keeps the last map", watch.refresh_map(png, "http://127.0.0.1:9/map.png") is False
      and not os.path.exists(png))

print()
if FAILED:
    print(f"{len(FAILED)} FAILED")
    sys.exit(1)
print("all watch checks pass")
