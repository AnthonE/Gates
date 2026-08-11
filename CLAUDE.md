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

⚠ **Separate the two kinds of dead citation, because only one is harmless.**
"The browser held this constant" is history and stays. "`ci/<gate>.mjs`
refuses a drift between them" is a claim that **something is enforced**, and
eleven deleted gates make many of those false in the present tense — the doc
reads as covered while nothing checks it. Swept 2026-08-09 in the `.md`
files and **2026-08-11 in `crates/`**, which had been the outstanding half:
15 doc comments claimed a deleted gate in the present tense ("still scores",
"holds the two equal", "refuses a drift") and now name it in the past with the
consequence stated, which is usually *nothing enforces this now*. Three gates
cited from `crates/` are still live — `knob_registry.mjs`, `parity.mjs`,
`haven_prize.mjs` — and eleven are gone, so the check when you add a citation
is `ls` the file, not memory. The first mirror anybody actually re-checked had
drifted (`TERRAIN.md` §7.1).

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
| `ART.md` | the art bible: measured targets off the reference set, the hard visual rules, the review checklist | **the visual bar; the art rubric scores against it** |
| `DECISIONS.md` | dated operator calls; **the knob registry** | authoritative on every **(knob)** |
| `MENUS.md` | the interaction surface audit: every screen and verb, ours against the reference, measured off the two Rust mod loaders' hook tables | **owns nothing** — a survey to cut items from, never a queue |
| `RENDER.md` | the **native** client's render path: the Bevy-draws-not-decides boundary, the slice order, the native visual gate, the budgets | owns the path, never the bar — `ART.md` outranks it everywhere |
| `reference/SPAWN.md` | how the reference game places and respawns world objects: four systems, the placement-check chain, the convar layer, and **§9 what it means for us** | **owns nothing** — research, not law. Read it before building placement; `TERRAIN.md` §7/§8 is our answer to it |
| `reference/AUDIO.md` | how the reference game decides what a player hears: the Unity mixer groups/snapshots it built first, the `audio.framebudget 0.3` convar, localized ambience, the 2–5 kHz carve, its four shipped audio bugs, and **§9 what it means for us** | **owns nothing** — research, not law, and a *cleaner* source than `SPAWN.md`: devblogs and the public convar list, nothing decompiled. Our answer is `crates/client/src/sound/` |
| `reference/BUILDING.md` | how the reference game decides **who may build here** *and* **what a shape costs**: the cupboard's authorized list (one pattern it reuses three times), privilege as a *volume emitted by the blocks* rather than a sphere around the cupboard, upkeep as a cost and the activity-keyed decay bug that caused it, the decay ladder, the demolish/rotate grace window, and — **§7b, 2026-08-10** — the 20 × 5 catalogue, twig as the editable draft, the socket separation between an opening and its insert, and the **four ratios** a hundred prices reduce to, with the half-wall's full price as the arbitrage they refuse on purpose. **§9 what it means for us** | **owns nothing** — research, not law, `DOORS.md`'s posture and its proxy caveat too, and §7b is tier 3 transcribed rather than fetched (§8 says so). Written because `DOORS.md` §9 kept pointing at it; §7b because the operator asked for the cost grammar next. **§9's last five items are the live half** — the first ten landed 2026-08-08/09 |
| `reference/DOORS.md` | how the reference game decides **who is allowed through a door**: the lock is a separate entity and a door with no lock is anyone's, the code lock's remembered list and its guest tier, the two goes it took them to rate-limit a keypad, knocking as a verb, and **§9 what it means for us** | **owns nothing** — research, not law, and `AUDIO.md`'s source posture with one caveat §0 states in full: every page fetch was blocked by this box's proxy, so the devblog and wiki tiers arrived as *search summaries*. The operator adopted it (`DECISIONS.md` 2026-08-08), so §9 is built: our answer is `crates/sim-core/lock.rs` |
| `reference/SAVES.md` | how the reference game remembers a player: **there is no player save file** — the body stays in the world as a sleeper and is saved because it is an entity, the save and the wire on one base class, the stop-the-world stall it never fixed, the wipe split, and **§9 what it means for us** | **owns nothing** — research, not law, and `AUDIO.md`'s clean source posture. The operator adopted its model (`DECISIONS.md` 2026-08-07), so §9 is a plan: read it before touching persistence. Our answer is `crates/server/src/store.rs` + `sim-core/src/persist.rs` |
| `reference/BALANCE.md` | which of our numbers are the reference game's and which are ours: what matched already, what moved on 2026-08-08, and **§4 what deliberately did not move** | **owns one thing, unlike the other `reference/*.md`** — §6's standing instruction, **rewritten 2026-08-10 and now a default rather than a permission**: take theirs, no case needed; a case is needed only to *differ*, and the only admissible one is a **mechanism** difference. Effort, a band, and source uncertainty are explicitly named as costs wearing principle's clothes (§6.2), a split source is broken by §6.3's ladder rather than deferred, and averaging stays forbidden. The bands in `CONTENT.md` §4 still decide whether it may land |
| `reference/RIPLIST.md` | the **queue** for ripping the reference's numbers: what is taken, what is outstanding, what is blocked on research nobody has done, what has no equivalent to take, and the six steps for executing one row | **owns nothing** — a worklist, and `BALANCE.md` §6 plus `CONTENT.md` §4's bands still decide. Read it before touching a balance number. Its §0 carries the **threat frame** (operator, 2026-08-09): their numbers are priced for contested farming and ours are not, so taking a yield without the interruption that balanced it is §4.1's false-familiarity trap one level out |
| `reference/NETWORK.md` | how the reference game does **netcode**, across twelve years of shipping it wrong in public: two networking libraries that each became the bug (a Lidgren stall first investigated as a DDoS, a RakNet fragment-reorder that needed entity checksums to even localize), the freeze that turned out to be serialization garbage rather than transport, the per-frame *time* budget for network processing, interest management sharpened from square grids to circles with per-entity radii to server-side occlusion, and the parallel-jobs arc that ended on one memory-pool lock | **owns nothing** — research, not law, `AUDIO.md`'s clean source posture with `DOORS.md`'s proxy caveat in full (every `rust.facepunch.com` fetch was blocked; tier 1 arrived as search summaries). **Its §9 is the exception to "owns nothing" in one respect**: unlike the other `reference/*.md`, §9 was written *against the tree* with a `file:line` on every claim, and §9.3 is the doc/code delta — including that **all seven of `NETCODE.md` §11's "Added CI gates" are unbuilt** |
| `reference/WATER.md` | how the reference game does water: the order it rebuilt the sea in (**surface → optics → motion → reflections → foam**, waves third), depth-based colour extinction and thickness-based visibility as one idea, shoreline wetness worked from the *land* side, what its own settings screen says water costs, and **§9 what it means for us** | **owns nothing** — research, not law, `AUDIO.md`'s clean source posture (devblogs and a settings screen, nothing decompiled). Our answer is `crates/client/src/render/water.rs` + `crates/client/src/sound/water.rs` |
| `reference/PROJECTILES.md` | how the reference game does **bows and arrows**: there is no `Bow` class (a bow is a one-round-magazine `BaseProjectile`, the same class as every gun), the projectile is **client-simulated** and audited by a thirteen-convar tolerance budget rather than a predicate, ballistics live on the **ammo** (`ItemModProjectile`) so one bow fires four different arrows, the arrow is an item three times over (~15 % break, 10 s lodge), hit detection takes the **most significant** body part not the first intersection, and **§9 what it means for us** | **owns nothing** — research, not law. Read it before touching `crates/sim-core/src/ranged.rs`. §9.1 is why our server-simulated arrow stays server-simulated; §9.3 is the one schema change it argues for (`[weapon.ballistic]` belongs on the ammo row), and it gets harder every arrow we add first |
| `reference/MONUMENTS.md` | how the reference game decides **where a large authored place goes**: placement as a solve rather than a guess (three rewrites in ten years, still moving), the collision list every worldgen system after monuments produced — rivers, cliffs, ice lakes, roads, ring roads, rails — terrain blending as authored per-monument masks rather than a flattened circle, the 2015 client-worldgen checksum mismatch they had to stop kicking for, vertical AOI layers, per-class interest ranges, what one moving monument actually costs, and **§9 what it means for us** | **owns nothing** — research, not law, and **the weakest provenance in this directory: §0 says so in full.** It is a summary of sources nobody here has opened (an operator briefing, 2026-08-10), so no number in it may reach `content/`. Its value is the ORDER, which is checkable against our tree. §9.2 is built (`SiteFootprint` / `site_sweep`); §9.3 is the real gap — our solver is two hand-written tiers |
| `reference/ANIMALS.md` | how survival games do animal mobs: the reference game's baked navmesh (100% CPU at boot) and the fixed think rate and dormancy it settled on, Valheim's ring spawners and two caps, Minecraft's mob cap and 1-in-800 despawn roll, and **§9 what it means for us** | **owns nothing** — research, not law, and `AUDIO.md`'s clean-source posture (devblogs, convar lists, wikis; nothing decompiled). The operator un-cut animals off it (`DECISIONS.md` 2026-08-08), so §9 is a landed design: read it before touching `crates/sim-core/src/mob.rs` |
| `WORLD.md` | the proposed register: **Gates as a threshold dimension** (the name is the fiction — it explains respawn, the wipe cadence and why banked OBOL leaves and carried OBOL does not), an ancient obsidian/lapis/gold civilization, the coast→interior gradient, the monument catalogue, **extraction** as a server-opened window at the haven's bank terminal, world states whose **default is broken** so a wipe cycle is a repair project, and an optional **ward** | **DESIGN — unspoken, none of it built, and a roadmap rather than a v1 spec** (operator, 2026-08-10: *"paths over time"*, and *"i think this is the max deviation"* — it is a ceiling, not a floor). Owns the fiction and nothing in `crates/`. Its §8 is the useful half: five collisions with live gates, two of them real — the visual rubric scores an obsidian world as a defect by construction, and a ward would invalidate `CONTENT.md` §4's TTK anchor *without reddening `test_content`*. §9.1 is the one piece of timing advice: **decide the register early, build it late**, because art made for the wrong register is remade |
| `reference/PLANTS.md` | how games grow a forest: the five-layer forest structure and which two of ours are empty, space colonization vs L-systems (and that we already ship the solver), Deussen's ecosystem sim as the reference for *placement*, octahedral impostors, and **§6 what it means for us** | **owns nothing** — research, not law, `AUDIO.md`'s clean source posture (a SIGGRAPH paper, four MIT repos, and our own dependency's source; `docs.rs` was proxy-blocked so the crate API came off `raw.githubusercontent.com`). Written because "are our trees good" has an arithmetic answer: **one species at three seeds, on a uniform 8 m scatter lattice that `ART.md` rule 7 forbids**. Read it before buying any foliage — §4 is why a mesh generator is the wrong tool — and before touching `render/tree.rs` or `terrain::scatter` |
| `assets/models/WANTED.md` | the 3D object inventory: 63 meshes and 6 texture sets with sizes read off the code, the glTF/origin/ORM pipeline rules, and what is already covered | **owns nothing** — a sourcing worklist, `RIPLIST.md`'s shape. `MANIFEST.md` records what ships; this records what does not exist yet |
| `PLAYERS.md` | the agent player: the verb set, the observation encoder, and the four walls that keep agent play measurable | **DESIGN — none of it built.** The research half is scry's `SUBSTRATE.md`; this owns only what an agent may do here |
| `marketing/` | what a stranger reads about **OBOL and MYRRH** somewhere that is not this repo — an explorer's token-info field, a DEX listing, a wallet's coin row — plus the four marks | **owns nothing in `crates/`**, and it is here because the coins are ours: scry has exactly one coin and it is SCRY (operator, 2026-08-07), so its repo keeps only our listing row. ⚠ **Every number in it is derived in `scry-forge`**, where the contracts and pool seeds live — re-derive there, paste here |
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

⚠ **Two enforcements named below were aspirational and are marked so
(re-checked 2026-08-09): there is no soak, and `test_raid_storm` does not
exist.** Both walls still hold on their other half — clippy for 3, per-site
cap tests for 4 — so neither is ungated, but the list overstated itself and
that is the exact failure the header warns about. Grep before citing a gate;
`DESIGN.md` §12 marks the same two.

1. **sim-core is pure.** No I/O, no clock, no threads, no `HashMap`/
   `HashSet` iteration, no libm/trig, floats restricted to
   `+ − × ÷ sqrt min max clamp floor-by-cast`. → clippy disallowed
   types/methods + `test_parity_wasm` (native and wasm bit-identical).
2. **Zero allocation in the tick after warmup.** → counting allocator,
   `test_alloc_zero`.
3. **No locks, no syscalls, no `String`/`format!`/logging in the sim
   thread.** Rings only; integer event codes only. → clippy walls
   (`sim-core/clippy.toml` disallows the lock, clock, I/O and `String` types
   by name, and `ci/gates.sh` runs clippy `-D warnings`). ⚠ **The soak
   tick-jitter assert this line also named does not exist** — there is no
   soak anywhere in the repo.
4. **Bounded everything.** Every queue, map, and per-tick work item has a
   cap in `limits.rs` and a stated overflow policy. No `push` on a
   client-driven path without a cap check. → the review wall, plus
   per-site cap tests across ~40 suites (`the_queue_is_bounded_and_says_so`,
   `event_ring_overflow_heals_by_resync`, `the_bag_cap_stays_neutral`,
   `the_autosave_sweep_is_bounded_and_skips_the_unchanged`,
   `the_voice_cap_refuses_rather_than_steals`). ⚠ **`test_raid_storm` does
   not exist** — the caps are gated one site at a time and nothing drives
   them all at once.
5. **Determinism is a gate, not a vibe.** Same build + seed + WAL →
   same state hashes. → `test_replay`, `test_terrain_golden`.
6. **The wire never drifts by accident.** Packet layouts change only with
   a version bump + regenerated goldens in the same commit. →
   `test_protocol_golden`. ⚠ **`PROTO_VER` is one of three version numbers
   and only this wall's** — `crates/protocol/src/version.rs` carries the
   table, and conflating any two of them is what that file exists to prevent.
   The release (`VER`, from `[workspace.package] version`, shown in the
   client's corner and floored per-shard by `min_client`) and the commit
   (`GIT_SHA`) are not this wall's business and do not bump goldens.
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
- **A type shared across the feature line has its exhaustive matches on
  only one side of it.** `ui::interact::Verb` compiles in both builds;
  every `match` that must cover it lives in `render/verbs.rs`, which
  `--features render` gates. So adding a variant is **green on
  `cargo test --workspace`, twice, and red at the Bevy gate** — the one
  that costs five minutes to reach. Found 2026-08-10 adding
  `Verb::Recycler`: the sim, the wire, the content and every non-render
  client test passed while `E` and `C` had no arm for the thing they were
  pointed at. Same shape for any `pub enum` or archetype table read from
  `render::`. When you add a variant, grep the feature-gated modules for
  its type before believing a green workspace run.
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
  `u32` (clippy green). `reference/FINDINGS.md` §1 had the shape of the
  gate that catches it, and it exists now: `crates/sim-core/tests/
  event_roles.rs` role-checks every event's payload against its
  `/// EV_*: a = … b = …` doc line — 32 of 32 as of 2026-08-09, each
  proven red under its own a/b swap. The trap stays listed because the
  mechanism (three green gates over a wrong payload) is what to remember
  when the NEXT event lands: an event without its role gate re-opens it.
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
- **A full-world save is a stop-the-world freeze, and the reference game has
  not fixed it in thirteen years.** Every server-host guide says the same two
  things: saving is heavy, and on a high-population shard it "causes a massive
  lag spike that freezes everyone in place". The only mitigation offered
  anywhere by anyone is `server.saveinterval` (default 600 s) — a knob whose
  two ends are *how much a crash costs* and *how often everyone stalls*, with
  no third option. What they did optimise is the other end, load: Devblog 96
  put building stability into the save so a restart could skip recomputing it.
  So the trap is not "saving is slow", it is that **a whole-world walk on the
  sim thread has no good cadence**, and picking one is choosing which player to
  disappoint. Ours is a bounded sweep instead — one player per tick,
  skip-if-unchanged, on a thread that is not the sim (`store.rs`) — and the
  world stores want the same treatment when they land, not a snapshot. Their
  loader also trusts the file outright; ours cannot, because a save is the one
  non-command path into `World`. `reference/SAVES.md` §4 and §9.3 have the
  measurements and the sources.
- **A big fixed array constructed on the stack can blow wasm's shadow
  stack, and the native build will not tell you.** `Box::new(Store::new())`
  materialises the whole struct in a frame before moving it to the heap.
  `World::new` already builds ~100 player records, a 1 024-piece store and a
  1 024-deploy store that way; on 2026-08-08 a 52 KB lock store on top of it
  turned `test_parity_wasm` into `RuntimeError: memory access out of bounds`
  inside `Deploys::new`, with every native test green — **wall 1's own gate
  failing for a reason with nothing to do with determinism**. wasm32's
  shadow stack is 1 MiB and there is no guard page, so the symptom is an
  out-of-bounds read and not a stack-overflow message. The fix is to fill on
  the heap (`vec![..; N].into_boxed_slice().try_into()`), which allocates
  where the old code allocated anyway. The stores that predate this still
  use the stack form; they fit today, and the next one to be added will not.
  **Seen three times in one day** — the lock store found it, the hearth crew
  hit it again, and the third surfaced *inside dlmalloc* rather than in the
  constructor, because the frame that tipped it over was the allocator's own.
  `crate::boxed_array` is the fix and the arrays in `Pieces` and `Deploys`
  all use it now; the piece store alone was 98 KB in a frame.
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
cargo run --release -p server --bin profile          # where a tick's 33 ms goes
cargo run -p client --features render --bin gates    # the game
cargo clippy -p client --features render --all-targets -- -D warnings
./ci/gates.sh                       # exactly what CI runs — run it before merge
```

**Cutting a release is a tag, and it is an operator act.** Bump
`[workspace.package] version` in the root `Cargo.toml` — the one place a
version is typed, inherited by all six crates — then
`git tag -a v<x.y.z> -m "…" && git push --no-verify origin v<x.y.z>`.
`.github/workflows/release.yml` re-runs the gates on the tagged commit,
refuses a tag that disagrees with the tree, builds Linux/Windows/macOS, and
leaves a **draft** release for a person to read and publish. A shard's
`min_client` floor is raised *after* that release is published, never before.

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
holding `art/RUBRIC.md` (ten visual criteria against the reference set). Both
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
`Xvfb`. Plus `rustup target add wasm32-unknown-unknown` for the parity gate
— **which is not a web build**: `sim-core` compiles to a second target so
`test_parity_wasm` can diff its state hashes against native byte for byte,
which is wall 1's enforcement and is worth the same with no browser in
existence. The crate that WAS a web build is gone (operator, 2026-08-08:
*"we use desktop build no more web"*) — `client-wasm` is `client-core`, its
1,635-line C-ABI bridge and the 1,266-line `ci/client_smoke.mjs` that drove
it are deleted, and what that gate actually asserted is
`crates/client-core/tests/wire.rs`.
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
the box, not the tree. `ci/gates.sh`'s native-client gate now names **all
three** in its own echo line — it listed only the first two when
`libudev-dev` was measured here, and following that line was itself a failure;
it was fixed, so the echo is trustworthy again. Re-confirmed 2026-08-09 on a
fresh container: the gate died at `wayland-sys` and installing the three the
line lists ran it green. Running the client
adds a fourth, and it is a runtime `.so` rather than a build dep, so
`pkg-config` never mentions it: `libxkbcommon-x11-0`, whose absence panics
inside `winit` at `App::run` with every gate already green. **Then ask the
second question for each** — and the answer moved under this entry once
already, which is its own lesson: this paragraph used to say `alsa` was
requested and unused, and since audio v0 (2026-08-06) **`bevy_audio` is
load-bearing** — `render/audio.rs` plays the generated bank through it — so
`alsa` earns its keep. `wayland` (and `x11`) are real — a shipped desktop
client faces both. `libudev` is `bevy_gilrs`, which grep still shows
**unused** (no gamepad code) — that and `vorbis` are the honest trim
targets, while `bevy_gltf`/`bevy_animation` joined the load-bearing set
with the mannequin (2026-08-07). `NOW.md` §0x's trim item tracks what a
grep actually shows rather than what this paragraph last said — re-verify
there before trimming anything.
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
  ⚠ **The pin catches a local edit and is blind to upstream moving**, and the
  two are not the same failure. Found 2026-08-09: the copy sat 326 lines
  behind the source with every gate in both repos green — no Windows
  transport (it `use`d `std::os::unix::net` unconditionally, so a Windows
  build of this client could not compile), no `prove`, no `profile`. Nothing
  gates a file in another repo, so the check is a command you run when you
  touch this seam: `sha256sum crates/client/src/scry_overlay.rs` must appear
  in `sdk/SHA256SUMS` upstream. Re-vendoring is `cp` + re-pin + `cargo test
  -p client --lib scry`, and check the CALL SITES, not just the compile —
  `Overlay::title` changed shape under us and only luck kept it uncalled.
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
- **game-icons.net** (CC BY 3.0) — the client's item and building icons,
  **shipped**: **64 of the 65** white-on-transparent PNGs in `assets/icons/`,
  rasterised from the project's public SVG archive by `ci/bake_icons.py`.
  (This line read "62" until 2026-08-11 and the tree said 65 — a count in a
  doc drifts every time an icon lands, which is why `CREDITS.md` and
  `STEMS` are the things gated and this is only a pointer.) The 65th,
  `burnt_meat.png`, is **ours** and the CC BY notice does not cover it;
  `CREDITS.md` keeps the two in separate tables so the line is written down
  rather than inferred. CC BY is a *notice* licence — `assets/icons/
  CREDITS.md` names the four authors whose work ships (lorc, delapouite,
  carl-olsen, john-redman) and `tests/ui.rs` §G fails if it stops travelling
  or if the baked set and `ui::icons::STEMS` drift apart. **Nothing is traced from the reference game**: an icon set
  anyone may redistribute with credit is not the IP rail's business, and the
  rail is what forbids copying Rust's own art.
- **Roboto Condensed** (Apache-2.0, © 2011 Google Inc.) — the client's UI
  face, **shipped**: `crates/client/fonts/RobotoCondensed-{Regular,Bold}.ttf`
  are compiled into the binary by `include_bytes!` and
  `fonts/LICENSE-ROBOTO.txt` is the notice that has to travel with them
  (`tests/ui.rs` §F fails if it stops travelling). Not a taste call — the
  reference game names `RobotoCondensed-Bold.ttf` as its own UI default in
  public source (`Facepunch/Rust.Community`, `CommunityEntity.UI.cs`), which
  is a *fact about the target*, in the sense `ART.md` means by measured.
  Nothing was traced and no proper noun ships; a typeface anyone may
  redistribute is not the IP rail's business.
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
