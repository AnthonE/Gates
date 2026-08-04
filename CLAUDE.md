# CLAUDE.md — Gates

The operating manual for anyone — human or agent loop — working this repo.
Read this first, every iteration. It is deliberately bounded: no dated
state, no counts, no rules for things that don't exist yet. If a claim
here is wrong, fix the claim; history goes in `DECISIONS.md`.

## What this is

A browser survival game (Rust-the-game tradition): Rust-language
authoritative server, three.js client, WebTransport/QUIC. A separate
product that orbits scry — sold through its Great Work board, coins from
its economy — importing none of its code. **The skeleton is the product**:
determinism, netcode, and the hot-path laws outrank every feature.

**Agents are first-class on both sides.** They build it — `AGENTS.md` is
the door for any harness — and they will play it: the deterministic core
doubles as an RL training environment (a stated goal, `DECISIONS.md`
2026-08-01), which is one more reason the walls never bend. An agent player
pays the same doors and earns the same coins as a human.

## The docs, and which wins

| doc | owns | when in doubt |
|---|---|---|
| `DESIGN.md` | product, pillars, economy, architecture, milestones | the frame |
| `NETCODE.md` | everything multiplayer + transport config | **beats DESIGN §5** |
| `TERRAIN.md` | worldgen, slots, collision, terrain rendering | |
| `CONTENT.md` | every item/recipe/damage/loot number, as data schemas | numbers live here, never in code |
| `ALPHA.md` | the alpha cut, staged economy arming (A1→A2→A3) | |
| `ART.md` | the art bible: measured targets off `Rust Images/`, the hard visual rules, the review checklist | **the visual bar; the art rubric scores against it** |
| `DECISIONS.md` | dated operator calls; **the knob registry** | authoritative on every **(knob)** |
| `MENUS.md` | the interaction surface audit: every screen and verb, ours against the reference, measured off the two Rust mod loaders' hook tables | **owns nothing** — a survey to cut items from, never a queue |
| `NOW.md` | what next | **the only list that answers that** |

Docs are dated notes, not law. Four things actually bind: the walls below,
the gates in CI, the operator's spoken decisions, and measurements. A doc
that disagrees with a passing gate is wrong — fix the doc.

## The walls (each with its enforcement — a law without a gate is a mood)

1. **sim-core is pure.** No I/O, no clock, no threads, no `HashMap`/
   `HashSet` iteration, no libm/trig, floats restricted to
   `+ − × ÷ sqrt min max clamp floor-by-cast`. → clippy disallowed
   types/methods + `test_parity_wasm` (native and wasm bit-identical).
2. **Zero allocation in the tick after warmup.** → counting allocator,
   `test_alloc_zero`.
3. **No locks, no syscalls, no `String`/`format!`/logging in the sim
   thread.** Rings only; integer event codes only. → clippy walls + soak
   tick-jitter assert.
4. **Bounded everything.** Every queue, map, and per-tick work item has a
   cap in `limits.rs` and a stated overflow policy. No `push` on a
   client-driven path without a cap check. → review wall + `test_raid_storm`.
5. **Determinism is a gate, not a vibe.** Same build + seed + WAL →
   same state hashes. → `test_replay`, `test_terrain_golden`.
6. **The wire never drifts by accident.** Packet layouts change only with
   a version bump + regenerated goldens in the same commit. →
   `test_protocol_golden`.
7. **Content never touches code.** New items, recipes, balance passes =
   `content/*.toml` only, validated at boot, content hash pinned into the
   WAL header (a replay replays the content it was played under). →
   `test_content`.
8. **Money is appearance-only from the house.** The never-table
   (`DESIGN.md` §3.3) is a wall, not a knob. Economy stages (A1/A2/A3)
   arm only by operator act. Tickers are bare: SCRY, OBOL, MYRRH — never
   a `$` prefix.

## Traps already paid for (learned from research or scry production —
do not rediscover)

- **wtransport must be pinned ≥ commit `0f7609a`** (or a release
  containing it) — 0.7.1 has a two-byte remote panic.
- A browser datagram write over `maxDatagramSize` **silently succeeds and
  sends nothing** — clamp every send against the live value.
- `send_datagram()` (drop-oldest), never `send_datagram_wait()` — a
  congestion stall must cost freshness, not latency.
- **Quantize both sides** or prediction drifts by rounding: the server
  sims on the values it transmits.
- The client is also a hot path: no per-frame allocations, no closures in
  the RAF loop, zero-copy typed-array parsing. GC pauses on the client
  feel identical to server blips.
- Stream-in AND stream-out are budgeted per frame on the client — the
  teardown spike is the half everyone forgets.
- A suite that skips on a missing dep must say SKIP loudly and exit
  nonzero in CI — a pass it didn't earn is the worst bug class.
- Never start a line of a commit body with `Operator, YYYY-MM-DD:` unless
  the same commit updates `DECISIONS.md`.
- **Median fps hides shader-compile stalls.** A static benchmark can read
  90+ fps while lazy WebGL program links cost 700 ms+ worst-frames in real
  play. Prewarm every program at boot; the gate is a COUNT (no program
  links after `inWorld`, asserted in `browser_smoke`), never a frame-time
  threshold.
- **A judge names the symptom; fix the cause.** Optimizing the judge's
  literal sentence is how a loop circles for three passes — elsewhere,
  "untextured" was really diffuse contrast crushed by an earlier fix for
  "too bright", and the correct change was the opposite of the feedback's
  direction. Diagnose the mechanism before acting on a ranked gap.
