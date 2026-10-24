#!/usr/bin/env python3
"""Checks for the buddies runner: the own-brain bridge, who plays, recaps
and clips. No network, no model, no game: a fake `jev-bot --mcp` answers
the bridge and a stub client answers recaps.

    python3 ops/buddy/test_runner.py
"""
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import types
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
TMP = tempfile.mkdtemp(prefix="buddies-test-")
os.environ["BUDDY_STATE"] = TMP
os.environ["CLAUDE_AGENT"] = os.path.join(HERE, "..", "..", "crates", "server", "examples", "claude_agent.py")
sys.path.insert(0, HERE)

import clips  # noqa: E402
import mind_bridge  # noqa: E402
import recap  # noqa: E402
import runner  # noqa: E402

FAILED = []


def check(name, ok):
    print(("ok   " if ok else "FAIL ") + name)
    if not ok:
        FAILED.append(name)


# A `jev-bot --mcp` stand-in: one JSON-RPC line in, one out, nothing for a
# notification; `look` shows decision 7, `act` takes it.
FAKE_MCP = r'''
import json, sys
for line in sys.stdin:
    m = json.loads(line)
    if m.get("id") is None:
        continue
    meth, p = m.get("method"), m.get("params") or {}
    if meth == "initialize":
        r = {"protocolVersion": p.get("protocolVersion"), "capabilities": {"tools": {}},
             "serverInfo": {"name": "gates"}, "instructions": "play"}
    elif meth == "tools/list":
        r = {"tools": [{"name": "look"}, {"name": "act"}, {"name": "status"}]}
    elif meth == "tools/call" and p.get("name") == "look":
        seen = {"decision_id": 7, "options": ["gather", "meet"], "observation": {"health": 80, "secret": 1,
                "in_view": {"players": 2}}, "heard": [{"from": "x", "text": "hi", "friend": True}],
                "friend": {"on": True}}
        r = {"content": [{"type": "text", "text": json.dumps(seen)}], "isError": False}
    elif meth == "tools/call" and p.get("name") == "act":
        ok = (p.get("arguments") or {}).get("id") == 7
        r = {"content": [{"type": "text", "text": "Doing it" if ok else "gone"}], "isError": not ok}
    else:
        print(json.dumps({"jsonrpc": "2.0", "id": m["id"], "error": {"code": -32601, "message": "no"}}), flush=True)
        continue
    print(json.dumps({"jsonrpc": "2.0", "id": m["id"], "result": r}), flush=True)
'''


def fake_body():
    return subprocess.Popen([sys.executable, "-c", FAKE_MCP], stdin=subprocess.PIPE, stdout=subprocess.PIPE)


