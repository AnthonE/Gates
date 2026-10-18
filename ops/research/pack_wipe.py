#!/usr/bin/env python3
"""Pack each finished wipe's agent play as an open dataset (CC-BY-4.0), for
researchers (`ops/gates-research-pack.timer`, daily).

    python3 ops/research/pack_wipe.py              # every closed wipe not packed yet
    python3 ops/research/pack_wipe.py --fixture    # a built-in wipe, packed and checked

A wipe is closed once the shard has archived it (`crates/server/src/wipe.rs`:
`<world>.wipes/<unix>/`, which holds its standings and its trust log). For
each one not packed yet this reads:

    the standings       `<world>.standings` in the archive (`standings.rs`
                        `text()`): every wallet's boards and the agent flag
    decision journals   the hosted agents' lines inside the wipe's hours, one
                        JSON line per decision (`ops/buddy/mind_bridge.py`,
                        `crates/server/examples/claude_agent.py`), keyed to
                        each agent's wallet by `keys/<id>.addr` or the
                        runner's public `rent.json`
    the trust log       with --trust-log BIN, `trust-log <archive>/<world>.trust
                        --jsonl`, the rows where both sides are agents

and writes `wipe-N/{decisions.jsonl.gz, standings.json, datasheet.json,
SHA256SUMS}` (and `trust.jsonl.gz` when there is one) under --out, then
`index.json` (format gates-data-1), served at
https://game.elopros.com/gates/data/ (`ops/game-elopros-status.nginx.conf`).

People are redacted, never dropped: a line a person said keeps its slot with
`from: "a person"` and `text: null`, a person's standings row keeps its
scores with no wallet and the name "a person", a friend is kept without its
name and wallet, and any person's name or wallet an agent wrote down is
replaced. Team chat carries lock codes, so its text is null for everyone.
"""
import argparse
import gzip
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

FORMAT = "gates-data-1"
LICENSE = "CC-BY-4.0"
PERSON = "a person"
WALLET = re.compile(r"0x[0-9a-fA-F]{40}")
ROW_FIELDS = 13  # standings.rs ROW_FIELDS
FLAG_AGENT = 1
CAVEATS = [
    "agents misreport their own reasons",
    "the autopilot played turns a harness missed: see answered_by",
    "people are redacted: their words are null and their name is 'a person'",
    "harness latency differs between agents",
]
CONTACT = "https://elopros.com/chat.html"
CITE = ("Gates wipe {wipe} agent play, Elo Pros (https://game.elopros.com/gates/data/), "
        "CC-BY-4.0, packed {day}.")
DECISION_SCHEMA = {
    "wipe": "the wipe number",
    "agent": "the agent's wallet (lowercase 0x address)",
    "t": "unix seconds the decision was taken",
    "goal": "the goal it picked, from the options it was offered",
    "reason": "the reason it gave, in its own words (unchecked)",
    "say": "the chat line it said (null on team chat)",
    "to": "where it said it: global, local or team",
    "speak": "a line it spoke out loud (voice), if any",
    "heard": "chat it heard since its last decision: from, agent, friend, channel, text, voice",
    "friend": "the player it plays with: online, on_my_team, in_sight, ... (no name or wallet)",
    "obs": "a few observation fields: health, food, water, threats, fights, goals, deaths",
    "brain": "house (our hosted mind) or own (its owner's harness, over MCP)",
    "harness": "what ran the mind (an MCP clientInfo name/version or a declared label), or null",
    "model": "the model that answered, when known, or null",
    "answered_by": "harness (its mind answered) or autopilot (the scripted policy played the turn)",
}
STANDINGS_SCHEMA = {
    "wallet": "the agent's wallet; null for a person",
    "name": "its name on the boards; 'a person' for a person",
    "agent": "joined as a declared agent this wipe",
    "island": "THE ISLAND score, whole points (content/arc.toml [standings])",
    "works": "THE WORKS: what it gave the works plus its deeds, whole points",
    "hoard": "THE HOARD: its share of its bases' contents at the wipe, whole points",
    "kills": "players killed",
    "deaths": "times killed by a player",
    "played_min": "minutes played this wipe",
    "extracted": "JUNK credited through THE EXCHANGE",
}
TRUST_SCHEMA = {
    "tick": "the shard tick",
    "verb": "door, auth or cont (crates/server/src/trustlog.rs)",
    "presence": "the owner's state: awake, asleep or gone",
    "actor": "the acting agent's wallet",
    "counterparty": "the agent whose thing it was",
}


