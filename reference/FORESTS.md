# reference/FORESTS.md — how the reference game seeds a forest

**Owns nothing** — research, not law, the posture every `reference/*.md`
holds. `TERRAIN.md` still owns placement, `ART.md` is still the bar, and
`CONTENT.md` §4's bands still decide whether a number may land.

Written because the operator looked at our forest and said it is not very
good, and asked what the gates should be doing about it. §7 is what ours
measures, §8 is the answer to the question, §9 is the design behind it.

**This is not `PLANTS.md` again.** That doc surveys how *plants are
generated* — space colonization, ez-tree, impostors — and its §0 says in
terms that it takes nothing from the reference game. This one is only about
the reference game and only about **placement**: what decides that a tree
goes *here*, that a forest ends *there*, and that the thing at the edge is a
different plant. Where it needs `SpawnHandler`'s internals it cites
`SPAWN.md` §3 rather than restating them.

---

## 0 · Provenance — and a correction to the standing caveat

**Tier 1, fetched whole.** `rust.facepunch.com` devblogs and
`wiki.facepunch.com` pages were retrieved in full on **2026-09-09**, not as
search summaries. That matters because it **contradicts `DOORS.md` §0's
caveat, which `NETWORK.md` and `VOICE.md` both inherited in full** — *every*
`rust.facepunch.com` fetch blocked by this box's proxy. It is not blocked
today.

This is `SOURCES.md` §0's own warning arriving for the third time, and the
rule it states is the one to keep: **reachability is a property of the
container, not of the hosts — probe, do not trust either claim.** Both the
"everything 403s" reading and the "they are open" reading were honest
measurements on different boxes on different days. `DURABILITY.md` got the
same result off `wiki.facepunch.com` on 2026-08-15 and is the strongest
provenance in this directory for the same reason.

Nothing here is decompiled. The sources are Facepunch's own devblogs, the
official wiki's Topology and Terrain pages, a public prefab index, and the
convar documentation the server-hosting community maintains.

**What the sources do not answer, stated so nobody assumes otherwise:** no
page reached here says *how the Forest splat is painted in the first place* —
what noise, what threshold, what shape. The chain from splat down to a
planted tree is documented (§2); the top of it is not. Every "forest shape"
claim below is therefore about **what consumes the mask**, never about how it
is drawn. `SPAWN.md` §11.3 is the same shape of admission.

---

## 1 · The finding, first

Their forest and ours differ in one structural way, and it is not the tree.

**A Rust forest is composed of several populations reading several masks;
ours is one occupant reading one weight.** They vary species, size class,
understory and edge *across space*, and each of those is a separate thing
with its own filter. We vary a single per-mille draw between four biomes.
That is why theirs reads as a place and ours reads as a texture.

Three measurements off our own tree, all seed-independent (§7):

1. **Our "Forest" biome is below the international threshold for the word.**
   Using each species' radius *ceiling* as if it were its crown — which
   overstates the answer — the shipped Forest biome reaches **~7.0 % canopy
   cover**. FAO's definition of forest is 10 %.
2. **The 8 m cell, not the weight table, is what caps it.** One occupant per
   64 m² is 156 stems/ha. A closed canopy (40 % cover) needs ~225 stems/ha at
   our own crown radii. **No weight anyone can write reaches a closed forest
   on this grid** — it is short by 1.44×, before the overstatement above.
3. **Our forest floor is emptier than our meadow.** `ScatterTable::
   alpha_default` gives Forest a bush weight of **50‰ against Meadow's 70‰**
   (`terrain.rs:2745`). Devblog 67's entire forest change was the opposite
   move — *"Made forests appear thicker by adding more bushes."*

Point 3 is the cheapest thing on this page: it is a two-number edit, and §8's
gate 3 goes **red on the tree as it stands today**, which makes it a finding
rather than a proposal.

---

## 2 · The chain: splat → topology → filter → tree

Their placement is a pipeline of masks, and each stage is a different kind of
thing. Read downward; every step is a hard gate on the one below.

