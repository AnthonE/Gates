#!/usr/bin/env python3
"""Rip the reference game's system map out of Oxide's patcher project.

Why this exists: you cannot read a shipped game's feature list off its
binary, but a mod loader has to name every method it intercepts, and
Oxide's patcher project is that list as data — hook name, the game class
it patches, the method signature, and Oxide's own category for it. That
is a system inventory, and it is machine-readable.

Source: OxideMod/Oxide.Rust, `resources/Rust.opj`, MIT (see README.md).
Only *facts* are extracted — names, classes, signatures. No code from the
project is copied, and nothing from it ships in the built game.

Usage (network required; see README.md for the offline path):

    git clone --depth 1 https://github.com/OxideMod/Oxide.Rust.git
    ./reference/rip-hooks.py Oxide.Rust/resources/Rust.opj \
        > reference/rust-systems.txt
"""

import collections
import json
import sys


def load(path):
    """Flatten every manifest's hook list into plain records."""
    doc = json.load(open(path))
    out = []
    for manifest in doc["Manifests"]:
        for entry in manifest.get("Hooks", []):
            hook = entry["Hook"]
            sig = hook.get("Signature") or {}
            params = [
                p if isinstance(p, str) else (p.get("TypeName") or p.get("Name"))
                for p in sig.get("Parameters", [])
            ]
            out.append(
                {
                    "name": hook.get("HookName") or hook.get("Name"),
                    "cls": hook.get("TypeName"),
                    "cat": hook.get("HookCategory"),
                    "meth": sig.get("Name"),
                    "ret": sig.get("ReturnType"),
                    "params": params,
                    "asm": hook.get("AssemblyName"),
                }
            )
    return out


def short(type_name):
    """`System.Collections.Generic.List`1<Item>` -> `List`1<Item>`."""
    return type_name.split(".")[-1] if type_name else ""


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    recs = load(sys.argv[1])

    by_cat = collections.defaultdict(lambda: collections.defaultdict(list))
    for r in recs:
        by_cat[r["cat"]][r["cls"]].append(r)

    classes = {r["cls"] for r in recs}
    print("# Rust system map, ripped from Oxide's patcher project")
    print("#")
    print("# Each line is: hook name, then the game method it patches.")
    print("# Grouped by Oxide's own HookCategory, then by patched class.")
    print("# A class with many hooks is a system with many verbs.")
    print("#")
    print(f"# {len(recs)} patch entries · {len(classes)} distinct game classes "
          f"· {len(by_cat)} categories")
    print("# Regenerate with reference/rip-hooks.py — see reference/README.md.")

    order = sorted(by_cat, key=lambda c: -sum(len(v) for v in by_cat[c].values()))
    for cat in order:
        total = sum(len(v) for v in by_cat[cat].values())
        print(f"\n{'=' * 70}")
        print(f"## {cat}  ({total} hooks, {len(by_cat[cat])} classes)")
        print("=" * 70)
        for cls in sorted(by_cat[cat], key=lambda c: -len(by_cat[cat][c])):
            hooks = by_cat[cat][cls]
            print(f"\n  {cls}  [{len(hooks)}]")
            for h in sorted(hooks, key=lambda x: x["name"] or ""):
                params = ",".join(short(p) for p in (h["params"] or []))
                print(f"    {h['name']:<42} {h['meth']}({params})")


if __name__ == "__main__":
    main()
