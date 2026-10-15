#!/usr/bin/env python3
"""Rented buddies: one Claude-minded jev-bot per wallet that paid for one
(`ops/gates-rent.service`).

A player buys credit on elopros.com/agents.html, in the platform's coin, at
what the calls cost (scry-forge `meter/rent.py`). This reads who has credit
(`RENT_ACTIVE_URL`), runs a buddy for each with `--friend` their wallet, and
writes what each has spent to `rent.json`, which game.elopros.com serves at
`/gates/rent.json` for the site to read back. Each buddy's own key, spend
log and credit file live under RENT_STATE:

    keys/<wallet>.key       its body, made once (`jev-bot --new-key`)
    spend/<wallet>.jsonl    one line per model call, in dollars (claude_agent.py)
    credit/<wallet>         the wallet's credit in dollars; the mind asks
                            nothing more once its spend reaches it

A buddy asks its model only while its friend is on the island; away, the
free autopilot plays. When credit runs out it leaves; a top-up brings it
back with the same body. A failed read of the credit list changes nothing.

Environment (defaults in brackets):

    RENT_ACTIVE_URL   [https://elopros.com/api/rent/active]
    RENT_STATE        [/var/lib/gates-rent]
    RENT_SERVER       [game.elopros.com:61234]
    RENT_SLOTS        [6] buddies at once; the rest wait, oldest renter first
    RENT_THINK_MS     [4000]   RENT_MAX_HOUR [400]   RENT_MAX_DAY [3000]
    JEV_BOT           [/home/master/gates-jev/jev-bot]
    CLAUDE_AGENT      [/home/master/gates-jev/claude_agent.py]
    CLAUDE_MODEL      the agent's own default when unset
    ANTHROPIC_API_KEY passed through to each buddy's mind
"""
import json
import os
import re
import signal
import subprocess
import sys
import time
import urllib.request

ACTIVE_URL = os.environ.get("RENT_ACTIVE_URL", "https://elopros.com/api/rent/active")
STATE = os.environ.get("RENT_STATE", "/var/lib/gates-rent")
SERVER = os.environ.get("RENT_SERVER", "game.elopros.com:61234")
SLOTS = int(os.environ.get("RENT_SLOTS", "6"))
THINK_MS = int(os.environ.get("RENT_THINK_MS", "4000"))
MAX_HOUR = int(os.environ.get("RENT_MAX_HOUR", "400"))
MAX_DAY = int(os.environ.get("RENT_MAX_DAY", "3000"))
JEV_BOT = os.environ.get("JEV_BOT", "/home/master/gates-jev/jev-bot")
AGENT = os.environ.get("CLAUDE_AGENT", "/home/master/gates-jev/claude_agent.py")
MODEL = os.environ.get("CLAUDE_MODEL", "claude-opus-5-5")
TICK_S = 20
START_MIN_USD = 0.05   # less than this left is not worth a login
RESTART_S = 60         # a buddy that exited waits this long to come back
FETCH_MAX = 1 << 20
WALLET = re.compile(r"^0x[0-9a-f]{40}$")
NAMES = ("Birch", "Rowan", "Hazel", "Alder", "Willow", "Aspen", "Cedar", "Maple",
         "Linden", "Juniper", "Holly", "Elder", "Hawthorn", "Spruce", "Briar", "Larch")
PERSONA = ("You are {name}: a cheerful little helper carved out of {wood} wood, rented "
           "by your friend to play Gates beside them. Loyal, brave and warm, a bit "
           "punny about trees, never mean. Short lines.")


def log(msg):
    print(f"rent: {msg}", file=sys.stderr, flush=True)


def path(*parts):
    return os.path.join(STATE, *parts)


def name_for(wallet):
    return NAMES[int(wallet[2:10], 16) % len(NAMES)]


def fetch_credit():
    """{wallet: (credit_usd, since)} from the site, or None when it can't say."""
    try:
        req = urllib.request.Request(ACTIVE_URL, headers={"User-Agent": "gates-rent"})
        with urllib.request.urlopen(req, timeout=10) as r:
            doc = json.loads(r.read(FETCH_MAX))
        out = {}
        for row in doc["wallets"]:
            w = str(row["wallet"]).lower()
            if WALLET.match(w):
                out[w] = (max(0.0, float(row["credit_usd"])), int(row.get("since") or 0))
        return out
    except (OSError, ValueError, KeyError, TypeError) as e:
        log(f"credit list unreadable, keeping the last one: {e}")
        return None


class Ledger:
    """Each wallet's spend, read on from where the last tick stopped."""

    def __init__(self):
        self.seen = {}  # wallet -> (bytes read, dollars, calls)

    def spent(self, wallet):
        off, usd, calls = self.seen.get(wallet, (0, 0.0, 0))
        try:
            with open(path("spend", wallet + ".jsonl"), "rb") as f:
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
        self.seen[wallet] = (off, usd, calls)
        return usd

    def per_call(self):
        usd = sum(v[1] for v in self.seen.values())
        calls = sum(v[2] for v in self.seen.values())
        return usd / calls if calls else None


