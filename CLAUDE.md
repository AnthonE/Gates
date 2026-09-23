# CLAUDE.md — Gates

**MVP fast-iteration mode** (operator, 2026-09-23: *"can we just put code in
and not do all the extra junk… im in MVP fast itteration mode"*). Ship code.
This page is the whole process. The long manual it replaced (every trap, wall
and war story) is in git: `git show ecb9e21:CLAUDE.md`. Grep it when
something weird happens. Don't read it up front.

## What this is

A survival game in the Rust-the-game tradition: authoritative Rust server,
Bevy client (desktop, plus the same code compiled to wasm for the browser),
WebTransport/QUIC.

| crate | what |
|---|---|
| `sim-core` | the deterministic simulation: world, combat, building, terrain |
| `protocol` | wire format (`PROTO_VER` in `src/lib.rs`) |
| `server` | the shard (`--bin shard`), load bots (`--bin bots`), `--bin profile` |
| `client-core` | prediction and netcode, no rendering |
| `client` | the game (`--features render`, `--bin gates`); wasm too |
| `client-web` | the browser entry point (`ci/build_web.sh`) |
| `sound`, `sound-worklet` | audio |
| `content` + `content/*.toml` | items, recipes, loot, balance numbers |

## The loop

1. Write the code.
2. `ci/quick.sh`: formats, runs a 1-second knob check, then clippies the
   workspace and the game. Add crate names to run their tests too
   (`ci/quick.sh sim-core protocol`).
3. Commit with a short message. Push your branch.

CI runs the full suite (`ci/gates.sh`) on the PR. There's no need to run it
locally.

## Don't

- Update `NOW.md`, `DECISIONS.md`, `HAVE.md` or other docs unless asked.
- Write findings notes, research docs, `BRANCH-NOTES.md` or long commit bodies.
- Add a test for every change, or mutation-test your tests. Add one when the
  logic is tricky or has broken before.
- Read the big docs before starting. Grep them when you need an answer:
  `DESIGN.md` (product), `NETCODE.md`, `TERRAIN.md`, `RENDER.md`, `ART.md`
  (visual bar), `CONTENT.md` (content schemas), `reference/*.md` (how Rust
  the game does a system), `NOW.md` (backlog).
- Register new numbers anywhere. They go in code, or in `content/*.toml` for
  items and balance. The one catch: about 700 existing constants are pinned in
  `DECISIONS.md` §Open, and CI fails if code and row disagree. `ci/quick.sh`
  catches that in a second. Update the value in the row.

## What CI will stop you on (so you know why)

1. **`sim-core` is deterministic.** No I/O, clock, threads, `HashMap`
   iteration or trig/libm, and floats are limited to `+ − × ÷ sqrt min max`.
   Clippy enforces it, and native vs wasm must hash identically.
2. **No allocation in the sim tick after warmup** (`test_alloc_zero`), and
   every queue has a cap in `sim-core/src/limits.rs`.
3. **Bevy draws, it does not decide.** Gameplay state lives in `sim-core` and
   `ClientCore`, never in the ECS.
4. **Wire change:** bump `PROTO_VER`, then run
   `cargo run -p protocol --example gen_goldens`.
5. **Worldgen change:** `terrain_golden` fails and shows the new hash as
   `left`. Paste it into `GOLDEN_TERRAIN_HASH`.
6. **Item/recipe/balance numbers** live in `content/*.toml`, validated at boot.

## Commands

```
ci/quick.sh [crate…]                                # the local check
cargo run -p server --bin shard                      # server (reads shard.toml)
cargo run -p client --features render --bin gates    # the game
cargo run -p server --bin bots -- 100                # 100 bots
cargo test -p <crate>                                # one crate's tests
./ci/gates.sh                                        # everything CI runs (~90 min cold)
```

## Traps that already cost real time

- `--features render` code is compiled only by
  `cargo clippy -p client --features render --all-targets`. `cargo test
  --workspace` never sees it, so a green workspace can still break the game.
  `ci/quick.sh` runs it.
- Two of the same component in one Bevy bundle panics at spawn (runtime).
  Clippy and the compiler both pass it.
- `ClientCore`'s `pop_*` rings are read once, in `render/feed.rs`. Another
  reader borrows that resource. Never add a second `pop_*` call.
- Big fixed arrays in `sim-core` go on the heap (`crate::boxed_array`).
  wasm's 1 MiB stack overflows silently otherwise.
- Datagrams: `send_datagram()` (drop-oldest), never `send_datagram_wait()`.
  Quantize values on both sides or prediction drifts.
- A `rust-lld` bus error (SIGBUS) means the disk is full.
  `cargo … | tail` hides cargo's exit code (`${PIPESTATUS[0]}`).

## Fresh box

- The game build needs `libwayland-dev libasound2-dev libudev-dev`
  (`apt-get update` first). Running it also needs `libxkbcommon-x11-0`.
- `rustup target add wasm32-unknown-unknown` for the wasm gates.
- `pip install numpy Pillow` for the asset gates.
- Server `*_wire` tests overflow the stack in debug:
  `RUST_MIN_STACK=16777216`.

## Rules that stay

- **Operator-only:** tagging a release, deploying the public shard,
  publishing the depot or the web page, anything on-chain, wiping a live
  shard, admin bans.
- Pushing a branch is fine. Never force-push `main`.
- Assets are CC0 or CC-BY (credited in the manifest). No NC or SA, because
  the game is sold. Never copy Rust's (Facepunch's) own art.
- `crates/client/src/elo_overlay.rs` is vendored from `AnthonE/scry-forge`
  and a test pins its hash. Fix it upstream and re-copy.
- The old JavaScript client (`web/`) stays deleted. The browser build is the
  Rust client compiled to wasm.
- The autonomous loop lives outside the repo (`/mnt/hive-data/gates-loop`,
  stop with `touch …/STOP`). Its judge rubric predates MVP mode.
