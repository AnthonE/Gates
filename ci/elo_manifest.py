#!/usr/bin/env python3
"""Gates' end of elo's repo desk — the store manifest and the signature over it.

    ./ci/elo_manifest.py --self-test    the gate: no network, no key, stdlib only
    ./ci/elo_manifest.py --check        the same, plus recover the signature
    ./ci/elo_manifest.py --print        the exact text to sign, for a signer we do not hold
    ./ci/elo_manifest.py --sign         write the signature beside the manifest (needs a key)

## What this seam is

elo reads two files from this repo's default branch and nothing else:

    elo.json         what this game is, right now — its store row and its feed
    elo.sig.json     a detached EIP-191 signature over elo.json's exact bytes

It applies what changed. Nothing here runs on elo's box and nothing of elo's
runs here: the whole transport is a commit, and the authority is the signature.
The standard is `GET https://elopros.com/api/library/GAME-REPO.md`.

## Two names, one pair

elo also reads the pair under the platform's former name (`LEGACY` below),
which is what this repo committed first. This script reads both the way elo
does: the first manifest in the tree is THE manifest, and its pair is taken
whole. elo never matches one name's manifest with the other's signature,
because a signature covers exact bytes — so an `elo.json` committed without
its `elo.sig.json` hides a signed legacy pair, and the store row stops moving.
The gate fails on exactly that.

Moving the row to the current names is one commit, by whoever holds the key:
rename the legacy manifest to `elo.json` and set `"elo": 1` in place of its old
key, run `--sign` (it writes `elo.sig.json` at the next seq), `git rm` the
legacy signature, and commit the pair together.

## What the GATE checks, and what it deliberately does not

These properties, all **local**, all fully decidable with the standard library:

  1. The manifest is a JSON object naming this game and standard 1.
  2. `version` equals the workspace version in `Cargo.toml`.
  3. If its signature exists, its `sha256` is the sha256 of the manifest's
     bytes — i.e. the signature was written for the manifest that is here.
  4. An unsigned `elo.json` never sits in front of a signed legacy pair.

Property 3 is the one that earns this gate. The failure it catches is not
exotic, it is the ordinary one: **edit the manifest, forget to re-sign.** elo
refuses that pair, correctly and silently as far as this repo is concerned —
the store row simply stops moving, and nothing here goes red. This makes it go
red here, in the same run that produced the edit.

What this does **not** do is re-implement elo's schema. Which listing fields
exist, what the caps are, which are the house's — all of that lives at elo and
is checked there, and a second copy here would be a second thing to drift.
Property 2 is not elo's rule either; it is this repo's, and it is the same
comparison `release.yml` already makes between a tag and the tree.

The gate needs no `eth-account` and never touches the network, so it cannot
skip-pass (`ci/gates.sh` header). `--check` recovers the signature and does need
`eth-account`; it says so loudly rather than passing without it.

## The text that gets signed

`--print` and `--sign` build it. It is the one thing here that also exists at
elo, so treat elo's copy as authoritative: `GET /api/store/repo/gates` serves
the exact string as `sign_this`, with the `next_seq` to use. If the two ever
disagree, elo's is right and this file is the bug. The gate does not build the
text at all — it only hashes — so a drift here can never redden CI for the
wrong reason.

## The seq

The anti-replay counter, and it only goes up. elo refuses a manifest whose seq
it has already applied, so bump it on every publish. elo keeps one counter per
title, whichever name signed it, so `--sign` bumps from the highest seq signed
under either name by default; ask elo what it will take with:

    curl -s https://elopros.com/api/store/repo/gates | python3 -m json.tool
"""
from __future__ import annotations

