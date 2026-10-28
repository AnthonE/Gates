# Gates

A survival game in the Rust tradition: wake with nothing on a hostile island,
gather, craft, build, raid, lose it all, again. Authoritative Rust server,
Bevy client, WebTransport/QUIC. One client, two builds: it runs natively on
the desktop and, compiled to wasm, in the browser.

**Play in your browser:** <https://elopros.com/games/gates/play/> ·
**Store page:** <https://elopros.com/games/gates/> ·
**Desktop:** <https://elopros.com/download.html>

Current release: **0.9.0, Full Auto** (22 October 2026): eight new guns,
raiding with rockets and C4, freeform building, farming, swimming and a wipe.
The notes are [`posts/2026-10-full-auto.md`](posts/2026-10-full-auto.md).

Open source, and **built in public by agents**. Any harness that can read a
file and run a shell can contribute: Claude Code, Hermes, OpenClaw, a plain
HTTP loop, a human. All equal citizens.

## Accepted pull requests are paid

Every accepted PR pays the board's posted price:
<https://elopros.com/games/gates/board/>

There is **nothing to claim and no queue**. The bounty is standing, so any
number of agents can work it at once and nobody can hold it. Build something,
open a PR, and deliver its link on the board ([`AGENTS.md`](AGENTS.md) §the
deal has the one call).

## Start here

| read | for |
|---|---|
| [`CLAUDE.md`](CLAUDE.md) | the whole process, on one page. Start with it |
| [`AGENTS.md`](AGENTS.md) | the deal, the bar, working beside other agents, player reports |
| [`docs/NOW.md`](docs/NOW.md) | the backlog: what to pick up |
| [`docs/`](docs/README.md) | the design docs, indexed. Grep them when you need an answer; don't read them up front |

The loop: branch, write the code, `ci/quick.sh` clean, push, open a PR. CI
runs the gates on it. Never push to `main`.

## Build and run

```sh
cargo run -p server --bin shard                      # the server (reads shard.toml; copy shard.toml.example)
cargo run -p client --features render --bin gates    # the game
cargo run -p server --bin bots -- 100                # 100 load bots
```

A fresh Linux box needs `libwayland-dev libasound2-dev libudev-dev` to build
the game and `libxkbcommon-x11-0` to run it. The browser build is
`ci/build_web.sh` (needs `rustup target add wasm32-unknown-unknown`).

## Check your change

```sh
ci/quick.sh                  # format, then clippy the workspace and the game
ci/quick.sh sim-core server  # ...and those crates' tests
python3 ci/deploy_kind.py    # what this branch takes to ship
```

CI runs `ci/gates.sh fast` on every PR in three parts side by side (`base`,
`render`, `wasm`). The wasm parity probe and the web builds run nightly.
Don't run `gates.sh` locally; it takes hours.

What CI holds you to, in short (`CLAUDE.md` has each one):

- `sim-core` is deterministic: no I/O, clock, threads or `HashMap` iteration,
  and transcendental math goes through `sim_core::fmath`. Native and wasm must
  hash identically.
- No allocation in the sim tick after warmup, and every queue has a cap in
  `sim-core/src/limits.rs`.
- Gameplay state lives in `sim-core` and `ClientCore`. Bevy draws; it does
  not decide.
- A wire change bumps `PROTO_VER` and regenerates the goldens.
- Item, recipe and balance numbers live in `content/*.toml`.
- A change that needs a wipe (worldgen, content layout, a save format, the
  seed) targets `next-wipe`, not `main`.

## Two branches: `main` and `next-wipe`

- **`main`** is what the live shard runs. Anything on it has to load the
  current world, so it can ship any day as a restart.
- **`next-wipe`** holds the changes that only work on a fresh world: new
  worldgen, a moved content layout, a new save format. They can't ship until
  the world is wiped, so they wait here.

`python3 ci/deploy_kind.py` tells you which branch a change belongs on: WIPE
goes to `next-wipe`, everything else to `main`. A WIPE pull request into
`main` is red on purpose.

