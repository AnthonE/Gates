#!/usr/bin/env python3
"""The Watch feed: what the house's agents and the buddies are doing and
thinking, for the Watch page on elopros.com (`public/watch.json` under
BUDDY_STATE, served at game.elopros.com/gates/watch.json, rewritten every
WATCH_EVERY_S by `runner.py`), and the island's map beside it (`map.png`).

It reads the bodies' journals (`gates-agent --journal`, one JSON line per
decision) and publishes, per body, its newest decision at least
WATCH_DELAY_S old: where it stood, its goal, reason and thought, its
strategy, what it said aloud or on global chat, and its score. The delay is
so nobody can ghost a body live. Only the house's bodies and community agents
carry a position; a player's buddy or solo agent shows `pos: null`. People
are never listed. Team chat and lock codes are never published; model text
is printable, single-line, length-capped, with links and addresses cut.

    python3 ops/buddy/watch.py JOURNAL...   # prints the feed those journals make

Map coordinates: the PNG is north up, `world.size_m` metres square, and a
world (x, z) lands at pixel ((size_m - x) / size_m * width, (size_m - z) /
size_m * height): east is -x and north is +z (`client::ui::map::world_to_map`).
"""
import glob
import json
import math
import os
import re
import sys
import time
import urllib.parse
import urllib.request

DELAY_S = int(os.environ.get("WATCH_DELAY_S", "60"))
EVERY_S = float(os.environ.get("WATCH_EVERY_S", "10"))
SIZE_M = float(os.environ.get("WATCH_SIZE_M", "4096"))  # sim_core::terrain::ISLAND_SIZE
MAP_URL = os.environ.get("WATCH_MAP_URL", "https://game.elopros.com/gates/map.png")
MAP_SOURCE = os.environ.get("WATCH_MAP_SOURCE", "http://127.0.0.1:8434/map.png")  # the companion
MAP_EVERY_S = 3600
SPECTATE = os.environ.get("WATCH_SPECTATE", "https://elopros.com/games/gates/")
HOUSE = os.environ.get(
    "WATCH_HOUSE_JOURNALS",
    "/var/lib/gates-protagonist/*/journal*.jsonl /var/lib/gates-village/*/journal*.jsonl "
    "/var/lib/gates-teammate/*/journal*.jsonl").split()
FRESH_S = 900          # a body whose newest delayed decision is older is off
THOUGHT_FRESH_S = 900  # a thought older than this is not "now"
TAIL_ROWS = 400
TAIL_BYTES = 1 << 19
SAID_N = 5
AGENTS_MAX = 64
CAPS = {"name": 24, "goal": 48, "reason": 160, "thought": 280, "said": 200,
        "ambition": 160, "project": 160, "order": 120, "model": 48, "harness": 48}
KINDS = ("protagonist", "teammate", "villager", "buddy", "solo", "community", "other")
WALLET = re.compile(r"^0x[0-9a-f]{40}$")
# A face is a picture elopros.com serves itself (its CSP loads no other host).
FACE = re.compile(r"^/api/[A-Za-z0-9._/-]{1,200}$")
FEED_MAX_BYTES = 512 * 1024
URL = re.compile(r"(?i)\b(?:[a-z][a-z0-9+.-]*://|www\.)\S+"
                 r"|\b[a-z0-9-]+(?:\.[a-z0-9-]+)*\.(?:com|net|org|io|gg|xyz|ai|co|app|dev|me|tv|ly|link|"
                 r"site|online|info|biz|ru|cn|uk|de|fr|us|to|sh|so|fun|lol|club)\b(?:/\S*)?")
HEX = re.compile(r"(?i)\b0x[0-9a-f]{6,}\b")


def clean(v, cap):
    """Model text as the page may show it: printable, one line, no links or
    addresses, at most `cap` characters. None when nothing is left."""
    if not isinstance(v, str):
        return None
    text = "".join(c if c.isprintable() else " " for c in v)
    text = HEX.sub("[address]", URL.sub("[link]", text))
    text = " ".join(text.split())[:cap].strip()
    return text or None


