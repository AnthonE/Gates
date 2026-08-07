# CLAUDE.md — Gates

The operating manual for anyone — human or agent loop — working this repo.
Read this first, every iteration. It is deliberately bounded: no dated
state, no counts, no rules for things that don't exist yet. If a claim
here is wrong, fix the claim; history goes in `DECISIONS.md`.

## What this is

A survival game (Rust-the-game tradition): Rust-language authoritative
server, **a native Rust desktop client** (Bevy; operator, 2026-08-05),
WebTransport/QUIC. A separate product that orbits scry — sold through its
Great Work board, coins from its economy — importing none of its code.
**The skeleton is the product**: determinism, netcode, and the hot-path
laws outrank every feature.

**The browser client is deleted** (operator, 2026-08-06: cut, then *"we have
it all backed up on github… we dont need it locally"*). `web/` and its eleven
gates are out of the tree. The native client is the only client.

It is not lost — it is in git history, on GitHub, and readable when a question
about a verb needs it: `git show <commit>:web/src/interact.js`. That matters
more than it sounds, because it WAS the reference implementation of every verb
the native client now carries (the two pick resolvers, `map.js`'s hillshade,
`refusals.js`'s tables), and the ~36 doc comments in `crates/` that cite
`web/src/props.js` for a constant still point at something real. Read it from
history; never restore it to the tree.

**Bevy draws, it does not decide.** `sim-core` keeps the walls and
`ClientCore` keeps prediction, so gameplay state never enters the ECS, where
it would retire the determinism walls with nothing in CI to notice. This is
the one client rule that still binds.

**There is no visual gate, and this is now a deliberate choice rather than a
debt.** The rule inherited from `MIGRATION.md` — a render path may not land
without its probes — is **retired** (operator, 2026-08-06). Two reasons, and
the first is this repo's own measurement: `vantages.mjs` passed all 36 checks
on a beige smear with no sky, no horizon and no object in it (trap list,
below), so the automated visual gate did not work when we had one. The second
is that the operator boots the game and looks, which is strictly better and
costs nothing to maintain. **Do not build a replacement pixel gate.** What may
be gated about a frame is arithmetic — the mesh fits the volume the sim
blocks, in Rust, the shape of `crates/client/tests/tree.rs`.

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
| `BUSINESS.md` | what we sell: IAP, the entry price, and the one thing that stays out (an advantage over another player) | **product, not engineering** — read it when building the store, never otherwise. Nothing in `crates/` reads it |
| `ART.md` | the art bible: measured targets off `Rust Images/`, the hard visual rules, the review checklist | **the visual bar; the art rubric scores against it** |
| `DECISIONS.md` | dated operator calls; **the knob registry** | authoritative on every **(knob)** |
| `MENUS.md` | the interaction surface audit: every screen and verb, ours against the reference, measured off the two Rust mod loaders' hook tables | **owns nothing** — a survey to cut items from, never a queue |
| `RENDER.md` | the **native** client's render path: the Bevy-draws-not-decides boundary, the slice order, the native visual gate, the budgets | owns the path, never the bar — `ART.md` outranks it everywhere |
| `reference/SPAWN.md` | how the reference game places and respawns world objects: four systems, the placement-check chain, the convar layer, and **§9 what it means for us** | **owns nothing** — research, not law. Read it before building placement; `TERRAIN.md` §7/§8 is our answer to it |
| `reference/AUDIO.md` | how the reference game decides what a player hears: the Unity mixer groups/snapshots it built first, the `audio.framebudget 0.3` convar, localized ambience, the 2–5 kHz carve, its four shipped audio bugs, and **§9 what it means for us** | **owns nothing** — research, not law, and a *cleaner* source than `SPAWN.md`: devblogs and the public convar list, nothing decompiled. Our answer is `crates/client/src/sound/` |
| `PLAYERS.md` | the agent player: the verb set, the observation encoder, and the four walls that keep agent play measurable | **DESIGN — none of it built.** The research half is scry's `SUBSTRATE.md`; this owns only what an agent may do here |
| `NOW.md` | what next | **the only list that answers that** |

Docs are dated notes, not law. Four things actually bind: the walls below,
the gates in CI, the operator's spoken decisions, and measurements. A doc
that disagrees with a passing gate is wrong — fix the doc.

## The walls (each with its enforcement — a law without a gate is a mood)

Seven, and every one of them has a gate you can run. **Monetization is not
here.** It used to be wall 8 and it had no enforcement, in a list whose own
header says a law without a gate is a mood — so it was product policy wearing
engineering clothes, and it cost context on every pass that never touched
money. It lives in `BUSINESS.md` now: read that when you are working on the
store, and not otherwise.

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
## Traps already paid for (learned from research or scry production —
do not rediscover)

- **wtransport must be pinned ≥ commit `0f7609a`** (or a release
  containing it) — 0.7.1 has a two-byte remote panic.
- A browser datagram write over `maxDatagramSize` **silently succeeds and
  sends nothing** — clamp every send against the live value. Browser-only as
  a *silent* failure; the native client speaks wtransport directly and the
  MTU ceiling is still real, so the clamp stays on both paths.
- `send_datagram()` (drop-oldest), never `send_datagram_wait()` — a
  congestion stall must cost freshness, not latency.
- **Quantize both sides** or prediction drifts by rounding: the server
  sims on the values it transmits.
- The client is also a hot path, and **half of this trap was JavaScript's,
  half is not.** Retired with the browser: closures in the RAF loop,
  typed-array parsing, and GC pauses — Rust has no collector, so the pause
  class the browser client had to design around does not exist natively.
  Still true on both: **no per-frame allocations** (a native frame can still
  stall on an allocator under a chunk build), and the general shape — a
  client-side hitch feels identical to a server blip to the player, so the
  client is held to the same discipline as the sim thread even though it is
  not the sim.
- Stream-in AND stream-out are budgeted per frame on the client — the
  teardown spike is the half everyone forgets.
- A suite that skips on a missing dep must say SKIP loudly and exit
  nonzero in CI — a pass it didn't earn is the worst bug class.
- Never start a line of a commit body with `Operator, YYYY-MM-DD:` unless
  the same commit updates `DECISIONS.md`.
- **Median fps hides shader-compile stalls.** A static benchmark can read
  90+ fps while lazy WebGL program links cost 700 ms+ worst-frames in real
  play. Prewarm every program at boot; the measure is a COUNT of links after
  the world is up, never a frame-time threshold. **The mechanism survives the
  port and is arguably worse natively**: Bevy specializes a pipeline lazily on
  first use, and a native pipeline compile is a bigger stall than a WebGL
  link. The gate that asserted it (`browser_smoke`) went with the browser and
  **has no native replacement** — so this is a live trap with nothing watching
  it. If a hitch shows up on first look at a new material, this is the first
  suspect. `RENDER.md` §2 carries the design across.
- **A clean merge is not a correct merge, and a destructive read is where
  that bites.** `ClientCore`'s own-fact rings (`pop_hit`, `pop_toast`, the
  refusals) hand each fact over exactly once. On 2026-08-06 two lanes each
  added a reader — the HUD's toast/hitmarker surface and the audio mixer —
  touching no common line, so **git merged them without a conflict and the
  result was silently broken**: the earlier-scheduled system drained every
  ring and the game made no sound for a hit, a gather, a craft or a refusal.
  Nothing failed, because each half is correct alone. The shape that cannot
  regress is **one drain at a fixed point in the frame into a resource readers
  borrow immutably** (`render/feed.rs`) — a second reader is then a `Res<_>`
  parameter, which cannot consume anything. The general rule: when two lanes
  are open, a queue with a single-consumer contract needs an owner named in
  code, not in a comment, and the gate for it is a grep for the call site
  (`tests/sound.rs`), because the defect is a call site and not a value.
