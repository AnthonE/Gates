#!/usr/bin/env python3
"""Does elopros.com still answer every call Gates makes?

    ./ci/elo_routes.py                    check the live origin (nightly)
    ./ci/elo_routes.py --origin URL       check another origin
    ./ci/elo_routes.py --self-test        the gate: fixtures only, no network

The shard, the client, the browser shell and the release scripts call the
platform, and nothing on either side noticed when an answer drifted: the item
store named catalog 1 `Obsidian Rock` over `Rock` while `content/skins.toml`
ships an `Obsidian Bat`, and the browser store drew a `rock.png` that 404s.

Three checks, each a list of lines a reader can act on:

1. every route in `ROUTES` is in the origin's `/api/catalog`, which the meter
   generates from its running app, so a route that moved or was dropped shows
   up there;
2. every read route answers 200 with JSON carrying the keys Gates reads, under
   the byte cap Gates' own reader enforces (one byte over and the game refuses
   the body, it never truncates it);
3. the item store's `skins[]` is `content/skins.toml` by `catalog_id`: the
   name, the covered item's display name from `content/items.toml` (the
   browser shell snake-cases it into `assets/icons/<name>.png`, which must
   exist here) and the tint.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
import time
import tomllib
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ORIGIN = "https://elopros.com"
# A wallet with nothing on the platform (private key 1's address), lowercase
# because `entitle::is_wallet` sends nothing else.
PROBE = "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
TIMEOUT = 30
UNCAPPED = 4 * 1024 * 1024  # for a reader with no cap of its own
UA = "gates-ci/elo_routes (+https://github.com/AnthonE/Gates)"


@dataclass(frozen=True)
class Route:
    method: str
    call: str  # as Gates sends it, `{wallet}` filled with PROBE
    template: str  # as /api/catalog spells it (bare: nginx adds /api)
    reader: str
    # What the reader acts on. `a.b` nests, `a[].b` is every row of a
    # non-empty list, `a|b` is either, a trailing `?` lets the value be null.
    # Empty = listed in the catalog only (a POST, a picture, a depot file).
    keys: tuple[str, ...] = ()
    cap: tuple[str, str] | None = None  # (source file, const) of the reader's byte cap


ROUTES = (
    Route("GET", "/api/launcher/servers/gates", "/launcher/servers/{slug}",
          "client menu, browser shell", ("servers[].addr",),
          ("crates/client/src/shardlist.rs", "MAX_DOC_BYTES")),
    Route("GET", "/api/launcher/manifest/gates", "/launcher/manifest/{slug}",
          "client hub, ci/release.sh", ("builds[].platform", "store.url", "news.url"),
          ("crates/client/src/manifest.rs", "MAX_MANIFEST_BYTES")),
    Route("GET", "/api/items/store/gates", "/items/store/{game}",
          "shard prices, browser store", ("configured", "skins[].catalog_id", "skins[].on_sale"),
          ("crates/server/src/skins.rs", "MAX_PRICES_BYTES")),
    Route("GET", "/api/items/of/{wallet}?limit=200", "/items/of/{wallet}",
          "shard skins, browser store", ("configured|items",),
          ("crates/server/src/skins.rs", "MAX_RESPONSE_BYTES")),
    Route("GET", "/api/face/{wallet}", "/face/{wallet}",
          "shard nametags, client menu", ("name?", "pic?"),
          ("crates/server/src/faces.rs", "MAX_RESPONSE_BYTES")),
    Route("GET", "/api/face/{wallet}/pic.png", "/face/{wallet}/pic.png", "client nametags"),
    Route("GET", "/api/ticket/gates/of/{wallet}", "/ticket/{game}/of/{wallet}",
          "shard join", ("entitled?",),
          ("crates/server/src/entitle.rs", "MAX_RESPONSE_BYTES")),
    Route("POST", "/api/ticket/gates/check", "/ticket/{game}/check", "shard roster sweep"),
    Route("GET", "/api/store/repo/gates", "/store/repo/{slug}", "ci/elo_manifest.py", ("valid",)),
    Route("POST", "/api/store/repo/pull", "/store/repo/pull", "ci/release.sh"),
    Route("GET", "/api/store/catalog", "/store/catalog", "browser news", ("titles[].slug",)),
    Route("GET", "/api/who?handle={wallet}", "/who", "browser account chip", ("found?",)),
    Route("GET", "/api/holdings/{wallet}", "/holdings/{wallet}", "browser account chip",
          ("coins[].sym",)),
    Route("GET", "/api/launcher/depot/gates/<build>/<platform>/files/<path>",
          "/launcher/depot/{slug}/{build}/{platform}/files/{path:path}", "ci/depot.py"),
)
STORE_CALL = "/api/items/store/gates"


def norm(path: str) -> str:
    """`/ticket/{game}/of/{wallet}` → `/ticket/{}/of/{}`: a renamed
    placeholder is the same route to its callers."""
    return re.sub(r"\{[^}]*\}", "{}", path)


def rust_cap(rel: str, name: str) -> int:
    """The reader's cap, read from its source so the two cannot disagree."""
    text = (ROOT / rel).read_text()
    m = re.search(rf"pub const {name}: usize = ([0-9_]+(?:\s*\*\s*[0-9_]+)*);", text)
    if not m:
        raise SystemExit(f"elo_routes: {rel} has no `pub const {name}: usize`; the reader moved, fix ROUTES")
    n = 1
    for f in m.group(1).split("*"):
        n *= int(f.strip().replace("_", ""))
    return n


