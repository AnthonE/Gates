# reference/MONUMENTS.md — how the reference game places monuments

Ripped lessons, not design. `SPAWN.md` answers *how the reference places and
respawns world objects*; this file answers the question one level up — **how it
decides where a large authored place goes, and what every other world system
had to give up so it could go there.** Written because we are about to build
our third and fourth destinations and the island currently has two, both placed
by hand-written code that does not generalise.

Dated 2026-08-10. §9 is the part that changes what we build, and one slice of
it is already built (§9.2).

## 0 · Provenance — read this first, it is weaker than the other files here

**Source: a research briefing from the operator, 2026-08-10, summarising the
reference game's public devblogs and commit log across roughly 2015–2026.**
That is a *second-hand* tier and it is the weakest provenance any file in this
directory carries. `AUDIO.md` and `WATER.md` are devblogs and a settings
screen read directly; `SPAWN.md` is decompiled structure; `DOORS.md` and
`NETWORK.md` are search summaries because this box's proxy blocked every
`rust.facepunch.com` fetch. This one is a summary of sources **nobody in this
repo has opened**, and the same proxy still blocks the tier-1 pages, so no
line below was re-verified from here.

Three consequences, and they bind how you may use this file:

- **Every number in §1–§8 is quoted, never measured.** "Excavator +45 %" is a
  figure from the briefing about a test map size we do not have. Do not carry
  one into `content/` — `reference/BALANCE.md` §6 and `CONTENT.md` §4's bands
  govern that road and neither accepts a quoted figure as a source.
- **Several cited dates are recent enough that we cannot check them** (an
  April 2026 interest-management change, a June 2026 navmesh replacement).
  They are carried as given. If one turns out to be wrong, the *mechanism* is
  still the finding — every one of them is a shape we can reason about from
  our own tree.
- **Nothing here is a class name read off a binary, and nothing ships.** No
  asset, no name, no layout, no number.

**The strength of this file is not its facts, it is its ORDER.** What it
records is the sequence in which one team hit each problem over eleven years,
and every entry is a system that was built before the thing that broke it. That
sequence is checkable against our own tree even when the dates are not, and it
is the only part of this file §9 leans on.

## 1 · Placement stopped being a test and became a solve

The first design was the obvious one: guess a position, ask the terrain whether
it works, accept or reject, repeat. It failed in the way that shape always
fails — valid locations were rejected, and whole seeds shipped without their
large monuments even though somewhere on the map would have held one.

The 2015 fix was not a better guess. They changed the terrain-anchoring system
to **search for the best placement altitude** rather than test a proposed one:
the site stopped being an input to a predicate and became the output of a
search.

That was not the end of it, which is the actual lesson:

- **Late 2017**: monument placement reworked wholesale to optimise the
  *overall distribution* — to fit as many useful locations onto one map as
  possible — because seeds were shipping with large empty regions.
- **June 2025**: monument distribution described as a long-standing issue,
  made worse by adding a new biome. The fix raised major-monument occurrence
  substantially on their test map size (quoted: Excavator +45 %, Airfield
  +37 %, Trainyard +34 %).

Ten years, three rewrites, still moving. The failure mode has one name every
time: **a sequential placer starves whatever it places last.** Each accepted
site removes area from the pool, and nothing in `pick → test → place → repeat`
knows that the site it just took was the only one the next monument could have
used.

The shape that does not have that failure:

```
generate candidates → score them → solve the layout → reserve footprints
                                                    → then build the world around it
```

## 2 · Every new worldgen system fought every existing monument

This is the densest part of the briefing and it reads as a list of collisions,
each one a system that shipped after monuments and had to be taught about them:

- **2016**: rivers intersecting dungeon/sewer systems. Cliff-versus-monument
  placement optimised *because it was hurting load time* — the collision
  question itself became a performance problem.
- **2017**: ice lakes blocked so many valid locations that the Arctic biome
  could come out with almost no monuments in it.
- **Roads**: monuments changed to **expose road connection points**, so
  procedural roads attach to a monument's authored roads instead of passing
  near them.
- **Ring roads**: broke placement badly enough to force three changes at once —
  place roadside monuments *after* large monuments, remove the ring road below
  a 4k map size, and enlarge the default world 4000 → 4250 **purely to buy
  monument placement room**.