- **A judge names the symptom; fix the cause.** Optimizing the judge's
  literal sentence is how a loop circles for three passes — elsewhere,
  "untextured" was really diffuse contrast crushed by an earlier fix for
  "too bright", and the correct change was the opposite of the feedback's
  direction. Diagnose the mechanism before acting on a ranked gap.
- **A pixel statistic cannot see whether the frame is a picture of
  anything, and ours proved it.** On 2026-08-05 `ci/vantages.mjs` passed
  all 36 checks on a beige smear with no sky, no horizon and no object in
  it — scoring the *highest* detail of four vantages (14.28 luma/px). Every
  assertion in that gate is contrast, chroma or luma neutrality, and a
  featureless wash satisfies all three. The same day's captures showed the
  real gap was **content density** — grass, understory, branches, props —
  against which no amount of surface field scores a point.
  **This is why there is no visual gate and why you must not write one**
  (operator, 2026-08-06). The conclusion drawn at the time was "add a
  structural assertion before the statistic"; the conclusion that actually
  held is that the whole approach was spending passes to avoid opening the
  game. A person looking at the frame is the visual gate, it is cheap, and it
  cannot be satisfied by a beige smear. Look at the picture instead of tuning
  the number — not before tuning the number.
- **A byte-golden is blind to what a field means.** Positional payloads
  are where the reference ecosystem actually bled: 49 of Oxide.Rust's
  commits touch a hook's arguments and ~27 correct a payload that had
  already shipped wrong — the right value in the wrong position, four
  hooks corrected more than once. Their patcher pinned an `MSILHash` per
  patched method, the exact analogue of our `test_protocol_golden`, and it
  caught none of them. Ours has the same hole: swap `a` and `b` at an
  `events.push` site and the encoder is untouched (golden green), the
  event queue is not in `state_hash` (replay green), and every field is
  `u32` (clippy green). `reference/FINDINGS.md` §1 has the shape of a gate
  that would catch it; until one exists, the `/// EV_*: a = … b = …` lines
  in `world.rs` are law with no gate.