- **Tonemap, sky, exposure, and fog are one owner.** Split across parallel
  passes they break each other's assumptions faster than they improve
  (measured elsewhere: three parallel rounds worsened visual defects
  60→66; one sequential owner over the coupled set cut them to 26). The
  lighting gap, when attacked, is a single iteration's single ownership.

## The loop discipline

- An iteration = pick from `NOW.md` → branch → build → **all gates green
  locally** → merge. A change that reddens a wall does not merge, ever.
- **Knobs are spoken, never invented.** Every tunable is either in
  `DECISIONS.md` as spoken, or carries its documented default. Inventing
  a number = writing it into `DECISIONS.md` §open, not into code.
- **Operator-only acts** (a loop proposes, never performs): arming A2/A3,
  anything on-chain, publishing the page or the link, deploying to the
  public shard, cert/domain changes, wipes of a live shard, admin bans.
- Parallel loops: one owner per crate per iteration; `protocol` and
  `limits.rs` changes never land from two branches in one merge window.
- When the operator's word conflicts with any doc including this one, the
  word wins; record it in `DECISIONS.md` the same day.

## Commands (derive, don't quote)

```
cargo test --workspace              # every gate that runs headless
cargo run -p server --bin shard     # the server (reads shard.toml)
cargo run -p server --bin bots -- 100
cargo run -p server --bin replay -- --wal <file>
./web/dev.sh                        # vite + wasm-pack watch
./ci/gates.sh                       # exactly what CI runs — run it before merge
```

## The loop that builds this repo

Most commits here are written by an autonomous loop, not typed. It lives at
`/mnt/hive-data/gates-loop` — **outside this repo, deliberately.** The builder is
told not to touch it and the rubrics are checksummed between passes; if the
harness lived in here, an agent would have write access to the criteria it is
scored against, and a checksum would be the only thing in the way.

| you want | do |
|---|---|
| start it | `tmux new -s gatesloop '/mnt/hive-data/gates-loop/gates-loop.sh'` |
| stop it | `touch /mnt/hive-data/gates-loop/STOP` — finishes the pass, then exits |
| stop it sooner | `touch /mnt/hive-data/gates-loop/YIELD` — the builder lands a coherent partial slice at its next boundary, it is judged as usual, then the runner exits |
| what it is doing | `/mnt/hive-data/gates-loop/loop-status.sh` |
| the frames it captured | `/mnt/hive-data/gates-loop/gallery.py`, then `ssh -L 8899:localhost:8899` |
| why a pass failed | `/mnt/hive-data/gates-loop/findings/pass-<id>-{judge,visual}.md` |
| undo a whole run | `git reset --hard gates-anchor-<stamp>` |
| `ci/gates.sh` is red on a clean tree | `GATES_FIX_RED=1 /mnt/hive-data/gates-loop/gates-loop.sh` — one pass, wall only |

Two judges score every pass and neither is the builder — and since harness v2
(operator, 2026-08-02) neither is even *spawned* by the builder: the builder
ends its pass on its branch, gates green and unmerged; the runner spawns the
judge holding `judge/RUBRIC.md` (ten procedural checks — the merge gate) and
performs the merge itself on a PASS, then captures and spawns the visual judge
holding `art/RUBRIC.md` (ten visual criteria against `Rust Images/`). Both
reports end in a `## Ranked gaps` section, and those gaps — not `NOW.md` — are
where the loop's direction is supposed to come from. Read the newest pair
before you steer.

**`git push` is blocked** by a `pre-push` hook the runner installs. Publishing
is an operator act: read the diff, then `git push --no-verify`.

**A gate that waits on a clock is not a gate on this box.** Eight cores here —
the rule was learned on the morr box's four at load 4–5, running a cargo
release build and three Chromium tabs against its own shard, and headroom does
not repeal it. On 2026-08-01 three runs of identical code failed on two
different assertions, and the recovery pass found the cause: the third tab was
racing two live renderers. Assert on observable state (`inWorld`, `snapshots >
n`) and never on elapsed milliseconds — the failure that started it reported
`inWorld=true` and timed out anyway. Widening a timeout is not a fix; it is the
same bug with a longer fuse.

## Third-party credit

- `.claude/skills/threejs-*` — the Three.js graphics skill pack, MIT,
  © 2026 Scott Sun (`THREEJS_GRAPHICS_SKILLS_LICENSE`). `threejs-shadow-systems`
  is the source of the client's light-space texel snapping and texel-scaled
  normal bias (`DECISIONS.md` §open, lighting v0). Guidance only — no code
  from the pack ships in this repo.
- `reference/rust-systems.txt` is ripped from `OxideMod/Oxide.Rust`'s
  `resources/Rust.opj` (MIT, © 2013–2020 Oxide Team and Contributors) —
  facts only: hook names, patched class names, method signatures,
  categories. `CarbonCommunity/Carbon.Hooks.*` (GPL-3.0) is cited as a
  coverage cross-check and deliberately never extracted from, so nothing
  committed here derives from GPL work. `reference/README.md` has the
  provenance and the regeneration command; nothing from either ships.
- `Claude-of-Duty` (github.com/mshumer/Claude-of-Duty, MIT, © 2026 mshumer)
  — source of the shader-prewarm trap, the bit-identical-capture discipline
  (fresh page per shot; engine clock, not `performance.now()`), and the
  coupled-lighting single-owner datum, all from its postmortem. Guidance
  only — no code from it ships in this repo.