- **Rails**: monuments spawning on rails; rails too close to monument
  connections; tunnel entrances colliding with monuments; cliffs overlapping
  entrances; ice-lake terrain anchors overlapping small monuments; and a
  client/server terrain mismatch where rails connected to monuments.

Read the list twice. Every entry is the same bug: **a new system generated
geometry and then asked "did I hit anything?"** The answer that ends the class
is to invert it — a monument publishes what it needs before anything else is
generated, and the later system consumes that rather than discovering it.

Concretely, the briefing's proposal, which is the reference's own end state
written out: a monument exposes **road ports, rail/transport ports, cave
entrances, shoreline/dock ports, a terrain footprint, a no-spawn footprint, a
vegetation-exclusion footprint, navigation entrances, an underground footprint,
and a reserved expansion volume.** Procgen connects the ports; it never
discovers the monument.

## 3 · Terrain blending was not flattening a patch

Early monuments looked like a building dropped onto a circular plateau,
because that is what they were: crude radial terrain blending around a site.

By 2017 they had **artist-authored terrain blending maps per monument**, so
different parts of one monument cut into or blend with the surrounding terrain
differently, and placement height selection changed to minimise *terrain
error* rather than to find flat ground.

The generalisation, and it is the same idea as §2 one level down: a monument
should ship a **set of masks with profiles**, not a radius —

```
HeightStamp · MaterialStamp · BiomeStamp · FoliageMask · RockMask
BuildBlockMask · NavMask · WaterMask · RoadPorts · SpawnMask
```

— so a ruin can have cliffs wrapping one side, a buried entrance on another,
and its influence bleeding outward, instead of everything inside one circle
being flat and everything outside it being untouched.

## 4 · Determinism became a networking problem

Originally every client generated the procedural world from the seed
independently, so the generator had to be deterministic *and* fast on old
hardware. In 2015 they tried verifying generated worlds with checksums and had
to **disable the mismatch kick** because clients were legitimately producing
different worlds from the server's.

That is the nightmare case for authoritative physics: server says the rock edge
is at 104.327, client says 104.411, a player shoots beside the rock, and
nothing in the system can say who is right.

Their answer was to stop asking clients to generate. By 2018 procedural maps
were cached as real map files and custom maps were downloaded rather than
generated; by 2024 a server's procedurally generated map is uploaded to their
backend so joining clients download the finished artifact.

## 5 · Underground monuments broke the AOI model

Their server divides the world into network groups and sends a player only what
is near. Those groups were effectively a **2D grid**, which held until
underground train tunnels shipped.

A player standing on the surface above a subway station received the tunnel's
entities; a player in the tunnel received the bases and NPCs above. Neither
could possibly see the other.

December 2022: the grids were **stacked vertically**, so surface and
underground occupy different networking layers. Quoted saving: up to ~0.4 ms of
client frametime on the surface, purely from no longer networking underground
NPCs to people standing on top of them.

The generalisation: **network distance is not Euclidean distance.** A cell is
`(x, z, layer)` — surface, underground tiers, interior, sky, ocean depth — and
a monument declares which layers it spans and how they connect.

## 6 · Interest range should not be uniform

Two later changes, the second quoted as April 2026:

- The networked region around a player went from a **square** to a blocky
  circle, so the corners stopped being needlessly far away.
- **Different networking ranges by entity class** — small decor and dropped
  objects stop being networked much sooner than vehicles.

The briefing's illustrative ladder (a *shape*, not our numbers): dropped stone
30 m · loot container 80 m · NPC 150 m · player 250 m · gunshot 600 m · world
boss 800 m · large monument animation 1200 m · weather global.

The anti-pattern named explicitly: one `NETWORK_RANGE` constant for everything.

## 7 · Moving monuments are a different feature, not a bigger one

Turning their ocean monument from static into a moving ship required new code
before it shipped at all, because players and NPCs had to be parented to a
large moving vehicle. The networking consequences arrived afterwards: NPCs,
dropped items and corpses parented to it sent updates constantly because they
*or their parent* were moving, and since the ship was broadcast globally its
children effectively were too. Servers struggled whenever it was present.

They then spent years on parented-object correctness — projectile validation,
corpses, ragdolls, elevators, vehicle interaction — and as recently as June
2026 improved server hit validation for parented players by using **historical
parent positions** during validation.

The bill for one moving monument: local-space transforms, parent transforms,
transform history, lag compensation against a moving frame, network relevance,
physics reference frames, an AI navigation reference frame, projectile
validation, dropped-item and corpse parenting, and client prediction of all of
it. **Do not put one in a first implementation unless it is the product.**