def _lacks_one(doc, key: str) -> str | None:
    nullable = key.endswith("?")
    parts = key.rstrip("?").split(".")
    nodes = [(doc, "")]
    for i, part in enumerate(parts):
        many = part.endswith("[]")
        name = part[:-2] if many else part
        last = i == len(parts) - 1
        nxt = []
        for node, at in nodes:
            here = f"{at}.{name}" if at else name
            if not isinstance(node, dict):
                return f"{at or 'the body'} is not an object"
            if name not in node:
                return f"no {here}"
            v = node[name]
            if many:
                if not isinstance(v, list):
                    return f"{here} is not a list"
                if not v:
                    return f"{here} is empty"
                nxt += [(e, f"{here}[{j}]") for j, e in enumerate(v)]
            elif v is None and not (last and nullable):
                return f"{here} is null"
            else:
                nxt.append((v, here))
        nodes = nxt
    return None


def lacks(doc, key: str) -> str | None:
    """None when `doc` carries `key`, else why not."""
    whys = [_lacks_one(doc, k.strip()) for k in key.split("|")]
    return None if None in whys else " and ".join(whys)


def snake(name: str) -> str:
    """The browser shell's icon name (`client-web/web/app.js` skinCard)."""
    return re.sub(r"[^a-z0-9]+", "_", name.lower()).strip("_")


def load_content(skins_text: str, items_text: str) -> tuple[dict[int, dict], list[str]]:
    """content/skins.toml's rows by catalog id, as the store should name them."""
    names = {i["id"]: i.get("name") for i in tomllib.loads(items_text).get("item", [])}
    want, bad = {}, []
    for s in tomllib.loads(skins_text).get("skin", []):
        cid, covers = s.get("catalog"), names.get(s.get("covers"))
        if type(cid) is not int:
            bad.append(f"content: {s.get('id')} has no integer catalog")
            continue
        if covers is None:
            bad.append(f"content: {s.get('id')} covers {s.get('covers')!r}, which content/items.toml does not name")
        if cid in want:
            bad.append(f"content: catalog {cid} is in content/skins.toml twice")
        tint = s.get("tint") or [0, 0, 0]
        want[cid] = {"name": s.get("name"), "covers": covers, "tint": "#%02x%02x%02x" % tuple(tint),
                     "texture": s.get("texture")}
    return want, bad


