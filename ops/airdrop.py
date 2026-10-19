#!/usr/bin/env python3
"""Give an unlock to every wallet that played this week and holds a coin.

    python3 ops/airdrop.py --world /home/master/gates-data/island.world \\
        --holders junk-holders.json --catalog 10 [--min-minutes 120] \\
        [--min-coin 1] [--agents] [--standings FILE] [--dry-run]

Rust gave its Frog Boots to every account that played its early access, and
owning them is what lets you craft them. This is that for a week of Gates.
A wallet gets catalog `--catalog` if it is on the shard's standings with at
least `--min-minutes` played this wipe and holds at least `--min-coin` whole
coins in `--holders`. Catalog 10 is the Abyss Rug, an `unlock = true` row in
`content/skins.toml`. Declared agents are left out unless `--agents`, as
payouts leave them out.

`--holders` is a holder snapshot from scry-forge:
`python3 meter/holder_snapshot.py snapshot --token <JUNK> --label JUNK`.
An unverified snapshot is refused. Take it near the wipe so a wallet that
sold before then misses out.

The standings are `<world>.standings`, this wipe's. After the wipe, pass the
archived file with `--standings` (`<world>.wipes/<unix>/`). The list is written
to `<world>.airdrop` (`server/src/gifts.rs`) and the shard reads it again
within a minute. Lines already there are kept, so a second run only adds.
`--dry-run` prints the count and the wallets and writes nothing.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
from pathlib import Path

WALLET = re.compile(r"^0x[0-9a-f]{40}$")
GUEST = "0x" + "0" * 40
FLAG_AGENT = 1


def played(standings: Path, min_minutes: int, agents: bool) -> set[str]:
    """Wallets with `min_minutes` played on this standings file.

    A row is `row key given deeds kills deaths hoard island played flags
    extracted taken label`, tab-separated (`server/src/standings.rs`).
    """
    out = set()
    for line in standings.read_text().splitlines():
        f = line.split("\t")
        if len(f) < 13 or f[0] != "row":
            continue
        key = f[1].lower()
        if not WALLET.match(key) or key == GUEST:
            continue
        if int(f[9]) & FLAG_AGENT and not agents:
            continue
        if int(f[8]) >= min_minutes:
            out.add(key)
    return out


def holding(snapshot: dict, min_coin: float) -> set[str]:
    """Wallets in a verified holder snapshot with `min_coin` whole coins.
    Contracts are left out (a pool holds coins for its traders)."""
    if not snapshot.get("verified"):
        sys.exit("airdrop: the holder snapshot is not verified, take it again")
    floor = int(round(min_coin * 10 ** int(snapshot.get("decimals", 18))))
    return {
        h["wallet"].lower()
        for h in snapshot.get("holders", [])
        if not h.get("is_contract") and int(h["raw"]) >= floor
    }


def read_list(path: Path) -> dict[str, set[int]]:
    rows: dict[str, set[int]] = {}
    if path.exists():
        for line in path.read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            wallet, *ids = line.split()
            rows.setdefault(wallet, set()).update(int(i) for i in ids)
    return rows


def write_list(path: Path, rows: dict[str, set[int]], note: str) -> None:
    text = "# Airdrops (ops/airdrop.py), read by the shard: wallet, then catalog ids.\n"
    text += f"# {note}\n"
    for wallet in sorted(rows):
        text += wallet + "".join(f" {i}" for i in sorted(rows[wallet])) + "\n"
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_text(text)
    os.replace(tmp, path)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--world", required=True, type=Path, help="the shard's world_file")
    ap.add_argument("--holders", required=True, type=Path, help="holder_snapshot.py output")
    ap.add_argument("--catalog", required=True, type=int, help="the unlock's catalog id")
    ap.add_argument("--min-minutes", type=int, default=120, help="played this wipe (120)")
    ap.add_argument("--min-coin", type=float, default=1.0, help="whole coins held (1)")
    ap.add_argument("--agents", action="store_true", help="give declared agents one too")
    ap.add_argument("--standings", type=Path, help="default: <world>.standings")
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args(argv)

    standings = a.standings or Path(f"{a.world}.standings")
    out = Path(f"{a.world}.airdrop")
    snapshot = json.loads(a.holders.read_text())
    players = played(standings, a.min_minutes, a.agents)
    holders = holding(snapshot, a.min_coin)
    given = players & holders
    rows = read_list(out)
    new = sorted(w for w in given if a.catalog not in rows.get(w, set()))

    print(f"airdrop: {standings}: {len(players)} wallets played {a.min_minutes}+ min")
    print(f"airdrop: {a.holders}: {len(holders)} hold {a.min_coin:g}+ "
          f"{snapshot.get('label', 'coins')} at block {snapshot.get('block')}")
    print(f"airdrop: {len(given)} get catalog {a.catalog}, {len(new)} of them new")
    if a.dry_run:
        for w in sorted(given):
            print(f"  {w}{'' if w in new else '  (already)'}")
        return 0
    for w in given:
        rows.setdefault(w, set()).add(a.catalog)
    note = (f"catalog {a.catalog}: played {a.min_minutes}+ min on {standings.name}, "
            f"held {a.min_coin:g}+ {snapshot.get('label', '')} at block "
            f"{snapshot.get('block')}, {time.strftime('%Y-%m-%d %H:%M UTC', time.gmtime())}")
    write_list(out, rows, note)
    print(f"airdrop: wrote {out} ({len(rows)} wallets); the shard reads it within a minute")
    return 0


if __name__ == "__main__":
    sys.exit(main())
