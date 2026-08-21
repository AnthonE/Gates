#!/usr/bin/env python3
"""Write the `elo-shardlist-v1` document — the list of shards players join.

    ./ci/shardlist.py                   write target/servers.json from shards.toml
    ./ci/shardlist.py --out -           write it to stdout
    ./ci/shardlist.py --self-test       the gate: no network, no cargo

**The shard list is the game's to serve.** That is elo's rule, not ours
(`docs/LAUNCHER.md` §6): the launcher reads `manifest.servers.url`, fetches it,
and renders the rows — it "does not invent, cache or rank them", and its broker
refuses even to proxy the fetch. So this file is Gates' end of a seam whose
other end has been built and dark for a while: the launcher's Servers window
names *this game* in its empty state, and one published document lights it.

Two consumers, one document. The launcher's Servers window is one; the game's
own intro screen (`crates/client/src/render/menu.rs`) is the other, and it
parses this with `crates/client/src/shardlist.rs`. The caps below are that
parser's caps, restated here so the generator cannot write a document its own
client would refuse.

## Three things this deliberately does NOT do

**It does not invent a player count — and it does not measure one either.**
`players` is omitted from a row unless `shards.toml` states one. This script
could poll each shard's `/status.json` and bake the answer in, and that would
be worse than the `?` it replaced: a number written at generation time
describes the shard as it was whenever an operator last ran this command, and
the document is then copied to an origin and served for hours or weeks. A
count that is confidently wrong is a live shard with four people on it reading
as empty, arriving by a slower road.

So a row carries **`status_url`** instead — where that shard answers
`GET /status.json` (`crates/server/src/status.rs`) — and each reader polls it
live. The generator stays a pure generator with no network (which is what lets
`--self-test` promise the same), and the number a player reads is one somebody
measured seconds ago. Both readers already do this: the game's menu polls every
`STATUS_POLL_SECS`, and so does the launcher's Servers window.

**It does not type the player cap.** `max_players` defaults to `MAX_PLAYERS`
parsed out of `crates/sim-core/src/limits.rs`, because a published cap that
disagrees with the cap the shard actually enforces is a lie the player only
discovers at a refused join. Override it per row only to publish something
*lower* than the sim allows.

**It does not publish.** Publishing is an operator act (`CLAUDE.md`): serving
this document is what makes the shards findable, and the address it advertises
is a live game server. This prints the command and performs neither it nor the
manifest edit on elo's side that points at the result.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

KIND = "elo-shardlist-v1"

# Where the document is served from once it is on the origin's disk.
#
# ⚠ This used to print `https://elopros.com/depot/gates/servers.json`
# and **that url could never have worked**: `/depot/` is not a location on
# that origin — depot bytes have always come through the meter — so the one
# act this script told an operator to perform ended at a 404, and
# `manifest.servers.url` stayed null for days while the public shard was up
# and both readers drew "no shards published". elo built the route rather
# than the nginx alias (`GET /api/launcher/servers/{slug}`, elo
# `docs/client/LAUNCHER.md` §6), serving `$SCRY_DEPOTS_DIR/<slug>/servers.json`
# byte-for-byte. The scp destination below was always right; only the url the
# manifest points at was wrong.
PUBLISH_URL = "https://elopros.com/api/launcher/servers/gates"

# The client's caps, from `crates/client/src/shardlist.rs`. Restated rather
# than imported because that is Rust; `--self-test` is what keeps them honest,
# and it reads the constants out of the Rust source instead of trusting these.
MAX_SHARDS = 64
MAX_FIELD_BYTES = 96
MAX_DOC_BYTES = 64 * 1024
# A url is legitimately longer than a name, so `status_url` has its own cap.
MAX_URL_BYTES = 256

LIMITS_RS = ROOT / "crates" / "sim-core" / "src" / "limits.rs"
SHARDS_TOML = ROOT / "shards.toml"


def sim_max_players(limits_rs: Path = LIMITS_RS) -> int:
    """`MAX_PLAYERS` as the sim defines it. Read, never typed — see the header."""
    text = limits_rs.read_text(encoding="utf-8")
    m = re.search(r"pub const MAX_PLAYERS:\s*usize\s*=\s*([0-9_]+)\s*;", text)
    if not m:
        raise SystemExit(
            f"shardlist: no `pub const MAX_PLAYERS` in {limits_rs}. It is the "
            "source of the published cap; publishing a typed one instead is "
            "exactly what this refuses to do."
        )
    return int(m.group(1).replace("_", ""))


def check_addr(addr: str) -> None:
    """The same shape check `shardlist::check_addr` makes, and for the reason.

    A NAME is the normal case: the public shard's certificate is issued for
    `game.elopros.com` and the transport needs that name for SNI, so an
    address here is never parsed as an IP:port pair.
    """
    if not addr or addr.strip() != addr:
        raise SystemExit(f"shardlist: {addr!r} is empty or padded")
    if "://" in addr or "/" in addr:
        raise SystemExit(f"shardlist: {addr!r} is a url, not a host:port")
    if any(c.isspace() for c in addr):
        raise SystemExit(f"shardlist: {addr!r} contains whitespace")
    if addr.startswith("["):
        host, _, rest = addr[1:].partition("]")
        if not rest.startswith(":"):
            raise SystemExit(f"shardlist: {addr!r} has no port")
        port = rest[1:]
    else:
        host, _, port = addr.rpartition(":")
        if not host:
            raise SystemExit(f"shardlist: {addr!r} has no port — expected host:port")
        if ":" in host:
            raise SystemExit(f"shardlist: {addr!r} is a bare IPv6; bracket it")
    if not host:
        raise SystemExit(f"shardlist: {addr!r} has no host")
    if not port.isdigit() or not 0 < int(port) < 65536:
        raise SystemExit(f"shardlist: {addr!r} has a bad port {port!r}")


def check_status_url(url: str) -> None:
    """The same shape check `shardlist::check_status_url` makes.

    Refused rather than carried: a url a reader cannot fetch becomes a row that
    blames the network for a typo in this file.
    """
    if not isinstance(url, str) or not url.strip():
        raise SystemExit("shardlist: status_url is empty")
    if url.strip() != url:
        raise SystemExit(f"shardlist: status_url {url!r} is padded")
    if len(url.encode()) > MAX_URL_BYTES:
        raise SystemExit(f"shardlist: status_url is over the {MAX_URL_BYTES}-byte cap")
    if any(c.isspace() for c in url):
        raise SystemExit(f"shardlist: status_url {url!r} contains whitespace")
    if not url.startswith(("https://", "http://")):
        raise SystemExit(f"shardlist: status_url {url!r} is not an http(s) url")
    rest = url.split("://", 1)[1]
    if not rest or rest.startswith("/"):
        raise SystemExit(f"shardlist: status_url {url!r} has no host")


def build_doc(rows: list[dict], cap: int) -> dict:
    """Turn parsed `shards.toml` rows into the document, validating as it goes."""
    if len(rows) > MAX_SHARDS:
        raise SystemExit(
            f"shardlist: {len(rows)} shards, over the {MAX_SHARDS} cap the client "
            "refuses at. Raise it on BOTH sides or publish fewer."
        )
    seen: set[str] = set()
    out = []
    for i, row in enumerate(rows):
        def need(key: str) -> str:
            v = row.get(key)
            if not isinstance(v, str) or not v.strip():
                raise SystemExit(f"shardlist: shard {i} has no {key}")
            if len(v.encode()) > MAX_FIELD_BYTES:
                raise SystemExit(
                    f"shardlist: shard {i} {key} is over the {MAX_FIELD_BYTES}-byte cap"
                )
            return v

        sid, name, addr = need("id"), need("name"), row.get("addr", "")
        check_addr(addr if isinstance(addr, str) else "")
        if len(addr.encode()) > MAX_FIELD_BYTES:
            raise SystemExit(f"shardlist: shard {i} addr is over the cap")
        if sid in seen:
            # The launcher keys its rows on this and would silently draw one.
            raise SystemExit(f"shardlist: duplicate shard id {sid!r}")
        seen.add(sid)

        entry = {"id": sid, "name": name, "addr": addr}
        # Lower than the sim's cap is a real choice; higher is a promise the
        # shard will break at the door.
        mp = row.get("max_players", cap)
        if not isinstance(mp, int) or mp <= 0:
            raise SystemExit(f"shardlist: shard {i} max_players must be a positive int")
        if mp > cap:
            raise SystemExit(
                f"shardlist: shard {i} publishes max_players {mp} over the sim's "
                f"MAX_PLAYERS {cap} — the shard would refuse the joiner the list invited"
            )
        entry["max_players"] = mp
        if "map" in row:
            entry["map"] = need("map")
        # Where this shard's LIVE count is read from. Optional — a shard whose
        # operator has not armed `status_addr` (or has not opened the port)
        # states nothing, and both readers keep drawing `?` for it.
        if "status_url" in row:
            check_status_url(row["status_url"])
            entry["status_url"] = row["status_url"]
        # `players` only if the file states one. See the header: nothing here
        # can measure it, and a zero would read as an empty shard.
        if "players" in row:
            p = row["players"]
            if not isinstance(p, int) or p < 0:
                raise SystemExit(f"shardlist: shard {i} players must be a non-negative int")
            entry["players"] = p
        out.append(entry)
    return {"servers": out}


def load_rows(path: Path) -> list[dict]:
    if not path.exists():
        raise SystemExit(
            f"shardlist: {path} does not exist. Copy shards.toml.example and edit it; "
            "an empty list is a valid document and the launcher renders it as "
            "'no shards up', so there is no reason to invent a row."
        )
    with path.open("rb") as fh:
        data = tomllib.load(fh)
    rows = data.get("shard", [])
    if not isinstance(rows, list):
        raise SystemExit("shardlist: [[shard]] must be an array of tables")
    return rows


def render(doc: dict) -> str:
    text = json.dumps(doc, indent=2, sort_keys=True) + "\n"
    if len(text.encode()) > MAX_DOC_BYTES:
        raise SystemExit(
            f"shardlist: the document is over the {MAX_DOC_BYTES}-byte cap the client reads at"
        )
    return text


def self_test() -> int:
    """No network, no cargo. Runs in ci/gates.sh."""
    passed = 0

    def ok(cond, label):
        nonlocal passed
        if not cond:
            print(f"FAIL: {label}")
            sys.exit(1)
        passed += 1
        print(f"  ok: {label}")

    def refuses(rows, cap, why):
        try:
            build_doc(rows, cap)
        except SystemExit:
            return True
        print(f"FAIL: accepted {why}")
        sys.exit(1)

    cap = sim_max_players()
    ok(cap > 0, f"MAX_PLAYERS reads out of limits.rs ({cap})")

    # The caps here must be the caps the client refuses at, or this script
    # will happily write documents the game cannot read.
    rs = (ROOT / "crates" / "client" / "src" / "shardlist.rs").read_text(encoding="utf-8")
    for name, val in (("MAX_SHARDS", MAX_SHARDS), ("MAX_FIELD_BYTES", MAX_FIELD_BYTES),
                      ("MAX_URL_BYTES", MAX_URL_BYTES)):
        m = re.search(rf"pub const {name}: usize = ([0-9_]+);", rs)
        ok(m is not None, f"{name} is declared in the client's parser")
        ok(int(m.group(1).replace("_", "")) == val, f"{name} agrees with the client ({val})")
    ok(f'"{KIND}"' in rs, "the client names the same document kind")

    doc = build_doc(
        [{"id": "eu-1", "name": "Gates EU 1", "addr": "game.elopros.com:61234",
          "map": "island"}], cap)
    row = doc["servers"][0]
    ok(row["addr"] == "game.elopros.com:61234", "a hostname survives as a hostname")
    ok(row["max_players"] == cap, "max_players defaults to the sim's own cap")
    # The load-bearing one: an unmeasured count must be ABSENT, not zero.
    ok("players" not in row, "no player count is invented")
    ok("ping_ms" not in row, "no ping is invented")

    ok("players" not in row, "no player count is BAKED either — status_url is how it lights")

    # `status_url` — the field that makes the count live. Optional, and the
    # row above proves its absence is fine.
    lit = build_doc(
        [{"id": "eu-1", "name": "Gates EU 1", "addr": "game.elopros.com:61234",
          "status_url": "https://game.elopros.com:8080/status.json"}], cap)["servers"][0]
    ok(lit["status_url"] == "https://game.elopros.com:8080/status.json",
       "a row may name where its live count is polled from")
    # Still no baked count beside it: naming the endpoint is not claiming a
    # number, and a reader that cannot reach it must draw `?`.
    ok("players" not in lit, "naming a status endpoint does not invent a count")

    for bad in ("game.elopros.com:8080/status.json", "ftp://h/status.json",
                "https://", "https:///status.json", "https://h/ status.json",
                "", "  https://h/s.json"):
        refuses([{"id": "a", "name": "n", "addr": "h:1", "status_url": bad}], cap,
                f"status_url {bad!r}")
    refuses([{"id": "a", "name": "n", "addr": "h:1",
              "status_url": "https://h/" + "x" * MAX_URL_BYTES}], cap,
            "a status_url over the cap")

    ok(build_doc([], cap) == {"servers": []}, "an empty list is a valid document")

    refuses([{"id": "a", "name": "n", "addr": "game.elopros.com"}], cap, "no port")
    refuses([{"id": "a", "name": "n", "addr": "https://h:1"}], cap, "a url")
    refuses([{"id": "a", "name": "n", "addr": "h:1", "max_players": cap + 1}], cap,
            "a cap over the sim's")
    refuses([{"id": "a", "name": "n", "addr": "h:1"},
             {"id": "a", "name": "n2", "addr": "h:2"}], cap, "a duplicate id")
    refuses([{"id": "a", "addr": "h:1"}], cap, "a row with no name")
    refuses([{"id": "x" * (MAX_FIELD_BYTES + 1), "name": "n", "addr": "h:1"}], cap,
            "an id over the field cap")
    refuses([{"id": f"s{i}", "name": "n", "addr": f"h:{i + 1}"}
             for i in range(MAX_SHARDS + 1)], cap, "more shards than the cap")

    # The example must be a document this script accepts, or it teaches a
    # shape that does not work.
    example = ROOT / "shards.toml.example"
    ok(example.exists(), "shards.toml.example is committed")
    with example.open("rb") as fh:
        rows = tomllib.load(fh).get("shard", [])
    text = render(build_doc(rows, cap))
    ok(json.loads(text)["servers"], "the example produces at least one shard")
    ok(len(text.encode()) <= MAX_DOC_BYTES, "the example is under the document cap")

    print(f"shardlist self-test: {passed} checks green")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--shards", default=str(SHARDS_TOML), help="the shard table")
    ap.add_argument("--out", default=str(ROOT / "target" / "servers.json"),
                    help="where to write it; - for stdout")
    ap.add_argument("--self-test", action="store_true", help="the gate; no network")
    a = ap.parse_args()

    if a.self_test:
        return self_test()

    text = render(build_doc(load_rows(Path(a.shards)), sim_max_players()))
    if a.out == "-":
        sys.stdout.write(text)
        return 0
    out = Path(a.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(text, encoding="utf-8")
    n = len(json.loads(text)["servers"])
    print(f"shardlist: {n} shard(s) -> {out}")
    print()
    print("Publishing is an operator act. To serve it:")
    print(f"  scp {out} <origin>:/data/apps/scry-data/depots/gates/servers.json")
    print("...and point elo's Gates manifest at it, once:")
    print(f'  "servers": {{"url": "{PUBLISH_URL}",')
    print(f'               "kind": "{KIND}"}}')
    print("Until that field is set, the launcher's Servers window stays dark by design.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