| stage | what it is | who reads it |
|---|---|---|
| **Splat** | the ground *texture* — Dirt, Forest, Grass, Gravel, Rock, Sand, Snow, Stones | the renderer, **and the topology rules** |
| **Biome** | Arid / Temperate / Tundra / Arctic / Jungle, not stackable | which *population* applies at all |
| **Topology** | ~30 named bits — Forest, Forestside, Field, Cliff, Road, Summit… | `SpawnFilter`, as a hard bitmask |
| **`SpawnFilter`** | topology mask × biome mask × a soft factor | every spawn candidate |
| **`ByteQuadtree`** | 255 × filter factor, summed-area, importance-sampled | `SampleNode`, `O(log N)` |
| **cluster + `localCap`** | `ClusterSizeMin..Max` from one leaf, capped per 20 m cell | the fill loop |

`SPAWN.md` §3 and §4 own the bottom three rows and measured them properly —
the quadtree is a **biased sampler and not a gate**, the exact re-check is
`Filter.GetFactor(pos) > 0` at the candidate, and the cluster/cap pair is
what gives the distribution its texture. Nothing here revises that.

**What this doc adds is the top three rows**, which `SPAWN.md` treated as one
opaque "the terrain question" and which are where the forest's *shape* comes
from.

The load-bearing detail is that **the ground texture and the trees are not
independently authored**. The wiki states the rule flatly: *"Forest topology
requires Forest splat in Temperate, Arid, and Tundra biomes, and requires
Snow splat in the Arctic biome."* You cannot have the trees without the
forest floor under them, because the mask that spawns the trees is only legal
where the texture already says forest. Ours are two independent derivations
off `(h, moisture, slope)` that happen to agree.

---

## 3 · A forest is not one mask. It is five, and the edge is one of them

This is the finding worth carrying. Their topology layer does not have "a
forest bit". It has a **vocabulary of forest-adjacent conditions**, each
spawning a different plant. Verbatim from the wiki's Topology table:

| topology | what it spawns |
|---|---|
| **Forest** | "Spawns Trees, Mushrooms, Berries, Logs" |
| **Forestside** | "Spawns small trees and bushes" |
| **Field** | "Spawns resource pickups, bushes, and some trees" |
| **Alt** | "Spawns Birch/American Beech trees" |
| **Summit** | "Reduces Tree Spawns" |
| Cliff / Road / Rail / Building | "Blocks everything from spawning" |
| Swamp | "Spawns swamp trees and sulfur pickups" |
| Beachside / Roadside / Railside | bushes — "dense bushes" on Railside |

Four things follow, and three of them are structural rather than cosmetic.

**3.1 · The edge is a first-class object.** `Forestside` is not a softer
Forest — it is a *different plant list*: small trees and bushes where the core
has mature trees. Devblog 32 says it was built deliberately: *"forests have a
border area that can have separate vegetation from the forest core."* This is
the sub-canopy and shrub layer `PLANTS.md` §2 says we are missing, and the
reference's answer is that **you do not get it by weighting the same cell
differently — you get it from a second mask that only exists at the
boundary.**

**3.2 · Species is spatial, not per-instance.** `Alt` is a topology bit whose
entire job is *"Spawns Birch/American Beech trees"*. Species is therefore a
property of **where you are standing**, painted as a region. §7 is that ours
is a coin flip off the slot's yaw.

**3.3 · The field is not the forest with fewer trees.** `Field` spawns "some
trees" — but those come from *different populations* (§4: `v2_temp_field_large`
and `_small`), and Devblog 162's plan for oaks was *"in fields and in tundra
biomes, as much larger trees that would be spawned rather sparsely."* So
crossing out of the woods changes the **species and the size class**, not
only the count. Ours changes one number, 260‰ → 70‰.

**3.4 · The vetoes are the same kind of thing as the spawns.** Cliff, Road,
Rail and Building are topology bits that block, sitting in one table beside
the bits that spawn. Ours are special-cased predicates inside `scatter_in`
(slope, water, road, haven). Theirs compose because they are data; **this is
the one place where our design is arguably the better one** — see §9.5.

---

## 4 · The population is keyed by (biome × role × size)

A `SpawnPopulation` is a `ScriptableObject` — prefab folder, target density,
rate, cluster parameters, filter (`SPAWN.md` §3). The part that matters for
forests is **how many of them there are and what distinguishes them**. The
public prefab index shows the autospawn folders keyed three ways at once:

```
v2_temp_forest              v2_temp_forest_deciduous_large
v2_temp_field_large         v2_temp_forest_deciduous_small
v2_temp_field_small         v2_temp_beachforest_small
v2_tundra_forest            v2_tundra_forest_small
v2_arctic_forest            v2_arctic_forest_snow
v2_arid_forest              v2_arid_cactus
```