## 8 · Navigation, and the performance singularity

**Nav.** January 2018: navmesh problems at monuments so NPCs could path tight
corridors. March 2019: the oil rig monument exposed enough limitations that the
humanoid AI was rewritten from scratch. June 2026: an experimental replacement
for the engine's navmesh entirely — intended to eventually allow NPCs on the
*moving* monuments, which need a moving navmesh — that budgets pathfinding,
handles obstacles off-thread, caps individual query execution time, and builds
navmesh in the background and caches it to disk. Twelve years in.

The briefing's conclusion: **bake monument nav with the monument.** The prefab
ships geometry, collision, a static nav tile, cover nodes, a patrol graph,
spawn points and connection portals; worldgen only joins the world nav to the
monument's tile through a portal, rather than deriving navigation through every
ruin at boot.

**Performance.** Monuments are where players, PvP, corpses, dropped items,
NPCs, vehicles, loot, effects and nearby bases all concentrate. One of theirs
became infamous for it, and the 2021 revamp cut its NPC population, thinned the
surrounding forest and rebuilt its huts partly for performance.

So the budget for a monument is never "one player walks through it costs
0.2 ms". It is 30 players plus 40 NPCs plus 8 bases plus 200 deployables plus
loot plus projectiles plus world-event state, all intersecting, while hundreds
of other players are connected elsewhere.

## 9 · What it means for us

Measured against the tree on 2026-08-10. Every line carries a cite; where the
answer is "absent", it says absent.

### 9.1 · What we already got right, and must not relitigate

- **Placement is already a search, not a guess.** `terrain::haven`
  (`sim-core/src/terrain.rs:1047`) scores `HAVEN_CANDIDATES = 64` bearings and
  takes the argmin of `relief + HAVEN_HEIGHT_W × height`; the lesser tier is
  chosen from *the same scored candidate list* by `pick_minor`
  (`terrain.rs:1224`), greedy under a separation floor. That is §1's 2015 fix
  and half of its 2017 one, arrived at without paying for either.
- **§4 is structurally unreachable for us, and it is our biggest single
  advantage over this whole history.** Three facts, stacked. The world is a
  pure function of the seed in `sim-core`, whose float discipline is wall 1
  (no libm, no trig, a restricted operator set, authored LUTs). The native
  client reaches that generator by **direct call to the same native code the
  server runs** (`RENDER.md` §3) — not a second implementation, and not even a
  second build. And `test_parity_wasm` asserts native and wasm produce
  bit-identical digests (`ci/gates.sh:161`) with worldgen explicitly required
  on the parity surface (`:183`), which is what keeps `client-core`'s wasm
  answer equal too. Their 2015 checksum-mismatch kick had to be disabled
  because two implementations genuinely disagreed; we have one, and its
  equality is a merge gate.

  **So the briefing's recommendation to copy their serialized world manifest
  is the one thing in it we should decline.** It is the right answer to a
  problem we do not have, and taking it would trade a proven compile-time
  invariant for a shipping problem (an artifact to build, host, version,
  invalidate and download). What *is* worth stealing from their 2024 state is
  narrower and is not a manifest: the observation that a joining client should
  never spend time generating what the server can name. We already do that —
  `Welcome` is four fields (`reference/NETWORK.md` §9.1).
- **The site refuses the position rather than patching the object.**
  `haven_ring_phase` and `haven_shelter_bearing` (`terrain.rs:943`, `:992`)
  reject a candidate that cannot stand its containers or its structure, rather
  than shipping a site with one of them moved or missing. That is `SPAWN.md`
  §5's chain, and it is what §2's whole list is made of failures to do.
- **An incomplete island is a refusal to boot, not a silent degradation.**
  `sites_complete` / `sites_live` (`terrain.rs:1466`, `:1481`) feed
  `check_island` (`server/src/boot.rs:51`). §1's "some seeds are missing large
  monuments" is a class of bug that cannot ship here without a shard refusing
  to start.
- **The AOI test is already radial with hysteresis** — `d2 <= AOI_ENTER_CM²`,
  176 m in / 208 m out (`limits.rs:61`, `server/src/core.rs:2449`). We start
  where §6's first change finished.