- **The item-move verb is the most bug-prone thing in the reference, and
  it fails as a kick.** Three Oxide fixes in 28 minutes on one 2019 day —
  the third titled as a fix of the fix — all one-line splice-point moves
  on move/stack/loot, all landing as *the server disconnecting the
  client*, because container state diverged and that reads as a forged
  request. The bug is validation ordering against the mutation, never
  arithmetic. Prediction makes it worse for us: the client has already
  drawn the move, so a container refusal must be computed on the same
  values the client predicted with (the quantize-both-sides law, applied
  to containers).
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
- **A partial slice, landed honestly, is a good iteration** — not a fallback.
  Land the coherent piece with the gates green, say in `NOW.md` what remains
  and what you learned about why, and stop there. The walls above are hard
  because they are gated and objectively checkable; nothing in them asks you
  to finish an item you cannot stand behind.
- **Prose is bounded: a `NOW.md` item ≤ ~25 lines, a commit body ≤ ~20.**
  `NOW.md` is a work queue — the next iteration reads every item before it can
  pick one, so an essay about a single constant is a tax on every pass after
  it. Detail worth keeping goes to `DECISIONS.md` §open (a knob) or a
  `gates-loop/findings/` note (a measurement), and the item points at it in one
  line. Length is not evidence of rigour and no gate or rubric scores it. This
  binds what you write; the items already over it are not a cleanup task, so
  trim one when you edit it anyway and leave the rest.

## Commands (derive, don't quote)

```
cargo test --workspace              # every gate that runs headless
cargo run -p server --bin shard     # the server (reads shard.toml)
cargo run -p server --bin bots -- 100
cargo run -p server --bin replay -- --wal <file>
cargo run -p client --features render --bin gates    # the game
cargo clippy -p client --features render --all-targets -- -D warnings
./ci/gates.sh                       # exactly what CI runs — run it before merge
```

`--features render` is off by default and everything about the client is
behind it (`crates/client/Cargo.toml` says why). It needs `libwayland-dev`
and `libasound2-dev` on a fresh box — Bevy's default features ask for them
through `winit` and `bevy_audio` and this client uses neither, which is a
trim that is owed (`NOW.md` §0x item 4).

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

**A container that has never run the native client is missing seven things,
and every one of them looks like a defect until you read the message.**
Found 2026-08-06 on a fresh clone, each one costing a rebuild. To BUILD
`--features render`: `libwayland-dev` (bevy's defaults include `wayland` as
well as `x11`), `libasound2-dev` (`bevy_audio` → cpal → `alsa-sys`),
`libudev-dev` (`bevy_gilrs`) — each dies as a `pkg-config` panic 40 lines into
a build script. To RUN a `--capture` probe: `libxkbcommon-x11-0` (winit
panics in `EventLoop::new`), `mesa-vulkan-drivers` + `libvulkan1`, and
`Xvfb`. Plus `rustup target add wasm32-unknown-unknown` for the wasm gates.
All of them are the same class — a wall that cannot run is not a wall, so
install them rather than trimming the feature.

Four things that cost time and are not obvious:

- `apt-get install` may 404 on a stale index; `apt-get update` first.
- **`cargo … | tail` reports `tail`'s exit code, not cargo's.** A backgrounded
  gate piped to `tail` reports success while the build is red — `${PIPESTATUS[0]}`.
