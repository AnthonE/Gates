#!/usr/bin/env python3
"""Checks the house characters (`ops/personas/*.toml`, read by
`gates-agent --character`): each parses, fits the mind request's caps
(persona <= 600 characters, traits 0-100), and each unit's instance has one.

    python3 ops/test_personas.py
"""
import glob
import os
import re
import sys
import tomllib

HERE = os.path.dirname(os.path.abspath(__file__))
KINDS = {"protagonist", "teammate", "villager", "buddy", "solo", "community", "other"}
TRAITS = {"ambition", "risk", "greed", "sociability", "honesty", "patience", "grit"}
FAILED = []


def check(name, ok):
    print(("ok   " if ok else "FAIL ") + name)
    if not ok:
        FAILED.append(name)


def character_ok(c, whole=True):
    if whole and not (c.get("name") and c.get("goal") and c.get("persona")):
        return "needs name, goal and persona"
    if len(c.get("persona", "")) > 600:
        return "persona over 600 characters"
    t = c.get("traits")
    if whole and set(t or {}) != TRAITS:
        return f"traits must be exactly {sorted(TRAITS)}"
    if any(not isinstance(v, int) or not 0 <= v <= 100 for v in (t or {}).values()):
        return "a trait outside 0-100"
    if any(not isinstance(q, str) or len(q) > 80 for q in c.get("quirks", [])):
        return "a quirk over 80 characters"
    return None


files = sorted(glob.glob(os.path.join(HERE, "personas", "*.toml")))
check("there are characters", len(files) >= 5)
for p in files:
    name = os.path.basename(p)
    with open(p, "rb") as f:
        doc = tomllib.load(f)
    check(f"{name}: a kind we know", doc.get("kind") in KINDS)
    if "culture" in doc:
        check(f"{name}: its culture is a whole character", character_ok(doc["culture"]) is None)
        for who, over in (doc.get("members") or {}).items():
            check(f"{name}: {who}'s override fits", re.match(r"^[\x20-\x7e]{1,16}$", who) and
                  character_ok(over, whole=False) is None)
    else:
        check(f"{name}: {character_ok(doc) or 'a whole character'}", character_ok(doc) is None)
# Each unit's instance has its character (`--character .../personas/%i.toml`).
for need in ("ryuzen", "millers", "thatchers", "flint", "doctor-wood"):
    check(f"{need}.toml is there", os.path.isfile(os.path.join(HERE, "personas", need + ".toml")))
print()
if FAILED:
    print(f"{len(FAILED)} FAILED")
    sys.exit(1)
print("all persona checks pass")