def log(msg):
    print(f"pack: {msg}", file=sys.stderr, flush=True)


def write_atomic(path, data):
    tmp = path + ".tmp"
    with open(tmp, "wb") as f:
        f.write(data)
    os.replace(tmp, path)


def read_standings(path):
    """(wipe, rows) of a standings file: rows as {key, label, agent, scores}."""
    wipe, rows = None, []
    with open(path) as f:
        for line in f:
            p = line.rstrip("\n").split("\t", ROW_FIELDS - 1)
            if p[0] == "wipe" and len(p) >= 2 and p[1].isdigit():
                wipe = int(p[1])
            if p[0] != "row" or len(p) < ROW_FIELDS - 1:
                continue
            n = lambda i: int(p[i]) if p[i].isdigit() else 0
            rows.append({
                "key": p[1].lower(), "label": p[ROW_FIELDS - 1] if len(p) >= ROW_FIELDS else "",
                "agent": n(9) & FLAG_AGENT != 0, "island": n(7), "works": (n(2) + n(3)) // 100,
                "hoard": n(6) // 100, "kills": n(4), "deaths": n(5), "played_min": n(8),
                "extracted": n(10),
            })
    return wipe, rows


def closed_wipes(world):
    """[(archive dir, unix it closed)] oldest first."""
    root = world + ".wipes"
    try:
        names = sorted((n for n in os.listdir(root) if n.isdigit()), key=int)
    except OSError:
        return []
    return [(os.path.join(root, n), int(n)) for n in names]


def agent_wallets(keys_dir, rent=None):
    """journal id -> its agent's wallet, from the runner's `keys/<id>.addr`,
    else what its public `rent.json` says (`ops/buddy/runner.py`)."""
    out = {}
    try:
        with open(rent) as f:
            for i, a in (json.load(f).get("agents") or {}).items():
                w = str((a or {}).get("address") or "").lower()
                if re.fullmatch(r"0x[0-9a-f]{40}", w):
                    out[str(i)] = w
    except (OSError, ValueError, TypeError, AttributeError):
        pass
    try:
        names = os.listdir(keys_dir)
    except OSError:
        return out
    for n in names:
        if n.endswith(".addr"):
            try:
                with open(os.path.join(keys_dir, n)) as f:
                    w = f.read().strip().lower()
            except OSError:
                continue
            if re.fullmatch(r"0x[0-9a-f]{40}", w):
                out[n[:-5]] = w
    return out


def journal_lines(journals_dir):
    """(journal id, line) for every journal line, unparsed lines skipped."""
    try:
        names = sorted(n for n in os.listdir(journals_dir) if n.endswith(".jsonl"))
    except OSError:
        return
    for n in names:
        try:
            with open(os.path.join(journals_dir, n)) as f:
                for raw in f:
                    try:
                        row = json.loads(raw)
                    except ValueError:
                        continue
                    if isinstance(row, dict) and isinstance(row.get("t"), (int, float)):
                        yield n[:-6], row
        except OSError:
            continue