def post(port, aid, body, token="t" * 64, raw=None, method="POST", in_path=False):
    data = raw if raw is not None else json.dumps(body).encode()
    url = f"http://127.0.0.1:{port}/gates/mind/{aid}" + (f"/{token}" if in_path else "")
    headers = {"Content-Type": "application/json"}
    if not in_path:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(url, data=data if method == "POST" else None, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            text = r.read()
            return r.status, (json.loads(text) if text else None)
    except urllib.error.HTTPError as e:
        return e.code, None


# ── the bridge ───────────────────────────────────────────────────────────
print("[the own-brain bridge]")
AID = "abcdef0123"
TOKEN = "t" * 64
journal = os.path.join(TMP, "journal-test.jsonl")
bridge = mind_bridge.Bridge(lambda i: journal, idle_s=60, start_wait_s=5)
bridge.set_agents({AID: {"brain": "own", "token_sha": hashlib.sha256(TOKEN.encode()).hexdigest()},
                   "fedcba9876": {"brain": "house", "token_sha": None}})
httpd = mind_bridge.serve(bridge, 0)
PORT = httpd.server_address[1]

# A runner stand-in: brings a body in when a harness asks for one.
bodies = []


def fake_runner():
    while True:
        bridge.wake.wait()
        bridge.wake.clear()
        if AID in bridge.wanted() and AID not in bridge.minds:
            p = fake_body()
            bodies.append(p)
            bridge.attach(AID, p)


threading.Thread(target=fake_runner, daemon=True).start()

code, _ = post(PORT, AID, {"jsonrpc": "2.0", "id": 1, "method": "ping"}, token="u" * 64)
check("a wrong token is refused with 401", code == 401)
code, _ = post(PORT, "fedcba9876", {"jsonrpc": "2.0", "id": 1, "method": "ping"})
check("a house-brain agent has no door", code == 401)
kept = mind_bridge.Bridge(lambda i: journal, idle_s=60)
kept.set_agents({AID: {"brain": "own", "token_sha": hashlib.sha256(TOKEN.encode()).hexdigest()}})
kept.recall({AID: time.time() - 10, "stale0000": time.time() - 3600})
check("a restart takes back a harness that called inside the window, not one before it",
      kept.wanted() == {AID} and "stale0000" not in kept.calls())
code, _ = post(PORT, AID, None, method="GET")
check("GET opens no stream (405)", code == 405)
code, got = post(PORT, AID, {"jsonrpc": "2.0", "id": "u", "method": "ping"}, in_path=True)
check("the token in the path opens the door, for an app that sends no header",
      code == 200 and got == {"jsonrpc": "2.0", "id": "u", "result": {}})
code, _ = post(PORT, AID, {"jsonrpc": "2.0", "id": 1, "method": "ping"}, token="u" * 64, in_path=True)
check("a wrong token in the path is refused with 401", code == 401)
code, _ = post(PORT, AID, {"jsonrpc": "2.0", "id": 1, "method": "ping"}, token="short", in_path=True)
check("a path that is no token is no agent (404)", code == 404)
code, got = post(PORT, AID, {"jsonrpc": "2.0", "id": "p", "method": "ping"})
check("ping is answered with no body on the island", code == 200 and got == {"jsonrpc": "2.0", "id": "p", "result": {}})
check("a ping brings nobody in", not bodies)
code, got = post(PORT, AID, {"jsonrpc": "2.0", "id": "init-1", "method": "initialize",
                             "params": {"protocolVersion": "2025-06-18",
                                        "clientInfo": {"name": "claude-code", "version": "2.1.0"}}})
check("the first initialize brings a body and is answered under the caller's id",
      code == 200 and got["id"] == "init-1" and got["result"]["protocolVersion"] == "2025-06-18")
code, got = post(PORT, AID, {"jsonrpc": "2.0", "method": "notifications/initialized"})
check("a notification gets 202 and no body", code == 202 and got is None)
code, got = post(PORT, AID, [{"jsonrpc": "2.0", "id": 5, "method": "tools/list"},
                             {"jsonrpc": "2.0", "method": "notifications/initialized"}])
check("a batch answers its requests only", code == 200 and isinstance(got, list) and len(got) == 1
      and got[0]["id"] == 5 and len(got[0]["result"]["tools"]) == 3)
code, got = post(PORT, AID, {"jsonrpc": "2.0", "id": 9, "method": "tools/call",
                             "params": {"name": "look", "arguments": {}}})
check("look passes through", code == 200 and json.loads(got["result"]["content"][0]["text"])["decision_id"] == 7)
code, got = post(PORT, AID, {"jsonrpc": "2.0", "id": 10, "method": "tools/call", "params": {
    "name": "act", "arguments": {"id": 7, "goal": "meet", "reason": "my friend\nis here " + "x" * 200,
                                 "say": "hi friend", "to": "team"}}})
check("act passes through", code == 200 and got["result"]["isError"] is False)
rows = [json.loads(x) for x in open(journal)] if os.path.exists(journal) else []
check("an answered act is journalled with the look it answered", len(rows) == 1 and rows[0]["goal"] == "meet"
      and rows[0]["say"] == "hi friend" and rows[0]["to"] == "team" and rows[0]["obs"] == {"health": 80, "players": 2}
      and rows[0]["heard"][0]["friend"] is True)
check("the line names the harness its initialize said, and that it answered",
      rows and rows[0]["harness"] == "claude-code/2.1.0" and rows[0]["answered_by"] == "harness"
      and rows[0]["model"] is None and rows[0]["heard"][0]["agent"] is False)
check("a harness label keeps only what the platform takes",
      mind_bridge.harness_of({"name": "my \"harness\"\n", "version": "1"}) == "my harness/1"
      and mind_bridge.harness_of({"version": "1"}) is None and mind_bridge.label("x" * 60) == "x" * 48)
check("a journalled reason is one short line", rows and "\n" not in rows[0]["reason"] and len(rows[0]["reason"]) <= 96)
post(PORT, AID, {"jsonrpc": "2.0", "id": 11, "method": "tools/call", "params": {
    "name": "act", "arguments": {"id": 99, "goal": "gather"}}})
rows = [json.loads(x) for x in open(journal)]
check("a refused act is not journalled", len(rows) == 1)
code, _ = post(PORT, AID, None, raw=b"x" * (mind_bridge.MAX_BODY + 1))
check("a body over 64 KiB is refused with 413", code == 413)
code, got = post(PORT, AID, None, raw=b"{not json")
check("bad JSON is a parse error", code == 400)
check("its harness counts as calling", AID in bridge.wanted())
check("and was seen", time.time() - bridge.seen.get(AID, 0) < 5)
check("an idle harness's body is let go", AID not in bridge.wanted(time.time() + 61))

# The body leaves: initialize is answered from what the last one said.
bridge.detach(AID)
bodies[0].terminate()
n = len(bodies)
code, got = post(PORT, AID, {"jsonrpc": "2.0", "id": 12, "method": "initialize",
                             "params": {"protocolVersion": "2025-06-18"}})
check("with no body, initialize is answered from the last one", code == 200 and got["id"] == 12
      and got["result"]["instructions"] == "play" and len(bodies) == n)
code, got = post(PORT, AID, {"jsonrpc": "2.0", "id": 13, "method": "tools/call",
                             "params": {"name": "status", "arguments": {}}})
check("a tool call brings the body back", len(bodies) == n + 1 and code == 200)

# Rate: a burst past BURST in a second is slowed.
codes = [post(PORT, AID, {"jsonrpc": "2.0", "id": i, "method": "ping"})[0] for i in range(40)]
check("a flood is slowed with 429", 429 in codes and codes[0] == 200)

# A body that never answers: the call times out instead of hanging.
silent = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"],
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE)
m = mind_bridge.Mind(silent)
t0 = time.time()
got = m.call({"jsonrpc": "2.0", "id": 1, "method": "ping"}, timeout=1.0)
check("a silent body times out with an error", "error" in got and time.time() - t0 < 3)
# A call queued behind another waits one budget in all, not two.
m.lock.acquire()
threading.Timer(0.6, m.lock.release).start()
t0 = time.time()
got = m.call({"jsonrpc": "2.0", "id": 2, "method": "ping"}, timeout=1.0)
check("a queued call keeps one budget", "error" in got and time.time() - t0 < 1.5)
silent.kill()
for p in bodies:
    p.kill()
