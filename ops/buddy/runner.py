#!/usr/bin/env python3
"""Buddies: Claude-minded jev-bots that players get on elopros.com/buddy.html
(`ops/gates-buddies.service`).

The platform (scry-forge `meter/buddy.py`) decides who exists and how much
each pool of credit holds; this runs them and says what they spent. Three
kinds come down the feed (`BUDDY_FEED_URL`):

    buddy      plays with its owner (`--friend`); thinks only while they are on
    solo       its owner's, plays on its own
    community  ours; plays on its own, paid from a pot anyone funds

An agent's pool is its owner's wallet (or the community pot). An agent may
spend what its pool has left; when the pool runs dry its agents leave, and
credit brings them back with the same body. A failed read of the feed
changes nothing. Per agent, under BUDDY_STATE:

    keys/<id>.key, .addr     its body, made once (`jev-bot --new-key`)
    face/<id>.json           the name and picture last signed onto its nametag
    spend/<id>.jsonl         one line per model call, in dollars (claude_agent.py)
    credit/<id>              its spend so far plus what its pool has left; the
                             mind asks nothing more past it
    status/<id>.json         {left_usd}: what it tells its friend when low
    notes/<id>.txt           its own notes, kept across restarts
    journal/<id>.jsonl       its decisions; the last few are published
    public/rent.json         what each agent spent, whether it runs, what it
                             is doing: game.elopros.com/gates/rent.json

Environment (defaults in brackets):

    BUDDY_FEED_URL        [https://elopros.com/api/buddy/agents]
    BUDDY_ORIGIN          [https://elopros.com] where pictures are fetched
    BUDDY_STATE           [/var/lib/gates-buddies]
    BUDDY_SERVER          [game.elopros.com:61234]
    BUDDY_SLOTS           [6] owned agents at once; the rest wait, oldest first
    BUDDY_COMMUNITY_SLOTS [2]
    BUDDY_THINK_MS [4000]   BUDDY_MAX_HOUR [400]   BUDDY_MAX_DAY [3000]
    JEV_BOT               [/home/master/gates-jev/jev-bot]
    CLAUDE_AGENT          [/home/master/gates-jev/claude_agent.py]
    CLAUDE_MODEL          [claude-haiku-5-5]
    ANTHROPIC_API_KEY     passed through to each mind
"""
import json
import os
import re
import signal
import subprocess
import sys
import time
import urllib.request

FEED_URL = os.environ.get("BUDDY_FEED_URL", "https://elopros.com/api/buddy/agents")
ORIGIN = os.environ.get("BUDDY_ORIGIN", "https://elopros.com").rstrip("/")
STATE = os.environ.get("BUDDY_STATE", "/var/lib/gates-buddies")
SERVER = os.environ.get("BUDDY_SERVER", "game.elopros.com:61234")
SLOTS = int(os.environ.get("BUDDY_SLOTS", "6"))
COMMUNITY_SLOTS = int(os.environ.get("BUDDY_COMMUNITY_SLOTS", "2"))
THINK_MS = int(os.environ.get("BUDDY_THINK_MS", "4000"))
MAX_HOUR = int(os.environ.get("BUDDY_MAX_HOUR", "400"))
MAX_DAY = int(os.environ.get("BUDDY_MAX_DAY", "3000"))
JEV_BOT = os.environ.get("JEV_BOT", "/home/master/gates-jev/jev-bot")
AGENT = os.environ.get("CLAUDE_AGENT", "/home/master/gates-jev/claude_agent.py")
MODEL = os.environ.get("CLAUDE_MODEL", "claude-haiku-5-5")
TICK_S = 20
START_MIN_USD = 0.02   # less than this left is not worth a login
RESTART_S = 60         # an agent that exited waits this long to come back
FRIEND_FRESH_S = 90    # a buddy that decided this recently is with its friend
FETCH_MAX = 1 << 20
LIVE_N = 5
ID = re.compile(r"^[0-9a-f]{10}$|^pot-[a-z0-9-]{1,24}$")
WALLET = re.compile(r"^0x[0-9a-f]{40}$")
NAME = re.compile(r"^[\x20-\x7e]{1,16}$")
RATES = {"claude-fable-5-1": (10, 50), "claude-opus-5-5": (4, 20),
         "claude-sonnet-5-5": (2, 10), "claude-haiku-5-5": (0.1, 0.5)}


def log(msg):
    print(f"buddies: {msg}", file=sys.stderr, flush=True)


def path(*parts):
    return os.path.join(STATE, *parts)


def write_atomic(p, text, mode=0o644):
    tmp = p + ".tmp"
    with open(tmp, "w") as f:
        f.write(text)
    os.chmod(tmp, mode)
    os.replace(tmp, p)


def fetch(url, cap=FETCH_MAX):
    req = urllib.request.Request(url, headers={"User-Agent": "gates-buddies"})
    with urllib.request.urlopen(req, timeout=10) as r:
        return r.read(cap)