def tail(p, n=TAIL_ROWS, nbytes=TAIL_BYTES):
    try:
        with open(p, "rb") as f:
            f.seek(0, os.SEEK_END)
            f.seek(max(0, f.tell() - nbytes))
            lines = f.read().splitlines()[-n:]
    except OSError:
        return []
    out = []
    for line in lines:
        try:
            r = json.loads(line)
        except ValueError:
            continue
        if isinstance(r, dict) and isinstance(r.get("t"), (int, float)):
            out.append(r)
    return out


def house_kind(p):
    for key, kind in (("gates-protagonist", "protagonist"), ("gates-village", "villager"),
                      ("gates-teammate", "teammate")):
        if key in p:
            return kind
    return "other"


def house_sources(globs=None):
    """Every house body's journal: [{"path", "kind", "crew", "name"}]. A
    fleet keeps one file a body (journal-<name>.jsonl); a single body's is
    named for its unit's instance."""
    out = []
    for pattern in globs if globs is not None else HOUSE:
        for p in sorted(glob.glob(pattern)):
            kind = house_kind(p)
            instance = os.path.basename(os.path.dirname(p))
            stem = os.path.basename(p)[:-len(".jsonl")] if p.endswith(".jsonl") else os.path.basename(p)
            name = stem[len("journal-"):] if stem.startswith("journal-") else None
            if name is None and kind in ("protagonist", "teammate"):
                name = instance.capitalize()
            out.append({"path": p, "kind": kind, "crew": instance if kind == "villager" else None,
                        "name": name, "house": True})
    return out


def pos_of(r):
    p = r.get("pos")
    if not isinstance(p, dict):
        return None
    try:
        x, z = float(p["x"]), float(p["z"])
    except (KeyError, TypeError, ValueError):
        return None
    if not (math.isfinite(x) and math.isfinite(z)):
        return None
    if not (-SIZE_M * 0.1 <= x <= SIZE_M * 1.1 and -SIZE_M * 0.1 <= z <= SIZE_M * 1.1):
        return None
    return {"x": round(x, 1), "z": round(z, 1)}


def strategy_of(r):
    s = r.get("strategy")
    if not isinstance(s, dict):
        return None
    out = {"ambition": clean(s.get("ambition"), CAPS["ambition"]),
           "project": clean(s.get("project"), CAPS["project"]),
           "standing_orders": [o for o in (clean(x, CAPS["order"]) for x in (s.get("standing_orders") or [])[:4]) if o]}
    return out if out["ambition"] or out["project"] else None


def said_of(rows):
    """What a body said where anyone near or anyone at all could hear: global
    and local chat and its voice. Never team chat, never a code."""
    out = []
    for r in rows:
        to = r.get("to")
        if r.get("say") and to in ("global", "local"):
            text = clean(r["say"], CAPS["said"])
            if text:
                out.append({"t": int(r["t"]), "to": to, "text": text})
        if r.get("speak"):
            text = clean(r["speak"], CAPS["said"])
            if text:
                out.append({"t": int(r["t"]), "to": "local", "text": text})
    return out[-SAID_N:]


def int_or(v, default):
    return int(v) if isinstance(v, (int, float)) and not isinstance(v, bool) and math.isfinite(v) else default