def parity(store, want: dict[int, dict], icons: set[str]) -> list[str]:
    """Every way the store's skins[] differs from content/skins.toml."""
    skins = store.get("skins") if isinstance(store, dict) else None
    if not isinstance(skins, list):
        return ["parity: the item store has no skins[]"]
    have, bad = {}, []
    for s in skins:
        cid = s.get("catalog_id") if isinstance(s, dict) else None
        if type(cid) is not int:
            bad.append(f"parity: a store skin has no integer catalog_id: {json.dumps(s)[:80]}")
            continue
        if cid in have:
            bad.append(f"parity: catalog {cid} is in the store twice")
        have[cid] = s
    for cid in sorted(want.keys() - have.keys()):
        bad.append(f"parity: catalog {cid} ({want[cid]['name']}) is in content/skins.toml, not in the store")
    for cid in sorted(have.keys() - want.keys()):
        bad.append(f"parity: catalog {cid} ({have[cid].get('name')}) is in the store, not in content/skins.toml")
    for cid in sorted(want.keys() & have.keys()):
        for field in ("name", "covers", "tint"):
            w, h = want[cid][field], have[cid].get(field)
            if field == "tint" and isinstance(h, str):
                h = h.lower()
            if h != w:
                bad.append(f"parity: catalog {cid} {field}: the store says {h!r}, content says {w!r}")
        # A painted skin: the store serves the same picture the game names.
        tex = want[cid].get("texture")
        url = have[cid].get("texture_url")
        if tex and not (isinstance(url, str) and url.endswith(f"/{tex[:16]}.png")):
            bad.append(f"parity: catalog {cid} is painted ({tex[:16]}…), the store's texture_url is {url!r}")
    for cid, w in sorted(want.items()):
        if w["covers"] and f"{snake(w['covers'])}.png" not in icons:
            bad.append(f"parity: catalog {cid}: the browser store draws assets/icons/{snake(w['covers'])}.png, "
                       "which Gates does not ship")
    return bad


def http_get(url: str, cap: int) -> tuple[int, bytes]:
    """(status, at most cap+1 bytes); status 0 is "could not reach"."""
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "application/json"})
    got = (0, b"")
    for attempt in range(2):
        try:
            with urllib.request.urlopen(req, timeout=TIMEOUT) as r:
                return r.status, r.read(cap + 1)
        except urllib.error.HTTPError as e:
            got = (e.code, b"")
        except (urllib.error.URLError, OSError) as e:
            got = (0, str(getattr(e, "reason", e)).encode())
        if got[0] and got[0] < 500:
            return got
        if attempt == 0:
            time.sleep(3)
    return got


def check(origin: str, fetch, want: dict[int, dict], icons: set[str], say=print) -> list[str]:
    """Every failure against `origin`; empty means green. `fetch(url, cap)`
    is `http_get` live and a dict lookup in the self-test."""
    bad: list[str] = []

    def note(ok: bool, line: str) -> None:
        say(("  ok  " if ok else "  !!  ") + line)
        if not ok:
            bad.append(line)

    def get_json(call: str, cap: int):
        status, body = fetch(origin + call, cap)
        if status != 200:
            return None, f"answered {status or 'nothing'}{': ' + body.decode(errors='replace')[:80] if body else ''}"
        if len(body) > cap:
            return None, f"is over the {cap}-byte cap its reader enforces, so the game refuses it"
        try:
            doc = json.loads(body)
        except ValueError:
            return None, "is not JSON"
        return (doc, None) if isinstance(doc, dict) else (None, "is not a JSON object")

    cat, why = get_json("/api/catalog", UNCAPPED)
    rows = cat.get("endpoints") if cat else None
    if not isinstance(rows, list):
        note(False, f"GET /api/catalog {why or 'has no endpoints[]'}")
        listed = None
    else:
        listed = {(e.get("method"), norm(str(e.get("path")))) for e in rows if isinstance(e, dict)}

    store = None
    for r in ROUTES:
        name = f"{r.method} {r.call.format(wallet='<wallet>')}"
        if listed is not None and (r.method, norm(r.template)) not in listed:
            note(False, f"{name}: {r.method} {r.template} is not in /api/catalog ({r.reader})")
            continue
        if not r.keys:
            if listed is not None:
                note(True, f"{name} (listed)")
            continue
        cap = rust_cap(*r.cap) if r.cap else UNCAPPED
        doc, why = get_json(r.call.format(wallet=PROBE), cap)
        if doc is None:
            note(False, f"{name} {why} ({r.reader})")
            continue
        whys = [w for w in (lacks(doc, k) for k in r.keys) if w]
        note(not whys, f"{name}" + (f": {'; '.join(whys)} ({r.reader})" if whys else ""))
        if r.call == STORE_CALL:
            store = doc

    if store is None:
        note(False, "parity: not checked, the item store read failed above")
    else:
        lines = parity(store, want, icons)
        for line in lines:
            note(False, line)
        if not lines:
            note(True, f"parity: the store's {len(want)} skins are content/skins.toml's")
    return bad