- The lavapipe ICD is `/usr/share/vulkan/icd.d/**`lvp_icd.json`**, not
  `lvp_icd.x86_64.json`. Point `VK_DRIVER_FILES` at a path that does not exist
  and the loader says only *"Unable to find a GPU"* — the filename is in
  `VK_LOADER_DEBUG=error,warn` and nowhere else. Working invocation:
  `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json DISPLAY=:N
  WGPU_BACKEND=vulkan target/release/gates --server … --capture <dir>`.
- **With no sound card, `bevy_audio` logs "No audio device found" and every
  voice is a silent no-op.** That is the correct behaviour and the capture
  probe confirms it: the client must not require an audio device to render.

**Not every red gate is a defect — but check whether the missing capability is
one WE asked for.** `bot_smoke` used to fail all four tests on a container
without IPv6 with `Address family not supported by protocol (os error 97)`, on
a clean tree, and this entry said to believe the box rather than the diff. The
first half was right and the conclusion was one step short: the bind was
**our** code asking for a dual-stack `[::]:0` socket to reach an IPv4 shard.
`bot_endpoint` and `client_endpoint` now bind `InAddrAnyV4` first and keep the
dual-stack bind as the fallback for a v6 shard, reporting both failures if
neither binds — and the gate runs. **Keep the diagnostic habit** —
`git stash -u` before believing a diff caused a red gate — and add the second
question: *is the capability genuinely absent, or are we requesting one we do
not need?* A wall that cannot run is not a wall, and that cuts both ways.
Still environmental on this box, and not the same thing: the `*_wire` server
suites overflow a test thread's 2 MiB stack in a debug build — `RUST_MIN_STACK`
(e.g. `16777216`) runs them green and no tree change is owed. Same class as: `wasm32-unknown-unknown` is not always installed, and the wasm gates
fail with `can't find crate for core` until `rustup target add
wasm32-unknown-unknown` — install it rather than skipping the gate, because a
wall that cannot run is not a wall. (The third of this kind was Playwright's
Chromium not matching the pinned build, fixed with a `VANTAGE_CHROME`
override — **gone with the browser gates**; no gate in this repo starts a
browser now, and none should.) Third, and the one a fresh box
hits first: **`cargo … --features render` needs three `-dev` packages** that a
headless image has no reason to carry — `libwayland-dev`, `libasound2-dev`,
`libudev-dev` — and each fails identically, as a `pkg-config exited with status
code 1` panic from a `*-sys` build script (`wayland-sys`, `alsa-sys`,
`libudev-sys`) with the crate named only in the backtrace. Install them; it is
the box, not the tree. `ci/gates.sh`'s native-client gate names the first two
in its own echo line; **`libudev-dev` is the third and was measured here**, so
a box that installs only what that line lists still fails. Running the client
adds a fourth, and it is a runtime `.so` rather than a build dep, so
`pkg-config` never mentions it: `libxkbcommon-x11-0`, whose absence panics
inside `winit` at `App::run` with every gate already green. **Then ask the
second question for each**, because one of the four does not survive it:
`alsa` is required solely because `bevy_audio` is on by default and this
client has **zero** audio call sites. That is not a missing capability, it is
one we request and do not use, and the trim is `NOW.md` §0x item 1. `wayland`
(and `x11`) are real — a shipped desktop client faces both — and `libudev` is
`bevy_gilrs`, which is gamepads and wanted.
Neither of these repeals the rule above:
they are missing capabilities, which are diagnosable and permanent, not timing,
which is neither.

## Vendored, and not to be edited here

- `crates/client/src/scry_overlay.rs` is **scry's SDK, byte-for-byte**
  (`sdk/rust/scry_overlay.rs` in `AnthonE/scry`). It is how this game reaches
  a running scry launcher for identity and signatures with no key in the game
  process and no crate added to the tree. `scry::VENDORED_SHA256` pins it and
  a test fails on any local edit — **fix it upstream and re-vendor**, because
  a patch applied here fixes Gates and leaves every other game on the broken
  copy. `crates/client/src/scry.rs` is our wrapper and is ours to change.
  Not third-party: same author, same licence, no notice owed.
- The depot the launcher installs is written by `ci/depot.py`, gated by
  `--self-test` in `ci/gates.sh`. It deliberately does **not** compute the
  depot digest — `scry digest` does, and a second implementation of the number
  that gets notarized is scry's invariant 3 with money attached.

## Third-party credit

- `.claude/skills/threejs-*` — the Three.js graphics skill pack, MIT,
  © 2026 Scott Sun (`THREEJS_GRAPHICS_SKILLS_LICENSE`). `threejs-shadow-systems`
  is the source of the client's light-space texel snapping and texel-scaled
  normal bias (`DECISIONS.md` §open, lighting v0). Guidance only — no code
  from the pack ships in this repo. **That is a licence statement, not a
  usage limit: read them.** `threejs-skill-router` routes a graphics task to
  the right ones. **They are now guidance about *technique*, not about this
  codebase**: the renderer they address is three.js and the browser client is
  cut, so a skill's API is no longer ours. What survives the change is what
  was always the valuable half — the shadow pack's texel snapping, the
  exposure pack's log-average metering, the atmosphere pack's LUT structure —
  and every one of those has to be re-expressed against Bevy, exactly as
  `SeedThree`'s TSL wind had to be re-expressed as GLSL. Renderer work starts
  at `RENDER.md`; reach for these for the physics, never for the calls.
- `reference/rust-systems.txt` is ripped from `OxideMod/Oxide.Rust`'s
  `resources/Rust.opj` (MIT, © 2013–2020 Oxide Team and Contributors) —
  facts only: hook names, patched class names, method signatures,
  categories. `CarbonCommunity/Carbon.Hooks.*` (GPL-3.0) is cited as a
  coverage cross-check and deliberately never extracted from, so nothing
  committed here derives from GPL work. `reference/README.md` has the
  provenance and the regeneration command; nothing from either ships.
- `bevy_procedural_tree` (github.com/Affinator/bevy_procedural_tree, MIT OR
  Apache-2.0) — the native client's conifer generator, a **dependency** whose
  code ships. It is `@dgreenheck/ez-tree`'s algorithm (MIT, © Daniel Greenheck)
  ported to Rust and Bevy. The browser client ran the same generator before it
  was deleted, so `props.js`'s swept parameters — in git history — are still
  evidence about this one.
  Only `meshgen::generate_tree_meshes` is called — settings and an `Rng` in,
  two meshes out; the crate's plugin is deliberately unused, because a plugin
  that regenerates entities on change would put tree state in the ECS.
- `SeedThree` (github.com/SkyeShark/SeedThree, MIT) — source of the wind
  design the client ships: one per-vertex `aWind` cantilever weight rooted at
  the trunk base, phase taken from the instance's world position so a gust
  crosses the forest instead of each tree twitching alone, and two sine
  octaves rather than one. Its `wind.js` is TSL/WebGPU node-material source; the
  deleted browser client re-expressed the design as a GLSL injection, and the
  native client has to re-express it again against Bevy — guidance only, no
  code from it ships. Its `impostor.js` (two
  crossed alpha cards baked from front/side ortho cameras in a worker, with the
  backend readback row order probed once against a known image) is the
  reference for `TERRAIN.md` §4's unbuilt billboard LOD; its emit side returns
  a `Group` per tree and would have to become an `InstancedMesh` pool here.
- `Claude-of-Duty` (github.com/mshumer/Claude-of-Duty, MIT, © 2026 mshumer)
  — source of the shader-prewarm trap, the bit-identical-capture discipline
  (fresh page per shot; engine clock, not `performance.now()`), and the
  coupled-lighting single-owner datum, all from its postmortem. Guidance
  only — no code from it ships in this repo.
- `SkyeShark/Eanpa-Sky` and `SkyeShark/SeedThree` (both MIT, © 2026 SkyeShark)
  — read while surveying the node stack. (They were catalogued in
  `MIGRATION.md` §8.3, deleted with the browser client; the licence facts that
  doc carried are restated here in full, because a deleted file cannot hold an
  obligation.) Nothing from either ships **yet** — and unlike the skill packs
  above, MIT means it may. Two routes are open and both require carrying the
  notice **at the donor site**, not a bullet here alone: SeedThree as vendored
  `.glb` output, and Eanpa-Sky's cloud noise, presets and density function
  copied into the renderer. Credit again, in the file, at that point.
  **Neither repo's audio may be touched**: Eanpa-Sky names four xeno-canto
  recordings as CC BY-NC-SA, and SeedThree ships bird recordings whose
  `README.txt` cites xeno-canto but **states no licence per file** —
  unresolved is not the same as permissive, and NC does not survive a sold
  product either way. This matters more now, not less: `sound/synth.rs`
  generates our bank at boot precisely to avoid this class of question.
