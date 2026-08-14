#!/usr/bin/env python3
"""publish_depot.py — package one platform's build and put it live on scry's origin.

    ./ci/publish_depot.py --platform linux-x86_64            build, upload, publish
    ./ci/publish_depot.py --platform win-x86_64 --no-build   package what is compiled
    ./ci/publish_depot.py --platform win-x86_64 --dry-run    do everything but write
    ./ci/publish_depot.py --platform linux-x86_64 --notarize ...and seal the digest

This is the hand-work of 2026-08-14 written down. Each step below was a command
somebody typed, in an order they had to remember, against an origin that fails
quietly when it is skipped. Four of them exist only because doing this by hand
went wrong at least once.

**It does not compute the depot digest and it does not package.** `ci/depot.py`
packages and `scry digest` computes the number that gets notarized — a second
implementation of either is the failure both of those files are shaped to
refuse. This script is the ORDER, the refusals, and the checks between.

## What it refuses, and why each refusal exists

1. **A dirty tree.** The build id is `<version>-g<sha>`, and for a dirty tree
   `depot.py` keys it on the binary's own hash instead. That form is correct
   but it is not reproducible from a commit, and a build a player is running
   must name one. Publishing is not the moment to be clever about this.

2. **A build id whose LEGACY directory is already taken.** The trap this used
   to guard is gone at the root: the origin keys by (build, platform) since
   2026-08-14, so `<build>/<platform>/` is this platform's own directory and
   two platforms from one commit are two depots. Publishing both from one
   commit is now the normal case, which is the point.

   What survives is the seam. A depot published before that change sits at
   `<build>/depot.json` with no platform segment and **stays there forever** —
   its url is baked into its own document, the digest covers the document, and
   that digest is sealed in `ScryNotary`. Writing a keyed directory beside it
   would leave two documents claiming one build, and the origin prefers the
   keyed one, so the notarized depot would quietly stop being served. That is
   a decision, not a default, so it refuses.

   **Do not "fix" any of this with a platform suffix on the id** — scry's
   `meter/gamerepo.py::version_of` strips `-g[0-9a-f]+…$` anchored at `$`, so
   `0.2.0-gsha-win-x86_64` stops parsing back to `0.2.0` and the store's
   declared-vs-published check goes permanently red.

3. **An upload that does not re-hash at the far end.** rsync reports what it
   sent, which is not the same claim as *the origin now holds these bytes*. We
   re-read every file where it landed and check it against the depot document
   before anything points at it.

4. **Flipping `published.json` before any of that.** Publishing is a pointer,
   not a directory listing (`meter/launcher.py` §3): a build goes live when
   that file names it. So it is written LAST, it touches only this platform's
   key, and the previous value is kept beside it.

## The signing key never comes here

`--notarize` runs `cast send` **on the origin box over ssh**, because that is
where `PRIVATE_KEY` lives and it is not going to travel. Notarization is
additive: nothing in the install path consults the chain, so a published build
is testable before it is sealed and this step can be run later or never.
"""
from __future__ import annotations

import argparse
import json
import shlex
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import depot as depot_mod  # the packager — one implementation, imported not copied

ROOT = depot_mod.ROOT
SLUG = depot_mod.SLUG
TARGETS = depot_mod.TARGETS

DEFAULT_HOST = "morr"
DEFAULT_DEPOTS = "/data/apps/scry-data/depots"
DEFAULT_API = "https://scry.moreright.xyz/api"
NOTARY = "0x0C15fA7829458118e3d26229F58FE0443f8b792c"  # ScryNotary, chain 4663
NOTARY_CHAIN = 4663


def run(cmd: list[str], **kw) -> subprocess.CompletedProcess:
    p = subprocess.run(cmd, text=True, capture_output=True, **kw)
    if p.returncode != 0:
        sys.exit(f"publish: `{' '.join(cmd)}` failed:\n{p.stdout}\n{p.stderr}".rstrip())
    return p


def ssh(host: str, script: str) -> str:
    """One remote command. `script` runs under the remote shell as written."""
    return run(["ssh", "-o", "BatchMode=yes", host, script]).stdout.strip()