def repo_content() -> tuple[dict[int, dict], list[str], set[str]]:
    want, bad = load_content((ROOT / "content/skins.toml").read_text(),
                             (ROOT / "content/items.toml").read_text())
    icons = {p.name for p in (ROOT / "assets/icons").glob("*.png")}
    return want, bad, icons


# ── the self-test ────────────────────────────────────────────────────────────

SKINS_FIXTURE = """
[[skin]]
id = "skin.bat_obsidian"
catalog = 1
name = "Obsidian Bat"
covers = "item.bat"
tint = [70, 62, 84]

[[skin]]
id = "skin.bow_bone"
catalog = 4
name = "Bone Bow"
covers = "item.bow"
tint = [236, 226, 204]
"""
ITEMS_FIXTURE = """
[[item]]
id = "item.bat"
name = "Bat"

[[item]]
id = "item.bow"
name = "Hunting Bow"

[[item]]
id = "item.rock"
name = "Rock"
"""
ICONS_FIXTURE = {"bat.png", "hunting_bow.png", "rock.png"}

# The live store as it drifted (2026-10-11): catalog 1 named for a rock.
STORE_ROCK = {"game": "gates", "configured": False, "skins": [
    {"catalog_id": 1, "name": "Obsidian Rock", "covers": "Rock", "tint": "#463e54", "on_sale": False},
    {"catalog_id": 4, "name": "Bone Bow", "covers": "Hunting Bow", "tint": "#ECE2CC", "on_sale": False},
]}


def _good() -> dict[str, dict]:
    store = json.loads(json.dumps(STORE_ROCK))
    store["skins"][0].update(name="Obsidian Bat", covers="Bat")
    return {
        "/api/launcher/servers/gates": {"servers": [
            {"id": "us-east-1", "addr": "game.example:61234",
             "status_url": "https://game.example/gates/status.json"}]},
        "/api/launcher/manifest/gates": {
            "builds": [{"kind": "native", "platform": "linux-x86_64"},
                       {"kind": "browser", "platform": "any", "url": "https://elopros.com/games/gates/play/"}],
            "store": {"url": "https://elopros.com/game.html?slug=gates#items", "label": "Skins"},
            "news": {"url": "https://elopros.com/community.html?slug=gates", "label": "News"}},
        "/api/items/store/gates": store,
        "/api/items/of/{wallet}?limit=200": {"configured": False, "items": []},
        "/api/face/{wallet}": {"wallet": PROBE, "name": None, "picture": None, "pic": None},
        "/api/ticket/gates/of/{wallet}": {"game": "gates", "ticketed": False, "entitled": True},
        "/api/store/repo/gates": {"game": "gates", "valid": True},
        "/api/store/catalog": {"titles": [{"slug": "gates", "record": []}]},
        "/api/who?handle={wallet}": {"handle": PROBE, "found": None},
        "/api/holdings/{wallet}": {"coins": [{"sym": "ELO", "whole": "0"}]},
    }