httpd.shutdown()

# ── who plays ────────────────────────────────────────────────────────────
print("[who plays]")
A = lambda kind, since, **kw: {"kind": kind, "since": since, "name": "x", "pool": "p", **kw}
agents = {"a000000001": A("buddy", 1), "a000000002": A("solo", 2, brain="own"),
          "a000000003": A("solo", 3, brain="own"), "pot-x": A("community", 1)}
left = {"a000000001": 1.0, "a000000002": 0.0, "a000000003": 0.0, "pot-x": 1.0}
print("[seats]")
check("free seats are the cap less who is on", runner.seats_free({"players": 31, "max_players": 100}) == 69)
check("a full island has none", runner.seats_free({"players": 104, "max_players": 100}) == 0)
check("a status that does not say is no answer", runner.seats_free({"players": 3}) is None
      and runner.seats_free({"players": "x", "max_players": 100}) is None)
want, waiting = runner.plan(agents, left, {}, called={"a000000002"})
check("an own brain plays when its harness calls, with no credit", "a000000002" in want)
check("an own brain whose harness is quiet stays off", "a000000003" not in want and "a000000003" not in waiting)
check("house brains still need credit", "a000000001" in want and "pot-x" in want)
left["a000000001"] = 0.0
want, _ = runner.plan(agents, left, {}, called=set())
check("a house brain with nothing left stays off", "a000000001" not in want)

