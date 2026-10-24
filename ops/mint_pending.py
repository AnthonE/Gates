#!/usr/bin/env python3
"""Send the tokens the shard owes: one `EloItem:GATES` mint per line of
`<world>.mints` not yet in `<world>.minted`, and a free copy of Gates
(`EloGameTicket:GATES`) to every person who has played and holds none.

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

**Copies.** Every wallet in `<world>.online.json` that is not an agent gets
a copy of the game: `mintFree(wallet)` on the ticket contract, whose free
door gives each wallet one, and which anyone may call, so the house pays
the gas and the key is still only on morr. A wallet that already holds a
copy (bought, or given) is noted and costs nothing. Each wallet is looked at
once and written to `<world>.copies` (`wallet ok|had|used tx`); the online
file is rewritten every minute, so a copy lands a minute or two after the
first join.

**Free till December.** Copies are given until `FREE_UNTIL` (1 Dec 2026,
00:00 UTC). At that moment `ops/gates-free-door.timer` runs this with
`--close-free-door`, which posts `setFreePerWallet(0)` (the owner's call,
from morr), so nobody, house or player, can mint one free after it. Copies
already given stay given; from then on a copy is bought.

Run by `ops/gates-mint.timer` every 30 s. Nothing to do, no ssh.
"""
from __future__ import annotations

import argparse
import datetime
import fcntl
import re
import subprocess
import sys
from pathlib import Path

WALLET = re.compile(r"^0x[0-9a-f]{40}$")
REF = re.compile(r"^[0-9A-Za-z-]{1,32}$")
#: Sales per run: a backlog drains over a few runs, not one long ssh.
BATCH = 20
#: The game is free until December (operator, 2026-10-19: "free till Dec").
FREE_UNTIL = datetime.datetime(2026, 12, 1, tzinfo=datetime.timezone.utc)
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
# The freshest endpoint, not the first that answers: a lagging one hands
# `cast send` a stale nonce (`nonce too low`, the notary on 2026-10-24).
RPC=""; BEST=-1
for r in $(echo "${ELO_RH_RPC_POOL:-}" | tr ',; ' '\n\n\n'); do
  [ "$(cast chain-id --rpc-url "$r" 2>/dev/null)" = "4663" ] || continue
  b=$(cast block-number --rpc-url "$r" 2>/dev/null) || continue
  if [ "$b" -gt "$BEST" ]; then BEST=$b; RPC=$r; fi
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


# The copies' morr side: mode, then wallets. Prints `ok|had|used|fail wallet
# tx-or-reason` per wallet. `used` is the free door spent for that wallet
# (a copy was given, then sold or sent on); `fail shut` is the door closed.
REMOTE_COPY = r'''
set -u
MODE=$1; shift
export PATH="$PATH:$HOME/.foundry/bin"
set -a; . /data/apps/secrets/keys.env; set +a
T=$(python3 -c 'import json; print(json.load(open("/data/apps/scry-forge/contracts/deployments.json"))["chains"]["4663"]["contracts"]["EloGameTicket:GATES"]["address"])' 2>/dev/null)
FROM=$(cast wallet address --private-key "$ELO_DEPLOY_KEY" 2>/dev/null)
RPC=""; BEST=-1
for r in $(echo "${ELO_RH_RPC_POOL:-}" | tr ',; ' '\n\n\n'); do
  [ "$(cast chain-id --rpc-url "$r" 2>/dev/null)" = "4663" ] || continue
  b=$(cast block-number --rpc-url "$r" 2>/dev/null) || continue
  if [ "$b" -gt "$BEST" ]; then BEST=$b; RPC=$r; fi
done
if [ -z "$T" ] || [ -z "$FROM" ] || [ -z "$RPC" ]; then
  for w in "$@"; do echo "fail $w no-chain"; done
  exit 0
fi
for wallet in "$@"; do
  had=$(cast call "$T" "entitled(address)(bool)" "$wallet" --rpc-url "$RPC" 2>/dev/null)
  if [ "$had" = "true" ]; then echo "had $wallet -"; continue; fi
  if [ "$had" != "false" ]; then echo "fail $wallet unread"; continue; fi
  sim=$(cast call --from "$FROM" "$T" "mintFree(address)" "$wallet" --rpc-url "$RPC" 2>&1)
  if [ $? -ne 0 ]; then
    case "$sim" in
      *"free mint shut or used"*)
        if [ "$(cast call "$T" "freePerWallet()(uint256)" --rpc-url "$RPC" 2>/dev/null)" = "0" ]; then
          echo "fail $wallet shut"
        else
          echo "used $wallet -"
        fi ;;
      *) echo "fail $wallet refused" ;;
    esac
    continue
  fi
  if [ "$MODE" != send ]; then echo "ok $wallet simulated"; continue; fi
  out=$(cast send "$T" "mintFree(address)" "$wallet" --rpc-url "$RPC" --private-key "$ELO_DEPLOY_KEY" --json 2>/dev/null)
  tx=$(printf '%s' "$out" | python3 -c 'import json,sys
try:
    r = json.load(sys.stdin)
    print(r["transactionHash"] if str(r.get("status")) in ("1", "0x1") else "")
except Exception:
    print("")' 2>/dev/null)
  if [ -n "$tx" ]; then echo "ok $wallet $tx"; else echo "fail $wallet send"; fi
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


def copies_todo(online: str, copies: str) -> list[str]:
    """People online (not agents) with no line in the copies log, in the
    online file's order. A file that does not parse is no one."""
    import json
    try:
        rows = json.loads(online or "{}").get("online") or []
    except (ValueError, AttributeError):
        return []
    done = {line.split()[0] for line in copies.splitlines() if line.split()}
    out = []
    for r in rows:
        w = r.get("wallet") if isinstance(r, dict) else None
        if isinstance(w, str) and WALLET.match(w) and not r.get("agent") \
                and w != "0x" + "0" * 40 and w not in done and w not in out:
            out.append(w)
    return out


def copy_results(stdout: str, asked: list[str]) -> tuple[list[str], list[str]]:
    """Lines for the copies log (`ok`, `had`, `used`), and the failures."""
    logged, failed = [], []
    for line in stdout.splitlines():
        f = line.split()
        if len(f) != 3 or f[1] not in asked:
            continue
        if f[0] in ("ok", "had", "used"):
            logged.append(f"{f[1]} {f[0]} {f[2]}")
        else:
            failed.append(f"{f[1]}: {f[2]}")
    return logged, failed


# Close the free door: the owner posts `setFreePerWallet(0)`. Prints
# `closed tx`, `already-shut -` or `fail reason`.
REMOTE_CLOSE = r'''
set -u
export PATH="$PATH:$HOME/.foundry/bin"
set -a; . /data/apps/secrets/keys.env; set +a
T=$(python3 -c 'import json; print(json.load(open("/data/apps/scry-forge/contracts/deployments.json"))["chains"]["4663"]["contracts"]["EloGameTicket:GATES"]["address"])' 2>/dev/null)
RPC=""; BEST=-1
for r in $(echo "${ELO_RH_RPC_POOL:-}" | tr ',; ' '\n\n\n'); do
  [ "$(cast chain-id --rpc-url "$r" 2>/dev/null)" = "4663" ] || continue
  b=$(cast block-number --rpc-url "$r" 2>/dev/null) || continue
  if [ "$b" -gt "$BEST" ]; then BEST=$b; RPC=$r; fi