def _world(bodies: dict, endpoints: list | None = None, raw: dict | None = None):
    """A fake origin: `bodies` keyed by Route.call, `raw` by URL for a body
    that is not a dict (or a status that is not 200)."""
    o = "https://origin.test"
    if endpoints is None:
        endpoints = [{"method": r.method, "path": r.template} for r in ROUTES]
    pages = {o + "/api/catalog": (200, json.dumps({"endpoints": endpoints}).encode())}
    for call, doc in bodies.items():
        pages[o + call.format(wallet=PROBE)] = (200, json.dumps(doc).encode())
    pages.update(raw or {})
    return o, lambda url, cap: pages.get(url, (404, b""))


def self_test() -> int:
    fails = 0

    def ok(cond: bool, label: str) -> None:
        nonlocal fails
        print(("  ok  " if cond else "  FAIL ") + label)
        fails += not cond

    want, cbad = load_content(SKINS_FIXTURE, ITEMS_FIXTURE)
    ok(not cbad and want[1] == {"name": "Obsidian Bat", "covers": "Bat", "tint": "#463e54"},
       "content: a skin's covers is its item's display name, its tint a hex string")

    def run(bodies, endpoints=None, raw=None, icons=ICONS_FIXTURE):
        o, fetch = _world(bodies, endpoints, raw)
        return check(o, fetch, want, icons, say=lambda _: None)

    def fails_with(bad: list[str], *needles: str) -> bool:
        return bool(bad) and all(any(n in b for b in bad) for n in needles)

    good = _good()
    probed = {r.call for r in ROUTES if r.keys}
    ok(probed == set(good), f"every read route has a good fixture ({sorted(probed ^ set(good))})")
    ok(run(good) == [], "the good world passes")

    rock = dict(good, **{STORE_CALL: STORE_ROCK})
    ok(fails_with(run(rock), "catalog 1 name: the store says 'Obsidian Rock'",
                  "catalog 1 covers: the store says 'Rock', content says 'Bat'"),
       "a store naming catalog 1 'Obsidian Rock' over 'Rock' fails parity")
    ok(len(run(rock)) == 2, "and only on that row: the tint compares case-blind")
    fixed = json.loads(json.dumps(STORE_ROCK))
    fixed["skins"][0].update(name="Obsidian Bat", covers="Bat")
    ok(run(dict(good, **{STORE_CALL: fixed})) == [], "the corrected store passes")

    extra = json.loads(json.dumps(fixed))
    extra["skins"].append({"catalog_id": 7, "name": "Odd", "covers": "Rock", "tint": "#000000", "on_sale": False})
    ok(fails_with(run(dict(good, **{STORE_CALL: extra})), "catalog 7 (Odd) is in the store, not in"),
       "a store skin content does not ship fails")
    short = json.loads(json.dumps(fixed))
    short["skins"].pop()
    ok(fails_with(run(dict(good, **{STORE_CALL: short})), "catalog 4 (Bone Bow) is in content/skins.toml, not in"),
       "a content skin missing from the store fails")
    ok(fails_with(run(good, icons={"bat.png"}), "assets/icons/hunting_bow.png"),
       "a covers with no icon in assets/icons fails")
    _, cbad = load_content(SKINS_FIXTURE.replace('"item.bow"', '"item.nope"'), ITEMS_FIXTURE)
    ok(any("item.nope" in b for b in cbad), "a skin covering an item items.toml does not name is a content fault")

    mf = json.loads(json.dumps(good["/api/launcher/manifest/gates"]))
    del mf["news"]
    ok(fails_with(run(dict(good, **{"/api/launcher/manifest/gates": mf})), "no news"),
       "a manifest without news.url fails")
    mf = json.loads(json.dumps(good["/api/launcher/manifest/gates"]))
    mf["store"]["url"] = None
    ok(fails_with(run(dict(good, **{"/api/launcher/manifest/gates": mf})), "store.url is null"),
       "a null where the reader needs a value fails")
    ok(run(dict(good, **{"/api/items/of/{wallet}?limit=200": {"configured": False}})) == [],
       "either side of `configured|items` passes")
    ok(fails_with(run(dict(good, **{"/api/items/of/{wallet}?limit=200": {"wallet": PROBE}})),
                  "no configured and no items"), "neither side fails")
    ok(fails_with(run(dict(good, **{"/api/launcher/servers/gates": {"servers": []}})), "servers is empty"),
       "an empty server list fails")
    ok(run(dict(good, **{"/api/launcher/servers/gates": {"servers": [{"addr": "a:1"}]}})) == [],
       "a server row without status_url passes: Gates draws it without a count")
    ok(run(dict(good, **{"/api/launcher/servers/gates": {"servers": [{"addr": "a:1", "status_url": None}]}})) == [],
       "and so does a null status_url")
    ok(fails_with(run(dict(good, **{"/api/launcher/servers/gates": {"servers": [{"status_url": "https://a/s"}]}})),
                  "no servers[0].addr"), "a server row without addr fails")
    ok(fails_with(run(dict(good, **{"/api/launcher/servers/gates": {"servers": [{"addr": None}]}})),
                  "servers[0].addr is null"), "a null addr fails")

    endpoints = [{"method": r.method, "path": r.template} for r in ROUTES if r.method == "GET"]
    ok(fails_with(run(good, endpoints), "POST /ticket/{game}/check is not in /api/catalog"),
       "a POST route the catalog dropped fails")
    endpoints = [{"method": r.method, "path": r.template.replace("{game}", "{slug}")} for r in ROUTES]
    ok(run(good, endpoints) == [], "a renamed placeholder is the same route")
    ok(norm("/a/{slug}/b/{path:path}") == "/a/{}/b/{}", "norm folds every placeholder")

    o = "https://origin.test"
    cap = rust_cap("crates/client/src/manifest.rs", "MAX_MANIFEST_BYTES")
    big = json.loads(json.dumps(good["/api/launcher/manifest/gates"]))
    big["_note"] = "x" * cap
    ok(fails_with(run(dict(good, **{"/api/launcher/manifest/gates": big})), f"over the {cap}-byte cap"),
       "a manifest the game would refuse for size fails")
    ok(fails_with(run(good, raw={o + "/api/store/repo/gates": (404, b"")}), "answered 404"),
       "a 404 fails")
    ok(fails_with(run(good, raw={o + "/api/store/catalog": (200, b"[]")}), "not a JSON object"),
       "a body that is not an object fails")
    ok(fails_with(run(good, raw={o + "/api/items/store/gates": (502, b"")}), "parity: not checked"),
       "a store that did not answer fails parity rather than skipping it")

    caps = [rust_cap(*r.cap) for r in ROUTES if r.cap]
    ok(all(c > 0 for c in caps), f"every reader's cap reads from its source ({caps})")
    real, rbad, icons = repo_content()
    store = {"skins": [dict(catalog_id=c, **w) for c, w in real.items()]}
    ok(bool(real) and not rbad and parity(store, real, icons) == [],
       f"the repo's {len(real)} skins load, name their items, and ship their icons")

    print(f"elo_routes self-test: {'OK' if not fails else f'{fails} FAILED'}")
    return 1 if fails else 0


def main() -> int:
    ap = argparse.ArgumentParser(description="the platform still answers what Gates calls")
    ap.add_argument("--self-test", action="store_true", help="fixtures only, no network")
    ap.add_argument("--origin", default=ORIGIN, help=f"default {ORIGIN}")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    want, bad, icons = repo_content()
    for line in bad:
        print("  !!  " + line)
    bad += check(args.origin.rstrip("/"), http_get, want, icons)
    print(f"elo_routes: {'OK' if not bad else f'{len(bad)} problem(s)'} against {args.origin}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