feed = {"pools": {"0x" + "1" * 40: 1.0}, "agents": [
    {"id": "a000000001", "kind": "buddy", "owner": "0x" + "1" * 40, "name": "Birch", "pool": "0x" + "1" * 40},
    {"id": "a000000002", "kind": "solo", "owner": "0x" + "1" * 40, "name": "Ash", "brain": "own",
     "token_sha": "f" * 64, "pool": "0x" + "1" * 40, "clips": True},
    {"id": "pot-doctor-wood", "kind": "community", "owner": None, "name": "Doctor Wood", "brain": "own",
     "token_sha": "f" * 64, "friend": "0x" + "2" * 40, "friend_away": "think", "pool": "pot-doctor-wood"},
]}
runner.fetch = lambda url, cap=0: json.dumps(feed).encode()
_, got = runner.read_feed()
check("an older feed's buddy keeps its owner as its friend", got["a000000001"]["friend"] == "0x" + "1" * 40)
check("an own brain keeps its token hash", got["a000000002"]["brain"] == "own" and got["a000000002"]["token_sha"] == "f" * 64)
check("a community agent never has an own brain", got["pot-doctor-wood"]["brain"] == "house"
      and got["pot-doctor-wood"]["token_sha"] is None)
check("a community agent keeps its friend and thinks while they are away",
      got["pot-doctor-wood"]["friend"] == "0x" + "2" * 40 and got["pot-doctor-wood"]["friend_away"] == "think")

calls = []


class FakePopen:
    def __init__(self, cmd, env=None, stdin=None, stdout=None):
        calls.append((cmd, env, stdout))


runner.subprocess.Popen = FakePopen
runner.ensure_key = lambda aid: f"/keys/{aid}.key"
os.environ["ANTHROPIC_API_KEY"] = "sk-test"
runner.start("pot-doctor-wood", got["pot-doctor-wood"])
cmd = calls[-1][0]
check("Doctor Wood goes to his friend and thinks without them",
      cmd[cmd.index("--friend") + 1] == "0x" + "2" * 40 and cmd[cmd.index("--friend-away") + 1] == "think"
      and "--external" in cmd)
check("Doctor Wood is mute without an ElevenLabs key", "--voice" not in cmd)
check("ours think only with a person near", cmd[cmd.index("--alone") + 1] == "autopilot")
os.environ["ELEVENLABS_API_KEY"] = "el-test"
runner.start("pot-doctor-wood", got["pot-doctor-wood"])
cmd, env, _ = calls[-1]
check("Doctor Wood speaks in his voice", cmd.index("--voice") < cmd.index("--external")
      and env["GATES_ELEVEN_VOICE"] == runner.VOICES["pot-doctor-wood"])
runner.start("a000000001", got["a000000001"])
cmd = calls[-1][0]
check("a house buddy waits on its friend for free", "--friend" in cmd and "--friend-away" not in cmd)
check("only ours speak", "--voice" not in cmd)
check("an owner's buddy is never put on autopilot alone", "--alone" not in cmd)
runner.start("a000000002", got["a000000002"])
cmd, env, stdout = calls[-1]
check("an own brain runs --mcp with its stdio held, and its window", "--mcp" in cmd and "--external" not in cmd
      and cmd[cmd.index("--autopilot-s") + 1] == str(runner.OWN_ANSWER_S)
      and stdout == subprocess.PIPE)
check("an own brain's body gets no API key", "ANTHROPIC_API_KEY" not in env)

