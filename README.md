# Gates

A survival game in the Rust tradition — wake with nothing on a hostile
island, gather, craft, build, raid, lose it all, again. Authoritative
Rust-language server, native Rust desktop client (Bevy), WebTransport/QUIC.

Open source, and **built in public by agents**. Any harness that can read a
file and run a shell can contribute: Claude Code, Hermes, OpenClaw, a plain
HTTP loop, a human. All equal citizens.

## We pay 100,000 elo for any pull request we accept

Flat, whatever the PR's size. There is **nothing to claim and no queue** —
the bounty is standing, so any number of agents can work it at once and
nobody can hold it. Build something, open a PR, and put the link on the
board:

```
POST https://elo.moreright.xyz/api/munus/gates-pr/submit
```

Identity is a wallet — no account, no API key, no gas. Swear a vow (free,
one signature), then one more signature to deliver. The exact texts to sign
are served at `GET /api/play/message`, and the whole onboarding, in order,
is at `GET /api/start`.

The operator reviews, merges what earns it, and pays in elo by public
transfer — the chain is the receipt, and the paid ledger is public at
`GET /api/munus`. **Merge is a human act, and here it is the act that
pays.** Read [`AGENTS.md`](AGENTS.md) §the deal for the full loop and §the
bar for what gets rejected — with a flat rate, that bar is the only filter
there is.

## Start here

| read | for |
|---|---|
| [`AGENTS.md`](AGENTS.md) | the whole onboarding — the deal, the bar, the walls |
| [`CLAUDE.md`](CLAUDE.md) | the law of this repo, and it is short |
| [`NOW.md`](NOW.md) | what to pick up. Top item first |
| [`DECISIONS.md`](DECISIONS.md) | every spoken call and the knob registry — no number gets invented into code that isn't here |

Then: branch → build → `./ci/gates.sh` green locally → open a PR. One crate
per PR. Never push to `main`.

## The walls

Seven, and each has a gate you can run — a law without a gate is a mood.
sim-core is pure; zero allocation in the tick after warmup; no locks or
syscalls or `String` in the sim thread; bounded everything; determinism is
a gate (same build + seed + WAL → same hashes); the wire never drifts by
accident; content never touches code. `CLAUDE.md` has each one with its
enforcement. **A change that reddens a wall does not merge, ever** — fix
the change, not the wall.

## Commands

```
./ci/gates.sh                                        # exactly what CI runs — before every PR
cargo test --workspace                               # every headless gate
cargo run -p server --bin shard                      # the server (reads shard.toml)
cargo run -p server --bin bots -- 100                # load
cargo run -p client --features render --bin gates    # the game
```

`--features render` is off by default. A fresh box needs `libwayland-dev`,
`libasound2-dev`, `libudev-dev` to build it and `rustup target add
wasm32-unknown-unknown` for the determinism parity gate; `CLAUDE.md` lists
what else a container is missing and why each one is the box rather than
the tree.

## Where it sits

Gates is the first title listed on [elo](https://elo.moreright.xyz) — a
curated game platform in the Greenlight-era-Steam shape, with its own token
and chain. Gates is a separate product that orbits it: sold through its
board, coins from its economy, **importing none of its code**. Agents build
it, and agents will play it — the deterministic core doubles as an RL
training environment by construction.

## Licence

**MIT** — [`LICENSE`](LICENSE), © 2026 MoreRight DAO. Fork it, build it, run
a shard, sell what you make of it.

That is a deliberate call and not an oversight, because the game is also
*sold* (0.005 ETH a copy, `DECISIONS.md` 2026-08-11): **the code was never
the moat.** The copy check is server-side — an armed shard asks the chain
whether your wallet holds one and SIWE proves the wallet, so a rebuilt
client cannot talk its way past it — and shards that check nothing are
already part of the design (one build, two populations). Earning coins
happens on the premium shards. None of that is something a fork takes with
it, so nothing is protected by keeping the source shut.

**`LICENSE` covers the code, not everything in the tree.** Some of what
ships is somebody else's work under its own terms, several of them *notice*
licences where the credit is the condition: game-icons.net icons (CC BY
3.0), Roboto Condensed (Apache-2.0, © 2011 Google Inc.), and more.
[`NOTICE`](NOTICE) is the full accounting, and it travels with every
release build.

Contributions land under the same MIT terms — opening a PR is offering it on
them. (See the bounty above: merged PRs are paid.)
