# reference/SPAWN.md — how the reference game places and respawns world objects

Ripped facts, not design. `rust-systems.txt` answers *what systems exist*,
`FINDINGS.md` answers *which ones bled*; this file answers **how the
reference game gets a tree onto the ground and back again after someone
chops it down**, because that is `TERRAIN.md` §1 stage 9 and §2 with a
decade of production behind it.

Dated 2026-08-04. Read against `TERRAIN.md` §2 (slots) and `NETCODE.md`
§5 (chunk events). §9 is the part that changes what we build.

## 0 · Provenance, and a warning about it — read this first

**Source: a decompiled `Assembly-CSharp`, not a licensed dump.** Specifically
`unet-dev/Decompiled-Assemblies`, a community ILSpy dump of an
**Oxide-patched** Rust server build — `World.Spawn` and eight other classes
carry `using Oxide.Core;` and inlined `Interface.CallHook`, so the hooks are
injected, not original. Age markers: network protocol **179**,
`WorldSerialization.Version == 9`.

This is **not** the licence posture of `rust-systems.txt`. That file is MIT
Oxide build data, regenerable, and the README says so. This one is
decompiled proprietary code, and the discipline is correspondingly harder:

- **Nothing was copied into this repo.** Every algorithm below is described
  in prose and in our own notation. No C# was transcribed, and none will be.
- **Nothing regenerates it.** There is no script here that fetches it, by
  design — unlike `rip-hooks.py`, this must not become a build input.
- Facts only, in the same sense as the README: class names, field names,
  constants, and the shape of an algorithm read for behaviour.

Everything below is read off one build. Where a number is a *serialized*
field it can be overridden by the scene or prefab asset that carries it and
**the code is not the authority** — the tables mark which is which. Treat a
class default as "the shipped value unless an artist moved it", which for
several of these is exactly what happened.

## 1 · There is not one system. There are four, split by one question

The split is not by object type. It is by **is the thing a networked entity**,
and it decides everything downstream — determinism, persistence, and whether
a felled tree comes back where it fell.

| # | system | what it places | seed-derived? | lives in |
|---|---|---|---|---|
| A | worldgen `ProceduralComponent`s | monuments, cliffs, decor, roads, rivers | **yes**, threaded seed | the `.map` file |
| B | `SpawnHandler` **populations** | **trees**, ore nodes, barrels, collectables, animals | **no** (§9.1) | the `.sav` file |
| C | `SpawnGroup` | monument loot: crates, barrels at fixed points | no | the `.sav` file |
| D | `SpawnIndividual` | map-file entities that must return to their exact spot | position is, draw isn't | in memory, from the map |

A is the system `TERRAIN.md` describes. B is the one that owns trees. **We
assumed our model was theirs and it is not**, and §9 is what that costs.

## 2 · System A — worldgen placement, the part that *is* a pure function

`WorldSetup.InitCoroutine` runs, in order: download-or-load the `.map` →
create terrain → load six terrain maps (`terrain`, `splat`, `biome`,
`topology`, `alpha`, `water`) → `World.Spawn()` the cached prefab list →
run every `ProceduralComponent` → save the `.map` → **checksum it**.

The determinism mechanism is one line and it is the good idea in the file:

> each `ProceduralComponent` is handed `seed = World.Seed + its index in the
> child list`, and draws through `SeedRandom.Range(ref seed, …)` — a
> **stateless generator threaded by reference**, never the global RNG.

