#!/usr/bin/env python3
"""What a pile of bug reports says — grouped by fingerprint, worst first.

    ./ci/reports.py ~/Pictures/gates        read a directory, print the board
    ./ci/reports.py <dir> --json out.json   write it for something to render
    ./ci/reports.py --self-test             the gate: no network, no client

`crates/client/src/report.rs` writes `gates-report-<stamp>-<fp>.json` beside
every report a player files. This is the other end: it reads a directory of
them and answers the only question a pile of reports is asked — **what is
broken, how many people hit it, and who should look at it.**

## Why grouping is the whole job

A report's `fingerprint` is deliberately blind to prose, clock, position and
player, and keyed on kind + build (+ the panic location, for a crash). That is
not a summary — it is the claim that two reports sharing one are ONE PIECE OF
WORK. So the useful view of forty files is not forty rows; it is a handful of
rows with counts, and this script is the only place that arithmetic lives.

It also means the counts are the ranking. Nobody has to triage a report to
learn that eleven people hit it and one person hit something else.

## What it deliberately does not do

No network, no writing into the reports, no deleting, and **no reading of the
Markdown half**. The `.json` is the machine's copy and the `.md` is the
person's; a tool that parsed prose would be a tool that disagreed with the
document over what it says. Anything a consumer needs is already a key —
`fingerprint`, `issue_title` and `read_first` are written into the file by
`Report::json` precisely so nothing downstream re-derives them.

A file that is not a report, or is a report from a `format` this does not
know, is **counted and named** rather than skipped quietly. A tool that
silently ignores what it cannot read is a tool that reports "nothing is
broken" when it has understood nothing.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import sys
import tempfile

# The `format` this understands. Bumped when a key changes meaning; a file
# from a newer one is named as unreadable rather than guessed at.
FORMAT = 1

# Every key a report must carry to be groupable. Deliberately short: these are
# the ones this script reads, and demanding more would make it refuse reports
# it could perfectly well count.
REQUIRED = ("format", "kind", "fingerprint", "issue_title", "read_first", "reported")


class Bad(Exception):
    """A file that is not a report this can read, with the reason."""


def load(path: pathlib.Path) -> dict:
    """One report, or `Bad` naming what is wrong with it."""
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as e:
        raise Bad(f"not readable json: {e}") from e
    if not isinstance(doc, dict):
        raise Bad("not an object")
    missing = [k for k in REQUIRED if k not in doc]
    if missing:
        raise Bad(f"missing {', '.join(missing)}")
    if doc["format"] != FORMAT:
        raise Bad(f"format {doc['format']}, this tool reads {FORMAT}")
    return doc


def group(reports: list[dict]) -> list[dict]:
    """Fold reports onto their fingerprints, most-reported first.

    The representative title is the **newest** report's, not the first: a
    later reporter has seen the bug after everyone else's, and the wording
    that survives should be the one written with the most knowledge of it.

    Ties break on `reported` descending, so a fresh single report outranks a
    stale single report — the thing somebody is hitting right now is the thing
    worth looking at, and a count alone cannot say that.
    """
    by_fp: dict[str, dict] = {}
    for r in reports:
        fp = r["fingerprint"]
        row = by_fp.get(fp)
        if row is None:
            by_fp[fp] = {
                "fingerprint": fp,
                "kind": r["kind"],
                "read_first": r["read_first"],
                "count": 1,
                "first_reported": r["reported"],
                "last_reported": r["reported"],
                "title": r["issue_title"],
                "builds": sorted({build_of(r)}),
            }
            continue
        row["count"] += 1
        row["first_reported"] = min(row["first_reported"], r["reported"])
        if r["reported"] >= row["last_reported"]:
            row["last_reported"] = r["reported"]
            row["title"] = r["issue_title"]
        row["builds"] = sorted(set(row["builds"]) | {build_of(r)})
    # Two stable passes rather than one key: `last_reported` is an ISO string
    # and descending order cannot be expressed by negating it, so recency is
    # sorted first and the count is layered over it. Python's sort is stable,
    # which is what makes that equivalent to one compound descending key.
    by_recency = sorted(by_fp.values(), key=lambda g: g["last_reported"], reverse=True)
    return sorted(by_recency, key=lambda g: -g["count"])


def build_of(r: dict) -> str:
    """`0.5.0 (bed9e02d6)`, or `unknown` for a report with no build block.

    A fingerprint already pins one build, so every report in a group shares
    this — but the field is read defensively anyway, because this script's job
    is to survive a file it did not write.
    """
    b = r.get("build")
    if not isinstance(b, dict):
        return "unknown"
    return f"{b.get('version', '?')} ({b.get('commit', '?')})"


def read_dir(d: pathlib.Path) -> tuple[list[dict], list[tuple[str, str]]]:
    """Every `gates-report-*.json` under `d`, and everything that was not one."""
    good: list[dict] = []
    bad: list[tuple[str, str]] = []
    for p in sorted(d.glob("gates-report-*.json")):
        try:
            good.append(load(p))
        except Bad as e:
            bad.append((p.name, str(e)))
    return good, bad


def board(groups: list[dict], bad: list[tuple[str, str]], total: int) -> str:
    """The board, as Markdown — what a page listing open reports would render."""
    out: list[str] = []
    out.append(f"# Gates · what is reported\n")
    out.append(
        f"{total} report{'' if total == 1 else 's'} · "
        f"{len(groups)} distinct{' · ' + str(len(bad)) + ' unreadable' if bad else ''}\n"
    )
    if not groups:
        out.append("Nothing readable in that directory.\n")
    else:
        out.append("| reports | what | since | read first | close with | title |")
        out.append("|---:|---|---|---|---|---|")
        for g in groups:
            out.append(
                f"| {g['count']} | `{g['kind']}` | {g['first_reported'][:10]} "
                f"| `{g['read_first']}` | `{g['fingerprint']}` | {g['title']} |"
            )
        out.append("")
        out.append(
            "Each row is one piece of work: reports sharing a fingerprint are the same "
            "bug seen by different people. **Fixing any of them pays** — any accepted "
            "pull request is 100,000 SCRY, flat and standing (`AGENTS.md` §the deal). "
            "There is nothing to claim and nobody is ahead of you.\n"
        )
        out.append(
            "**Close with** is the line to put in the pull request — `Closes reports: "
            "<fingerprint>`. It is not bookkeeping: it is what says which reporters "
            "the merge owes, and a fix that names none pays only its author.\n"
        )
    if bad:
        out.append("## Not readable as a report\n")
        for name, why in bad:
            out.append(f"- `{name}` — {why}")
        out.append("")
    return "\n".join(out)


def self_test() -> int:
    """The gate. Builds reports in a temp directory and asserts the folding.

    Every case here is one this tool is wrong without: that duplicates collapse
    and singles do not, that the count outranks the clock and the clock breaks a
    tie, that the newest wording wins, and — the one that matters most — that a
    file it cannot read is counted rather than dropped.
    """
    checks = 0

    def check(cond: bool, msg: str) -> None:
        nonlocal checks
        checks += 1
        if not cond:
            print(f"GATE FAIL: {msg}", file=sys.stderr)
            raise SystemExit(1)

    def rep(fp: str, kind: str, when: str, title: str, doc: str = "TERRAIN.md") -> dict:
        return {
            "format": FORMAT,
            "kind": kind,
            "fingerprint": fp,
            "issue_title": title,
            "read_first": doc,
            "reported": when,
            "build": {"version": "0.5.0", "commit": "bed9e02d6"},
        }

    with tempfile.TemporaryDirectory() as td:
        d = pathlib.Path(td)
        files = [
            rep("aaa", "world", "2026-08-20T10:00:00Z", "[world] rock in the wall"),
            rep("aaa", "world", "2026-08-20T11:00:00Z", "[world] boulder inside the haven"),
            rep("aaa", "world", "2026-08-20T09:00:00Z", "[world] stuck in a rock"),
            rep("bbb", "net", "2026-08-20T12:00:00Z", "[net] rubber banding", "NETCODE.md"),
            rep("ccc", "look", "2026-08-19T12:00:00Z", "[look] sky is inside out", "ART.md"),
        ]
        for i, r in enumerate(files):
            (d / f"gates-report-2026082{i}-000000-{r['fingerprint']}.json").write_text(
                json.dumps(r), encoding="utf-8"
            )
        # Three files that must be NAMED rather than dropped.
        (d / "gates-report-broken.json").write_text("{not json", encoding="utf-8")
        (d / "gates-report-future.json").write_text(
            json.dumps({**rep("ddd", "look", "2026-08-20T00:00:00Z", "t"), "format": 99}),
            encoding="utf-8",
        )
        (d / "gates-report-partial.json").write_text(
            json.dumps({"format": FORMAT, "kind": "look"}), encoding="utf-8"
        )
        # And one that is not a report at all, which must be invisible rather
        # than an error: a player's screenshot directory holds other files.
        (d / "settings.json").write_text("{}", encoding="utf-8")

        good, bad = read_dir(d)
        check(len(good) == 5, f"5 readable reports, got {len(good)}")
        check(len(bad) == 3, f"3 unreadable named, got {len(bad)}: {bad}")
        check(
            all(n.startswith("gates-report-") for n, _ in bad),
            "a file that is not a report at all must not be listed as a broken one",
        )
        check(
            any("format 99" in why for _, why in bad),
            "an unknown format must say so rather than be guessed at",
        )

        g = group(good)
        check(len(g) == 3, f"3 distinct bugs, got {len(g)}")
        check(g[0]["fingerprint"] == "aaa" and g[0]["count"] == 3, f"the 3 must lead: {g}")
        check(
            g[0]["title"] == "[world] boulder inside the haven",
            f"the NEWEST wording represents the group, got {g[0]['title']!r}",
        )
        check(
            g[0]["first_reported"] == "2026-08-20T09:00:00Z",
            "since is the earliest sighting, not the first file read",
        )
        # Two singles: the fresher one outranks the staler one.
        check(
            [x["fingerprint"] for x in g[1:]] == ["bbb", "ccc"],
            f"a tie on count breaks on recency: {[x['fingerprint'] for x in g[1:]]}",
        )
        check(g[0]["read_first"] == "TERRAIN.md", "the doc travels with the group")
        check(g[0]["builds"] == ["0.5.0 (bed9e02d6)"], f"builds: {g[0]['builds']}")

        text = board(g, bad, len(good))
        check("| 3 |" in text, "the count is the ranking and must be drawn")
        check(
            "`aaa`" in text and "close with" in text,
            "the fingerprint is what a PR names, so the board has to show it",
        )
        check("100,000 SCRY" in text, "the board says what fixing one pays")
        check("Not readable as a report" in text, "unreadable files are named on the board")

        # An empty directory is a normal answer, not a crash.
        with tempfile.TemporaryDirectory() as empty:
            eg, eb = read_dir(pathlib.Path(empty))
            check((eg, eb) == ([], []), "an empty directory reads as empty")
            check("Nothing readable" in board(group(eg), eb, 0), "and says so")

    print(f"reports: {checks} checks passed")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("dir", nargs="?", help="a directory holding gates-report-*.json")
    ap.add_argument("--json", metavar="FILE", help="write the grouped board as JSON")
    ap.add_argument("--self-test", action="store_true", help="the gate; no network")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.dir:
        ap.error("give a directory, or --self-test")
    d = pathlib.Path(a.dir).expanduser()
    if not d.is_dir():
        print(f"reports: {d} is not a directory", file=sys.stderr)
        return 1
    good, bad = read_dir(d)
    groups = group(good)
    print(board(groups, bad, len(good)))
    if a.json:
        pathlib.Path(a.json).write_text(
            json.dumps({"format": FORMAT, "total": len(good), "groups": groups,
                        "unreadable": [{"file": n, "why": w} for n, w in bad]}, indent=1),
            encoding="utf-8",
        )
    # Nonzero when nothing could be read but files were there — a pile of
    # unreadable reports is a finding, not a quiet success.
    return 1 if bad and not good else 0


if __name__ == "__main__":
    raise SystemExit(main())