import argparse
import getpass
import hashlib
import json
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# (manifest, signature), in the order elo's reader tries them (header).
CURRENT = ("elo.json", "elo.sig.json")
LEGACY = ("scry.json", "scry.sig.json")
LEGACY_KEY = "scry"   # the standard's key in a legacy manifest; elo still reads it
PAIRS = tuple((ROOT / m, ROOT / s) for m, s in (CURRENT, LEGACY))
# THE pair is the first whose manifest is in the tree, as elo picks it. With
# neither there, the current names, so every message names what to write.
MANIFEST, SIG = next(((m, s) for m, s in PAIRS if m.is_file()), PAIRS[0])
SLUG = "gates"
STANDARD = 1

FAILURES: list[str] = []


def bad(msg: str) -> None:
    FAILURES.append(msg)
    print(f"  FAIL {msg}")


def ok(msg: str) -> None:
    print(f"  ok   {msg}")


def workspace_version() -> str:
    """The version in the workspace table. `crates/*/Cargo.toml` inherit it with
    `version.workspace = true`, so this is the one place it is typed — the same
    source `ci/depot.py` follows for a build id."""
    txt = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    body = txt.split("[workspace.package]", 1)
    if len(body) != 2:
        raise SystemExit("ci/elo_manifest.py: Cargo.toml has no [workspace.package] table")
    m = re.search(r'^version\s*=\s*"([^"]+)"', body[1], re.M)
    if not m:
        raise SystemExit("ci/elo_manifest.py: no version in [workspace.package]")
    return m.group(1)


def message(digest: str, seq: int) -> str:
    """The exact text elo recovers a signer from. Byte-for-byte — see the
    module header on which copy is authoritative."""
    return (f"elo repo\ngame: {SLUG}\nmanifest: {digest}\nseq: {seq}\n"
            f"by signing, this wallet publishes this manifest as '{SLUG}' on elo, "
            f"under its own address, in public.")


def read_manifest() -> tuple[bytes, dict]:
    if not MANIFEST.is_file():
        raise SystemExit(f"ci/elo_manifest.py: {MANIFEST.name} is not in the tree")
    raw = MANIFEST.read_bytes()
    try:
        return raw, json.loads(raw.decode("utf-8"))
    except Exception as e:  # noqa: BLE001
        raise SystemExit(f"ci/elo_manifest.py: {MANIFEST.name} is not readable JSON: {e}")


def read_sig(path: Path | None = None) -> dict | None:
    path = path or SIG
    if not path.is_file():
        return None
    try:
        got = json.loads(path.read_text(encoding="utf-8"))
    except Exception as e:  # noqa: BLE001
        raise SystemExit(f"ci/elo_manifest.py: {path.name} is not readable JSON: {e}")
    return got if isinstance(got, dict) else {}


def others() -> list[tuple[Path, Path]]:
    """The pairs elo is NOT reading: everything but THE pair."""
    return [(m, s) for m, s in PAIRS if m != MANIFEST]