def read_feed():
    """(pools, agents) from the platform, or None when it can't say."""
    try:
        doc = json.loads(fetch(FEED_URL))
        pools = {str(k): max(0.0, float(v)) for k, v in doc["pools"].items()}
        agents = {}
        for a in doc["agents"]:
            if not ID.match(str(a.get("id"))) or a.get("kind") not in ("buddy", "solo", "community"):
                continue
            if a["kind"] == "buddy" and not WALLET.match(str(a.get("owner"))):
                continue
            if not NAME.match(str(a.get("name") or "")):
                continue
            agents[a["id"]] = a
        return pools, agents
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as e:
        log(f"feed unreadable, keeping the last one: {e}")
        return None


class Ledger:
    """Each agent's spend, read on from where the last tick stopped."""

    def __init__(self):
        self.seen = {}  # id -> (bytes read, dollars, calls)

    def spent(self, aid):
        off, usd, calls = self.seen.get(aid, (0, 0.0, 0))
        try:
            with open(path("spend", aid + ".jsonl"), "rb") as f:
                f.seek(off)
                for line in f:
                    if not line.endswith(b"\n"):
                        break  # a line still being written: next tick
                    off += len(line)
                    try:
                        usd += float(json.loads(line).get("usd", 0))
                        calls += 1
                    except (ValueError, AttributeError, TypeError):
                        pass
        except OSError:
            pass
        self.seen[aid] = (off, usd, calls)
        return usd

    def per_call(self):
        usd = sum(v[1] for v in self.seen.values())
        calls = sum(v[2] for v in self.seen.values())
        return usd / calls if calls >= 20 else None


def tail(p, n=LIVE_N):
    """The last `n` JSON lines of a file, without reading it all."""
    try:
        with open(p, "rb") as f:
            f.seek(0, os.SEEK_END)
            size = f.tell()
            f.seek(max(0, size - 16384))
            lines = f.read().splitlines()[-n:]
    except OSError:
        return []
    out = []
    for line in lines:
        try:
            out.append(json.loads(line))
        except ValueError:
            pass
    return out


def ensure_key(aid):
    key = path("keys", aid + ".key")
    if not os.path.exists(key):
        out = subprocess.run([JEV_BOT, "--new-key", key], capture_output=True, text=True, timeout=30)
        if out.returncode != 0:
            raise RuntimeError(f"--new-key failed: {out.stderr.strip()[-200:]}")
        m = re.search(r"0x[0-9a-fA-F]{40}", out.stdout)
        if m:
            write_atomic(path("keys", aid + ".addr"), m.group(0).lower() + "\n")
        log(f"{aid}: new body {m.group(0) if m else '?'}")
    return key


def address(aid):
    try:
        with open(path("keys", aid + ".addr")) as f:
            a = f.read().strip()
        return a if re.match(r"^0x[0-9a-f]{40}$", a) else None
    except OSError:
        return None


def ensure_face(aid, a):
    """Sign the agent's name and picture onto its nametag when either changed."""
    want = {"name": a["name"], "pic_sha": a.get("pic_sha")}
    try:
        with open(path("face", aid + ".json")) as f:
            if json.load(f) == want:
                return
    except (OSError, ValueError):
        pass
    pic = path("face", aid + ".png")
    try:
        with open(pic, "wb") as f:
            f.write(fetch(ORIGIN + str(a.get("pic") or ""), 300 * 1024))
        out = subprocess.run([JEV_BOT, "--set-face", "--agent-key", ensure_key(aid),
                              "--name", a["name"], "--pic", pic],
                             capture_output=True, text=True, timeout=60)
        if out.returncode != 0:
            raise RuntimeError(out.stderr.strip()[-200:])
        write_atomic(path("face", aid + ".json"), json.dumps(want))
        log(f"{aid}: face set ({a['name']})")
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as e:
        log(f"{aid}: face not set yet: {e}")


def start(aid, a):
    env = dict(os.environ)
    env.update({
        "CLAUDE_MODEL": MODEL,
        "GATES_SPEND": path("spend", aid + ".jsonl"),
        "GATES_CREDIT_FILE": path("credit", aid),
        "GATES_STATUS": path("status", aid + ".json"),
        "GATES_NOTES": path("notes", aid + ".txt"),
        "GATES_JOURNAL": path("journal", aid + ".jsonl"),
        "GATES_PERSONA": str(a.get("persona") or "")[:600],
    })
    cmd = [JEV_BOT, "--server", SERVER, "--agent-key", ensure_key(aid), "--agent-name", a["name"]]
    if a["kind"] == "buddy":
        cmd += ["--friend", a["owner"]]
    cmd += ["--seconds", "86400", "--think-ms", str(THINK_MS), "--timeout-ms", "20000",
            "--max-requests-hour", str(MAX_HOUR), "--max-requests-day", str(MAX_DAY),
            "--external", sys.executable, AGENT]
    log(f"{aid}: {a['name']} ({a['kind']}) joins")
    return subprocess.Popen(cmd, env=env, stdin=subprocess.DEVNULL)