- **There is no navmesh to bake, and that is a feature.** The terrain is a
  pure function, so an animal steers and `movement::step` decides
  (`sim-core/src/mob.rs`, `NOW.md` §0m). §8's twelve-year navmesh arc has no
  analogue here *while mobs steer*. It acquires one the day an NPC has to path
  a corridor — see 9.4.

### 9.2 · Built this pass: a site publishes masks, not a radius

§3, applied to what we have. Before this pass an authored site had **one**
footprint — `HAVEN_RADIUS_M` / `WAYSTATION_RADIUS_M` — answering one question,
"does the scatter grid stand anything here" (`terrain.rs`, `in_haven` /
`in_waystation`), and every other world system either asked that same circle or
was never wired to the site list at all.

The measured consequence was ground clutter. `clutter_fill` had no `Haven`
parameter, so grass, twigs and scree grew **straight across the haven pad and
both waystations**, while the carriageway running through them was correctly
swept to grit. The road got an override; the destination it leads to did not.
That is §2's shape exactly, one level down: the clutter population shipped
after the sites and was never told they exist.

Measured on the pad's floor (r = 10.64 m) over the three seeds
`tests/clutter.rs` drives, before and after: **661 / 62 / 506 grass-and-litter
elements and 80 / 11 / 24 understory elements, all now zero** — ~870 grit
elements in their place on each, so the coverage count is unchanged and only
its identity moved. (Seed 1's low count is a sandier pad: `splat` was already
drawing Pebble there of its own accord, which is why the gate's discriminator
is the understory and not the kind.)

Landed:

- `SiteFootprint` (`terrain.rs:1623`) — a site publishes `scatter_m` (the grid
  veto it always had) and `swept_m` (the made floor, new), with the ordering
  and the arrangement-covers asserted at the definition (wall 4).
- `swept_m` is **derived, not spoken**: the container ring plus one clutter
  cell, so a site that moves its ring drags its floor with it, the way
  `skirt_base_r` reaches off `occupant_volume`.
- `site_sweep` (`terrain.rs:1694`) is a **scalar with a profile**, not a
  circle: 1.0 on the floor, 0.0 at the scatter mask, smoothstepped across the
  band between them, max over the live sites. Its consumers dither each
  element against it with a hash byte they had already drawn, so the boundary
  is a thinning population rather than a ring on the ground. That is §3's
  entire lesson and it is the one a naive fix would have got wrong.
- Consumed by both clutter strata and by the prop skirts. Coverage becomes
  grit rather than nothing, because a hole would be a hole in `ART.md` rule 4's
  guarantee; the understory is refused outright, because a made floor has not
  earned one.

Gated by `sim-core/tests/clutter.rs` §S, against a control site list parked
offshore, so all three claims are exact rather than statistical: the floor is
swept, **the wilderness is bit-identical**, and the band contains both
outcomes. Each is proven red under its own mutant (sweep disabled; sweep as a
hard circle; the band collapsed to zero width).

Not built, and each is a row this struct gains when a reader exists:
`BuildBlockMask` (nothing in `build.rs`/`deploy.rs` mentions a site, so a
player may build on the pad today — whether that is wrong is a design call, not
a defect, and it is in `DECISIONS.md` §open), `HeightStamp` (there is no carve
at all — `TERRAIN.md` §1 stage 8 finds flat ground rather than making it, and
`NOW.md` §4b prices the change), `NavMask`, `WaterMask`.

### 9.3 · The gap that matters most: our solver is two hand-written tiers