# ── the refusals ─────────────────────────────────────────────────────────────

def require_clean_tree() -> str:
    if run(["git", "status", "--porcelain"], cwd=ROOT).stdout.strip():
        sys.exit(
            "publish: the tree has uncommitted changes. A published build id must\n"
            "         name a commit a player could check out. Commit or stash first."
        )
    return run(["git", "rev-parse", "--short=9", "HEAD"], cwd=ROOT).stdout.strip()


def refuse_legacy_collision(host: str, depots: str, build: str, platform: str) -> None:
    """Would this upload land on top of a depot published under the OLD layout?

    scry keys by (build, platform) since 2026-08-14, so `<build>/<platform>/` is
    this platform's own directory and two platforms from one commit no longer
    collide — that is the whole point and it is why this function no longer
    refuses the ordinary case.

    What it still guards is the seam. A depot published BEFORE the change sits
    at `<build>/depot.json` with no platform segment, and it stays there
    forever: its url is baked into its own document, the digest covers the
    document, and that digest is sealed in ScryNotary, so moving it would
    orphan the number on chain. Writing `<build>/<platform>/` beside a legacy
    `<build>/depot.json` is therefore safe for the bytes but ambiguous for a
    reader — two documents claiming one build. The origin prefers the keyed one
    and would quietly stop serving what was notarized, so this refuses and asks
    for a decision instead.
    """
    remote = f"{depots}/{SLUG}/{build}/depot.json"
    got = ssh(host, f"cat {shlex.quote(remote)} 2>/dev/null || true")
    if not got:
        return
    try:
        there = json.loads(got).get("platform")
    except json.JSONDecodeError:
        sys.exit(f"publish: {remote} exists and is not readable JSON — look before writing")
    sys.exit(
        f"publish: REFUSING — {SLUG}/{build}/ holds a LEGACY {there} depot,\n"
        f"         published before the origin keyed by (build, platform). It cannot\n"
        f"         move: its url is inside its own digest and that digest is on chain.\n"
        f"         Publishing {platform} under this same build id would leave two\n"
        f"         documents claiming one build, and the origin prefers the keyed one\n"
        f"         — so the notarized depot would quietly stop being served.\n"
        f"         Package from a different commit, or retire that build first."
    )


# ── the steps ────────────────────────────────────────────────────────────────

def package(platform: str, do_build: bool) -> Path:
    """Delegate to `ci/depot.py`, then check it produced what we expect."""
    cmd = [sys.executable, str(ROOT / "ci" / "depot.py"), "--platform", platform]
    if not do_build:
        cmd.append("--no-build")
    print(f"== package · {' '.join(cmd[1:])}")
    p = subprocess.run(cmd, cwd=ROOT, text=True)
    if p.returncode != 0:
        sys.exit("publish: packaging failed — nothing uploaded")

    out = ROOT / "target" / "depot"
    if platform != depot_mod.PLATFORM:
        out = out / platform
    build = depot_mod.build_id()
    doc = out / build / "depot.json"
    if not doc.is_file():
        sys.exit(f"publish: expected {doc} and it is not there")
    d = json.loads(doc.read_text())
    if d.get("build") != build or d.get("platform") != platform:
        sys.exit(
            f"publish: {doc} says build={d.get('build')} platform={d.get('platform')},\n"
            f"         expected build={build} platform={platform}. Refusing to upload."
        )
    return out / build


def upload(stage: Path, host: str, depots: str, build: str, platform: str,
           dry: bool) -> None:
    dest = f"{host}:{depots}/{SLUG}/{build}/{platform}/"
    print(f"== upload · {stage} → {dest}")
    if dry:
        print("   (dry-run: nothing sent)")
        return
    run(["rsync", "-a", "--info=stats1", f"{stage}/", dest])