def stop(aid, name, proc):
    log(f"{aid}: {name} leaves")
    proc.terminate()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()


def per_hour(ledger):
    """The most an hour of thinking can cost: the measured cost of a call (or
    a 4000-in, 400-out guess until 20 calls are measured) times the calls an
    hour allows."""
    each = ledger.per_call()
    if each is None:
        p_in, p_out = RATES.get(MODEL, (10, 50))
        each = (4000 * p_in + 400 * p_out) / 1e6
    return round(each * min(MAX_HOUR, 3_600_000 // THINK_MS), 4)


def plan(agents, left, running):
    """Which agents should be on the island: not paused or retired, with
    credit, owned ones in BUDDY_SLOTS and ours in BUDDY_COMMUNITY_SLOTS.
    Running agents keep their slots; free ones go oldest first. Returns
    (want, waiting)."""
    want, waiting = set(), set()
    for group, cap in ((("buddy", "solo"), SLOTS), (("community",), COMMUNITY_SLOTS)):
        ok = [i for i, a in agents.items() if a["kind"] in group and not a.get("paused")
              and not a.get("retired") and left.get(i, 0) >= START_MIN_USD]
        keep = [i for i in ok if i in running]
        rest = sorted((i for i in ok if i not in running), key=lambda i: (agents[i].get("since") or 0, i))
        for i in keep + rest:
            (want if len([w for w in want if agents[w]["kind"] in group]) < cap else waiting).add(i)
    return want, waiting


def main():
    for d in ("spend", "credit", "status", "notes", "journal", "face", "public"):
        os.makedirs(path(d), exist_ok=True)
    os.makedirs(path("keys"), mode=0o700, exist_ok=True)
    os.chmod(path("keys"), 0o700)
    ledger = Ledger()
    pools, agents = {}, {}  # the last good read
    running, exited = {}, {}
    done = []
    signal.signal(signal.SIGTERM, lambda *_: done.append(1))
    signal.signal(signal.SIGINT, lambda *_: done.append(1))
    log(f"{SLOTS}+{COMMUNITY_SLOTS} slots on {SERVER}, model {MODEL}, feed {FEED_URL}")
    while not done:
        fresh = read_feed()
        if fresh is not None:
            pools, agents = fresh
        now = time.time()
        spent = {i: ledger.spent(i) for i in agents}
        pool_left = {}
        for i, a in agents.items():
            pool_left[a["pool"]] = pool_left.get(a["pool"], pools.get(a["pool"], 0.0)) - spent[i]
        left = {i: pool_left.get(a["pool"], 0.0) for i, a in agents.items()}
        for i, a in agents.items():
            write_atomic(path("credit", i), f"{spent[i] + max(0.0, left[i]):.6f}\n")
            write_atomic(path("status", i + ".json"), json.dumps({"left_usd": round(max(0.0, left[i]), 4)}))
        want, waiting = plan(agents, left, running)
        for i, proc in list(running.items()):
            if proc.poll() is not None:
                log(f"{i}: exited {proc.returncode}")
                del running[i]
                exited[i] = now
            elif i not in want:
                stop(i, agents.get(i, {}).get("name", i), proc)
                del running[i]
        for i in sorted(want - set(running)):
            if now - exited.get(i, 0) < RESTART_S:
                continue
            try:
                ensure_face(i, agents[i])
                running[i] = start(i, agents[i])
            except (OSError, RuntimeError, subprocess.SubprocessError) as e:
                log(f"{i}: could not start: {e}")
                exited[i] = now
        state = {"v": 2, "at": int(now), "model": MODEL, "server": SERVER, "slots": SLOTS,
                 "community_slots": COMMUNITY_SLOTS, "per_hour_usd": per_hour(ledger), "agents": {}}
        for i in agents:
            rows = tail(path("journal", i + ".jsonl"))
            last_t = float(rows[-1].get("t") or 0) if rows else 0.0
            state["agents"][i] = {
                "spent_usd": round(spent[i], 6), "running": i in running, "waiting": i in waiting,
                "address": address(i),
                "friend_on": agents[i]["kind"] == "buddy" and i in running and now - last_t < FRIEND_FRESH_S,
                "live": [{"t": r.get("t"), "goal": r.get("goal"), "reason": r.get("reason"),
                          "say": r.get("say"), "to": r.get("to")} for r in rows],
            }
        write_atomic(path("public", "rent.json"), json.dumps(state, separators=(",", ":")))
        for _ in range(TICK_S):
            if done:
                break
            time.sleep(1)
    for i, proc in running.items():
        stop(i, agents.get(i, {}).get("name", i), proc)


if __name__ == "__main__":
    main()