§1 is the half we have; §2's inversion is the half we do not. `haven()` and
`pick_minor` produce exactly two kinds of site, and each kind's check chain is
its own function by a deliberate call recorded in the code
(`waystation_ring_phase`'s doc: *"the two tiers are allowed to diverge — a
later POI kind with a different check chain is the point of the hook"*). That
call is still right. What is **not** paid for is everything around it:

- The separation floor is one constant, `WAYSTATION_MIN_SEP_M`
  (`terrain.rs:740`), asserted against the two tiers' radii by hand. A third
  kind needs a pairwise rule, not a third constant.
- There is no reservation ledger. `pick_minor` re-tests distance against the
  pad and against each already-taken site inline. That is correct at two and
  is §1's starvation shape at five.
- **Order is already load-bearing and already right, by accident of having
  only two tiers**: the pad resolves first and nothing after it may move the
  pad (`terrain.rs:1191`). That is §2's "place roadside monuments after large
  monuments", and it should become an explicit tier field before a third row
  makes it implicit.
- **We have no ports, and we do not currently need them** — which is the
  interesting part. Our sites are *chosen on the road* (`haven()` inverts the
  road's own centre-line definition to land on the ring by construction), so
  §2's road-connection problem is solved by placement order rather than by a
  port table. That holds until a site wants to be somewhere the road is not.
  When one does, the port is the cheap answer and the road-finding search is
  not.

### 9.3a · Built this pass: the drawn world is gated against the blocked world

§4's failure mode does not apply to us as *arithmetic* (9.1), but it applies
in full as **duplicated authorship**, and we had a live instance of it. The
two authored structures were declared once in `sim_core::terrain` as the
volume the sim blocks and once in `render/props.rs` as the mesh the client
draws; the gate that held them equal was a browser `.mjs` deleted with the
browser client, and they had drifted (`TERRAIN.md` §7.1): the canopy nine
rows to a 4.1 m finial against six rows topping at 2.09 m, the shelter
fourteen rows against nine — no corner posts, no tower-cap, a 9.2 m peak
drawn at 5.6 m. A player was stopped ~0.7 m outside posts they could see.
That is their rock-edge bug with the floats taken out of it.

Landed:

- **The mesh is DERIVED from the sim's table, not mirrored from it**
  (`render/props.rs`, `authored()`). One list. The `* 0.5` full-size →
  half-extent conversion — the transcription hazard that let the two come
  apart while both looked plausible — is written once against a length the
  type system pins. Only the colours stay client-side, because what a wall
  is made of is not a fact about the volume it occupies.
- **`archetype_mesh` / `archetype_lift` / `SINK_M`** make the renderer's
  geometry a pure function, so a test can ask the question the draw asks.
  Before this every archetype was an expression inside `assets()`, reachable
  only from a running app — which is why `OCCUPANT_R_M`'s own doc says
  "nothing in the Rust workspace can see a triangle, so the asserts below
  prove only that this file agrees with itself".
- **`crates/client/tests/greybox.rs`** (5 tests): the half-extent conversion,
  every row reaching the mesh by vertex count, the authored pair's bounds
  equalling the published broad phase in *both* directions, every other
  archetype fitting the volume the sim blocks, and a coverage check that a new
  occupant arrives measured or explicitly excused. Proven red under the
  historical drift itself (truncate the canopy to six rows → "drawn peak
  3.0000 m against the sim's published 4.1000 m") and under the units bug.

Measured while building it, and **closed the same day on the operator's
call**: the generated props blocked wider than they drew. `blob_mesh`
displaces vertices inward from its nominal radius, so a boulder
`OCCUPANT_R_M` called 1.5 m actually reaches 1.1145 m — a 0.39 m invisible
collision skirt, and 0.52 m of headroom on the ore nodes. It is the canopy's
defect in the *survivable* direction (stopped short of something you can see,
rather than walking through something solid) and it was found the same way:
by a gate that could finally measure a triangle. The rows are the measured
bounds now and the ratchet is an equality gate.

**The general lesson is worth more than the fix.** Every one of these rows
was written off the *generator's parameter* — "DodecahedronGeometry(1.5)" —
rather than off the geometry the generator produced, and a parameter is not a
measurement. That is §4's failure at its smallest scale: two descriptions of
one object, both plausible, drifting because nothing compared them. It is
also why `walk.rs`'s boulder fixture went red on the fix and was right to —
it asserted `stop > 1.5`, pinning the nominal, so it had been testing the
constant rather than the collision.

### 9.3b · Built this pass: the same seed can no longer mean two islands

§4, applied to persistence rather than to the wire. The world file's header
already refused a wrong seed and a moved content hash (`worldfile.rs`) — but
the seed is not the island. Change a noise constant, a road width or a site's
scoring weight and the *same* seed generates different ground, and the file
would have loaded happily with every base at coordinates describing terrain
that no longer exists: foundations in the air, doorways inside hills. That is
their rock-edge disagreement with a restart in the middle of it.

The header now carries `probe_terrain(seed)` — the digest
`test_terrain_golden` already pins and the parity gate already diffs, so the
number the shard refuses on is the number CI refuses on. Computed once at
boot, never per save. `WORLD_FILE_FORMAT` turned 1 → 2 and the new field
spends the padding the header was written with. The refusal names the remedy
in the operator's own terms, and `world_persist.rs` asserts both the refusal
and that the message says "wipe".

**This is detection, not policy.** The policy is the operator's and is
spoken: a worldgen change is a wipe (2026-08-10), matching the reference
game's own posture — they force wipes on map-affecting changes and could not
avoid it either, because there is no converter that can move a base onto
ground that changed shape under it. What this buys is that the wipe is a
message at boot instead of a bug report a week later.

### 9.4 · What is absent, ranked by when it will bite

1. **Class S has no interest filter at all** — the join walk drips the entire
   piece store to every client regardless of distance (`NETCODE.md` §7,
   `NOW.md` §0n1). §5 and §6 are refinements of a filter; we do not have the
   filter. **A monument is the worst possible place to discover that**, because
   §8's concentration is exactly where the piece store is densest.
2. **Per-entity interest ranges (§6) do not exist.** One radial test serves
   every class (`core.rs:2449`). Cheap to add, and the ladder in §6 is a shape
   we can price against our own budgets — but it belongs with the chunk
   subscription, not before it, or we build two spatial truths.
3. **Vertical AOI layers (§5) are premature and should stay that way.** We have
   no underground, no interiors and no sky. The layer field is the right shape
   the day a monument spans one; adding it now would be a wire field with no
   producer.
4. **Nav (§8) enters the moment an NPC defends a monument.** Today nothing
   fights back (`NOW.md` §0m item 2). When something does, bake the tile with
   the site — §8's end state — rather than deriving it at boot.
5. **Moving monuments (§7): refused, on the record.** Not "later" — the bill
   in §7 is a feature, and nothing in `DESIGN.md` asks for one.
6. **The §8 benchmark shape is owed and is not written.** `NETCODE.md` §11's
   seven gates are still unbuilt. `test_raid_storm` now exists
   (`crates/sim-core/tests/raid_storm.rs`, 2026-08-14) but it is wall 4's
   caps gate, not §11's wire storm — it times nothing, which is exactly what
   a benchmark shape needs. Whatever it becomes, the load case is *at a
   destination*, not on open ground.
7. **Only x86_64 ↔ wasm32 is gated, and a third CPU is not runnable here.**
   aarch64 (Apple Silicon) and Windows/MSVC are latent; the depot is Linux
   only, so nothing is broken today. The op set is IEEE-754-*specified* — the
   ban on `mul_add`, on every transcendental and on libm leaves only
   correctly-rounded operations — so cross-target equality is safe by
   construction, and wasm32 is a genuinely different LLVM backend agreeing,
   which is evidence rather than nothing. What was missing is that "safe by
   construction" had no check at all on the axis we *can* reach: **float
   contraction is opt-level-dependent**, and both sides of the wasm diff are
   `--release`. `ci/gates.sh` now also diffs the debug probe against the
   release one, which is the same failure an ARM build would show. Verified
   identical on 2026-08-10. Running a real third target needs qemu and a cross
   linker, neither of which this box has — it belongs with the first Mac build,
   and it is one `diff` when that exists.
8. **Worldgen changing under a live shard is now a refusal** (see 9.3b), which
   is the *detection*. The economics are the operator's and are settled: **a
   worldgen change is a wipe** (2026-08-10), which is the reference game's own
   posture. What has no mechanism is the wipe itself — `NOW.md` §0q item 2
   still describes it as unscoped.

### 9.5 · One thing to steal verbatim

§1's sequence, as the acceptance test for our own solver when it grows a third
row: **candidates → score → solve → reserve → then everything else.** Our
scatter grid, clutter population and prop skirts all already run *after* the
sites resolve and read them as an input. That ordering is the asset. The rule
to hold is that nothing may ever be generated first and asked about collisions
second — every entry in §2's list is that rule being broken once.

## 10 · Still open — logged, not queued

- **The tier-1 sources have never been opened from this box.** §0 says why. If
  the proxy is ever fixed, the specific claims worth re-verifying first are the
  2022 vertical-grid change (the only measured number in the whole briefing:
  ~0.4 ms) and the 2020 world-size bump (4000 → 4250 for placement room),
  because both are *decisions with a cost attached* and would be worth citing
  precisely.
- **Whether a destination should be build-blocked** is a design question this
  file cannot answer and deliberately does not. It is in `DECISIONS.md` §open.
- **The reserved-expansion-volume idea from §2 has no equivalent here** and may
  never need one: our sites are small and the island is 2048 m. Recorded so a
  later pass does not mistake its absence for an oversight.
