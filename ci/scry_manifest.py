#!/usr/bin/env python3
"""Gates' end of scry's repo desk — `scry.json` and the signature over it.

    ./ci/scry_manifest.py --self-test    the gate: no network, no key, stdlib only
    ./ci/scry_manifest.py --check        the same, plus recover the signature
    ./ci/scry_manifest.py --print        the exact text to sign, for a signer we do not hold
    ./ci/scry_manifest.py --sign         write scry.sig.json (needs a key)

## What this seam is

scry reads two files from this repo's default branch and nothing else:

    scry.json        what this game is, right now — its store row and its feed
    scry.sig.json    a detached EIP-191 signature over scry.json's exact bytes

It applies what changed. Nothing here runs on scry's box and nothing of scry's
runs here: the whole transport is a commit, and the authority is the signature.
The standard is `GET https://scry.moreright.xyz/api/library/GAME-REPO.md`.

## What the GATE checks, and what it deliberately does not

Two properties, both **local**, both fully decidable with the standard library:

  1. `scry.json` is a JSON object naming this game and standard 1.
  2. `version` equals the workspace version in `Cargo.toml`.
  3. If `scry.sig.json` exists, its `sha256` is the sha256 of `scry.json`'s
     bytes — i.e. the signature was written for the manifest that is here.

Property 3 is the one that earns this gate. The failure it catches is not
exotic, it is the ordinary one: **edit the manifest, forget to re-sign.** scry
refuses that pair, correctly and silently as far as this repo is concerned —
the store row simply stops moving, and nothing here goes red. This makes it go
red here, in the same run that produced the edit.

What this does **not** do is re-implement scry's schema. Which listing fields
exist, what the caps are, which are the house's — all of that lives at scry and
is checked there, and a second copy here would be a second thing to drift.
Property 2 is not scry's rule either; it is this repo's, and it is the same
comparison `release.yml` already makes between a tag and the tree.

The gate needs no `eth-account` and never touches the network, so it cannot
skip-pass (`ci/gates.sh` header). `--check` recovers the signature and does need
`eth-account`; it says so loudly rather than passing without it.

## The text that gets signed

`--print` and `--sign` build it. It is the one thing here that also exists at
scry, so treat scry's copy as authoritative: `GET /api/store/repo/gates` serves
the exact string as `sign_this`, with the `next_seq` to use. If the two ever
disagree, scry's is right and this file is the bug. The gate does not build the
text at all — it only hashes — so a drift here can never redden CI for the
wrong reason.

## The seq

The anti-replay counter, and it only goes up. scry refuses a manifest whose seq
it has already applied, so bump it on every publish. `--sign` bumps it from the
current `scry.sig.json` by default; ask scry what it will take with:

    curl -s https://scry.moreright.xyz/api/store/repo/gates | python3 -m json.tool
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
MANIFEST = ROOT / "scry.json"
SIG = ROOT / "scry.sig.json"
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
        raise SystemExit("ci/scry_manifest.py: Cargo.toml has no [workspace.package] table")
    m = re.search(r'^version\s*=\s*"([^"]+)"', body[1], re.M)
    if not m:
        raise SystemExit("ci/scry_manifest.py: no version in [workspace.package]")
    return m.group(1)


def message(digest: str, seq: int) -> str:
    """The exact text scry recovers a signer from. Byte-for-byte — see the
    module header on which copy is authoritative."""
    return (f"scry repo\ngame: {SLUG}\nmanifest: {digest}\nseq: {seq}\n"
            f"by signing, this wallet publishes this manifest as '{SLUG}' on scry, "
            f"under its own address, in public.")


def read_manifest() -> tuple[bytes, dict]:
    if not MANIFEST.is_file():
        raise SystemExit(f"ci/scry_manifest.py: {MANIFEST.name} is not in the tree")
    raw = MANIFEST.read_bytes()
    try:
        return raw, json.loads(raw.decode("utf-8"))
    except Exception as e:  # noqa: BLE001
        raise SystemExit(f"ci/scry_manifest.py: {MANIFEST.name} is not readable JSON: {e}")


def read_sig() -> dict | None:
    if not SIG.is_file():
        return None
    try:
        got = json.loads(SIG.read_text(encoding="utf-8"))
    except Exception as e:  # noqa: BLE001
        raise SystemExit(f"ci/scry_manifest.py: {SIG.name} is not readable JSON: {e}")
    return got if isinstance(got, dict) else {}


def self_test(*, recover: bool) -> int:
    raw, doc = read_manifest()
    digest = hashlib.sha256(raw).hexdigest()
    print(f"== scry manifest ({MANIFEST.name}, {len(raw)} bytes, sha256 {digest[:16]}…)")

    if doc.get("scry") != STANDARD:
        bad(f'"scry" is {json.dumps(doc.get("scry"))}; this desk speaks standard {STANDARD}')
    else:
        ok(f"standard {STANDARD}")

    if doc.get("game") != SLUG:
        bad(f'"game" is {json.dumps(doc.get("game"))}, not "{SLUG}" — scry refuses a '
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
            "so scry would refuse it as changing nothing")
    else:
        carries = [k for k in ("listing", "updates") if doc.get(k)]
        ok(f"carries {', '.join(carries)}")

    # Wall 8, on the text that actually leaves this repo. `AGENTS.md` states the
    # bare-ticker rule as this repo's own law and names a `ci/gates.sh` docs
    # check as its gate — that check does not exist, so the rule has been
    # ungated. This closes it for `scry.json` and ONLY for `scry.json`, which is
    # where it now has teeth: an update's title and body become a public post on
    # scry's feed, and scry refuses a `$`-prefixed ticker outright — so a `$` here
    # is not a style slip, it is a manifest that silently stops applying.
    before = len(FAILURES)
    for i, u in enumerate(doc.get("updates") or []):
        if not isinstance(u, dict):
            continue
        text = f"{u.get('title') or ''}\n{u.get('body') or ''}"
        for hit in re.findall(r"\$([A-Z][A-Z0-9]{1,9})\b", text):
            bad(f'updates[{u.get("id") or i}] writes "${hit}" — tickers are bare here and '
                f'scry refuses a $-prefixed one, so this manifest would not apply. '
                f'Write "{hit}".')
    if doc.get("updates") and len(FAILURES) == before:
        ok("no $-prefixed tickers in any update")

    sig = read_sig()
    if sig is None:
        # Not a failure: a manifest is written before it is signed, and scry
        # reports that state as unsigned rather than as an error. It IS a
        # failure to have signed for different bytes, which is the case below.
        ok(f"{SIG.name} is not committed yet — scry will read this manifest as "
           f"unsigned and apply nothing. `--sign` writes it.")
    elif str(sig.get("sha256") or "").lower() != digest:
        bad(f"{SIG.name} was signed for sha256 "
            f"{str(sig.get('sha256') or '(absent)')[:16]}… and {MANIFEST.name} hashes to "
            f"{digest[:16]}… — the manifest moved and the signature did not. "
            f"Re-sign: ./ci/scry_manifest.py --sign")
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

    if FAILURES:
        print(f"\nGATE FAIL: scry manifest — {len(FAILURES)} problem(s)", file=sys.stderr)
        return 1
    print("\nscry manifest: ok")
    return 0


def do_sign(args) -> int:
    raw, _doc = read_manifest()
    digest = hashlib.sha256(raw).hexdigest()
    prior = read_sig() or {}
    seq = args.seq if args.seq is not None else int(prior.get("seq") or 0) + 1
    text = message(digest, seq)

    if args.show:
        print(text)
        print(f"\n# sha256({MANIFEST.name}) = {digest}\n# seq = {seq}\n"
              f"# sign that text, then: ./ci/scry_manifest.py --sign --seq {seq} "
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
        key = args.key or os.getenv("SCRY_DEV_KEY")
        if key:
            acct = Account.from_key(key)
        elif args.keystore:
            blob = json.loads(Path(args.keystore).read_text(encoding="utf-8"))
            pw = os.getenv("SCRY_DEV_PASSPHRASE") or getpass.getpass("keystore passphrase: ")
            acct = Account.from_key(Account.decrypt(blob, pw))
        else:
            raise SystemExit(
                "no key. --key, SCRY_DEV_KEY, --keystore, or --print for a signer we "
                "never touch. scry holds no key either; that is the whole posture.")
        sig = acct.sign_message(encode_defunct(text=text)).signature.hex()
        wallet = acct.address
        signature = sig if sig.startswith("0x") else "0x" + sig

    SIG.write_text(json.dumps({
        "wallet": wallet, "seq": seq, "sha256": digest, "signature": signature,
    }, indent=1) + "\n", encoding="utf-8")
    print(f"wrote {SIG.name}  (seq {seq}, sha256 {digest[:16]}…, wallet {wallet})")
    print(f"commit {MANIFEST.name} and {SIG.name} together — the signature covers those bytes.")
    print(f"then:  curl -s https://scry.moreright.xyz/api/store/repo/{SLUG}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description="Gates' scry manifest and its signature")
    ap.add_argument("--self-test", action="store_true", help="the gate (no key, no network)")
    ap.add_argument("--check", action="store_true", help="the gate plus signature recovery")
    ap.add_argument("--sign", action="store_true", help="write scry.sig.json")
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
