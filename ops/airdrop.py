#!/usr/bin/env python3
"""Who an airdrop goes to: every wallet that played this week and holds a coin.

    python3 ops/airdrop.py --standings /home/master/gates-data/island.world.standings \\
        --holders junk-holders.json --out abyss-rug.wallets [--min-minutes 120] \\
        [--min-coin 1]

Rust gave its Frog Boots to every account that played its early access, and
owning them is what lets you craft them. This is that for a week of Gates: a
wallet on the standings with at least `--min-minutes` played this wipe that
holds at least `--min-coin` whole coins in `--holders` makes the list. Agents
and people are counted the same.

`--holders` is a holder snapshot from the platform repo,
`python3 meter/holder_snapshot.py snapshot --token <JUNK> --label JUNK`, taken
near the wipe. An unverified snapshot is refused. The standings are this
wipe's `<world>.standings`; after the wipe, pass the archived one in
`<world>.wipes/<unix>/`.

The list is one wallet a line. The platform's `contracts/item_airdrop.py` turns
it into the mints: one token each on the item contract, which is what the
shard counts as owning the unlock (`content/skins.toml`). The token trades
like any item, so whoever holds it can craft.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

WALLET = re.compile(r"^0x[0-9a-f]{40}$")
GUEST = "0x" + "0" * 40


def played(standings: Path, min_minutes: int) -> set[str]:
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
        if WALLET.match(key) and key != GUEST and int(f[8]) >= min_minutes:
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


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--standings", required=True, type=Path, help="<world>.standings")
    ap.add_argument("--holders", required=True, type=Path, help="holder_snapshot.py output")
    ap.add_argument("--out", required=True, type=Path, help="the list, one wallet a line")
    ap.add_argument("--min-minutes", type=int, default=120, help="played this wipe (120)")
    ap.add_argument("--min-coin", type=float, default=1.0, help="whole coins held (1)")
    a = ap.parse_args(argv)

    snapshot = json.loads(a.holders.read_text())
    players = played(a.standings, a.min_minutes)
    holders = holding(snapshot, a.min_coin)
    wallets = sorted(players & holders)
    a.out.write_text("".join(w + "\n" for w in wallets))
    print(f"airdrop: {a.standings}: {len(players)} wallets played {a.min_minutes}+ min")
    print(f"airdrop: {a.holders}: {len(holders)} hold {a.min_coin:g}+ "
          f"{snapshot.get('label', 'coins')} at block {snapshot.get('block')}")
    print(f"airdrop: {len(wallets)} on the list, written to {a.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