class Redactor:
    """Who counts as an agent, and what of a person may not leave."""

    def __init__(self, agents, people_names):
        self.agents = set(agents)
        names = sorted({n for n in people_names if len(n) >= 3}, key=len, reverse=True)
        self.names = re.compile(r"\b(" + "|".join(map(re.escape, names)) + r")\b") if names else None

    def text(self, v):
        if not isinstance(v, str):
            return v
        v = WALLET.sub(lambda m: m.group(0).lower() if m.group(0).lower() in self.agents else PERSON, v)
        return self.names.sub(PERSON, v) if self.names else v

    def heard(self, h):
        if not isinstance(h, dict):
            return None
        agent = h.get("agent") is True
        channel = h.get("channel") if h.get("channel") in ("global", "local", "team") else "local"
        out = {"from": self.text(h.get("from")) if agent else PERSON, "agent": agent,
               "friend": bool(h.get("friend")), "channel": channel,
               "text": self.text(h.get("text")) if agent and channel != "team" else None}
        if h.get("voice"):
            out["voice"] = True
        return out

    def friend(self, f):
        if not isinstance(f, dict):
            return None
        return {k: v for k, v in f.items() if k not in ("name", "wallet")}

    def decision(self, wipe, wallet, row):
        to = row.get("to") if row.get("to") in ("global", "local", "team") else None
        return {
            "wipe": wipe, "agent": wallet, "t": row.get("t"),
            "goal": row.get("goal"), "reason": self.text(row.get("reason")),
            "say": None if to == "team" else self.text(row.get("say")), "to": to,
            "speak": self.text(row.get("speak")) or None,
            "heard": [x for x in (self.heard(h) for h in row.get("heard") or []) if x],
            "friend": self.friend(row.get("friend")),
            "obs": row.get("obs") if isinstance(row.get("obs"), dict) else {},
            "brain": "own" if row.get("brain") == "own" else "house",
            "harness": row.get("harness") if isinstance(row.get("harness"), str) else None,
            "model": row.get("model") if isinstance(row.get("model"), str) else None,
            "answered_by": "autopilot" if row.get("answered_by") == "autopilot" else "harness",
        }


def trust_rows(bin_path, trust_dir, agents):
    """The agent-agent rows of a wipe's trust log, or [] when there is none."""
    if not bin_path or not os.path.isdir(trust_dir):
        return []
    try:
        run = subprocess.run([bin_path, trust_dir, "--jsonl"], capture_output=True, text=True, timeout=600)
    except (OSError, subprocess.SubprocessError) as e:
        log(f"trust log {trust_dir}: {e}")
        return []
    out = []
    for line in run.stdout.splitlines():
        try:
            r = json.loads(line)
            a = (r["actor"].get("wallet") or "").lower()
            c = (r["counterparty"].get("wallet") or "").lower()
        except (ValueError, KeyError, TypeError, AttributeError):
            continue
        if a in agents and c in agents:
            out.append({"tick": r.get("tick"), "verb": r.get("verb"), "presence": r.get("presence"),
                        "actor": a, "counterparty": c})
    return out


def gz(rows):
    """JSON lines, gzipped with no name or time in the header (same rows, same bytes)."""
    body = "".join(json.dumps(r, separators=(",", ":"), sort_keys=True) + "\n" for r in rows)
    return gzip.compress(body.encode(), mtime=0)


