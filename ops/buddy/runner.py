#!/usr/bin/env python3
"""Buddies: Claude-minded jev-bots that players get on elopros.com/buddy.html
(`ops/gates-buddies.service`).

The platform (scry-forge `meter/buddy.py`) decides who exists and how much
each pool of credit holds; this runs them and says what they spent. Three
kinds come down the feed (`BUDDY_FEED_URL`):

    buddy      plays with its owner (`--friend`); thinks only while they are on
    solo       its owner's, plays on its own
    community  ours; plays on its own, paid from a pot anyone funds; one may
               keep a friend (Doctor Wood keeps ours) and goes to them when
               they are on, thinking either way (`--friend-away think`)

An agent's pool is its owner's wallet (or the community pot). An agent may
spend what its pool has left; when the pool runs dry its agents leave, and
credit brings them back with the same body. A failed read of the feed
changes nothing.

A buddy or solo agent may have its owner's brain instead of ours (`brain:
own`): it runs `jev-bot --mcp`, and its owner's harness plays it through
`mind_bridge.py` at game.elopros.com/gates/mind/<id>. It needs no credit,
comes onto the island when its harness calls and leaves when it has been
quiet BUDDY_OWN_IDLE_S. Once an hour each agent with a day behind it gets a
recap (`recap.py`), billed to its pool; agents whose owners turned clips on
are listed for `clips.py auto` (`ops/gates-clips.service`). Per agent, under
BUDDY_STATE:

    keys/<id>.key, .addr     its body, made once (`jev-bot --new-key`)
    face/<id>.json           the name and picture last signed onto its nametag
    spend/<id>.jsonl         one line per model call, in dollars (claude_agent.py)
    credit/<id>              its spend so far plus what its pool has left; the
                             mind asks nothing more past it
    status/<id>.json         {left_usd}: what it tells its friend when low
    notes/<id>.txt           its own notes, kept across restarts
    journal/<id>.jsonl       its decisions; the last few are published
    recaps/<id>.jsonl        its daily recaps; the last three are published
    public/clips/<id>/       its highlight clips and their index.json
                             (`clips.py`): game.elopros.com/gates/clips/
    clips-queue.json         who wants clips, for `clips.py auto`
    public/rent.json         what each agent spent, whether it runs, what it
                             is doing: game.elopros.com/gates/rent.json

Environment (defaults in brackets):

    BUDDY_FEED_URL        [https://elopros.com/api/buddy/agents]
    BUDDY_ORIGIN          [https://elopros.com] where pictures are fetched
    BUDDY_STATE           [/var/lib/gates-buddies]
    BUDDY_SERVER          [game.elopros.com:61234]
    BUDDY_SLOTS           [6] owned agents at once; the rest wait, oldest first
    BUDDY_COMMUNITY_SLOTS [2]
    BUDDY_FREE_SEATS      [20] seats on the island kept for people: no agent
                          comes in while fewer are free (ones on stay)
    BUDDY_STATUS_URL      [https://game.elopros.com/gates/status.json] where
                          the shard says how many are on and its cap
    BUDDY_THINK_MS [4000]   BUDDY_MAX_HOUR [400]   BUDDY_MAX_DAY [3000]
    JEV_BOT               [/home/master/gates-jev/jev-bot]
    CLAUDE_AGENT          [/home/master/gates-jev/claude_agent.py]
    CLAUDE_MODEL          [claude-haiku-5-5]
    BUDDY_MIND_PORT       [8433] the own-brain bridge, on loopback
    BUDDY_OWN_IDLE_S      [1800]
    ANTHROPIC_API_KEY     passed through to each house mind, and recaps
    ELEVENLABS_API_KEY    the voices in VOICES speak with it; mute without
"""
import json
import os
import re
import signal
import subprocess
import sys
import threading
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mind_bridge  # noqa: E402
import recap  # noqa: E402