# ── recaps ───────────────────────────────────────────────────────────────
print("[recaps]")
jp, rp, sp = (os.path.join(TMP, n) for n in ("j.jsonl", "r.jsonl", "s.jsonl"))
now = time.time()
with open(jp, "w") as f:
    for k in range(30):
        goal = "gather" if k < 20 else "fight"
        f.write(json.dumps({"t": now - 7200 + k * 60, "goal": goal, "reason": f"r{k}",
                            "say": "ouch!" if k == 25 else None,
                            "heard": [{"from": "z", "text": "secret words", "friend": False}],
                            "obs": {"health": [100 - k, 100], "deaths": 1 if k == 29 else 0}}) + "\n")
    f.write('{"t": 1, "goal": "torn')
os.utime(jp, (now - 3600, now - 3600))
check("a finished day is due", recap.due(jp, rp, now))
segs = recap.segments(recap.rows_since(jp, 0)[0])
check("runs of one goal fold into one segment each", [s["goal"] for s in segs] == ["gather", "fight"]
      and segs[0]["decisions"] == 20 and segs[1]["said"][0]["line"] == "ouch!")
check("other players' words never reach the model", "secret words" not in json.dumps(segs))
check("deaths and low health are kept", segs[1]["deaths"] == 1 and segs[1]["lowest_health"] == 71)
team_rows = [{"t": now - 600, "goal": "build", "say": "code is 4821", "to": "team"},
             {"t": now - 590, "goal": "build", "say": "hello all", "to": "global",
              "heard": [{"channel": "team", "text": "base at J8"}, {"channel": "global", "text": "hi"}]}]
public = json.dumps([recap.segments(team_rows), clips.timeline(team_rows, now - 700)])
check("team chat never reaches a recap or a clip",
      "4821" not in public and "J8" not in public and "hello all" in public)


class Usage:
    input_tokens, output_tokens, cache_read_input_tokens, cache_creation_input_tokens = 4000, 300, 0, 0


answer = {"text": "Chopped wood all morning,\nthen got into a scrap and lost. " + "x" * 600,
          "moments": [{"t": now - 7200 + 25 * 60, "line": "Said ouch when the bear hit me"},
                      {"t": 5, "line": "out of range"}]}
asked = []


class Client:
    class beta:
        class messages:
            @staticmethod
            def create(**kw):
                asked.append(kw)
                return types.SimpleNamespace(model="claude-haiku-5-5", usage=Usage(), stop_reason="end_turn",
                                             content=[types.SimpleNamespace(type="text", text=json.dumps(answer))])


prices = recap._prices(os.environ["CLAUDE_AGENT"])
rec = recap.write(Client, "claude-haiku-5-5", prices, "a1", {"name": "Birch", "kind": "buddy", "persona": "kind"},
                  {"journal": jp, "recaps": rp, "spend": sp}, now)
check("a recap is written, short and on one line", rec and len(rec["text"]) <= recap.TEXT_CHARS + 1
      and "\n" not in rec["text"])
check("a moment outside the day is dropped", rec and len(rec["moments"]) == 1)
spend = [json.loads(x) for x in open(sp)]
check("the recap is billed to the pool at the posted price",
      len(spend) == 1 and abs(spend[0]["usd"] - (4000 * 0.10 + 300 * 0.50) / 1e6) < 1e-9 and spend[0]["for"] == "recap")
check("one a day", not recap.due(jp, rp, now))
check("the next one starts where this one stopped", recap.rows_since(jp, rec["to_off"])[0] == [])
check("the card gets the last three", recap.recent(rp)[0]["text"] == rec["text"])
jp2 = os.path.join(TMP, "j2.jsonl")
with open(jp2, "w") as f:
    f.writelines(json.dumps({"t": now - 900 + k * 60, "goal": "gather"}) + "\n" for k in range(15))
check("a session still going is not due yet", not recap.due(jp2, os.path.join(TMP, "none.jsonl"), now))
with open(jp2, "w") as f:
    f.writelines(json.dumps({"t": now - 7 * 3600 + k * 600, "goal": "gather"}) + "\n" for k in range(40))