def verify_at_origin(host: str, depots: str, build: str, platform: str) -> int:
    """Re-hash every file where it landed. rsync's report is not this claim."""
    here = f"{depots}/{SLUG}/{build}/{platform}"
    print("== verify · re-hashing every file at the origin")
    checker = (
        "import json,hashlib,pathlib,sys;"
        "d=json.load(open('depot.json'));bad=[]\n"
        "for f in d['files']:\n"
        "    p=pathlib.Path(f['path'])\n"
        "    if not p.is_file(): bad.append((f['path'],'missing')); continue\n"
        "    if hashlib.sha256(p.read_bytes()).hexdigest()!=f['sha256']: bad.append((f['path'],'sha256'))\n"
        "    elif p.stat().st_size!=f['bytes']: bad.append((f['path'],'bytes'))\n"
        "    elif f['executable'] and not (p.stat().st_mode & 0o111): bad.append((f['path'],'mode'))\n"
        "print(json.dumps({'n':len(d['files']),'bad':bad}))\n"
    )
    out = ssh(host, f"cd {shlex.quote(here)} && python3 -c {shlex.quote(checker)}")
    r = json.loads(out.splitlines()[-1])
    if r["bad"]:
        sys.exit(f"publish: the origin does not hold what we packaged: {r['bad'][:10]}")
    print(f"   {r['n']} file(s) verified byte-for-byte")
    return r["n"]


def publish_pointer(host: str, depots: str, build: str, platform: str, dry: bool) -> None:
    """Last, and only this platform's key. A build is live when this names it."""
    p = f"{depots}/{SLUG}/published.json"
    print(f"== publish · {platform} → {build}")
    if dry:
        print("   (dry-run: pointer untouched)")
        return
    writer = (
        "import json,pathlib,shutil\n"
        f"p=pathlib.Path({p!r})\n"
        "d=json.loads(p.read_text()) if p.exists() else {}\n"
        "before=dict(d)\n"
        "if p.exists(): shutil.copy2(p,str(p)+'.bak')\n"
        f"d[{platform!r}]={build!r}\n"
        "p.write_text(json.dumps(d))\n"
        "print(json.dumps({'before':before,'after':d}))\n"
    )
    out = ssh(host, f"python3 -c {shlex.quote(writer)}")
    r = json.loads(out.splitlines()[-1])
    print(f"   before {r['before']}")
    print(f"   after  {r['after']}")


def confirm_live(api: str, build: str, platform: str) -> str | None:
    """Ask the public surface, not the disk. Reports the digest it serves."""
    print("== confirm · reading the live manifest")
    p = run(["curl", "-s", "--max-time", "30", f"{api}/launcher/manifest/{SLUG}"])
    try:
        rows = json.loads(p.stdout).get("builds", [])
    except json.JSONDecodeError:
        sys.exit(f"publish: {api}/launcher/manifest/{SLUG} did not answer JSON")
    for b in rows:
        if b.get("platform") == platform:
            state, ver = b.get("depot_state"), b.get("version")
            mark = "ok" if ver == build and state == "published" else "MISMATCH"
            print(f"   {mark}: {platform} → {ver} ({state}, {b.get('files')} files, "
                  f"{b.get('bytes')} bytes)")
            if mark != "ok":
                sys.exit(f"publish: the origin serves {ver}/{state}, expected {build}/published")
            return b.get("depot_digest")
    sys.exit(f"publish: the live manifest has no {platform} row")


