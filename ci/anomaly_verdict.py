#!/usr/bin/env python3
"""Turn a shard's anomaly log into the alpha gate's verdict.

    ./ci/anomaly_verdict.py gates-anomaly.jsonl     summary, then PASS or FAIL
    ./ci/anomaly_verdict.py --self-test             the gate: no files, no cargo

`ALPHA.md` §6: "zero silent failures in the anomaly log". The log
(`crates/server/src/anomaly.rs`) makes every watched counter loud — a line
with the tick it moved and by how much — so what is left to judge is which of
those lines are the shard failing at its own job. That is the class below
called FAULT, and one moved FAULT counter is a FAIL. A PRESSURE counter is a
cap or a budget doing what it was built to do; a REFUSAL is the door saying
no to somebody else's mistake or mischief. Both are reported, neither fails.

A counter this file has not classified fails the verdict too: the log's own
rule is that "is this a failure?" has an answer only a person has, so a new
`WATCHED` row has to be answered here. `--self-test` reads `WATCHED` out of
anomaly.rs and refuses a row no class names.
"""

import argparse
import json
import re
import sys
from pathlib import Path

TICK_HZ = 30
ANOMALY_RS = Path(__file__).resolve().parent.parent / "crates/server/src/anomaly.rs"

# The shard failed at its own job: a tick, a ring, an encode or a disk.
FAULT = {
    "ticks_dropped",
    "input_ring_drops",
    "snap_ring_skips",
    "snap_send_errors",
    "encode_range_errors",
    "ev_sim_dropped",
    "ev_send_errors",
    "chat_ring_drops",
    "save_ring_drops",
    "save_write_errors",
    "world_save_errors",
    "world_load_errors",
    "skins_dropped",
    "trust_ring_drops",
    "trust_sim_overflow",
}
# A cap or a budget working as designed: worth a look, not a failure.
PRESSURE = {
    "snap_entities_shed",
    "forced_resyncs",
    "ev_resyncs",
    "saves_evicted",
    "sleepers_evicted",
    "chat_undelivered",
    "handshake_errors",
}
# The door said no: an old client, a full shard, a forged frame, an admin.
REFUSAL = {
    "refused_version",
    "refused_build",
    "refused_auth",
    "refused_ticket",
    "refused_full",
    "refused_banned",
    "entitle_unknown",
    "entitle_kicked",
    "skins_unknown",
    "skin_prices_unknown",
    "input_dg_bad",
    "input_dg_forged",
    "actions_bad",
    "chat_bad",
    "chat_rate_limited",
    "admin_kicked",
    "admin_refused",
    "aim_stale_refused",
    "favour_disagree",
    "spectate_input_refused",
    "spectate_actions_refused",
}


def clock(tick: int) -> str:
    s = tick // TICK_HZ
    return f"{s // 3600}:{s // 60 % 60:02}:{s % 60:02}"


def class_of(name: str) -> str:
    if name in FAULT:
        return "fault"
    if name in PRESSURE:
        return "pressure"
    if name in REFUSAL:
        return "refusal"
    return "unclassified"


def read(lines):
    """Fold the log into per-counter totals and the subject records."""
    counters = {}  # name -> [total, lines, first tick, last tick]
    bugs, admin, refused, bad = [], 0, 0, 0
    for n, line in enumerate(lines, 1):
        line = line.strip()
        if not line:
            continue
        try:
            rec = json.loads(line)
            tick, kind = int(rec["tick"]), rec["kind"]
        except (ValueError, KeyError, TypeError):
            bad += 1
            continue
        if kind == "counter":
            c = counters.setdefault(rec.get("counter", "unknown"), [0, 0, tick, tick])
            c[0] += int(rec.get("delta", 0))
            c[1] += 1
            c[2], c[3] = min(c[2], tick), max(c[3], tick)
        elif kind == "bug":
            bugs.append((tick, rec.get("who", 0), rec.get("note", "")))
        elif kind == "admin":
            admin += 1
        elif kind == "admin_refused":
            refused += 1
        else:
            bad += 1
    return counters, bugs, admin, refused, bad