def pack(args, archive, closed, opened, now):
    """Pack one closed wipe; its index entry, or None when there is nothing to pack."""
    base = os.path.basename(args.world)
    standings_file = os.path.join(archive, base + ".standings")
    if not os.path.exists(standings_file):
        return None
    wipe, rows = read_standings(standings_file)
    if wipe is None:
        return None
    ids = agent_wallets(args.keys, args.rent)
    agents = {r["key"] for r in rows if r["agent"]} | set(ids.values())
    lines = [(ids[i], row) for i, row in journal_lines(args.journals) if i in ids]
    if opened is None:
        # The first wipe on record: it began with its first decision.
        opened = min((row["t"] for _, row in lines if row["t"] < closed), default=closed)
    people = [r["label"] for r in rows if not r["agent"]]
    people += [h.get("from") for _, row in lines for h in row.get("heard") or []
               if isinstance(h, dict) and h.get("agent") is not True and isinstance(h.get("from"), str)]
    people += [row["friend"].get("name") for _, row in lines
               if isinstance(row.get("friend"), dict) and isinstance(row["friend"].get("name"), str)]
    red = Redactor(agents, [p for p in people if p and p != PERSON])
    decisions = [red.decision(wipe, w, row) for w, row in lines if opened <= row["t"] < closed]
    decisions.sort(key=lambda d: (d["t"], d["agent"]))
    board = sorted(({
        "wallet": r["key"] if r["agent"] else None,
        "name": red.text(r["label"]) if r["agent"] else PERSON,
        "agent": r["agent"],
        **{k: r[k] for k in ("island", "works", "hoard", "kills", "deaths", "played_min", "extracted")},
    } for r in rows), key=lambda r: (-r["island"], r["wallet"] or "", r["name"]))
    trust = trust_rows(args.trust_log, os.path.join(archive, base + ".trust"), agents)
    in_pack = {d["agent"] for d in decisions} | {r["wallet"] for r in board if r["wallet"]}

    name = f"wipe-{wipe}"
    out_dir = os.path.join(args.out, name)
    os.makedirs(out_dir, exist_ok=True)
    files = {"decisions.jsonl.gz": gz(decisions),
             "standings.json": json.dumps({"wipe": wipe, "rows": board}, indent=1).encode()}
    if trust:
        files["trust.jsonl.gz"] = gz(trust)
    datasheet = {
        "format": FORMAT, "wipe": wipe, "seed": args.seed, "commit": args.commit, "proto": args.proto,
        "from": int(opened), "to": int(closed),
        "counts": {"agents": len(in_pack), "decisions": len(decisions),
                   "standings_rows": len(board), "people_rows": sum(1 for r in board if not r["agent"]),
                   "trust_rows": len(trust)},
        "files": sorted(files) + ["datasheet.json", "SHA256SUMS"],
        "schema": {"decisions.jsonl.gz": DECISION_SCHEMA, "standings.json": STANDINGS_SCHEMA,
                   **({"trust.jsonl.gz": TRUST_SCHEMA} if trust else {})},
        "license": LICENSE, "caveats": CAVEATS,
        "cite": CITE.format(wipe=wipe, day=time.strftime("%Y-%m-%d", time.gmtime(now))),
        "contact": CONTACT,
    }
    files["datasheet.json"] = json.dumps(datasheet, indent=1).encode()
    for f, data in files.items():
        write_atomic(os.path.join(out_dir, f), data)
    sums = "".join(f"{hashlib.sha256(files[f]).hexdigest()}  {f}\n" for f in sorted(files)).encode()
    write_atomic(os.path.join(out_dir, "SHA256SUMS"), sums)
    log(f"{name}: {len(decisions)} decisions from {len(in_pack)} agents, {len(board)} standings rows")
    return {
        "wipe": wipe, "from": int(opened), "to": int(closed), "seed": args.seed, "commit": args.commit,
        "agents": len(in_pack), "decisions": len(decisions),
        "bytes": sum(len(d) for d in files.values()) + len(sums),
        "files": {"decisions": f"{name}/decisions.jsonl.gz", "standings": f"{name}/standings.json",
                  "datasheet": f"{name}/datasheet.json", "sums": f"{name}/SHA256SUMS"},
    }


def run(args, now=None):
    """Pack every closed wipe not packed yet and rewrite the index. Returns
    the wipes packed this time."""
    now = int(time.time() if now is None else now)
    os.makedirs(args.out, exist_ok=True)
    index_path = os.path.join(args.out, "index.json")
    try:
        with open(index_path) as f:
            entries = {int(e["wipe"]): e for e in json.load(f).get("wipes", [])}
    except (OSError, ValueError, KeyError, TypeError, AttributeError):
        entries = {}
    packed, opened = [], None
    for archive, closed in closed_wipes(args.world):
        base = os.path.basename(args.world)
        try:
            wipe, _ = read_standings(os.path.join(archive, base + ".standings"))
        except OSError:
            wipe = None
        if wipe is not None and (args.force or wipe not in entries):
            entry = pack(args, archive, closed, opened, now)
            if entry:
                entries[entry["wipe"]] = entry
                packed.append(entry["wipe"])
        opened = closed
    index = {"format": FORMAT, "license": LICENSE, "updated": now,
             "wipes": [entries[w] for w in sorted(entries)]}
    write_atomic(index_path, json.dumps(index, indent=1).encode())
    return packed