FEED_URL = os.environ.get("BUDDY_FEED_URL", "https://elopros.com/api/buddy/agents")
ORIGIN = os.environ.get("BUDDY_ORIGIN", "https://elopros.com").rstrip("/")
STATE = os.environ.get("BUDDY_STATE", "/var/lib/gates-buddies")
SERVER = os.environ.get("BUDDY_SERVER", "game.elopros.com:61234")
SLOTS = int(os.environ.get("BUDDY_SLOTS", "6"))
COMMUNITY_SLOTS = int(os.environ.get("BUDDY_COMMUNITY_SLOTS", "2"))
FREE_SEATS = int(os.environ.get("BUDDY_FREE_SEATS", "20"))
STATUS_URL = os.environ.get("BUDDY_STATUS_URL", "https://game.elopros.com/gates/status.json")
# The shard admits eight handshakes from one address at a time, and every
# body here dials from this box's: a tick starts at most this many.
STARTS_PER_TICK = 4
THINK_MS = int(os.environ.get("BUDDY_THINK_MS", "4000"))
MAX_HOUR = int(os.environ.get("BUDDY_MAX_HOUR", "400"))
MAX_DAY = int(os.environ.get("BUDDY_MAX_DAY", "3000"))
JEV_BOT = os.environ.get("JEV_BOT", "/home/master/gates-jev/jev-bot")
AGENT = os.environ.get("CLAUDE_AGENT", "/home/master/gates-jev/claude_agent.py")
MODEL = os.environ.get("CLAUDE_MODEL", "claude-haiku-5-5")
MIND_PORT = int(os.environ.get("BUDDY_MIND_PORT", "8433"))
OWN_IDLE_S = float(os.environ.get("BUDDY_OWN_IDLE_S", "1800"))
TICK_S = 20
WAKE_GAP_S = 2         # a harness waking the loop gets a tick at most this often
RECAP_EVERY_S = 3600
CLIPS_MIN_USD = 0.05   # what a pool needs left for a clip's picking call
START_MIN_USD = 0.02   # less than this left is not worth a login
RESTART_S = 60         # an agent that exited waits this long to come back
FRIEND_FRESH_S = 90    # a buddy that decided this recently is with its friend
FETCH_MAX = 1 << 20
LIVE_N = 5
ID = re.compile(r"^[0-9a-f]{10}$|^pot-[a-z0-9-]{1,24}$")
WALLET = re.compile(r"^0x[0-9a-f]{40}$")
SHA = re.compile(r"^[0-9a-f]{64}$")
CLIP_FILE = re.compile(r"^[0-9A-Za-z._-]{1,80}\.mp4$")
NAME = re.compile(r"^[\x20-\x7e]{1,16}$")
# Which of ours speak aloud on the island (`jev-bot --voice`), in which
# ElevenLabs voice. The house's key pays per line, so only these; they talk
# and do not hear (no transcriber key).
VOICES = {"pot-doctor-wood": "s2wvuS7SwITYg8dqsJdn"}
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


def seats_free(doc):
    """Free seats on the island from the shard's status, or None when it
    does not say (an agent then comes in: the shard refuses past its cap)."""
    try:
        cap, on = int(doc["max_players"]), int(doc["players"])
    except (KeyError, TypeError, ValueError):
        return None
    return max(0, cap - on) if cap > 0 else None


def read_seats():
    try:
        return seats_free(json.loads(fetch(STATUS_URL)))
    except (OSError, ValueError):
        return None


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
            # An older feed names no friend: a buddy's is its owner.
            friend = a.get("friend", a["owner"] if a["kind"] == "buddy" else None)
            a["friend"] = friend if WALLET.match(str(friend)) else None
            a["friend_away"] = "think" if a.get("friend_away") == "think" else "autopilot"
            own = a.get("brain") == "own" and a["kind"] in ("buddy", "solo")
            a["brain"] = "own" if own else "house"
            a["token_sha"] = a.get("token_sha") if own and SHA.match(str(a.get("token_sha"))) else None
            a["clips"] = a.get("clips") is True
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
            if json.load(f) == want and address(aid):
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
        # A key brought from elsewhere has no .addr: --set-face names it.
        m = re.search(r"face set for (0x[0-9a-fA-F]{40})", out.stdout)
        if m and not address(aid):
            write_atomic(path("keys", aid + ".addr"), m.group(1).lower() + "\n")
        write_atomic(path("face", aid + ".json"), json.dumps(want))
        log(f"{aid}: face set ({a['name']})")
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as e:
        log(f"{aid}: face not set yet: {e}")