def verdict(counters, bad) -> tuple:
    """(passed, the counters that failed it)."""
    failing = sorted(n for n in counters if class_of(n) in ("fault", "unclassified"))
    return not failing and bad == 0, failing


def report(path: str) -> int:
    with open(path, encoding="utf-8") as f:
        counters, bugs, admin, refused, bad = read(f)
    for cls in ("fault", "unclassified", "pressure", "refusal"):
        rows = sorted((n, c) for n, c in counters.items() if class_of(n) == cls)
        if not rows:
            continue
        print(f"{cls}:")
        for name, (total, lines, first, last) in rows:
            print(f"  {name:28} {total:>8}  in {lines} line(s), {clock(first)} .. {clock(last)}")
    print(f"bugs filed: {len(bugs)}")
    for tick, who, note in bugs:
        print(f"  {clock(tick)}  player {who}: {note}")
    print(f"admin acts: {admin}, refused: {refused}")
    if bad:
        print(f"unreadable lines: {bad}")
    passed, failing = verdict(counters, bad)
    if passed:
        print("verdict: PASS — no fault in the log")
        return 0
    why = ", ".join(failing) or f"{bad} unreadable line(s)"
    print(f"verdict: FAIL — {why}")
    return 1


def self_test() -> int:
    checks = 0
    src = ANOMALY_RS.read_text(encoding="utf-8")
    block = src[src.index("pub const WATCHED: &[&str] = &[") :]
    watched = re.findall(r'^\s*"([a-z_0-9]+)",', block[: block.index("];")], re.M)
    assert len(watched) >= 40, f"only {len(watched)} WATCHED rows scraped"
    checks += 1
    for name in watched:
        assert class_of(name) != "unclassified", f"WATCHED row `{name}` has no class here"
        checks += 1
    for a, b in ((FAULT, PRESSURE), (FAULT, REFUSAL), (PRESSURE, REFUSAL)):
        assert not a & b, f"a counter in two classes: {a & b}"
        checks += 1
    stale = (FAULT | PRESSURE | REFUSAL) - set(watched)
    assert not stale, f"classified but not watched: {sorted(stale)}"
    checks += 1

    log = [
        '{"tick":90,"kind":"counter","counter":"refused_full","delta":2}',
        '{"tick":120,"kind":"bug","who":256,"a":1,"b":2,"c":3,"note":"stuck \\"here\\""}',
        '{"tick":150,"kind":"admin","what":0,"who":256,"a":300,"b":0,"c":0}',
        '{"tick":300,"kind":"counter","counter":"ev_resyncs","delta":5}',
        '{"tick":330,"kind":"counter","counter":"ev_resyncs","delta":1}',
    ]
    counters, bugs, admin, refused, bad = read(log)
    assert counters["ev_resyncs"] == [6, 2, 300, 330], counters
    assert bugs == [(120, 256, 'stuck "here"')] and admin == 1 and refused == 0
    assert verdict(counters, bad) == (True, [])
    checks += 1
    counters, _, _, _, bad = read(log + ['{"tick":400,"kind":"counter","counter":"save_write_errors","delta":1}'])
    assert verdict(counters, bad) == (False, ["save_write_errors"])
    checks += 1
    counters, _, _, _, bad = read(log + ['{"tick":400,"kind":"counter","counter":"brand_new","delta":1}'])
    assert verdict(counters, bad) == (False, ["brand_new"]), "an unclassified counter must fail"
    checks += 1
    counters, _, _, _, bad = read(log + ["not json"])
    assert verdict(counters, bad)[0] is False, "an unreadable line must fail"
    checks += 1
    assert clock(30 * 3725) == "1:02:05"
    checks += 1
    print(f"anomaly_verdict self-test: {checks} checks green")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("log", nargs="?", help="the shard's anomaly_file (JSONL)")
    ap.add_argument("--self-test", action="store_true", help="the gate; no files")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.log:
        ap.error("name a log, or pass --self-test")
    return report(a.log)


if __name__ == "__main__":
    sys.exit(main())