Three axes, visible in the names: **biome** (`temp` / `tundra` / `arctic` /
`arid`), **role** (`forest` / `field` / `beachforest`), and **size class**
(`_large` / `_small`, and `deciduous` split from the default conifer). Roughly
**120 distinct tree prefabs** hang off these — pine, birch, douglas fir, oak,
palm, American beech and swamp trees a–f, each in several sizes, plus snow
variants.

Two mechanisms are worth taking and both are cheap:

- **`_large` and `_small` are separate populations over the same ground**,
  with their own densities. That is how a stand gets a size distribution
  instead of one height ± a wobble — and it is the sub-canopy layer again,
  arriving by a different route than §3.1's edge.
- **Variety is a quota, not a per-draw roll.** `UpdateWeights` deals the
  target count across the prefab variants and `GetRandomPrefab` *decrements*
  as it draws (`SPAWN.md` §3.4), so a population cannot drift to
  all-one-variant over a wipe. A per-instance hash — ours — has no such
  floor; it is right in expectation and says nothing about any actual window.

One balance consequence, because it is exactly the trap `RIPLIST.md` §0
warns about. Their size classes started as a *yield* difference and it played
badly: Devblog 162 removed *"some tiny trees from the spawn table"* and made
*"the remaining smaller trees now yield the same resources per hit as the big
trees in order to make chopping them down worth your time."* So **take the
size classes as a visual and structural mechanism and leave the yield flat**,
which is what `content/gatherables.toml` already does — its own comment says
the row is their large tree and *"the species spread is a terrain question we
do not model."* Adding size classes must not silently make that comment false.

---

## 5 · The order they built it in

