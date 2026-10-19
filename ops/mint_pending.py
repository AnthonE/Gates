#!/usr/bin/env python3
"""Send the tokens the shard sold: one `EloItem:GATES` mint per line of
`<world>.mints` not yet in `<world>.minted`.

    python3 ops/mint_pending.py --world /home/master/gates-data/island.world
    python3 ops/mint_pending.py --world … --dry-run     # say what it would send

THE OUTFITTER sells the Abyss Rug's unlock for junk (`content/arc.toml`).
The shard takes the price and writes the sale to `<world>.mints`
(`crates/server/src/mints.rs`): `ref wallet catalog`. This mints each one
on morr, where the key is (`ELO_DEPLOY_KEY` in `/data/apps/secrets/keys.env`,
the item contract's owner and minter): the key never comes to this box,
only the sale's public fields go over ssh. Each sent mint is appended to
`<world>.minted` as `ref wallet catalog tx`, which the shard watches and
answers by reading the wallet's items again. That is when the unlock
arrives, a minute or two after the sale.

The `mintRef` is keccak("gates:buy:<catalog>:<wallet>:<ref>"), so a sale is
minted once however often this runs: one the chain already holds
(`mintRef used`) is logged as sent without a second transaction. Every mint
is simulated (`cast call`) before it is sent, so a sale the contract would
refuse costs no gas and is retried on the next run.

Run by `ops/gates-mint.timer` every 30 s. No sale, no ssh.
"""
from __future__ import annotations

import argparse
import fcntl
import re
import subprocess
import sys
from pathlib import Path

WALLET = re.compile(r"^0x[0-9a-f]{40}$")
REF = re.compile(r"^[0-9A-Za-z-]{1,32}$")
#: Sales per run: a backlog drains over a few runs, not one long ssh.
BATCH = 20
CHAIN_ID = 4663  # RH-Chain, where EloItem:GATES lives

# The morr side. Positional arguments only, each checked here before it is
# sent: the mode (`send`, or `check` to simulate and send nothing), then
# ref, wallet, catalog, three per sale. Prints one line per sale,
# `ok|used|fail ref wallet catalog tx-or-reason`, and never the RPC URL or
# the key (cast's own errors are reduced to a reason word).
REMOTE = r'''
set -u
MODE=$1; shift
export PATH="$PATH:$HOME/.foundry/bin"
set -a; . /data/apps/secrets/keys.env; set +a
ITEM=$(python3 -c 'import json; print(json.load(open("/data/apps/scry-forge/contracts/deployments.json"))["chains"]["4663"]["contracts"]["EloItem:GATES"]["address"])' 2>/dev/null)
FROM=$(cast wallet address --private-key "$ELO_DEPLOY_KEY" 2>/dev/null)
RPC=""
for r in $(echo "${ELO_RH_RPC_POOL:-}" | tr ',; ' '\n\n\n'); do
  if [ "$(cast chain-id --rpc-url "$r" 2>/dev/null)" = "4663" ]; then RPC=$r; break; fi
done
if [ -z "$ITEM" ] || [ -z "$FROM" ] || [ -z "$RPC" ]; then
  while [ $# -ge 3 ]; do echo "fail $1 $2 $3 no-chain"; shift 3; done
  exit 0
fi
while [ $# -ge 3 ]; do
  ref=$1; wallet=$2; cat=$3; shift 3
  mref=$(cast keccak "gates:buy:$cat:$wallet:$ref")
  sim=$(cast call --from "$FROM" "$ITEM" "mint(address,uint32,uint8,bytes32)" "$wallet" "$cat" 0 "$mref" --rpc-url "$RPC" 2>&1)
  if [ $? -ne 0 ]; then
    case "$sim" in
      *"mintRef used"*) echo "used $ref $wallet $cat -" ;;
      *) echo "fail $ref $wallet $cat refused" ;;
    esac
    continue
  fi
  if [ "$MODE" != send ]; then echo "ok $ref $wallet $cat simulated"; continue; fi
  out=$(cast send "$ITEM" "mint(address,uint32,uint8,bytes32)" "$wallet" "$cat" 0 "$mref" --rpc-url "$RPC" --private-key "$ELO_DEPLOY_KEY" --json 2>/dev/null)
  tx=$(printf '%s' "$out" | python3 -c 'import json,sys
try:
    r = json.load(sys.stdin)
    print(r["transactionHash"] if str(r.get("status")) in ("1", "0x1") else "")
except Exception:
    print("")' 2>/dev/null)
  if [ -n "$tx" ]; then echo "ok $ref $wallet $cat $tx"; else echo "fail $ref $wallet $cat send"; fi
done
'''


