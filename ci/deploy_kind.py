#!/usr/bin/env python3
"""What a change takes to ship: the four deploy kinds, read off a git range.

    python3 ci/deploy_kind.py                     # HEAD against origin/main
    python3 ci/deploy_kind.py --base origin/next-wipe
    python3 ci/deploy_kind.py --forbid-wipe       # exit 1 if it is a wipe

  CLIENT  the game only (client, page, sound, assets): publish the page and
          the depots; nobody is dropped.
  SHARD   server, sim, protocol or content rows' numbers: ci/release.sh --go,
          a restart (`--warn` gives the island the /restart countdown).
  WIRE    PROTO_VER moved: the same release, and every player has to reload
          or update, because old clients are refused. Not a wipe.
  WIPE    the live saves can no longer be read: worldgen moved
          (GOLDEN_TERRAIN_HASH), the content layout moved (GOLDEN_LAYOUT_HASH:
          a row added, removed, renamed or reordered), a save format moved, or
          the public seed did. It waits for the wipe: it goes to next-wipe.

The range is the merge base of --base and HEAD to HEAD, so a PR is judged on
its own changes, not on what main gained since it branched. The wipe signals
are the pinned constants, which CI already makes every such PR move (the two
goldens fail until their hash is pasted), so reading their values is exact.
"""

import argparse
import os
import re
import subprocess
import sys

# (kind, why, file, regex for the value). Compared as integers, so a golden
# re-spelt from decimal to hex does not read as moved.
PINS = [
    ("WIPE", "worldgen moved", "crates/sim-core/tests/terrain_golden.rs",
     r"const GOLDEN_TERRAIN_HASH: u64 = ([0-9A-Fa-fx_]+);"),
    ("WIPE", "the content layout moved (a row added, removed, renamed or reordered)",
     "crates/content/tests/layout_golden.rs",
     r"const GOLDEN_LAYOUT_HASH: u64 = ([0-9A-Fa-fx_]+);"),
    ("WIPE", "the player store's format moved", "crates/server/src/store.rs",
     r"pub const SAVE_FORMAT: u16 = ([0-9_]+);"),
    ("WIPE", "the player store's slot count moved", "crates/server/src/store.rs",
     r"pub const MAX_SAVED_PLAYERS: usize = ([0-9_]+);"),
    ("WIPE", "the world file's format moved", "crates/server/src/worldfile.rs",
     r"pub const WORLD_FILE_FORMAT: u16 = ([0-9_]+);"),
    ("WIPE", "the world save's format moved", "crates/sim-core/src/worldsave.rs",
     r"pub const WORLD_SAVE_FORMAT: u16 = ([0-9_]+);"),
    ("WIPE", "the public seed moved", "shard-public.toml", r"(?m)^seed\s*=\s*([0-9_]+)"),
    ("WIRE", "PROTO_VER moved", "crates/protocol/src/lib.rs",
     r"pub const PROTO_VER: u16 = ([0-9_]+);"),
]

ORDER = ["NONE", "CLIENT", "SHARD", "WIRE", "WIPE"]

# The game alone: nothing the shard links or reads. `client-core` and `voice`
# are not here because the server crate depends on both.
CLIENT = ("crates/client/", "crates/client-web/", "crates/sound/", "crates/sound-worklet/",
          "assets/")
# Nothing a player runs: docs, tooling, ops, and tests (a golden moving is a
# PIN above, read by value, not by its file).
NO_DEPLOY = ("posts/", "reference/", "findings/", "ci/", "ops/", ".github/", ".claude/",
             "docs/", ".gitignore")
TESTS = re.compile(r"^crates/[^/]+/(tests|benches|examples)/")

NEXT = {
    "NONE": "nothing to deploy",
    "CLIENT": "publish the page and the depots; nobody is dropped",
    "SHARD": "ci/release.sh --go (a restart; --warn counts the island down first)",
    "WIRE": "ci/release.sh --go; every player reloads or updates (not a wipe)",
    "WIPE": "hold it for the wipe: send it to next-wipe, not main",
}


def git(*args, check=True):
    out = subprocess.run(["git", *args], capture_output=True, text=True)
    if check and out.returncode != 0:
        sys.exit(f"deploy_kind: git {' '.join(args)}: {out.stderr.strip()[:300]}")
    return out


def value(ref, path, pattern):
    out = git("show", f"{ref}:{path}", check=False)
    if out.returncode != 0:
        return None
    m = re.search(pattern, out.stdout)
    return int(m.group(1).replace("_", ""), 0) if m else None


def classify(base_ref, head):
    base = git("merge-base", base_ref, head).stdout.strip()
    files = [f for f in git("diff", "--name-only", base, head).stdout.splitlines() if f]
    reasons = []
    kind = "NONE"
    for k, why, path, pattern in PINS:
        was, now = value(base, path, pattern), value(head, path, pattern)
        if was is not None and now is not None and was != now:
            reasons.append((k, f"{why}: {path} {was:#x} -> {now:#x}" if "HASH" in pattern
                            else f"{why}: {path} {was} -> {now}"))
            if ORDER.index(k) > ORDER.index(kind):
                kind = k
    areas = {"client": [], "shard": [], "none": []}
    for f in files:
        if f.endswith(".md") or f.startswith(NO_DEPLOY) or TESTS.match(f):
            areas["none"].append(f)
        elif f.startswith(CLIENT):
            areas["client"].append(f)
        else:
            areas["shard"].append(f)
    floor = "SHARD" if areas["shard"] else "CLIENT" if areas["client"] else "NONE"
    if ORDER.index(floor) > ORDER.index(kind):
        kind = floor
    return base, kind, reasons, areas


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--base", default="origin/main")
    ap.add_argument("--head", default="HEAD", help="what to judge (default HEAD)")
    ap.add_argument("--forbid-wipe", action="store_true",
                    help="exit 1 when the change is a wipe (CI, for PRs into main)")
    a = ap.parse_args()
    base, kind, reasons, areas = classify(a.base, a.head)

    lines = [f"deploy kind: {kind} — {NEXT[kind]}",
             f"  range: {base[:9]}..{a.head} (merge base with {a.base})"]
    for k, why in reasons:
        lines.append(f"  {k}: {why}")
    for name in ("shard", "client"):
        fs = areas[name]
        if fs:
            shown = ", ".join(fs[:6]) + (f" and {len(fs) - 6} more" if len(fs) > 6 else "")
            lines.append(f"  {name}: {shown}")
    print("\n".join(lines))

    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(f"### Deploy kind: {kind}\n\n{NEXT[kind]}\n\n")
            for k, why in reasons:
                f.write(f"- **{k}**: {why}\n")
            f.write("\n")
    out = os.environ.get("GITHUB_OUTPUT")
    if out:
        with open(out, "a") as f:
            f.write(f"kind={kind}\n")

    if a.forbid_wipe and kind == "WIPE":
        print("\n  !! a wipe cannot merge to main mid-wipe: main must stay deployable.\n"
              "     Retarget this PR to next-wipe, which merges into main on wipe day.",
              file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