The other `reference/*.md` that pay off are the ones that record the
*sequence* (`WATER.md` §1's surface → optics → motion → reflections → foam).
Theirs, from the devblogs:

1. **Distribution before density** (Devblog 32). Trees became globally
   networked — *"they're not created in a relatively small area around you,
   they're spawned everywhere by default"* — and in the same pass forests got
   *"a lot bigger and a bit less dense."* Coverage first, then thinning.
2. **The edge, immediately after** (Devblog 32, same post). The border area
   with its own vegetation is not a late polish item; it lands in the same
   breath as the distribution.
3. **The understory** (Devblog 67). *"Made forests appear thicker by adding
   more bushes."* Thickness was bought with the shrub layer, not with stems.
4. **The meshes, and only then more density** (Devblog 162 era). Old
   SpeedTree-store trees were replaced with custom optimized meshes, and the
   optimization is what *paid for* the density: the new meshes "allowed an
   increase in tree density by a good margin."
5. **The renderer's ceiling, last** (the Performance Update). Impostors, an
   atlas merge halving draw calls, and the Max Tree Meshes cap.

**Step 4 is the one to internalize.** Density was not a slider they turned
when the forest looked thin — it was the *dividend* of making a tree cheaper.
That is the same coupling `PLANTS.md` §6.2 argues for from the other
direction (distribution, then measure, then LOD), and it says the order
plainly: **you buy stems with triangles, so the budget work comes first and
the density number is what you spend the winnings on.**

---

## 6 · Their budget is a cap by construction, ours is a measurement

The Performance Update's tree work is one idea worth copying exactly.

A graphics option limits the number of **independently rendered tree meshes
regardless of how many trees there are**; everything past the limit is
"forced ... to be rendered as billboards, only rendering the trees closest to
the camera as meshes." The impostor renderer was then rebuilt to "maximize
hardware batching potential," which "essentially eliminated all CPU cost" —
after impostors had themselves become the CPU bottleneck once view distance
went up.

The shape is: **rank by distance, cap by count, bill the remainder to the
cheap representation.** The cap is a constant; the forest's density cannot
break it, because the cap is not a function of the forest.

Ours is distance-only. `tree.rs:964` builds two `VisibilityRange`s around
`TREE_LOD_SWAP_M = 80.0` (`tree.rs:870`), so **a clump inside 80 m has no
ceiling at all** — which is precisely the case clumping is designed to
produce, and `PLANTS.md` §6.2 already noted that better distribution makes
the budget problem worse. `tests/tree.rs` prints the p90 arithmetic so it
cannot be forgotten; printing is not a bound.

---

## 7 · What ours measures, against the tree

Measured 2026-09-09 with `cargo run --release -p sim-core --example
terrain_stats` on three seeds, whole-island (0..2048 on both axes — the
window `sim-core/tests/relief.rs` exists to stop anyone getting wrong again).

| seed | forest cells | trees | bushes | 40 m all-forest window | dispersion |
|---|---|---|---|---|---|
| `0x0047_4154_4553` | 16,321 | 5,581 | 2,268 | mean 6.279 | 3.004 |
| `20260731` | 14,972 | 5,136 | 2,236 | mean 6.169 | 2.925 |
| `1024` | 11,423 | 4,617 | 2,181 | mean 6.277 | 3.071 |

**The clumping half is genuinely done**, and this is the part to not break: a
dispersion of ~3.0 against a closed-form independent-draw null of 1.0, with an
empty-window share of 2.7 % against the null's 0.07 %. Groves and clearings
exist and are gated (`sim-core/tests/scatter.rs`).

The density arithmetic, from those windows and our own constants:

| quantity | value | where |
|---|---|---|
| 40 m window, all-forest | 6.17–6.28 trees / 1,600 m² | measured, 3 seeds |
| **forest stem density** | **38.6–39.2 stems/ha** | derived from the above |
| crown radius, conifer / broadleaf | 1.70 m / 2.90 m (**ceilings**) | `tree.rs:113` |
| mean crown area at the 50/50 yaw split | 17.75 m² | `props.rs:2037` |
| **canopy cover, Forest biome** | **≤ 7.0 %** | overstated: radii are ceilings |
| grid ceiling, 1 occupant / 64 m² | 156 stems/ha → **≤ 27.7 % cover** | `terrain.rs:23` |
| stems/ha needed for 40 % cover | ~225 | **above the grid ceiling** |
| forest : meadow tree weight | 260‰ : 70‰ = **3.7 : 1** | `terrain.rs:2745` |
| forest : meadow **bush** weight | 50‰ : 70‰ = **0.71 : 1** | `terrain.rs:2745` |
| species pool | 2 species × 3 seeds | `tree.rs:79`, `tree.rs:113` |
| species selection | `slot.yaw % pool`, **client-side**, both rings | `props.rs:2037`, `props.rs:1977` |
| size classes | 1 (height per species, scale 0.9–1.1) | `terrain.rs:2764` |
| forest edge | **none** — no boundary concept exists | `terrain.rs:2802` |
| LOD switch | distance only, 80 m | `tree.rs:870`, `tree.rs:964` |

Two of these are worth stating as sentences because they are not obvious from
the table.

**Species is not a sim fact.** Both rings pick the mesh variant as
`slot.yaw as usize % pool` — the near one at `props.rs:2037`, the impostor
ring at `props.rs:1977`. Yaw is drawn per cell for rotation, so species is
a coin flip uncorrelated with everything — a birch and a pine stand side by
side with no reason, and no gate in `sim-core` can assert anything about
species at all, because the sim does not know. §3.2 is that theirs is a
painted region.

**`PLANTS.md` is stale in our favour on two rows, checked rather than
trusted.** It says the pool is "one species at three seeds" and that the
billboard LOD is queued; the tree has `SPECIES` with two entries
(`tree.rs:113`) and `impostor_of` landed (`tree.rs:1028`, 1.94 M → 510 k on
the p90 ring). Fixing that doc is not this doc's job, but the row is flagged
in §9.6 — and this is the drift `CLAUDE.md` names at the top of the file: the
command is the claim, not the paragraph.

---

## 8 · What the gates should do

The operator's question. Seven, ordered by what they would have caught, and
every one is arithmetic over `terrain::scatter` — no clock, no pixels, no
GPU, which is what `CLAUDE.md` says a frame may be gated on.

The honest baseline first, because a gate proposal that misstates what exists
is the failure mode this repo has paid for repeatedly. **What is gated
today:** `sim-core/tests/scatter.rs::test_scatter_density_preserved` holds
*island-wide* live slots in 8,000–12,000 and trees above 1,000, on four
seeds; `test_scatter_clusters` holds the dispersion against a closed-form
null; `terrain_golden` pins the values. **What none of them assert:** any
per-biome quantity whatsoever. Every gate below is new, and the reason they
are all missable by the current set is the same — **an island-wide total is
blind to how it is distributed between biomes**, which is exactly the fact
this doc is about.

**Gate 1 — canopy cover per biome, not stem count.** Assert the realized
cover fraction (stems × crown area ÷ biome area) per biome lands in a stated
band. This is the number that decides whether the word "forest" is true, it
is a pure function of scatter plus a radius our own code publishes, and it is
the one that would have surfaced §1's 7 % without anyone looking at a frame.
A stem count cannot do this job: it moves under `CELL_SIZE`, under crown
radius, and under the species mix, and stays green through all three.

**Gate 2 — the contrast, not the average.** Assert `forest_density /
meadow_density ≥ N`. A player learns a biome by the transition, and the
current gate's island-wide band is satisfied by a uniform smear — raise
meadow trees and drop forest trees by the same count and every existing gate
stays green while the forest stops existing. This is the same defect shape as
`loot_storm.rs`'s ring saturation: a total that is right about the whole and
silent about the structure.

**Gate 3 — the understory is denser inside the forest than outside it.**
Assert forest shrub-layer occupancy > meadow's. One line, and it is **red on
the tree today** (50‰ vs 70‰, §1). Devblog 67 is the whole argument for
which direction it should point.

**Gate 4 — the edge exists.** Assert that cells within N m of a
forest/meadow boundary carry a different occupant mix from forest-core cells.
**This gate is unwritable until the mechanism lands** (§9.2) — named here so
the feature arrives with its gate rather than after it, which is the rule
that made `event_roles.rs` worth writing.

**Gate 5 — size classes.** Assert a forest window's height histogram covers
≥ K classes. Today it is 1, and `Slot::scale`'s ±10 % is not a class. This
gate is what stops §4's `_large`/`_small` idea from being implemented as a
wider wobble.

**Gate 6 — species distribution, which cannot be gated yet and that is the
finding.** No `sim-core` test can assert anything about species while
selection lives at `props.rs:2037` on the client. Either species moves into
the `Slot` (drawn from the same cell hash, so the client keeps mirroring it
for free) or this gate is impossible forever. **Naming the precondition is
the deliverable here** — the gate is one assert once species is a sim fact,
and no amount of client-side testing substitutes.

**Gate 7 — the frame budget as a cap, not a print.** Assert that the count of
mesh-LOD trees in the draw ring cannot exceed a constant, by construction. §6
is that ours is distance-only, so the bound does not exist; `tests/tree.rs`
prints p90 arithmetic, and a print is not an assert. This one must land
**before** any density increase, because raising density is precisely what
breaks it — and per §5 step 4, the budget work is what pays for the density
in the first place.

Gates 1–3 and 5 are writable today against `terrain::scatter` and cost
microseconds; gate 3 is red now. Gates 4, 6 and 7 each name a mechanism that
must exist first, which is the useful half of proposing them.

⚠ **One warning that applies to all of them, from `lattice.rs`'s entry in
`CLAUDE.md`: run the mutant.** A gate over a distribution is easy to write so
that it passes on anything — a band wide enough to hold both the current tree
and the defect is worse than no gate, because it reads as coverage. Each
band above must be proven red under a deliberate move of the weight it
claims to hold.

---

## 9 · What it means for us

### 9.1 · The ceiling is the whole story, and it now has a number

`PLANTS.md` §3.2 said the ceiling was the open half and listed three ways to
raise it. §1.2 prices it: **a closed canopy is unreachable on the 8 m grid at
any weight**, short by 1.44× before the crown-radius overstatement. So the
choice is not "should we raise the ceiling" but which of `PLANTS.md`'s three
options to buy, and its own costs still stand — a second occupant per cell
breaks `gather::cell_key` (wire, save, client mirror), and halving
`CELL_SIZE` quadruples the live `SlotLives` rows against `TERRAIN.md` §6.

**Nothing in this doc changes those costs, and this doc does not pick.** What
it adds is that "leave it" — option 3 — is now a decision with a stated
consequence rather than a shrug: it means our Forest biome stays parkland,
and the word forest stays aspirational. That belongs to the operator, in
`DECISIONS.md` §open, before anyone touches a weight.

### 9.2 · The edge is the cheapest structural win and it is a sim-core slice

§3.1 is the mechanism we are missing that costs the least to add: a
forest/meadow boundary distance, and a different occupant mix inside it. It
needs no new occupant kinds if it draws bush and small-tree weights from the
existing table, it is a pure function of the cell (the same `clump` field
already gives every cell a view of its neighbourhood), and it is what makes a
treeline read as a treeline instead of as a density gradient.

It reddens `test_terrain_golden` and `test_replay` by design — a regenerate,
not a break, and a wipe of every existing world, which is the same price
`PLANTS.md` §6.5 already flagged for clumping.

### 9.3 · Species must become a sim fact before it can mean anything

§7 and gate 6. Today species is `yaw % pool` on the client, so it cannot
correlate with biome, cannot be gated, and cannot ever produce §3.2's painted
`Alt` region — a birch stand is not expressible. Moving the species draw into
`Slot` (from the same `cell_hash` channel the yaw comes from) costs one field,
keeps the client mirroring for free, and unlocks both the gate and the
mechanic. Do it before adding a third species, not after.

### 9.4 · Take the size split; leave the yields flat

§4. `_large`/`_small` as separate populations is how a stand gets structure,
and Devblog 162 is the evidence that varying *yield* with size played badly
enough that they flattened it. `content/gatherables.toml`'s comment about the
species spread being "a terrain question we do not model" becomes false the
day size classes land — update it in the same commit or it joins the dead
citations `CLAUDE.md` opens by warning about.

### 9.5 · One place where ours is better, and should not be traded away

Their vetoes are topology bits (Cliff, Road, Rail, Building) that compose
because they are data. Ours are predicates inside `scatter_in`. That reads
like a gap and is not: `SPAWN.md` §9.1 and §9.5 already established that our
placement is **reproducible from the seed** where theirs is not, and that our
occupancy test is a bit where theirs is a physics query. A data-driven veto
table is worth having for composability, but not at the cost of making
placement a stateful sampler — wall 1 forbids the relaxation loop and the
determinism is what lets the client mirror the scatter instead of being told
it.

### 9.6 · Ranked, and what is not owed

1. **Gate 3** — one line, red today, no design question attached.
2. **Gates 1 and 2** — the two numbers that would have caught this whole doc.
3. **Species into `Slot`** (§9.3) — unblocks gate 6 and the `Alt` mechanic.
4. **The edge** (§9.2) — the cheapest structural win, with gate 4.
5. **Gate 7 and the LOD cap** — required *before* any density rise (§5.4).
6. **The `CELL_SIZE` decision** (§9.1) — operator's, and not a code task.

**Not owed by this doc:** any number reaching `content/`. Every band in §8 is
a proposal and belongs in `DECISIONS.md` §open before it lands, per
`CLAUDE.md`'s knobs-are-spoken rule. And `PLANTS.md`'s two stale rows (§7)
want a one-line correction when someone next edits that file — not a cleanup
pass of its own.

---

## Sources

All fetched 2026-09-09 unless noted.

- [Topology — Rust Wiki](https://wiki.facepunch.com/rust/Topology) — the ~30-bit topology table quoted in §3, and the Forest-requires-Forest-splat rule
- [Terrain — Rust Wiki](https://wiki.facepunch.com/rust/Terrain) — the layer stack (splat / biome / topology / alpha) and what each controls
- [Devblog 32](https://rust.facepunch.com/news/devblog-32) — globally networked trees; *"forests have a border area that can have separate vegetation from the forest core"*; bigger and less dense
- [Devblog 58](https://rust.facepunch.com/news/devblog-58) — forests that "dynamically grow, get chopped and regrow according to forest areas and biome information specified by the artist"; the decor density slider
- [Devblog 67](https://rust.facepunch.com/news/devblog-67) — *"Made forests appear thicker by adding more bushes"*; Procgen9
- [Devblog 162](https://rust.facepunch.com/news/devblog-162) — tiny trees removed from the spawn table; small trees' yield flattened to match large
- [The Performance Update](https://rust.facepunch.com/news/the-performance-update) — Max Tree Meshes, the billboard fallback, the rebuilt impostor renderer, the atlas merge
- [Procedural Generation Customization — Rust Wiki](https://wiki.facepunch.com/rust/procedural_generation_customization) — the world-config fields (biome and topology-tier percentages, prefab black/whitelists)
- [Cool9978/Rust-Prefab-List](https://github.com/Cool9978/Rust-Prefab-List/blob/main/prefabs.md) — the autospawn folder names and species inventory quoted in §4
- `reference/SPAWN.md` §3–§4 — `SpawnHandler`, the `ByteQuadtree`, `ClusterSizeMin..Max`, `localCap`, `SpawnFilter`. Not re-derived here