def start(aid, a):
    """Bring an agent onto the island: our mind (`claude_agent.py`) behind
    `--external`, or its owner's harness behind `--mcp`, whose stdio the
    bridge holds."""
    own = a.get("brain") == "own"
    cmd = [JEV_BOT, "--server", SERVER, "--agent-key", ensure_key(aid), "--agent-name", a["name"]]
    if a.get("friend"):
        cmd += ["--friend", a["friend"]]
        # An owner's harness costs us nothing, so it decides whoever is on.
        if own or a.get("friend_away") == "think":
            cmd += ["--friend-away", "think"]
    # Ours think only while a person is near; alone, the free scripted policy
    # plays. An owner's agent thinks whenever it plays: its owner pays for it.
    if a["kind"] == "community" and not own:
        cmd += ["--alone", "autopilot"]
    cmd += ["--seconds", "86400", "--think-ms", str(THINK_MS),
            "--max-requests-hour", str(MAX_HOUR), "--max-requests-day", str(MAX_DAY)]
    log(f"{aid}: {a['name']} ({a['kind']}, {a['brain']} brain) joins")
    if own:
        env = {k: v for k, v in os.environ.items() if k != "ANTHROPIC_API_KEY"}
        return subprocess.Popen(cmd + ["--mcp"], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
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
    if aid in VOICES and os.environ.get("ELEVENLABS_API_KEY"):
        cmd += ["--voice"]
        env["GATES_ELEVEN_VOICE"] = VOICES[aid]
    cmd += ["--timeout-ms", "20000", "--external", sys.executable, AGENT]
    return subprocess.Popen(cmd, env=env, stdin=subprocess.DEVNULL)


def spec(a):
    """What a running body was started with: a change to any of it is a
    restart (an owner's new brain, name, friend or character)."""
    return (a.get("brain"), a["name"], a.get("friend"), a.get("friend_away"),
            str(a.get("persona") or "")[:600], a.get("pic_sha"))


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


def plan(agents, left, running, called=frozenset()):
    """Which agents should be on the island: not paused or retired, with
    credit (or, for an own brain, a harness that has `called` lately), owned
    ones in BUDDY_SLOTS and ours in BUDDY_COMMUNITY_SLOTS. Running agents
    keep their slots; free ones go oldest first. Returns (want, waiting)."""
    def ready(i, a):
        if a.get("paused") or a.get("retired"):
            return False
        if a.get("brain") == "own":
            return i in called
        return left.get(i, 0) >= START_MIN_USD

    want, waiting = set(), set()
    for group, cap in ((("buddy", "solo"), SLOTS), (("community",), COMMUNITY_SLOTS)):
        ok = [i for i, a in agents.items() if a["kind"] in group and ready(i, a)]
        keep = [i for i in ok if i in running]
        rest = sorted((i for i in ok if i not in running), key=lambda i: (agents[i].get("since") or 0, i))
        for i in keep + rest:
            (want if len([w for w in want if agents[w]["kind"] in group]) < cap else waiting).add(i)
    return want, waiting


def clips_of(aid):
    """An agent's published highlight clips, newest last (`clips.py publish`)."""
    try:
        with open(path("public", "clips", aid, "index.json")) as f:
            rows = json.load(f)
    except (OSError, ValueError):
        return []
    out = []
    for r in rows if isinstance(rows, list) else []:
        if isinstance(r, dict) and CLIP_FILE.match(str(r.get("file") or "")):
            out.append({"path": f"/gates/clips/{aid}/{r['file']}",
                        "caption": str(r.get("caption") or "")[:300], "t": r.get("t")})
    return out[-5:]


def clips_queue(agents, left, running, now):
    """Who `clips.py auto` may tape: clips on, on the island, a face to end
    the clip on and enough left in its pool for the picking call."""
    rows = []
    for i, a in sorted(agents.items()):
        addr = address(i)
        if (not a.get("clips") or i not in running or not addr
                or not os.path.exists(path("face", i + ".png"))):
            continue
        if left.get(i, 0) < CLIPS_MIN_USD:
            continue
        rows.append({"id": i, "name": a["name"], "kind": a["kind"], "address": addr,
                     "persona": str(a.get("persona") or "")[:600],
                     "pic": path("face", i + ".png"), "journal": path("journal", i + ".jsonl"),
                     "spend": path("spend", i + ".jsonl"), "out": path("public", "clips", i)})
    write_atomic(path("clips-queue.json"), json.dumps({"at": int(now), "agents": rows}, indent=1))


def recap_jobs(agents, left, now):
    jobs = []
    for i, a in agents.items():
        if a.get("retired") or left.get(i, 0) < recap.MIN_LEFT_USD:
            continue
        paths = {"journal": path("journal", i + ".jsonl"), "recaps": path("recaps", i + ".jsonl"),
                 "spend": path("spend", i + ".jsonl")}
        if recap.due(paths["journal"], paths["recaps"], now):
            jobs.append((i, a, paths))
    return jobs


def anthropic_client():
    import anthropic
    return anthropic.Anthropic(max_retries=1, timeout=60)


def main():
    for d in ("spend", "credit", "status", "notes", "journal", "face", "recaps", "public"):
        os.makedirs(path(d), exist_ok=True)
    os.makedirs(path("public", "clips"), exist_ok=True)
    os.makedirs(path("keys"), mode=0o700, exist_ok=True)
    os.chmod(path("keys"), 0o700)
    ledger = Ledger()
    bridge = mind_bridge.Bridge(lambda i: path("journal", i + ".jsonl"), idle_s=OWN_IDLE_S)
    try:
        mind_bridge.serve(bridge, MIND_PORT)
    except OSError as e:
        log(f"own-brain bridge not served on {MIND_PORT}: {e}")
    pools, agents = {}, {}  # the last good read
    running, exited, specs = {}, {}, {}
    recaps_at, recapping = 0.0, None
    done = []

    def quit_(*_):
        done.append(1)
        bridge.wake.set()

    signal.signal(signal.SIGTERM, quit_)
    signal.signal(signal.SIGINT, quit_)
    log(f"{SLOTS}+{COMMUNITY_SLOTS} slots on {SERVER}, model {MODEL}, feed {FEED_URL}, minds on {MIND_PORT}")
    while not done:
        bridge.wake.clear()
        fresh = read_feed()
        if fresh is not None:
            pools, agents = fresh
        bridge.set_agents(agents)
        now = time.time()
        spent = {i: ledger.spent(i) for i in agents}
        pool_left = {}
        for i, a in agents.items():
            pool_left[a["pool"]] = pool_left.get(a["pool"], pools.get(a["pool"], 0.0)) - spent[i]
        left = {i: pool_left.get(a["pool"], 0.0) for i, a in agents.items()}
        for i, a in agents.items():
            write_atomic(path("credit", i), f"{spent[i] + max(0.0, left[i]):.6f}\n")
            write_atomic(path("status", i + ".json"), json.dumps({"left_usd": round(max(0.0, left[i]), 4)}))
        want, waiting = plan(agents, left, running, bridge.wanted(now))
        for i, proc in list(running.items()):
            if proc.poll() is not None:
                log(f"{i}: exited {proc.returncode}")
                bridge.detach(i)
                del running[i]
                exited[i] = now
            elif i not in want or specs.get(i) != spec(agents[i]):
                bridge.detach(i)
                stop(i, agents.get(i, {}).get("name", i), proc)
                del running[i]
        # People first: no agent comes in while the island is nearly full,
        # and a few at a time when it may.
        free = read_seats() if want - set(running) else None
        started = 0
        for i in sorted(want - set(running)):
            if now - exited.get(i, 0) < RESTART_S:
                continue
            if started >= STARTS_PER_TICK or (free is not None and free - started <= FREE_SEATS):
                break
            started += 1
            try:
                ensure_face(i, agents[i])
                running[i] = start(i, agents[i])
                specs[i] = spec(agents[i])
                if agents[i].get("brain") == "own":
                    bridge.attach(i, running[i])
            except (OSError, RuntimeError, subprocess.SubprocessError) as e:
                log(f"{i}: could not start: {e}")
                exited[i] = now
        if now - recaps_at >= RECAP_EVERY_S and not (recapping and recapping.is_alive()):
            recaps_at = now
            jobs = recap_jobs(agents, left, now)
            if jobs:
                recapping = threading.Thread(target=recap.run, daemon=True, name="recaps",
                                             args=(anthropic_client, MODEL, AGENT, jobs, log))
                recapping.start()
        clips_queue(agents, left, running, now)
        state = {"v": 2, "at": int(now), "model": MODEL, "server": SERVER, "slots": SLOTS,
                 "community_slots": COMMUNITY_SLOTS, "per_hour_usd": per_hour(ledger), "agents": {}}
        for i in agents:
            rows = tail(path("journal", i + ".jsonl"))
            last_t = float(rows[-1].get("t") or 0) if rows else 0.0
            seen = bridge.seen.get(i)
            state["agents"][i] = {
                "spent_usd": round(spent[i], 6), "running": i in running, "waiting": i in waiting,
                "address": address(i),
                "friend_on": bool(agents[i].get("friend")) and i in running and now - last_t < FRIEND_FRESH_S,
                "mind_seen": int(seen) if seen else None,
                # rent.json is public; team chat carries lock codes and the
                # base's whereabouts, so a team line shows that it was said
                # and never what.
                "live": [{"t": r.get("t"), "goal": r.get("goal"), "reason": r.get("reason"),
                          "say": None if r.get("to") == "team" else r.get("say"), "to": r.get("to")}
                         for r in rows],
                "recaps": recap.recent(path("recaps", i + ".jsonl")),
                "clips": clips_of(i),
            }
        write_atomic(path("public", "rent.json"), json.dumps(state, separators=(",", ":")))
        # Sleep a tick, or less when a harness calls for its body (but never
        # less than WAKE_GAP_S, so a busy harness cannot spin the feed).
        t0 = time.monotonic()
        while not done and time.monotonic() - t0 < TICK_S:
            if not bridge.wake.is_set():
                bridge.wake.wait(1.0)
            elif time.monotonic() - t0 >= WAKE_GAP_S:
                break
            else:
                time.sleep(0.2)
    for i, proc in running.items():
        stop(i, agents.get(i, {}).get("name", i), proc)


if __name__ == "__main__":
    main()
