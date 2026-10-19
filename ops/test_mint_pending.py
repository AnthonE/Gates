#!/usr/bin/env python3
"""Checks for the minter (`ops/mint_pending.py`): what is pending, and how the
morr side's answer is read. No ssh, no chain.

    python3 ops/test_mint_pending.py
"""
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mint_pending as mp  # noqa: E402

A = "0x" + "a1" * 20
B = "0x" + "b2" * 20
FAILS = []


def check(name, ok):
    print(("ok   " if ok else "FAIL ") + name)
    if not ok:
        FAILS.append(name)


mints = f"# header\n100 {A} 10\n101 {B} 10\nbad line\n102 {A.upper()} 10\n100 {A} 10\n"
check("a sale is read once, in order, and a bad line skipped",
      mp.pending(mints, "") == [("100", A, 10), ("101", B, 10)])
check("a sent sale is not pending",
      mp.pending(mints, f"100 {A} 10 0xabc\n") == [("101", B, 10)])
check("a rebuy is a new sale", mp.pending(f"200 {A} 10\n", f"100 {A} 10 0xabc\n") == [("200", A, 10)])

asked = [("100", A, 10), ("101", B, 10)]
out = (f"ok 100 {A} 10 0xdead\nfail 101 {B} 10 refused\n"
       f"ok 999 {A} 10 0xbeef\ngarbage\n")
sent, failed = mp.results(out, asked)
check("an ok is logged with its tx", sent == [f"100 {A} 10 0xdead"])
check("a refusal is reported, not logged", failed == [f"101 {B} 10: refused"])
check("an answer for a sale not asked is ignored", all("999" not in s for s in sent))
sent, _ = mp.results(f"used 101 {B} 10 -\n", asked)
check("a sale the chain already holds is logged as sent", sent == [f"101 {B} 10 -"])

with tempfile.TemporaryDirectory() as d:
    world = Path(d) / "island.world"
    calls = []
    mp.run_remote = lambda host, batch: (calls.append(batch) or
                                         "".join(f"ok {r} {w} {c} 0x{i:x}\n" for i, (r, w, c) in enumerate(batch)))
    check("no queue, no ssh", mp.main(["--world", str(world)]) == 0 and not calls)
    Path(f"{world}.mints").write_text(f"100 {A} 10\n")
    check("a sale is sent", mp.main(["--world", str(world)]) == 0 and len(calls) == 1)
    check("and logged", Path(f"{world}.minted").read_text() == f"100 {A} 10 0x0\n")
    check("and not sent again", mp.main(["--world", str(world)]) == 0 and len(calls) == 1)

# ── copies ───────────────────────────────────────────────────────────────
import json  # noqa: E402
G = "0x" + "0" * 40
online = json.dumps({"at": 1, "online": [
    {"wallet": A, "agent": False}, {"wallet": B, "agent": True},
    {"wallet": G, "agent": False}, {"wallet": A, "agent": False}, {"name": "no wallet"}]})
check("people, not agents or guests, each once", mp.copies_todo(online, "") == [A])
check("a wallet looked at is not again", mp.copies_todo(online, f"{A} had -\n") == [])
check("an unreadable online file is no one", mp.copies_todo("{not json", "") == [])
logged, failed = mp.copy_results(f"ok {A} 0xfeed\nhad {B} -\nfail {G} shut\nok 0x{'c3'*20} 0x1\n", [A, B, G])
check("given and already-held are logged", logged == [f"{A} ok 0xfeed", f"{B} had -"])
check("a shut door is reported and retried", failed == [f"{G}: shut"])

with tempfile.TemporaryDirectory() as d:
    world = Path(d) / "island.world"
    asked = []
    mp.run_remote_copies = lambda host, ws, mode="send": (asked.append(list(ws)) or
                                                          "".join(f"ok {w} 0xc0" + "\n" for w in ws))
    Path(f"{world}.online.json").write_text(online)
    NOV = ["--now", "2026-11-01T00:00:00+00:00"]  # inside the free window, whatever today is
    check("a person online is given a copy", mp.main(["--world", str(world), *NOV]) == 0 and asked == [[A]])
    check("and it is logged", Path(f"{world}.copies").read_text() == f"{A} ok 0xc0\n")
    check("and never asked again", mp.main(["--world", str(world), *NOV]) == 0 and len(asked) == 1)
    check("--no-copies leaves them", mp.main(["--world", str(world), "--no-copies", *NOV]) == 0 and len(asked) == 1)

# ── free till December ─────────────────────────────────────────────────────
with tempfile.TemporaryDirectory() as d:
    world = Path(d) / "island.world"
    asked.clear()
    Path(f"{world}.online.json").write_text(online)
    check("no copy is given from December on",
          mp.main(["--world", str(world), "--now", "2026-12-01T00:00:00+00:00"]) == 0 and not asked)
    check("one is given on the last day of November",
          mp.main(["--world", str(world), "--now", "2026-11-30T23:59:00+00:00"]) == 0 and asked == [[A]])
    closed = []
    mp.close_free_door = lambda host: closed.append(host) or 0
    check("the door stays open in November",
          mp.main(["--world", str(world), "--close-free-door", "--now", "2026-11-30T23:59:00+00:00"]) == 0
          and not closed)
    check("and closes in December",
          mp.main(["--world", str(world), "--close-free-door", "--now", "2026-12-01T00:00:05+00:00"]) == 0
          and closed == ["morr"])

print("\nall minter checks pass" if not FAILS else f"\n{len(FAILS)} FAILED")
sys.exit(1 if FAILS else 0)