def self_test(*, recover: bool) -> int:
    raw, doc = read_manifest()
    digest = hashlib.sha256(raw).hexdigest()
    print(f"== elo manifest ({MANIFEST.name}, {len(raw)} bytes, sha256 {digest[:16]}…)")

    # The standard's key is `"elo"`. elo still takes the key a manifest was
    # first written under, reading `"elo"` first, and so does this; a refusal
    # names only `"elo"`, because it is an instruction.
    std = doc.get("elo", doc.get(LEGACY_KEY))
    if std != STANDARD:
        bad(f'"elo" is {json.dumps(std)}; this desk speaks standard {STANDARD}')
    else:
        ok(f"standard {STANDARD}")

    if doc.get("game") != SLUG:
        bad(f'"game" is {json.dumps(doc.get("game"))}, not "{SLUG}" — elo refuses a '
            f"manifest that names a title other than the repo it came from")
    else:
        ok(f'names "{SLUG}"')

    want = workspace_version()
    got = doc.get("version")
    if got is None:
        ok(f"no version declared (the depot is then the only answer; Cargo.toml has {want})")
    elif got != want:
        bad(f'"version" is {got!r} and Cargo.toml has {want!r} — they are typed by '
            f"different hands and nothing else compares them. Fix one.")
    else:
        ok(f"version {got} matches the workspace")

    if not (doc.get("listing") or doc.get("updates")):
        bad("this manifest carries neither a \"listing\" block nor any \"updates\", "
            "so elo would refuse it as changing nothing")
    else:
        carries = [k for k in ("listing", "updates") if doc.get(k)]
        ok(f"carries {', '.join(carries)}")

    # Wall 8, on the text that actually leaves this repo. `AGENTS.md` states the
    # bare-ticker rule as this repo's own law and names a `ci/gates.sh` docs
    # check as its gate — that check does not exist, so the rule has been
    # ungated. This closes it for the store manifest and ONLY for it, which is
    # where it now has teeth: an update's title and body become a public post on
    # elo's feed, and elo refuses a `$`-prefixed ticker outright — so a `$` here
    # is not a style slip, it is a manifest that silently stops applying.
    before = len(FAILURES)
    for i, u in enumerate(doc.get("updates") or []):
        if not isinstance(u, dict):
            continue
        text = f"{u.get('title') or ''}\n{u.get('body') or ''}"
        for hit in re.findall(r"\$([A-Z][A-Z0-9]{1,9})\b", text):
            bad(f'updates[{u.get("id") or i}] writes "${hit}" — tickers are bare here and '
                f'elo refuses a $-prefixed one, so this manifest would not apply. '
                f'Write "{hit}".')
    if doc.get("updates") and len(FAILURES) == before:
        ok("no $-prefixed tickers in any update")

    sig = read_sig()
    hidden = [m for m, s in others() if m.is_file() and s.is_file()]
    stale = [p.name for m, s in others() for p in (m, s) if p.is_file()]
    if sig is None and hidden:
        # The one unsigned state that is worse than before the edit: elo reads
        # THE pair first, so the signed pair behind it is no longer read at all.
        bad(f"{MANIFEST.name} has no {SIG.name} beside it, and elo reads it before "
            f"the signed {hidden[0].name} — so elo now sees an unsigned manifest and "
            f"the store row stops moving. Sign it (`--sign`) in the same commit.")
    elif sig is None:
        # Not a failure: a manifest is written before it is signed, and elo
        # reports that state as unsigned rather than as an error. It IS a
        # failure to have signed for different bytes, which is the case below.
        ok(f"{SIG.name} is not committed yet — elo will read this manifest as "
           f"unsigned and apply nothing. `--sign` writes it.")
    elif str(sig.get("sha256") or "").lower() != digest:
        bad(f"{SIG.name} was signed for sha256 "
            f"{str(sig.get('sha256') or '(absent)')[:16]}… and {MANIFEST.name} hashes to "
            f"{digest[:16]}… — the manifest moved and the signature did not. "
            f"Re-sign: ./ci/elo_manifest.py --sign")
    else:
        ok(f"{SIG.name} covers these exact bytes (seq {sig.get('seq')})")
        if recover:
            try:
                from eth_account import Account
                from eth_account.messages import encode_defunct
            except ImportError:
                bad("--check needs eth-account to recover the signature "
                    "(`pip install eth-account`). The gate does not; this flag does.")
            else:
                text = message(digest, int(sig.get("seq") or 0))
                try:
                    rec = Account.recover_message(encode_defunct(text=text),
                                                  signature=str(sig.get("signature") or ""))
                except Exception as e:  # noqa: BLE001
                    bad(f"the signature does not recover: {e}")
                else:
                    claimed = str(sig.get("wallet") or "")
                    if rec.lower() != claimed.lower():
                        bad(f"it recovers {rec}, and {SIG.name} claims {claimed}")
                    else:
                        ok(f"signature recovers {rec}")
    if stale and sig is not None:
        ok(f"elo reads {MANIFEST.name} first and no longer reads the rest: "
           f"`git rm {' '.join(stale)}`")

    if FAILURES:
        print(f"\nGATE FAIL: elo manifest — {len(FAILURES)} problem(s)", file=sys.stderr)
        return 1
    print("\nelo manifest: ok")
    return 0


