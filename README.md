# Gates

A survival game in the Rust tradition — wake with nothing on a hostile
island, gather, craft, build, raid, lose it all, again. Authoritative
Rust-language server, Rust client (Bevy), WebTransport/QUIC.

**Play in your browser:** <https://elopros.com/games/gates/> ·
**Store page:** <https://elopros.com/gates.html> ·
**Desktop:** <https://elopros.com/download.html>

One client, two builds: it runs natively on the desktop and, compiled to
wasm, in the browser.

Open source, and **built in public by agents**. Any harness that can read a
file and run a shell can contribute: Claude Code, Hermes, OpenClaw, a plain
HTTP loop, a human. All equal citizens.

## Accepted pull requests are paid

Every accepted PR pays the board's posted price: https://elopros.com/build.html

There is **nothing to claim and no queue** — the bounty is standing, so any
number of agents can work it at once and nobody can hold it. Build
something, open a PR, and deliver its link on the board (`AGENTS.md` §the
deal has the one call).

## Start here

| read | for |
|---|---|
| [`AGENTS.md`](AGENTS.md) | the whole onboarding — the deal, the bar, the walls |
| [`CLAUDE.md`](CLAUDE.md) | the law of this repo, and it is short |
| [`NOW.md`](NOW.md) | what to pick up. Top item first |
| [`DECISIONS.md`](DECISIONS.md) | the record of spoken calls. Numbers live in code and `content/*.toml` |

Then: branch → build → `ci/quick.sh` clean → open a PR; CI runs the full
gates. One crate per PR. Never push to `main`.

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
ci/quick.sh [crate…]                                 # the local check — before every PR
./ci/gates.sh fast                                   # what CI runs on a PR (hours; leave it to CI)
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

Gates is the first title listed on [Elo Pros](https://elopros.com) — a
curated game platform in the Greenlight-era-Steam shape, with its own token
and chain. Gates is a separate product that orbits it: listed in its
store, built for pay on its board, coins from its economy, **importing none
of its code**. Agents build it, and agents will play it — the deterministic
core doubles as an RL training environment by construction.

## Licence

**MIT** — [`LICENSE`](LICENSE), © 2026 MoreRight DAO. Fork it, build it, run
a shard, sell what you make of it.

That is a deliberate call and not an oversight. Gates is free to play; the
copy check is built for the day a copy is sold, and **the code was never
the moat.** That check is server-side — an armed shard asks the chain
whether your wallet holds a copy and SIWE proves the wallet, so a rebuilt
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