done
[ -n "$T" ] && [ -n "$RPC" ] || { echo "fail no-chain"; exit 0; }
if [ "$(cast call "$T" "freePerWallet()(uint256)" --rpc-url "$RPC" 2>/dev/null)" = "0" ]; then
  echo "already-shut -"; exit 0
fi
out=$(cast send "$T" "setFreePerWallet(uint256)" 0 --rpc-url "$RPC" --private-key "$ELO_DEPLOY_KEY" --json 2>/dev/null)
tx=$(printf '%s' "$out" | python3 -c 'import json,sys
try:
    r = json.load(sys.stdin)
    print(r["transactionHash"] if str(r.get("status")) in ("1", "0x1") else "")
except Exception:
    print("")' 2>/dev/null)
if [ -n "$tx" ]; then echo "closed $tx"; else echo "fail send"; fi
'''


def free_now(now: datetime.datetime | None = None) -> bool:
    """Are copies still given free?"""
    return (now or datetime.datetime.now(datetime.timezone.utc)) < FREE_UNTIL


def close_free_door(host: str) -> int:
    p = subprocess.run(
        ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", host, "bash", "-s"],
        input=REMOTE_CLOSE, capture_output=True, text=True, timeout=300,
    )
    out = (p.stdout.strip().splitlines() or ["fail no-answer"])[-1]
    print(f"free door: {out}")
    return 0 if out.split()[0] in ("closed", "already-shut") else 1


def run_remote_copies(host: str, wallets: list[str], mode: str = "send") -> str:
    p = subprocess.run(
        ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", host, "bash", "-s", "--", mode, *wallets],
        input=REMOTE_COPY,
        capture_output=True,
        text=True,
        timeout=600,
    )
    return p.stdout


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
    ap.add_argument("--no-copies", action="store_true", help="send the sales only")
    ap.add_argument("--close-free-door", action="store_true",
                    help="post setFreePerWallet(0) once FREE_UNTIL has passed (the December timer)")
    ap.add_argument("--now", help=argparse.SUPPRESS)  # tests: an ISO time to run at
    a = ap.parse_args(argv)
    now = (datetime.datetime.fromisoformat(a.now) if a.now
           else datetime.datetime.now(datetime.timezone.utc))
    if a.close_free_door:
        if free_now(now):
            print(f"free door: stays open until {FREE_UNTIL.isoformat()}")
            return 0
        return close_free_door(a.host)

    mints_p = Path(f"{a.world}.mints")
    minted_p = Path(f"{a.world}.minted")
    online_p = Path(f"{a.world}.online.json")
    copies_p = Path(f"{a.world}.copies")
    lock = open(f"{a.world}.minted.lock", "w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        return 0  # the last run is still sending
    read = lambda p: p.read_text() if p.exists() else ""
    todo = pending(read(mints_p), read(minted_p))[:BATCH]
    give = not a.no_copies and free_now(now)
    people = copies_todo(read(online_p), read(copies_p))[:BATCH] if give else []
    if a.dry_run:
        for ref, wallet, cat in todo:
            print(f"would mint catalog {cat} to {wallet} (sale {ref})")
        for wallet in people:
            print(f"would give {wallet} a copy, unless it holds one")
        return 0
    if a.check:
        if todo:
            print(run_remote(a.host, todo, "check"), end="")
        if people:
            print(run_remote_copies(a.host, people, "check"), end="")
        return 0
    failures = 0
    if todo:
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
        failures += len(failed)
    if people:
        logged, failed = copy_results(run_remote_copies(a.host, people), people)
        if logged:
            with open(copies_p, "a") as f:
                f.write("".join(line + "\n" for line in logged))
        for line in logged:
            print(f"copy: {line}")
        for line in failed:
            print(f"copy: not given {line}, retried next run", file=sys.stderr)
        failures += len(failed)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