def seed_of(config):
    """`seed = N` from a shard.toml, or None."""
    try:
        with open(config) as f:
            for line in f:
                m = re.match(r"^\s*seed\s*=\s*(\d+)\s*(#.*)?$", line)
                if m:
                    return int(m.group(1))
    except OSError:
        pass
    return None


# ── the fixture ───────────────────────────────────────────────────────────
AGENT_W = "0x" + "a1" * 20
OTHER_AGENT_W = "0x" + "b2" * 20
HUMAN_W = "0x" + "c3" * 20
FRIEND_W = "0x" + "d4" * 20


def _standings(wipe, rows):
    head = f"# Gates standings, written by the shard.\nwipe\t{wipe}\n"
    return head + "".join(f"row\t{k}\t{given}\t0\t{kills}\t{deaths}\t{hoard}\t{island}\t{played}\t{flag}\t0\t0\t{label}\n"
                          for k, given, kills, deaths, hoard, island, played, flag, label in rows)


def fixture():
    """Pack a built-in pair of wipes into a temp dir and check what came out."""
    tmp = tempfile.mkdtemp(prefix="gates-pack-")
    failed = []

    def check(name, ok):
        print(("ok   " if ok else "FAIL ") + name)
        if not ok:
            failed.append(name)

    try:
        world = os.path.join(tmp, "gates-data", "island.world")
        state = os.path.join(tmp, "buddies")
        for d in ("journal", "keys"):
            os.makedirs(os.path.join(state, d))
        t1, t2 = 1_700_000_000, 1_700_604_800
        for ts, wipe in ((t1, 1), (t2, 2)):
            a = os.path.join(world + ".wipes", str(ts))
            os.makedirs(os.path.join(a, "island.world.trust"))
            with open(os.path.join(a, "island.world.standings"), "w") as f:
                f.write(_standings(wipe, [
                    (AGENT_W, 12000, 2, 1, 5000, 170, 300, 1, "Rook"),
                    (OTHER_AGENT_W, 100, 0, 3, 0, 1, 40, 1, "Wren"),
                    (HUMAN_W, 50000, 4, 0, 9000, 590, 600, 0, "Alice Smith"),
                ]))
        with open(os.path.join(state, "keys", "abcdef0123.addr"), "w") as f:
            f.write(AGENT_W + "\n")
        os.makedirs(os.path.join(state, "public"))
        with open(os.path.join(state, "public", "rent.json"), "w") as f:
            json.dump({"v": 2, "agents": {"pot-wren": {"address": OTHER_AGENT_W}, "x": {"address": None}}}, f)
        own = {"t": t2 - 100, "goal": "meet", "brain": "own", "harness": "claude-code/2.1.0", "model": None,
               "answered_by": "harness", "reason": f"Alice Smith asked; she is {HUMAN_W}", "say": "omw",
               "to": "global", "obs": {"health": 80},
               "friend": {"name": "Alice Smith", "wallet": FRIEND_W, "online": True},
               "heard": [{"from": "Alice Smith", "agent": False, "friend": True, "channel": "global",
                          "text": f"come to my base, my wallet is {HUMAN_W}"},
                         {"from": "Wren", "agent": True, "friend": False, "channel": "local", "text": "hi Rook"},
                         {"from": "Wren", "agent": True, "friend": False, "channel": "team", "text": "code 1234"},
                         {"from": "Bob", "friend": False, "channel": "local", "text": "an old line"}]}
        team = {"t": t2 - 50, "goal": "build", "say": "the code is 1234", "to": "team", "reason": "home",
                "heard": [], "obs": {}, "harness": "claude_agent.py/api", "model": "test-model",
                "answered_by": "harness"}
        early = {"t": t1 - 10, "goal": "explore", "reason": "wipe one", "heard": [], "obs": {}}
        late = {"t": t2 + 10, "goal": "explore", "reason": "next wipe", "heard": [], "obs": {}}
        with open(os.path.join(state, "journal", "abcdef0123.jsonl"), "w") as f:
            for r in (early, own, late):
                f.write(json.dumps(r) + "\n")
            f.write("not json\n")
        with open(os.path.join(state, "journal", "pot-wren.jsonl"), "w") as f:
            f.write(json.dumps(team) + "\n")
        with open(os.path.join(state, "journal", "nobody0000.jsonl"), "w") as f:
            f.write(json.dumps({"t": t2 - 5, "goal": "explore"}) + "\n")
        # A `trust-log --jsonl` stand-in: one agent-agent row, one with a person.
        fake = os.path.join(tmp, "trust-log")
        with open(fake, "w") as f:
            f.write("#!/bin/sh\n"
                    f"echo '{json.dumps({'tick': 5, 'verb': 'door', 'presence': 'asleep', 'actor': {'wallet': AGENT_W}, 'counterparty': {'wallet': OTHER_AGENT_W}})}'\n"
                    f"echo '{json.dumps({'tick': 6, 'verb': 'cont', 'presence': 'awake', 'actor': {'wallet': AGENT_W}, 'counterparty': {'wallet': HUMAN_W}})}'\n")
        os.chmod(fake, 0o755)
        out = os.path.join(tmp, "out")
        args = argparse.Namespace(world=world, journals=os.path.join(state, "journal"),
                                  keys=os.path.join(state, "keys"), out=out, trust_log=fake,
                                  rent=os.path.join(state, "public", "rent.json"),
                                  seed=20260731, commit="unknown", proto=None, force=False)
        packed = run(args, now=t2 + 3600)
        check("both closed wipes are packed", packed == [1, 2])
        index = json.load(open(os.path.join(out, "index.json")))
        w2 = next(e for e in index["wipes"] if e["wipe"] == 2)
        check("the index is gates-data-1, CC-BY-4.0", index["format"] == FORMAT and index["license"] == LICENSE)
        check("a wipe's entry names its files relative to the data root",
              w2["files"] == {"decisions": "wipe-2/decisions.jsonl.gz", "standings": "wipe-2/standings.json",
                              "datasheet": "wipe-2/datasheet.json", "sums": "wipe-2/SHA256SUMS"})
        check("wipe 2 runs from wipe 1's close to its own", (w2["from"], w2["to"]) == (t1, t2))
        d2 = os.path.join(out, "wipe-2")
        size = sum(os.path.getsize(os.path.join(d2, f)) for f in os.listdir(d2))
        check("bytes counts every file of the wipe", w2["bytes"] == size)
        rows = [json.loads(x) for x in gzip.decompress(open(os.path.join(d2, "decisions.jsonl.gz"), "rb").read())
                .decode().splitlines()]
        check("only the wipe's own hours, only known agents", [r["goal"] for r in rows] == ["meet", "build"]
              and w2["decisions"] == 2 and w2["agents"] == 2)
        meet = rows[0]
        person = meet["heard"][0]
        check("a person's line keeps its slot, nameless and wordless",
              person == {"from": PERSON, "agent": False, "friend": True, "channel": "global", "text": None})
        check("an agent's line is kept", meet["heard"][1]["text"] == "hi Rook" and meet["heard"][1]["from"] == "Wren")
        check("team chat says nothing, even an agent's", meet["heard"][2]["text"] is None
              and rows[1]["say"] is None and rows[1]["to"] == "team")
        check("a line with no agent flag is a person's", meet["heard"][3]["from"] == PERSON
              and meet["heard"][3]["text"] is None)
        check("a friend keeps no name or wallet", meet["friend"] == {"online": True})
        check("a person named or addressed by an agent is replaced",
              meet["reason"] == f"{PERSON} asked; she is {PERSON}")
        check("labels ride through", (meet["harness"], meet["answered_by"], rows[1]["model"])
              == ("claude-code/2.1.0", "harness", "test-model"))
        board = json.load(open(os.path.join(d2, "standings.json")))["rows"]
        check("a person's standings row keeps its scores and loses its wallet",
              board[0] == {"wallet": None, "name": PERSON, "agent": False, "island": 590, "works": 500,
                           "hoard": 90, "kills": 4, "deaths": 0, "played_min": 600, "extracted": 0})
        check("an agent's row keeps its wallet", board[1]["wallet"] == AGENT_W and board[1]["name"] == "Rook")
        trust = [json.loads(x) for x in gzip.decompress(open(os.path.join(d2, "trust.jsonl.gz"), "rb").read())
                 .decode().splitlines()]
        check("the trust log keeps agent-agent rows only", len(trust) == 1 and trust[0]["counterparty"] == OTHER_AGENT_W)
        blob = b"".join(open(os.path.join(d2, f), "rb").read() if not f.endswith(".gz")
                        else gzip.decompress(open(os.path.join(d2, f), "rb").read()) for f in os.listdir(d2))
        check("no person's wallet or name is anywhere in the pack",
              HUMAN_W.encode() not in blob and FRIEND_W.encode() not in blob
              and b"Alice" not in blob and b"Bob" not in blob)
        sheet = json.load(open(os.path.join(d2, "datasheet.json")))
        check("the datasheet carries the licence, caveats, seed and a contact",
              sheet["license"] == LICENSE and sheet["caveats"] == CAVEATS and sheet["seed"] == 20260731
              and sheet["contact"] == CONTACT and "decisions.jsonl.gz" in sheet["schema"])
        ok_sums = True
        for line in open(os.path.join(d2, "SHA256SUMS")):
            digest, name = line.split()
            ok_sums &= hashlib.sha256(open(os.path.join(d2, name), "rb").read()).hexdigest() == digest
        check("SHA256SUMS matches every file", ok_sums)
        check("a packed wipe is not packed again", run(args, now=t2 + 7200) == [])
        check("and the index still holds both", [e["wipe"] for e in json.load(open(os.path.join(out, "index.json")))["wipes"]] == [1, 2])
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    if failed:
        print(f"\n{len(failed)} pack checks FAILED")
        return 1
    print("\nall pack checks pass")
    return 0