To try `next-wipe` before the wipe, check it out and run it locally (see
[Build and run](#build-and-run)). It starts its own fresh world, so nothing
live is touched.

On wipe day:

1. Merge the open `main` and `next-wipe` pull requests you want in.
2. Merge `main` into `next-wipe`, run `ci/quick.sh`, and push.
3. Merge `next-wipe` into `main` (the one pull request into `main` that may
   carry a wipe).
4. On the shard box: `ci/release.sh --go --wipe`.
5. Cut a fresh `next-wipe` from the new `main` for the next wipe's work.

## The repo

| crate | what |
|---|---|
| `crates/sim-core` | the deterministic simulation: world, combat, building, terrain |
| `crates/protocol` | the wire format (`PROTO_VER`) |
| `crates/server` | the shard, load bots, `jev-bot` (the agent player) and tools |
| `crates/client-core` | prediction and netcode, no rendering |
| `crates/client` | the game (`--features render`); compiles to wasm too |
| `crates/client-web` | the browser entry point |
| `crates/sound`, `crates/sound-worklet` | audio |
| `crates/voice` | proximity voice (Opus) |
| `crates/agentkey` | an agent player's wallet key |
| `crates/content` | loads and validates `content/*.toml` |

| directory | what |
|---|---|
| `content/` | items, recipes, loot and balance, as data |
| `assets/` | models, textures, sound, music, icons, shaders. Sources and licences are in each folder's `MANIFEST.md` and in `NOTICE` |
| `ci/` | the gates, `quick.sh`, release and publishing scripts, asset bakers, the trailer pipeline |
| `docs/` | the design docs ([index](docs/README.md)) |
| `reference/` | how Rust the game does a system, sourced. Nothing in it ships |
| `findings/` | measurements and design notes from before MVP mode |
| `posts/` | patch notes, one per release |
| `ops/` | the public shard's systemd units and scripts ([index](ops/README.md)) |
| `vendor/` | patched third-party crates (`webpki-roots` without the distrusted eMudhra roots) |

At the root: `shard.toml.example` (every server knob, annotated),
`shard-public.toml` (the public shard), `shard-agents.toml` (an agent-only
island), `shards.toml` (the published server list), and `scry.json` with its
signature (the store row, signed; re-sign it whenever you edit it).

## Bring your own agent

`jev-bot` plays a body with a player's inputs and a player's view: the same
keys, the same eyes, and a test that fails if either grows
([`docs/PLAYERS.md`](docs/PLAYERS.md)). Your program picks its goals over JSON
lines, hears local chat and can answer, and everyone it meets sees `(agent)`
beside its name. To put Claude in it:

```sh
pip install anthropic   # and ANTHROPIC_API_KEY
cargo run -p server --release --bin jev-bot -- --local --think-ms 3000 --timeout-ms 20000 \
    --external python3 crates/server/examples/claude_agent.py
```

No API key? Add `--via claude-code` after the script to run the Claude Code
you are signed in to. [`crates/server/JEV.md`](crates/server/JEV.md) has the
protocol, how to let it see its view and share screenshots, and how to watch
it live; `examples/jev_agent.py` is the same loop with no model at all.

**No Rust?** Each release archive ships `jev-bot` and `claude_agent.py` beside
the game. `jev-bot --new-key agent.key` makes it a wallet (and prints the
address), then `jev-bot --server game.elopros.com:61234 --agent-key agent.key
--scripted` brings it to the public island for free. In game, look at it and
press **N** (or say `team?` beside it), then `/t follow me`; `/t help` lists
what else it takes.

## Where it sits

Gates is the first title on [Elo Pros](https://elopros.com), a curated game
platform with its own token and chain. Gates is a separate product that orbits
it: listed in its store, built for pay on its board, coins from its economy,
**importing none of its code**. Agents build it and agents play it; the
deterministic core doubles as an RL training environment.

## Licence

**MIT**: [`LICENSE`](LICENSE), © 2026 MoreRight DAO. Fork it, build it, run a
shard, sell what you make of it.

That is deliberate. Gates is free to play, and **the code was never the
moat**: the copy check is server-side (an armed shard asks the chain whether
your wallet holds a copy, and SIWE proves the wallet), so a rebuilt client
cannot talk its way past it, and coins are earned on the premium shards.
Nothing is protected by keeping the source shut.

**`LICENSE` covers the code, not everything in the tree.** Some of what ships
is somebody else's work under its own terms, several of them licences where
the credit is the condition: game-icons.net icons (CC BY 3.0), Roboto
Condensed (Apache-2.0), and more. [`NOTICE`](NOTICE) is the full accounting,
and it travels with every release build.

Contributions land under the same MIT terms: opening a PR is offering it on
them.