That is our splitmix64-on-`(seed, cell, channel)` discipline arrived at from
the other direction: they thread a mutable seed, we hash coordinates. Ours
is strictly better (theirs is order-dependent — insert a component and every
later component's world moves; ours is not), but the intent is identical and
the outcome is the same: **the whole map file is a function of the seed, and
a checksum proves it.**

**`PlaceDecorUniform`** is the closest thing they have to our scatter pass,
and it is nearly our stage 9:

1. walk a **uniform grid** at `ObjectDistance` (10 m) over the terrain;
2. jitter both axes by ±`ObjectDithering` (5 m);
3. accept with probability **`factor²`**, where `factor` is the `SpawnFilter`
   value at that point (§4) — squared, so marginal terrain thins out much
   faster than linearly;
4. run the placement-check chain (§5);
5. `World.AddPrefab("Decor", …)` — into the map file, into the checksum.

Grid, jitter, weight, veto, emit. That is our scatter pass with a different
RNG and a squared acceptance curve, and **the squaring is worth stealing**
(§9.4).

**`PlaceMonuments`** is more interesting than its output suggests, and it is
the machinery `TERRAIN.md` §1 stage 8 will want when the haven pad becomes
POIs. It does not place monuments one at a time. It builds **10 complete
candidate layouts**, scores each, and keeps the best:

- prefabs are shuffled by seed, then sorted by `PrefabPriority`;
- per prefab, up to **10 000** rejection-sampled attempts at a position that
  passes `factor² ≥ rand`, a **`Distance` (500 m) minimum-separation check
  against everything already placed in this candidate**, and the full check
  chain of §5;
- a placed prefab scores `(priority + 1)⁴` — a quartic, so one high-priority
  monument outweighs a pile of low ones;
- the highest-scoring of the 10 candidate layouts wins and is emitted.

The quartic plus best-of-10 is how "every map has the big monuments" is
enforced without hard-coding placement. Cheap, and it never fails to a map
with no monuments at all — the failure mode is a slightly worse layout.

## 3 · System B — `SpawnHandler` populations, which is what owns trees

A `SpawnPopulation` is a `ScriptableObject`: a prefab folder, a target
density, a rate, cluster parameters, and a `SpawnFilter`. **The numbers live
in the asset, not in code** — which is our wall 7 (`content/*.toml`, never in
code) independently arrived at. The code carries only defaults.

### 3.1 · Building the distribution (once, at boot)

`SpawnHandler.UpdateDistributions()`, per population:

1. Allocate an `N × N` **byte** field, `N = NextPowerOfTwo(World.Size × 0.25)`
   — so a 4000 m map gets a 1024² grid, ≈ 3.9 m cells. (The character-spawn
   distribution uses `× 0.5`, one step finer.)
2. Fill it **in parallel** with `255 × SpawnFilter.GetFactor(normX, normZ)`
   at each cell centre.
3. Feed it to a **`ByteQuadtree`**: level 0 is the field, and each level up
   stores the **sum of its four children**. This is a summed-area tree, and
   summing is the whole trick — an interior node's value *is* the total
   weight beneath it.
4. `Density` = mean of the byte field ÷ 255 — the fraction of the map this
   population can live on. It scales the target count (§3.3).

### 3.2 · Sampling a position

`SampleNode()` descends the quadtree from the root: at each level it draws
one uniform and picks the child whose **cumulative share of the four
siblings' sums** covers the draw. Importance sampling, `O(log N)`, no
scanning, and the byte field is the target distribution exactly.

One real detail: if all four children are zero the total is zero, the
comparison is `NaN`, every branch fails, and it falls through to child 4. So
**the quadtree is a biased sampler and not a gate** — a zero-weight region
can still be sampled. The hard gate is the explicit
`Filter.GetFactor(pos) > 0` re-check at the spawn site, which the caller
does on every single candidate. Worth naming because it is the correct
structure: *sample cheaply and approximately, reject exactly.*

`Sample()` then tries up to **15 times** to turn that leaf into a position:
uniform point inside the leaf, plus ±`ClusterDithering`; take the heightmap
height; if `PlacementCheckMask` is set, raycast down `PlacementCheckHeight`
(25 m) and require the hit layer to be in `PlacementMask` (this is the
"don't spawn on a player's roof" rule); then `CheckSphere` at
`RadiusCheckDistance` (5 m) against `RadiusCheckMask` and reject on a hit.
Yaw is uniform 0–360°; `AlignToNormal` optionally tilts to the terrain
normal.

### 3.3 · How many should exist

```
targetCount = round( worldArea_m²
                   × TargetDensity × 1e-6      # per km² → per m²
                   × densityScalar             # convar, §7
                   × distribution.Density )    # if ScaleWithSpawnFilter
```

`1e-6` is the km²→m² conversion, which is why the asset tooltip says "usually
per square km". `GetCurrentCount` is not a scan — `SpawnDistribution` keeps a
live `Count`, a per-prefab-ID dictionary, and a 20 m `WorldSpaceGrid<int>`,
all maintained by `Spawnable.OnEnable`/`OnDisable`. **A killed tree
decrements the count the instant its GameObject disables.** That is the
entire respawn trigger: there is no timer on the tree.

### 3.4 · The fill loop, and the local-density cap

Both the initial fill and the repeating tick call one routine with a
`numToFill` and an attempt budget `numToTry`:

```
localCap = max(ClusterSizeMax, gridCellArea(400 m²) × 2 × currentDensity)
UpdateWeights(distribution, targetCount)          # per-prefab quotas
while numToFill ≥ ClusterSizeMin and numToTry > 0:
    node    = distribution.SampleNode()            # one quadtree leaf
    cluster = rand[ClusterSizeMin, ClusterSizeMax] # a clump, not a point
    for each of cluster:
        if Sample(node) and Filter.GetFactor(pos) > 0
                        and distribution.GetCount(pos) < localCap:
            Spawn(...); numToFill--
        numToTry--
```

Three things in there are load-bearing:

- **Clusters are drawn from one leaf.** `ClusterSizeMin..Max` objects come
  out of the same quadtree node, so trees arrive in clumps rather than as
  uniform noise. This is how a forest looks like a forest.
- **`localCap` is the anti-clumping brake on top of it** — `2 ×` the target
  density over a 20 m × 20 m cell. Cluster spreads, cap contains. The pair is
  the whole texture of the distribution.
- **The attempt budget is the only thing bounding the loop.** Initial fill
  gets `deficit × SpawnAttemptsInitial` (20), a tick gets
  `deficit × SpawnAttemptsRepeating` (10). Under a filter that has become
  hostile — say a player has built over the good ground — the loop burns its
  budget and simply under-fills. It never spins.

`UpdateWeights` deals the target count across the prefab variants, subtracts
what already exists per prefab ID, and `GetRandomPrefab` draws from and
decrements those quotas — so variety is a **quota, not a per-draw roll**, and
a population cannot drift to all-one-variant over a wipe.

### 3.5 · The respawn tick, and the fact that matters

Per population, every `spawn.tick_populations` (**60 s**), yielding a frame
between populations:

```
deficit = targetCount − currentCount
deficit = round(deficit × SpawnRate × rateScalar)
n       = min(deficit, MaxSpawnsPerTick)     # class default 100
fill(n, attempts = n × SpawnAttemptsRepeating)
```

**The respawned tree does not appear where the old one stood.** Nothing in
the pipeline remembers the dead one's position — it decremented a counter and
that is all it ever was. The refill samples the quadtree fresh. Over a wipe a
Rust forest slowly *migrates* toward the high-factor centre of its own
filter, because every death is re-rolled against the distribution and the
distribution is peaked.

This is a **deliberate difference from our §2 slot model, not an oversight**,
and it is the one design question in this file (§9.2).

### 3.6 · Persistence, and the limit enforcer

Trees are saved entities. `Spawnable.Save` writes the population's filename
ID; `Spawnable.Load` resolves it back and re-registers with the distribution.
`ServerMgr.Initialize` is explicit about the consequence:

```
UpdateDistributions()  →  SaveRestore.Load()  →  if (no save loaded) InitialSpawn()  →  StartSpawnTick()
```

**The initial spawn runs on a fresh wipe only.** Every later boot restores
tens of thousands of individually serialized tree entities from disk. After a
load, `SaveRestore` calls `EnforceLimits(false)`, which for every population
with `EnforcePopulationLimits` counts live instances and **kills the excess**
— `Take(n)` off an unordered `FindObjectsOfType` scan, so *which* trees die
is arbitrary. That is the migration path for a density nerf: change the
asset, reboot, the surplus is culled at random.

## 4 · `SpawnFilter` — the terrain question, asked in one call

`GetFactor(normX, normZ) → [0,1]`, evaluated against three baked maps, in
this order, short-circuiting to 0:

1. **Topology** — a bitmask (Field, Cliff, Summit, Beach, Forest, Ocean,
   Road, Roadside, Swamp, River, Lake, Powerline, Runway, Building,
   Mountain, Clutter, Tier0/1/2, Mainland, Hilltop, …) tested three ways:
   `TopologyAny` (must match ≥1), `TopologyNot` (must match none),
   `TopologyAll` (must match every bit).
2. **Biome** — bitmask over Arid / Temperate / Tundra / Arctic, against the
   cell's *dominant* biome.
3. **Splat** — and this one is not a test, it is the **return value**: the
   splat weight of the requested ground types at that point. Grass at 0.7
   returns 0.7.

So the "filter" is a hard mask (topology, biome) multiplied into a soft
ground-material weight (splat), and `factor²` at the acceptance site (§2)
squares the soft half. `Test()` — used where a boolean is needed — is
`GetFactor > 0.5`.

The three maps are also **exactly the extension point**: a monument writes
`Monument` topology into the map at worldgen (`ApplyTerrainPlacements` /
`ApplyTerrainModifiers`), and every population that excludes `Monument`
thereby leaves it alone, with no code aware that monuments exist. Same for
roads. **Topology is how one system vetoes another without either knowing
the other's name.**

## 5 · The placement-check chain (all four systems share it)

Attached to prefabs as `PrefabAttribute`s, so the *asset* declares its own
placement rules and the spawner stays generic:

| attribute | question |
|---|---|
| `DecorComponent` | adjust pos/rot/scale before anything is tested |
| `TerrainAnchor` | can the mesh's anchor points reach ground? (`MinimizeError` or `MinimizeMovement`) |
| `TerrainCheck` | is this point within ±`Extents` of the heightmap? |
| `TerrainFilter` | does a *point-local* `SpawnFilter` pass here? |
| `WaterCheck` | is this point under the water map? (used both ways) |
| bounds check | `Physics.CheckBox` over the entity's own bounds — the "is something already here" test |

Every stage is a veto, and all of them run **before** the entity is created.
Note what the occupancy test is: **a physics box overlap against live
colliders**, per candidate, per attempt. On our side that is a bitset lookup;
this is the cost of not having a slot model (§9.2).

## 6 · Systems C and D — fixed points and exact respawns

**`SpawnGroup`** is the monument/loot mechanism: a MonoBehaviour holding
weighted prefab entries and a set of child `BaseSpawnPoint`s. It keeps its
own `LocalClock`, ticks under `SpawnHandler`'s coroutine, and on fire spawns
`rand[numToSpawnPerTickMin, Max]` capped at `maxPopulation − currentPopulation`.
Spawn points are tried round-robin from a random start until an active one is
found. Population is tracked by a `SpawnPointInstance` component welded onto
each spawned entity, which notifies the group and the point on spawn and
**on `OnDestroy`** — same "the object's own lifetime is the bookkeeping"
trick as `Spawnable`. Interval:

```
delta    = (respawnDelayMax + respawnDelayMin) / 2 / PlayerScale(spawn.player_scale)
variance = (respawnDelayMax − respawnDelayMin) / 2 / PlayerScale(spawn.player_scale)
```

`WantsTimedSpawn()` is `respawnDelayMax != +∞` — infinity is the "spawn once,
never again" encoding.

**`SpawnIndividual`** is the one that is nearly ours, and it is a
three-field struct: prefab ID, position, rotation. `Spawnable.Add()`
registers one automatically for any spawnable entity that is created **while
the world is loading from the map file** (not from a save), that has saving
enabled, and that does not sync its position. Every
`spawn.tick_individuals` (**300 s**) the handler re-spawns every registered
individual **at its exact original transform**, gated only by the bounds
overlap check — which fails harmlessly while the original is still standing.

So the reference game *does* have a "respawn at the recorded spot" mechanism.
It costs one struct per slot and one box overlap per slot per 5 minutes, and
they use it for map-file entities and not for trees.

## 7 · The convar layer — one knob shape, applied twice

Two distinct player-count responses, and the difference is the point:

```
PlayerFraction() = clamp01(activePlayers / maxplayers)
PlayerLerp(a,b)  = lerp(a, b, PlayerFraction())          # populations: 0.5 → 1.0
PlayerExcess()   = max(0, (activePlayers − player_base) / player_base)
PlayerScale(s)   = max(1, PlayerExcess() × s)            # groups: 1× → faster
```

- **Populations** scale their density and rate between `min_*` and `max_*` by
  how full the server is — an empty server carries **half** the trees and
  regrows them at **half** the rate. Opt-in per population
  (`ScaleWithServerPopulation`, default off; `EnforcePopulationLimits` and
  `ScaleWithSpawnFilter` default on).
- **Groups** divide their respawn delay by an *excess* factor above
  `player_base` (100) — so loot at monuments comes back faster only once the
  server is over its nominal population, and never slower than baseline.

`spawn.fill_populations` / `fill_groups` / `fill_individuals` are the manual
"top everything up now" verbs, and `spawn.report` / `spawn.scalars` print
current/target per population and the four scalars. **An operator-visible
census of the whole system, in two commands.** We should have that.

## 8 · Placement of *networked* objects — the visibility grid

`NetworkVisibilityGrid` implements `Network.Visibility.Provider`, and it is
what makes tens of thousands of tree entities affordable:

- The world is a **fixed `cellCount × cellCount` grid** — so cell size
  is `gridSize / cellCount` and **scales with map size**; the *number* of
  groups is constant.
- Group ID = `x × cellCount + y + startID`, with **two reserved IDs below it:
  0 = global** (`globalBroadcast` entities, everyone subscribes) and
  **1 = limbo**.
- A connection subscribes to a **diamond of radius `visibilityRadius`** in
  cell steps around its own group, plus group 0. Radius 2 = 13 cells.
- `BaseNetworkable.UpdateNetworkGroup()` re-tests on move: `IsInside` first
  (cheap), and only on a miss does it `GetGroup` and switch, sending a
  `GroupChange` packet. **`switchTolerance` (20 m) is hysteresis** — a group's
  bounds are treated as 20 m larger than they are for the leave test, so an
  entity walking a cell boundary does not thrash subscriptions.
- Cell bounds are `cellSize × 1 048 576 × cellSize` — deliberately unbounded
  vertically. **The grid is 2D; altitude never affects visibility.**
- Subscription updates are budgeted: `UpdateSubscriptions(removeLimit,
  addLimit)` and re-queues itself when it runs out.

**Correcting this file's first cut: there is nothing here to carry.** It
claimed we had neither hysteresis nor a global lane. We have both, and both
are gated — `AOI_ENTER_CM` / `AOI_EXIT_CM` in `limits.rs` are a 176 m / 208 m
planar band with `test_aoi_hysteresis` on it, and `EV_DEATH` and `EV_DOOR`
are broadcast rather than AOI'd on the stated grounds that a death is a world
fact, which is their group 0 by another name. A doc that disagrees with a
passing gate is wrong (`CLAUDE.md`), and this one was.

The one genuine difference is a tradeoff, not a gap: **their cell *count* is
fixed and ours is fixed the other way.** They pin `cellCount` and let cell
size scale with the map, which bounds the size of the group table and lets
one build serve 1000 m and 6000 m maps. We pin 64 m cells and let the count
follow, which bounds the work *inside* a cell. For one fixed 2 km island ours
is the right end of that trade, and it stops being right only if map size
ever becomes a server option — which `TERRAIN.md` §6 says it is not.

## 9 · What this means for Gates

### 9.1 · Their resource placement is not reproducible from the seed. Ours is.

Every draw in system B comes from Unity's **global** `UnityEngine.Random`.
`Random.InitState` appears exactly once in everything read — in
`ResourceDepositManager` (§9.6) — and nothing re-seeds it before
`InitialSpawn()`, which runs after a time-sliced load. The `.map` file, which
is checksummed and shareable, contains the terrain maps and the
`World.AddPrefab` output of system A **and no populations at all**.

The practical consequence, stated carefully because it is a strong claim read
off one build: **two servers on the same seed and size get the same terrain,
roads, monuments and decor, and there is no mechanism in this code that would
give them the same trees.** They also cannot: the client would have to be
told, and trees are entities, so it already is.

**We are strictly ahead here and it is the whole bet.** `TERRAIN.md` §0's
"a 2 km island costs zero bytes to join" is only true because our scatter is
`h(seed, cell)`, and this file is the evidence that the alternative is
tens of thousands of serialized entities and a save file. Do not trade it.

### 9.2 · The one real design question: does a felled tree come back where it fell?

Theirs: **no** (§3.5) — the population is a count, and a refill is a fresh
sample, so forests migrate toward their filter's peak over a wipe.
Ours: **yes** — `TERRAIN.md` §2 is one bit and one timer per slot, respawning
in place.

Neither is wrong, and ours is cheaper and more predictable. But name what
in-place respawn costs, because we chose it by default and not by argument:

- **A depleted area stays depleted in shape.** Their model heals a
  clear-cut by pulling density in from the whole distribution; ours regrows
  exactly the field that was cut, on a timer. For a 100-player 2 km island
  that is probably what we want — players learn where the trees are, and a
  farm route is a real thing you can own.
- **It removes the "spawn camping migrates" pressure valve.** Worth
  knowing, not worth building.
- **The slot list is fixed at generation, so density can only ever go down.**
  Theirs can rebalance a live map by editing an asset and rebooting
  (`EnforceLimits` culls, the tick refills). Ours would need a content-hash
  bump and, by wall 7, a replay-compatibility story. That is the actual cost
  and it is a **wipe-boundary constraint**: scatter table changes are wipe
  events. Worth writing into `DECISIONS.md` §open as a stated consequence
  rather than discovering it at the first balance pass.

Recommendation: **keep in-place respawn.** It is the reason our terrain is
free to join. But adopt their *clustering* (§9.3) and their *squared
acceptance* (§9.4), which are the two things that make their distribution
look better than a per-cell independent roll.

### 9.3 · Our scatter is per-cell independent. Theirs clusters. That is visible.

`terrain.rs::scatter` draws one hash per 8 m cell and decides that cell alone.
Every slot is independent of its neighbours, which is **white noise** — no
groves, no clearings, just uniform-density speckle at whatever the biome
weight says. `TERRAIN.md` §1 stage 6 calls forest "wood, cover, low
visibility", and independent draws cannot deliver cover; they deliver an
orchard.

Theirs gets clumping from `ClusterSizeMin..Max` out of one quadtree leaf,
braked by a 20 m local cap. We cannot copy the mechanism — it needs a
stateful sampler and we are a pure function — but we can get the same texture
for one extra hash read: **make the biome weight itself a low-frequency
noise field**, so cell weight is `biome_weight × clump(seed, x, z)` where
`clump` is a cheap 2–3 octave value-noise channel we already have the
machinery for (`terrain.rs` has `moisture` doing exactly this shape). Groves
where the field is high, clearings where it is low, still `O(1)`, still pure,
still one draw per cell. This is the highest-value item in this file.

### 9.4 · Steal the squared acceptance

Both `PlaceDecorUniform` and `PlaceMonuments` accept on `factor² ≥ rand`, not
`factor ≥ rand`. Squaring is not a fudge: it makes marginal terrain fall off
quadratically, so a biome edge reads as a **gradient with a soft tail**
instead of a step at the threshold. Our stage 9 does a straight weighted draw
against the biome row and vetoes on slope — a hard edge exactly where the
biome function changes. Free improvement, one multiply, and it moves in the
same direction `TERRAIN.md` §4's "soft ramps centred on `biome()`'s own
edges" already went for materials. **The scatter should ramp the way the
splat does.**

### 9.5 · Their occupancy test is a physics query. Ours is a bit. Keep it.

Every candidate in every system runs `Physics.CheckBox` / `CheckSphere` /
a downward raycast against live colliders (§5). That is the tax for having no
slot model, and it is why the fill loop needs an attempt budget at all. Our
`server/slot.rs` bitset answers the same question in one bit with no world
query — the wall-4 "bounded everything" discipline paying out. Worth stating
in `TERRAIN.md` §2 as a *reason* for the design, not just a description of it.

### 9.6 · The trick they used exactly where we would have: seed the RNG per cell

`ResourceDepositManager.CreateFromPosition` — the survey-charge/quarry
deposit under a 20 m cell — **saves the global RNG state, re-seeds from a
hash of (cell index, `World.Seed + World.Salt`), draws the whole deposit,
and restores the state.** Deterministic per (seed, cell), forever, inside a
stateful engine.

They knew the technique. They applied it to the one system where a player can
re-ask the same question about the same square metre and must get the same
answer — and did not apply it to trees, because trees are entities and
entities are already networked. **The line between "must be a pure function"
and "may be state" in their codebase is exactly the line between "the client
must be able to derive it" and "the server tells you".** Ours is drawn in the
same place and much further over, and this is the cleanest confirmation in
the file that the shape is right.

### 9.7 · Two operator verbs we are missing

`spawn.report` (current/target per population) and `spawn.scalars` (the four
live scalars) are how their operator sees whether the world is full. We have
`ALPHA.md`'s staged arming and no equivalent census. A `slots` console verb —
per biome and archetype: standing / harvested / next respawn — is small, is
purely diagnostic, and is the thing anyone will want the first time a shard
feels empty. Not queued here; noted.

## 10 · Numbers of record

**Authority column is load-bearing.** `code` cannot be overridden. `default`
is a serialized field's class default and the shipped asset may differ —
`FINDINGS.md` §1's warning about trusting a payload's shape applies to
trusting a default's value.

| thing | value | authority |
|---|---|---|
| population tick | 60 s | convar `spawn.tick_populations` |
| individual tick | 300 s | convar `spawn.tick_individuals` |
| density scalar | 0.5 → 1.0 by player fraction | convars `spawn.min/max_density` |
| rate scalar | 0.5 → 1.0 by player fraction | convars `spawn.min/max_rate` |
| group rate base / scale | 100 players / 2× | convars `spawn.player_base/_scale` |
| spawns per tick per population | ≤ 100 | default (`MaxSpawnsPerTick`) |
| attempts per object, initial fill | 20 | default (`SpawnAttemptsInitial`) |
| attempts per object, repeat fill | 10 | default (`SpawnAttemptsRepeating`) |
| position attempts per candidate | 15 | code |
| character spawn-point attempts | 60 | code |
| distribution resolution | `NextPow2(worldSize × 0.25)` | code |
| character-spawn resolution | `NextPow2(worldSize × 0.5)` | code |
| local density grid | 20 m cells (400 m²) | code |
| local density cap | `max(ClusterSizeMax, 400 m² × 2 × density)` | code |
| placement raycast height | 25 m | default |
| radius clearance check | 5 m | default |
| cluster size / dithering | 1 / 1 / 0 | default (assets override) |
| target density unit | per km² (`× 1e-6`) | code |
| decor grid / jitter | 10 m / ±5 m | default |
| monument separation / attempts / candidates | 500 m / 10 000 / 10 | default / code / code |
| monument priority weight | `(priority + 1)⁴` | code |
| acceptance test (worldgen) | `factor² ≥ rand` | code |
| group population / per-tick / delay | 5 / 1–2 / 10–20 s | default |
| visibility grid | fixed `cellCount²`, cell = `gridSize / cellCount` | default (32 / 100) |
| visibility radius | 2 cells (13-cell diamond) | default |
| group switch hysteresis | 20 m | default |
| reserved group IDs | 0 = global, 1 = limbo | code |
| world size | 4000 default, clamped 1000–6000 | code |
| seed / salt fallbacks | 123456 / 654321 | code |

## 11 · Still open — logged, not queued

Placement and respawn are mined out; §9 is the residue and `NOW.md` carries
the one item worth building. What follows is everything else this research
touched and did not finish, so nobody re-derives it.

### 11.1 · The gather verb — three deltas, all content-shaped

Adjacent to placement, not part of it: `ResourceDispenser`, `TreeEntity`, and
`OreResourceEntity` own what happens *after* the swing lands. **We already
have the headline mechanic and ours is better** — their bonus-hit marker is a
spawned `tree_marking` **entity** per marked tree plus a `ClientRPC` per hit;
our `gather::weak_mark8` is a hash of (seed, cell, player, hit count) through
the yaw LUT, so the mark costs no entity, no bytes, and derives identically on
server, replay, and any future client ghost. Keep that.

Three real differences remain, and **all three are numbers and curve shapes,
so they belong in `CONTENT.md` and `DECISIONS.md` §open, not in code**:

- **Their bonus ramps and punishes; ours is flat and forgiving.** A tree
  pays `1 + clamp(0.125 n, 0, 1)` — up to **2×** over eight consecutive
  marker hits. An ore node pays `1 + clamp(0.5 n, 0, 2)` — up to **3×** over
  four, and **a miss resets the streak to zero**. Ours pays a flat
  `weak_pct` on any weak hit, and `ws_hits` only ever increments (the chase
  restarts on switching nodes, never on a miss). So a wrong-sector swing
  costs us nothing but the one bonus, where it costs them the whole ladder.
  Two decisions in there — *does the bonus escalate*, and *does a miss
  break it* — and the per-archetype curve difference (2× over 8 for wood,
  3× over 4 for ore) says they tuned it per resource, not globally.
- **Their node is a fixed pool; the tool sets speed, not total.**
  `containedItems` is a list that depletes, and a swing takes
  `gatherDamage / maxHealth × itemShare` out of it — so a tree is worth N
  wood no matter what you swing at it, and a better tool only empties it
  sooner. Worth checking whether our per-tool yield row times hits-to-exhaust
  makes tool tier a **total-yield multiplier** instead, because those two
  compound very differently over a wipe and only one of them is an economic
  anchor. Not asserting we got it wrong — asserting nobody has decided it
  on purpose.
- **`finishBonus`** — a bonus pile paid only if the node was finished by
  *gathering*, with at most `maxDestroyFractionForFinishBonus` (0.2) of it
  destroyed by non-gather damage. An explicit "chop it, don't blow it up"
  incentive. We have no equivalent and may not want one; logged because it
  is the kind of thing that is invisible until someone asks why explosives
  do not farm.

### 11.2 · Topology as a cross-system veto — moved to `TERRAIN.md`, and corrected

The durable fact is §4's: a monument writes `Monument` topology into the map
at worldgen, so **every population that excludes `Monument` avoids monuments
without one line of code knowing monuments exist.** Cross-system veto through
a shared spatial channel, no system naming another. That much is real and
worth having.

**The first cut of this section then prescribed their mechanism, which is the
mistake this whole file exists to avoid.** It said to build "a mask channel
sampled like `moisture`" and called it free. It is neither. A raster is the
right answer *for a game with a map file to bake into and dozens of monuments
to keep out of each other's way*; we have one ring and one pad, and a handful
of seed-derived control points distance-tested per sample is very likely
cheaper and keeps `TERRAIN.md` §0 intact. Pre-building the channel now would
be committing to Rust's answer to a question our architecture may answer
differently — with **zero producers to put in it**, since stages 7–8 do not
exist.

What was real underneath it is a constraint on those stages, not a mechanism,
and it now lives in **`TERRAIN.md` §1 after stage 9**, which is the doc that
owns worldgen and the one that will actually be open when the road gets
built: `terrain.rs` has no state today, the pad is a global argmax and the
road needs distance to a warped coastline, so both need something derived
once at init and queried — bounded, warmup-only, and bit-identical across
native and wasm. Nothing to do here until stage 7.

### 11.3 · What the source cannot answer, and where it would come from

- **The actual densities.** `TargetDensity`, `SpawnRate`, cluster sizes and
  filters live in Unity `ScriptableObject` assets, not code, and are in no
  decompile — so "how many trees on a 4 km map" is not answerable here, only
  the formula is (§3.3). Their content/code split is our wall 7, which is
  exactly why. If we ever want to calibrate our 8–12k slot band against
  theirs, the cheap route is a published `spawn.report` dump from a live
  server, not more source.
- **The `ProceduralComponent` roster and its order.** Read off the scene
  hierarchy at runtime; only `PlaceDecorUniform` and `PlaceMonuments` are
  confirmed by name. The ordering *mechanism* (§2) is confirmed, the roster
  is not. Matters when POIs land, not before.
- **Whether any of it still holds.** Protocol 179 is old and at least one
  signature has moved since (`SpawnHandler.SpawnGroups` is now
  `List<ISpawnGroup>`). This file claims the **architecture** — four systems,
  split by networked-or-not — not the field list.

### 11.4 · Re-fetching, if a later pass needs the source again

Nothing here is cached in the repo and the working copies were scratch, so a
future pass starts from zero. The classes that carried the answers, so nobody
has to search for them twice: `SpawnHandler`, `SpawnPopulation`,
`SpawnDistribution`, `SpawnFilter`, `ByteQuadtree`, `Spawnable`,
`SpawnGroup`, `SpawnIndividual`, `BaseSpawnPoint`, `SpawnPointInstance`,
`WorldSetup`, `World`, `Prefab`, `PlaceMonuments`, `PlaceDecorUniform`,
`TerrainCheck`, `TerrainFilter`, `TerrainAnchor`, `WaterCheck`,
`NetworkVisibilityGrid`, `ResourceDepositManager`, `ResourceDispenser`,
`ResourceEntity`, `TreeEntity`, `OreResourceEntity`, `CollectibleEntity`,
`LootContainer`, `ConVar/Spawn`. §0's terms apply to every one of them.