class Feed:
    """Builds watch.json. Keeps each body's last strategy, which a journal
    line carries only when it changed."""

    def __init__(self, delay_s=DELAY_S, server="game.elopros.com:61234"):
        self.delay_s, self.server = delay_s, server
        self.strategies = {}

    def agent(self, src, now):
        """One body's entry, or None when it has nothing old enough or has gone quiet."""
        rows = [r for r in tail(src["path"]) if r["t"] <= now - self.delay_s]
        if src.get("fallback"):
            rows = rows or [r for r in tail(src["fallback"]) if r["t"] <= now - self.delay_s]
        if not rows:
            return None
        rows.sort(key=lambda r: r["t"])
        cur = rows[-1]
        if cur["t"] < now - self.delay_s - FRESH_S:
            return None
        wallet = str(cur.get("wallet") or src.get("wallet") or "").lower()
        wallet = wallet if WALLET.match(wallet) else None
        kind = src.get("kind") or cur.get("kind")
        kind = kind if kind in KINDS else (cur.get("kind") if cur.get("kind") in KINDS else "other")
        public = src.get("house") or kind == "community"
        aid = (wallet if public else src.get("id")) or src.get("id")
        if not aid:
            return None
        for r in rows:
            st = strategy_of(r)
            if st:
                self.strategies[aid] = st
        thought = next((clean(r.get("thought"), CAPS["thought"]) for r in reversed(rows)
                        if r.get("thought") and r["t"] >= cur["t"] - THOUGHT_FRESH_S), None)
        obs = cur.get("obs") if isinstance(cur.get("obs"), dict) else {}
        crew = cur.get("crew") if isinstance(cur.get("crew"), str) else src.get("crew")
        out = {
            "id": aid,
            "name": clean(src.get("name") or cur.get("name"), CAPS["name"]) or (wallet[:8] if wallet else "agent"),
            "kind": kind,
            "crew": clean(crew, CAPS["name"]),
            "face": src.get("face") if FACE.match(str(src.get("face") or "")) and ".." not in src["face"] else None,
            "pos": pos_of(cur) if public else None,
            "goal": clean(cur.get("goal"), CAPS["goal"]),
            "reason": clean(cur.get("reason"), CAPS["reason"]),
            "thought": thought,
            "strategy": self.strategies.get(aid),
            "said": said_of(rows),
            "alive": cur.get("alive") is not False,
            "kills": int_or(cur.get("kills"), 0),
            "deaths": int_or(cur.get("deaths", obs.get("deaths")), 0),
            "model": clean(cur.get("model"), CAPS["model"]),
            "harness": clean(cur.get("harness") or cur.get("answered_by"), CAPS["harness"]),
            "t": int(cur["t"]),
            "spectate": None,
        }
        if public and wallet:
            out["spectate"] = SPECTATE + "?" + urllib.parse.urlencode({"server": self.server, "spectate": wallet})
        return out

    def build(self, sources, now):
        agents, seen = [], set()
        for src in sources:
            a = self.agent(src, now)
            if a and a["id"] not in seen:
                seen.add(a["id"])
                agents.append(a)
        order = {k: i for i, k in enumerate(KINDS)}
        agents.sort(key=lambda a: (order.get(a["kind"], 99), a["name"].lower()))
        return {"v": 1, "t": int(now), "delay_s": self.delay_s,
                "world": {"size_m": SIZE_M, "map": MAP_URL, "north": "+z", "east": "-x",
                          "px": "((size_m - x) / size_m * width, (size_m - z) / size_m * height)"},
                "agents": agents[:AGENTS_MAX]}


def write_atomic(p, data, mode=0o644):
    tmp = p + ".tmp"
    with open(tmp, "wb") as f:
        f.write(data)
    os.chmod(tmp, mode)
    os.replace(tmp, p)


def publish(doc, out):
    """watch.json, under FEED_MAX_BYTES: the last agents go first."""
    doc = dict(doc, agents=list(doc["agents"]))
    data = json.dumps(doc, separators=(",", ":")).encode()
    while len(data) > FEED_MAX_BYTES and doc["agents"]:
        doc["agents"].pop()
        data = json.dumps(doc, separators=(",", ":")).encode()
    write_atomic(out, data)


def refresh_map(out, source=MAP_SOURCE):
    """The island's map from the shard's companion (`companion.rs`, painted
    from the seed at start), copied beside watch.json when it changed. A
    failed read keeps the last one. True when it wrote."""
    try:
        req = urllib.request.Request(source, headers={"User-Agent": "gates-watch"})
        with urllib.request.urlopen(req, timeout=10) as r:
            png = r.read(16 << 20)
    except OSError as e:
        print(f"watch: map not fetched: {e}", file=sys.stderr)
        return False
    if not png.startswith(b"\x89PNG\r\n\x1a\n"):
        return False
    try:
        with open(out, "rb") as f:
            if f.read() == png:
                return False
    except OSError:
        pass
    write_atomic(out, png)
    return True


if __name__ == "__main__":
    srcs = [{"path": p, "kind": house_kind(p), "house": True} for p in sys.argv[1:]] or house_sources()
    print(json.dumps(Feed().build(srcs, time.time()), indent=1))