def sales(text: str) -> list[tuple[str, str, int]]:
    """`ref wallet catalog` lines, checked; anything else is skipped."""
    out = []
    for line in text.splitlines():
        f = line.split()
        if len(f) < 3 or line.lstrip().startswith("#"):
            continue
        ref, wallet, cat = f[0], f[1], f[2]
        if REF.match(ref) and WALLET.match(wallet) and cat.isdigit() and int(cat) < 2**32:
            out.append((ref, wallet, int(cat)))
    return out


def pending(mints: str, minted: str) -> list[tuple[str, str, int]]:
    """The sales not yet sent, in sale order, each once."""
    done = set(sales(minted))
    out, seen = [], set()
    for s in sales(mints):
        if s not in done and s not in seen:
            seen.add(s)
            out.append(s)
    return out


def results(stdout: str, asked: list[tuple[str, str, int]]) -> tuple[list[str], list[str]]:
    """The morr side's answer: lines to append to the minted log (`ok` and
    `used`, only for sales that were asked), and the failures to report."""
    asked_set = set(asked)
    sent, failed = [], []
    for line in stdout.splitlines():
        f = line.split()
        if len(f) != 5 or not f[3].isdigit():
            continue
        verdict, sale = f[0], (f[1], f[2], int(f[3]))
        if sale not in asked_set:
            continue
        if verdict in ("ok", "used"):
            sent.append(f"{sale[0]} {sale[1]} {sale[2]} {f[4]}")
        else:
            failed.append(f"{sale[0]} {sale[1]} {sale[2]}: {f[4]}")
    return sent, failed


def run_remote(host: str, batch: list[tuple[str, str, int]], mode: str = "send") -> str:
    args = [mode] + [str(x) for s in batch for x in s]
    p = subprocess.run(
        ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", host, "bash", "-s", "--", *args],
        input=REMOTE,
        capture_output=True,
        text=True,
        timeout=600,
    )
    return p.stdout


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--world", required=True, type=Path, help="the shard's world file")
    ap.add_argument("--host", default="morr", help="where the key is (ssh)")
    ap.add_argument("--dry-run", action="store_true", help="list what would be sent")
    ap.add_argument("--check", action="store_true",
                    help="simulate every pending mint on morr and send nothing")
    a = ap.parse_args(argv)

    mints_p = Path(f"{a.world}.mints")
    minted_p = Path(f"{a.world}.minted")
    lock = open(f"{a.world}.minted.lock", "w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        return 0  # the last run is still sending
    read = lambda p: p.read_text() if p.exists() else ""
    todo = pending(read(mints_p), read(minted_p))[:BATCH]
    if not todo:
        return 0
    if a.dry_run:
        for ref, wallet, cat in todo:
            print(f"would mint catalog {cat} to {wallet} (sale {ref})")
        return 0
    if a.check:
        print(run_remote(a.host, todo, "check"), end="")
        return 0
    sent, failed = results(run_remote(a.host, todo), todo)
    if sent:
        with open(minted_p, "a") as f:
            f.write("".join(line + "\n" for line in sent))
    for line in sent:
        print(f"mint: sent {line}")
    for line in failed:
        print(f"mint: not sent {line}, retried next run", file=sys.stderr)
    if len(sent) + len(failed) < len(todo):
        print(f"mint: {len(todo) - len(sent) - len(failed)} sales had no answer from {a.host}",
              file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