def _cast_send(digest: str, label: str, memo: str) -> str:
    """The remote one-liner. Everything secret stays inside this shell.

    ⚠ **`SCRY_RH_RPC_POOL` is a COMMA-SEPARATED LIST, not a url**, and passing
    it whole to `--rpc-url` fails as `HTTP 401 UNAUTHORIZED` — which reads like
    a bad key and is really a malformed url, because the first endpoint's
    credential arrives with the rest of the list glued to it. Measured
    2026-08-14 while writing this. The pool exists so that a dead provider is
    not a dead chain (`meter/rpc_pool.py` §2), so we try each in turn and use
    the first that answers rather than trusting position 0.

    The keyed urls are never printed: the credential is in the url PATH, and
    this origin publishes its RPC url from ten surfaces — which is the whole
    reason `SCRY_RH_RPC` (public, keyless) and `SCRY_RH_RPC_POOL` (keyed,
    private) are two different knobs.
    """
    return (
        "set -a; . /data/apps/secrets/keys.env; set +a; "
        'RPC=""; '
        'for u in $(printf %s "$SCRY_RH_RPC_POOL" | tr "," " "); do '
        '  if ~/.foundry/bin/cast chain-id --rpc-url "$u" >/dev/null 2>&1; then RPC="$u"; break; fi; '
        "done; "
        'if [ -z "$RPC" ]; then echo "no reachable endpoint in SCRY_RH_RPC_POOL" >&2; exit 1; fi; '
        f"~/.foundry/bin/cast send {NOTARY} "
        f"'notarize(bytes32,string,string)' {digest} {shlex.quote(label)} {shlex.quote(memo)} "
        '--rpc-url "$RPC" --private-key "$PRIVATE_KEY" --json'
    )


def notarize(host: str, depots: str, build: str, digest: str, label: str,
             memo: str, dry: bool) -> None:
    """Seal the digest on chain 4663, with the key that never leaves the origin.

    Additive by design: nothing in the install path consults the chain, so this
    can run long after the build is live, or not at all.
    """
    print(f"== notarize · {digest} as {label!r}")
    if len(label) > 128 or not label:
        sys.exit("publish: ScryNotary wants a label of 1..128 chars")
    if len(memo) > 512:
        sys.exit("publish: ScryNotary wants a memo of <= 512 chars")
    if dry:
        print("   (dry-run: no transaction sent)")
        return
    out = ssh(host, _cast_send(digest, label, memo))
    try:
        r = json.loads(out.splitlines()[-1])
        print(f"   tx {r.get('transactionHash')}  block {int(r.get('blockNumber','0x0'),16)}"
              f"  status {r.get('status')}")
    except (json.JSONDecodeError, ValueError):
        print("   " + out.replace("\n", "\n   "))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--platform", default=depot_mod.PLATFORM, choices=sorted(TARGETS))
    ap.add_argument("--no-build", action="store_true", help="package what is compiled")
    ap.add_argument("--dry-run", action="store_true", help="every check, no writes")
    ap.add_argument("--notarize", action="store_true", help="seal the digest on chain")
    ap.add_argument("--scry", default=None, help="path to a `scry` binary for --notarize")
    ap.add_argument("--host", default=DEFAULT_HOST, help="origin box (ssh alias)")
    ap.add_argument("--depots", default=DEFAULT_DEPOTS, help="depot root on the origin")
    ap.add_argument("--api", default=DEFAULT_API, help="public API base to confirm against")
    a = ap.parse_args()

    sha = require_clean_tree()
    build = depot_mod.build_id()
    print(f"== {SLUG} {a.platform} · commit {sha} · build {build}"
          + ("  [DRY RUN]" if a.dry_run else ""))

    refuse_legacy_collision(a.host, a.depots, build, a.platform)
    stage = package(a.platform, do_build=not a.no_build)
    upload(stage, a.host, a.depots, build, a.platform, a.dry_run)
    if not a.dry_run:
        verify_at_origin(a.host, a.depots, build, a.platform)
    publish_pointer(a.host, a.depots, build, a.platform, a.dry_run)
    digest = None if a.dry_run else confirm_live(a.api, build, a.platform)

    if a.notarize:
        local = None
        if a.scry:
            local = run([a.scry, "digest", str(stage / "depot.json")]).stdout.strip()
            if digest and local != digest:
                sys.exit(
                    f"publish: `scry digest` says {local} and the origin serves {digest}.\n"
                    "         Notarizing either would seal a number nobody can reproduce."
                )
        seal = local or digest
        if not seal:
            sys.exit("publish: no digest to notarize (--dry-run, or pass --scry)")
        notarize(a.host, a.depots, build, seal, f"{SLUG}-{a.platform}",
                 f"{SLUG} {build} {a.platform} depot", a.dry_run)

    print("\nDONE" + (" (dry run — nothing changed)" if a.dry_run else ""))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