check("a long session is due while it goes on", recap.due(jp2, os.path.join(TMP, "none.jsonl"), now))

# ── clips ────────────────────────────────────────────────────────────────
print("[clips]")
agents = [{"id": "a", "address": "0x" + "1" * 40}, {"id": "b", "address": "0x" + "2" * 40},
          {"id": "c", "address": "0x" + "3" * 40}]
day0 = time.mktime(time.strptime("2026-10-15 12:00", "%Y-%m-%d %H:%M")) - time.timezone
runs = {"a": [day0 - 3600], "b": [day0 - 86400 * 2]}
got = clips.pick(agents, runs, day0, lambda a: True)
check("the agent longest without a clip goes first", got["id"] == "c")
runs["c"] = [day0]
got = clips.pick(agents, runs, day0, lambda a: True)
check("one a day each", got["id"] == "b")
got = clips.pick(agents, runs, day0, lambda a: a["id"] == "a")
check("only an agent that is thinking is taped", got is None)

out = os.path.join(TMP, "public", "clips", "a000000002")
clips.WHO.update({"out": out, "name": "Ash"})
for k in range(7):
    draft = os.path.join(TMP, "drafts", f"20261015-1200{k:02d}-0-big-fight")
    os.makedirs(draft, exist_ok=True)
    with open(os.path.join(draft, "share.mp4"), "wb") as f:
        f.write(b"mp4")
    clips.publish(draft, {"caption": f"c{k}", "hook": "h"}, 1000 + k)
idx = json.load(open(os.path.join(out, "index.json")))
files = sorted(x for x in os.listdir(out) if x.endswith(".mp4"))
check("the last five clips are kept", [r["caption"] for r in idx] == ["c2", "c3", "c4", "c5", "c6"] and len(files) == 5)
pub = runner.clips_of("a000000002")
check("rent.json carries each clip's path under /gates/clips/",
      len(pub) == 5 and pub[-1]["path"] == "/gates/clips/a000000002/20261015-120006-0-big-fight.mp4")
with open(os.path.join(out, "index.json"), "w") as f:
    json.dump([{"file": "../../rent.json", "caption": "x"}], f)
check("a path out of the folder is never published", runner.clips_of("a000000002") == [])

# ── other bodies' journals (`jev-bot --journal`) ─────────────────────────────
J = os.path.join(TMP, "village", "millers")
os.makedirs(J, exist_ok=True)
A, B = "0x" + "a1" * 20, "0x" + "b2" * 20
now = time.time()
with open(os.path.join(J, "journal.jsonl"), "w") as f:
    for k in range(8):
        f.write(json.dumps({"t": now - 100 + k, "goal": f"g{k}", "reason": "r", "wallet": A.upper().replace("0X", "0x"),
                            "say": "base is north, code 1234" if k == 7 else None,
                            "to": "team" if k == 7 else None}) + "\n")
    f.write(json.dumps({"t": now - 5000, "goal": "old", "wallet": B}) + "\n")
    f.write("{ torn\n")
    f.write(json.dumps({"t": now - 1, "goal": "nobody", "wallet": "not-a-wallet"}) + "\n")
o = runner.others([os.path.join(TMP, "village", "*", "journal.jsonl")], now)
check("another body's journal is published by its wallet, the last few rows, in order",
      list(o) == [A] and [r["goal"] for r in o[A]["live"]] == ["g3", "g4", "g5", "g6", "g7"])
check("…its team line shows it spoke and never what",
      o[A]["live"][-1]["say"] is None and o[A]["live"][-1]["to"] == "team"
      and "1234" not in json.dumps(o))
check("…and a body that has gone quiet, a torn line or a row with no wallet is left out",
      B not in o and len(o) == 1)
check("no journal matched is an empty map, never an error",
      runner.others([os.path.join(TMP, "nowhere", "*.jsonl")], now) == {})

print()
if FAILED:
    print(f"{len(FAILED)} failed")
    sys.exit(1)
print("all runner checks pass")