def do_sign(args) -> int:
    raw, _doc = read_manifest()
    digest = hashlib.sha256(raw).hexdigest()
    # elo's counter is the title's, not the file's: a first `elo.sig.json`
    # continues from the legacy signature's seq, or elo refuses it as replayed.
    prior = max(int((read_sig(s) or {}).get("seq") or 0) for _, s in PAIRS)
    seq = args.seq if args.seq is not None else prior + 1
    text = message(digest, seq)

    if args.show:
        print(text)
        print(f"\n# sha256({MANIFEST.name}) = {digest}\n# seq = {seq}\n"
              f"# sign that text, then: ./ci/elo_manifest.py --sign --seq {seq} "
              f"--signature 0x… --wallet 0x…", file=sys.stderr)
        return 0

    if args.signature:
        if not args.wallet:
            raise SystemExit("--signature needs --wallet: the address that made it")
        from eth_account import Account
        from eth_account.messages import encode_defunct
        rec = Account.recover_message(encode_defunct(text=text), signature=args.signature)
        if rec.lower() != args.wallet.lower():
            raise SystemExit(f"that signature recovers {rec}, not {args.wallet} — it was "
                             f"made over a different seq or a different manifest")
        wallet, signature = args.wallet, args.signature
    else:
        try:
            from eth_account import Account
        except ImportError:
            raise SystemExit("signing needs eth-account (`pip install eth-account`), or "
                             "use --print and sign the text with whatever holds your key")
        from eth_account.messages import encode_defunct
        # Each variable's former name is still read, after the current one.
        key = args.key or os.getenv("ELO_DEV_KEY") or os.getenv("SCRY_DEV_KEY")
        if key:
            acct = Account.from_key(key)
        elif args.keystore:
            blob = json.loads(Path(args.keystore).read_text(encoding="utf-8"))
            pw = (os.getenv("ELO_DEV_PASSPHRASE") or os.getenv("SCRY_DEV_PASSPHRASE")
                  or getpass.getpass("keystore passphrase: "))
            acct = Account.from_key(Account.decrypt(blob, pw))
        else:
            raise SystemExit(
                "no key. --key, ELO_DEV_KEY, --keystore, "
                "or --print for a signer we "
                "never touch. elo holds no key either; that is the whole posture.")
        sig = acct.sign_message(encode_defunct(text=text)).signature.hex()
        wallet = acct.address
        signature = sig if sig.startswith("0x") else "0x" + sig

    SIG.write_text(json.dumps({
        "wallet": wallet, "seq": seq, "sha256": digest, "signature": signature,
    }, indent=1) + "\n", encoding="utf-8")
    print(f"wrote {SIG.name}  (seq {seq}, sha256 {digest[:16]}…, wallet {wallet})")
    print(f"commit {MANIFEST.name} and {SIG.name} together — the signature covers those bytes.")
    stale = [p.name for m, s in others() for p in (m, s) if p.is_file()]
    if stale:
        print(f"and `git rm {' '.join(stale)}` in that commit — elo reads {MANIFEST.name} first.")
    print(f"then:  curl -s https://elopros.com/api/store/repo/{SLUG}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description="Gates' elo manifest and its signature")
    ap.add_argument("--self-test", action="store_true", help="the gate (no key, no network)")
    ap.add_argument("--check", action="store_true", help="the gate plus signature recovery")
    ap.add_argument("--sign", action="store_true", help="write the signature beside the manifest")
    ap.add_argument("--print", dest="show", action="store_true",
                    help="print the text to sign and exit — no key is touched")
    ap.add_argument("--seq", type=int)
    ap.add_argument("--key", help="a hex private key")
    ap.add_argument("--keystore", help="an encrypted V3 keystore file")
    ap.add_argument("--signature", help="a signature produced elsewhere")
    ap.add_argument("--wallet", help="the address that made --signature")
    args = ap.parse_args()

    if args.sign or args.show:
        return do_sign(args)
    if args.self_test or args.check:
        return self_test(recover=args.check)
    ap.print_help()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