def main():
    env = os.environ.get
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--fixture", action="store_true", help="pack a built-in wipe into a temp dir and check it")
    ap.add_argument("--world", default=env("GATES_WORLD", "/home/master/gates-data/island.world"),
                    help="the shard's world_file; its wipes are archived beside it")
    ap.add_argument("--journals", default=env("GATES_JOURNALS", "/var/lib/gates-buddies/journal"))
    ap.add_argument("--keys", default=env("GATES_AGENT_KEYS", "/var/lib/gates-buddies/keys"),
                    help="where each journal's agent wallet is (<id>.addr)")
    ap.add_argument("--rent", default=env("GATES_RENT", "/var/lib/gates-buddies/public/rent.json"),
                    help="the runner's rent.json, which names each agent's wallet too")
    ap.add_argument("--out", default=env("GATES_DATA", "/srv/gates-data"))
    ap.add_argument("--trust-log", default=env("GATES_TRUST_LOG", ""), help="the trust-log binary (optional)")
    ap.add_argument("--shard-config", default=env("GATES_SHARD_CONFIG", "/home/master/gates-deploy/shard-public.toml"),
                    help="where the seed is read")
    ap.add_argument("--seed", type=int, default=None)
    ap.add_argument("--commit", default=env("GATES_COMMIT", "unknown"))
    ap.add_argument("--proto", type=int, default=int(env("GATES_PROTO")) if env("GATES_PROTO", "").isdigit() else None)
    ap.add_argument("--force", action="store_true", help="pack every closed wipe again")
    args = ap.parse_args()
    if args.fixture:
        sys.exit(fixture())
    if args.seed is None:
        args.seed = seed_of(args.shard_config)
        if args.seed is None:
            sys.exit(f"pack: no seed in {args.shard_config}; pass --seed")
    if not re.fullmatch(r"[0-9a-f]{7,40}|unknown", args.commit):
        sys.exit("pack: --commit is hex or unknown")
    packed = run(args)
    log(f"packed {packed or 'nothing new'} into {args.out}")


if __name__ == "__main__":
    main()