def write_atomic(p, text, mode=0o644):
    tmp = p + ".tmp"
    with open(tmp, "w") as f:
        f.write(text)
    os.chmod(tmp, mode)
    os.replace(tmp, p)


def ensure_key(wallet):
    key = path("keys", wallet + ".key")
    if not os.path.exists(key):
        out = subprocess.run([JEV_BOT, "--new-key", key], capture_output=True, text=True, timeout=30)
        if out.returncode != 0:
            raise RuntimeError(f"--new-key failed: {out.stderr.strip()[-200:]}")
        log(f"{wallet}: new body {out.stdout.strip()[-60:]}")
    return key


def start(wallet):
    name = name_for(wallet)
    env = dict(os.environ)
    env.update({
        "GATES_SPEND": path("spend", wallet + ".jsonl"),
        "GATES_CREDIT_FILE": path("credit", wallet),
        "GATES_PERSONA": PERSONA.format(name=name, wood=name.lower()),
    })
    cmd = [JEV_BOT, "--server", SERVER, "--agent-key", ensure_key(wallet),
           "--agent-name", name, "--friend", wallet, "--seconds", "86400",
           "--think-ms", str(THINK_MS), "--timeout-ms", "20000",
           "--max-requests-hour", str(MAX_HOUR), "--max-requests-day", str(MAX_DAY),
           "--external", sys.executable, AGENT]
    log(f"{wallet}: {name} joins")
    return subprocess.Popen(cmd, env=env, stdin=subprocess.DEVNULL)


def stop(wallet, proc):
    log(f"{wallet}: {name_for(wallet)} leaves")
    proc.terminate()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()


def per_hour(ledger):
    """The most an hour of play can cost: the measured cost of a call (or a
    4000-in, 400-out guess before there is one) times the calls an hour allows."""
    each = ledger.per_call()
    if each is None:
        rates = {"claude-fable-5-1": (10, 50), "claude-opus-5-5": (4, 20),
                 "claude-sonnet-5-5": (2, 10), "claude-haiku-5-5": (0.1, 0.5)}
        p_in, p_out = rates.get(MODEL, (10, 50))
        each = (4000 * p_in + 400 * p_out) / 1e6
    return round(each * min(MAX_HOUR, 3_600_000 // THINK_MS), 4)


def main():
    for d in ("spend", "credit", "public"):
        os.makedirs(path(d), exist_ok=True)
    os.makedirs(path("keys"), mode=0o700, exist_ok=True)
    os.chmod(path("keys"), 0o700)
    ledger = Ledger()
    credit = {}            # wallet -> (credit_usd, since), the last good read
    running = {}           # wallet -> Popen
    exited = {}            # wallet -> when it last exited
    done = []
    signal.signal(signal.SIGTERM, lambda *_: done.append(1))
    signal.signal(signal.SIGINT, lambda *_: done.append(1))
    log(f"{SLOTS} slots on {SERVER}, model {MODEL}, credit from {ACTIVE_URL}")
    while not done:
        fresh = fetch_credit()
        if fresh is not None:
            credit = fresh
        now = time.time()
        left = {}
        for w, (usd, _since) in credit.items():
            write_atomic(path("credit", w), f"{usd:.6f}\n")
            left[w] = usd - ledger.spent(w)
        for w, proc in list(running.items()):
            if proc.poll() is not None:
                log(f"{w}: exited {proc.returncode}")
                del running[w]
                exited[w] = now
            elif left.get(w, 0) <= 0:
                stop(w, proc)
                del running[w]
        # Running buddies keep their slots; free ones go oldest renter first.
        queue = sorted((w for w in left if left[w] >= START_MIN_USD and w not in running),
                       key=lambda w: (credit[w][1], w))
        waiting = set()
        for w in queue:
            if len(running) >= SLOTS:
                waiting.add(w)
            elif now - exited.get(w, 0) >= RESTART_S:
                try:
                    running[w] = start(w)
                except (OSError, RuntimeError, subprocess.SubprocessError) as e:
                    log(f"{w}: could not start: {e}")
                    exited[w] = now
        state = {
            "v": 1, "at": int(now), "model": MODEL, "slots": SLOTS,
            "per_hour_usd": per_hour(ledger),
            "wallets": {w: {"spent_usd": round(ledger.spent(w), 6), "name": name_for(w),
                            "running": w in running, "waiting": w in waiting}
                        for w in credit},
        }
        write_atomic(path("public", "rent.json"), json.dumps(state, separators=(",", ":")))
        for _ in range(TICK_S):
            if done:
                break
            time.sleep(1)
    for w, proc in running.items():
        stop(w, proc)


if __name__ == "__main__":
    main()
