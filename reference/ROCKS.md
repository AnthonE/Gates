# reference/ROCKS.md — how the reference game weaves rock into its world

**Owns nothing** — research, not law, the posture every `reference/*.md`
holds. `TERRAIN.md` still owns placement, `ART.md` is still the bar,
`CONTENT.md` §4's bands still decide whether a number may land, and every
number in §9 is a proposal for `DECISIONS.md` §open, not a knob.

Written because the operator asked (2026-09-13) how the reference game
*"weaves in 3D ground objects with the world — giant rock faces and other
boulder-like creations"*, what the devblogs said, and what a plan around
**biome seeding** looks like — and, in the same breath, what we do wrong or
are missing. §7 is what ours measures, §8 is that list, §9 is the plan.

**This is not `SPAWN.md` or `FORESTS.md` again.** `SPAWN.md` owns the four
placement *systems* and is cited rather than restated; `FORESTS.md` owns the
tree. This one is only about **rock**: the cliff, the formation, the boulder,
the small rock, the clutter stone, and the ore that the reference hangs off
all of them — what decides that a rock goes *here*, in *this* biome, at
*this* size, and how the mesh is made to belong to the ground it stands on.

---

## 0 · Provenance

**Tier 1, fetched whole, 2026-09-13, from this box.** `rust.facepunch.com`
and `wiki.facepunch.com` both answered 200 with full page text on the first
probe — one more entry for `SOURCES.md` §0's log, and the rule it states is
unchanged: *probe*, because the box is the variable. Devblogs 50, 53, 54,
55, 58, 59, 62, 63, 68, 91, 103, 105, 109, 128, 134, 140, 196, 197, the
World Revamp post (2021), World Update 2.0 (2024) and Surviving a Decade
(2024) are the primary text; the wiki's Topology, Terrain, Ore Nodes and
Procedural Generation Customization pages are the reference tables.

⚠ **One weakening, stated because the other files here do not carry it.**
The pages were fetched whole, but the quotations below were *extracted from
them by a summarising prompt* ("quote verbatim every paragraph about…"),
not read end to end by a person. Every quoted sentence is the page's; the
*selection* is a summariser's, and a sentence it did not surface is not in
this file. That is stronger than `DOORS.md`'s search summaries and weaker
than `DURABILITY.md`'s read-whole tables. Treat a quote as exact and an
absence as unproven.

**Tier 3, for the inventory only:** the public prefab index
(`Cool9978/Rust-Prefab-List`, the same file `FORESTS.md` §4 read its forest
populations off) for the folder taxonomy in §2, and Rustafied's 2024
world-update preview for two sentences about traversal. Nothing decompiled;
`SPAWN.md` §0's terms are not needed here and its findings are cited by
section.

**What the sources do not answer, so nobody assumes otherwise:** any density
(how many formations a 4 km map carries), the actual rules of the component
that places a cliff beyond Devblog 54's one sentence, and whether a placed
cliff or formation paints topology under itself — §3 *infers* that last
one from the wiki's Topology table read against `SPAWN.md` §4, and marks
it inferred.

---

## 1 · The finding, first

**Their rock is a hierarchy of tiers keyed by (biome × role × size), placed
biggest-first, with the cliff as the first tier and the ore as the last.
Ours is one tier — a 2.2 m boulder — in one column of one table, drawn at
the same rate in every biome, vetoed off the one place they start, with the
ore beside it drawn independently.**

Four measurements off our own tree, three seeds, whole-island
(`crates/sim-core/examples/rock_stats.rs`, §7 has the table):

1. **Rock is weather.** 3.7–4.7 boulders per hectare in beach, meadow and
   forest alike, and only 2.4–3.0× that in the highland. The ore, by
   contrast, is 9× denser in the highland than the meadow — the ore has a
   home and the rock does not.
2. **The cliff is bare, and it does nothing to what stands beside it.**
   `scatter_in` vetoes every cell over `CLIFF_SLOPE_RATIO`, which is 15–17‰
   of the land, ~600 cells — the cell the reference puts its first tier on.
   And the cells *touching* a cliff, inside the highland so the row weight
   cannot masquerade as a cliff effect, carry **7.3 rocks per 100 cells
   against 7.4 on open highland** (9.7 vs 9.7, 6.9 vs 8.6 on the other two
   seeds). The reference's `Cliffside` topology exists to spawn ore at a
   cliff's foot; ours has no concept of a foot.
3. **The ore is no nearer a rock than a bush is.** In windows lying wholly
   in the highland, 82–95 % of nodes have a boulder within 20 m — and so do
   83–90 % of bushes and trees, which have no reason to. There is no
   coupling; there is a row that draws both.
4. **Rock does not cluster, and the fix that clustered the forest cannot
   reach it — by arithmetic.** The grove field scales the whole row, so a
   rock is clumped by the same field as a tree. But a count drawn cell by
   cell against a shared random rate has dispersion `1 + mean² · CV² /
   null`, and the mean is what the field multiplies: 6.2 trees per 40 m
   window read **2.9–3.1**, and 0.6 rocks per window read **1.1–1.3** —
   within a few hundredths of what the trees' own CV² predicts for
   something that rare (§7). No weight on the field changes that; only a
   *parent* does. That is §9.2, and it is the reason this file is a plan
   rather than a knob.

---

## 2 · The inventory: a few meshes, many populations

Read off the prefab index. Folder names are the reference's own; the
"system" column is `SPAWN.md` §1's split, by where the folder sits.

| tier | folders (`assets/bundled/prefabs/autospawn/…`) | meshes | system |
|---|---|---|---|
| **mountain** | not autospawn — artist-made prefabs "fitted into the world" (Devblog 59) | authored | A, map file |
| **cliff** | `decor/v2_cliff` (`rock_cliff_b_1..4`), `decor/v2_cliff_snow` (same four), `decor/v2_cliffmicro` (`rock_cliff_a_1..4`, `rock_cliff_c_1..4`) | 3 source meshes, `content/nature/rocks/rock_cliff_{a,b,c}` | A, map file |
| **formation** | `decor/v2_rockformation` (`rock_formation_a..i`), `decor/v2_rockformation_snow` (a..i), `decor/v2_rockformation_underwater` (a..f) | 9 | A, map file |
| **medium rock, by role** | `decor/hilltop`, `decor/hilltop_snow`, `decor/v2_rockfield`, `decor/v2_rockfield_snow` (each `rock_med_a..c`), `decor/rock-waterside` (6), `decor/rock-underwater` (6), `decor/v2_rockbeachside` (2) | 3 (`rock_med_a..c`) | A, map file |
| **small rock, by biome** | `decor/v2_temp_forest_rocks`, `decor/v2_tundra_forest_rocks`, `decor/v2_arctic_forest_rocks`, `decor/v2_arid_rocks` (each `rock_med_c` + `rock_small_a..c`) | 3 + 1 | A, map file |
| **clutter stone** | `clutter/v2_rocks_small`, `clutter/v2_rocks_small_snow`, `clutter/v2_misc_rocks_sand`, `clutter/v2_misc_rocks_crustascean`, `clutter/v2_temp_forest_rocks_small` (each `rock_small_a..c`) | 3 | the decor-slider tier (Devblog 58) |
| **ore** | `resource/ores`, `resource/ores_sand`, `resource/ores_snow` (each stone / metal / sulfur) | 3 | **B**, respawning |

Two things to read off that table before any devblog.

**2.1 · Biome seeding is a population per (biome × role), not a mesh per
biome.** `content/nature/rocks/` holds **sixteen** source prefabs — three
cliffs, nine formations, three medium, three small, near enough — and the
autospawn tree re-seeds them through **twenty-five** populations. The same
`rock_med_c` is a hilltop rock, a rockfield rock, a forest rock in three
biomes and a beachside rock, each with its own filter, density and (§5)
tint. `FORESTS.md` §4 found the same shape for trees (biome × role × size
class); for rock the size axis is even more load-bearing, because a cliff
and a clutter stone are the same substance at a 30× scale.

**2.2 · The respawning tier is the ore and only the ore.** Every rock is
system A — in the map file, seed-derived, static for the wipe — and the
ore is system B, a `SpawnPopulation` with a count and a tick (`SPAWN.md`
§3). So "rock" in the reference is *terrain*, the way our scatter is, and
the ore is the one living thing hung off it. Ours draws both from one
hash in one pass, which is fine, and puts them in the same *row*, which is
§1's point 3.

The wiki's Topology table is the veto/spawn vocabulary the populations read
(`FORESTS.md` §3 has the forest half; this is the rock half, verbatim):

| topology | what it does |
|---|---|
| **Cliff** | "Blocks everything from spawning" |
| **Cliffside** | "Spawns ore nodes and blocks junkpiles from spawning" |
| **Clutter** / **Decor** | "Spawns harvestable ores and decorative rocks (Unconfirmed)" |
| **Mountain** | "Prevents animals, ores and decor from spawning in this area, adds loot barrels (Unconfirmed)" |
| **Offshore** | "Spawns underwater clutter, rock formations and sunken ships with loot" |
| Summit | "Reduces Tree Spawns (Unconfirmed)" |

`Cliff` is a veto and `Cliffside` is a spawn, in one table — the cliff mesh
keeps everything off itself and makes its own foot an ore site. §4 is what
that buys.

---

## 3 · A cliff is a mesh placed on the heightfield, not the heightfield

Devblog 54 (2015-04), Andre, the sentence this whole file hangs on:

> "On the asset side of things I added proper cliffs, micro cliffs, and
> overhangs to the procedural map. Cliffs are placed wherever the terrain
> falls off very steeply. Micro cliffs are added to fields and hillsides
> that fall off slightly, and overhangs can mostly be observed on beaches."

Three placements, one law each, keyed on **how steeply the ground falls** —
our stage 5 slope mask, read three ways instead of one. The terrain under a
cliff stays a smooth heightfield; the rock *structure* is the mesh's, and so
is the collider: Devblog 109 records "a loading time regression related to
non-uniform collider scaling of cliff meshes", which is only a sentence you
can write about a cliff that is a mesh with a collider, scaled per instance.

What they kept changing about it, in order:

- **55**: "Reduced the number of overhang rocks that are placed on beaches"
  — the third placement was over-dense on arrival.
- **105**: "Better cliff mesh placement and more variety"; **109**: "Micro
  cliffs can no longer spawn on roads" and loading-time work "mostly cliff
  and monument" — a cliff is placed *after* the road and must be vetoed by
  it, which is `SPAWN.md` §4's topology veto again.
- **196** (2018-01): "Procedural maps now have a lot more cliffs and hills,
  particularly in the temperate and arctic biomes, and the hilltops have
  been visually redesigned with additional clusters of massive rocks" —
  changelog "Added hilltop rocks"; **197**: "the new hilltop rocks and
  cliffs don't interfere with monuments and blend nicer with the terrain",
  and "Fixed hilltop rocks sometimes overlapping monuments" — the
  `MONUMENTS.md` §2 collision list, with rock as the offender.
- **World Revamp** (2021-05): "New generation cliff placement and cliff
  visuals" alongside "new cliffs and rocks visuals".
- **World Update 2.0** (2024-10), Damian Lazarski: "We have completely
  reworked all cliffs in the game. The new cliffs come in two types, tall
  coastal cliffs placed close to the ocean and sloped inland cliffs
  covering hills and mountains. A lot of effort was made into making them
  not only look realistic, but also easy to traverse. In addition we have
  added many new coastal rocks close to the shore to further spice up the
  world." Rustafied's preview of the same patch (tier 3): "they come with
  plenty of spots where you can jump up or scale your way to the top", and
  the new canyons "are sporting some of the snazzy new cliff faces".
- **Surviving a Decade** (2024-01), still open after nine years:
  "Procedurally generated caves, additionally, a new biome, we'll be
  exploring and improving cliffs and adding more variation of rock
  formations."

So the cliff is the tier that was **never finished** — placed in 2015,
re-placed in 2016, thickened in 2018, re-placed and re-drawn in 2021,
split into two kinds in 2024, and named as open work the same year. Two
readings, and both matter for §9.3: it is the highest-value rock object
they have (it is where the verticality and the base spots are), and it is
the one whose placement is a *terrain* question rather than an art one,
which is why it kept moving with the terrain generator.

**Inferred, not read:** a placed cliff paints `Cliff` topology under
itself (and `Cliffside` around it), the way `SPAWN.md` §4 records a
monument painting `Monument` — nothing fetched says so in those words, but
`Cliff` "blocks everything from spawning" is meaningless unless something
paints it, and the wiki's own advice for custom maps ("Use Building
topology to prevent grass/decor from spawning on underground entrances")
is the same mechanism spoken aloud. Where the ground under a cliff mesh is
made rock (`Rock` splat), the mesh and its ground agree by construction.
That is `FORESTS.md` §2's "Forest topology requires Forest splat" for rock.

**Ours, for contrast:** there is no cliff mesh. The cliff *is* the
heightfield past `CLIFF_SLOPE_RATIO` (`terrain.rs:28`), its normal is the
field's analytic gradient (`tests/contour.rs` holds it C¹ on purpose), and
the one thing that marks it is `splat_from`'s cliff term forcing the `rock`
identity (`terrain.rs:3210`) — which since 2026-08-27 is `Gravel004`,
scree, doing the cliff face's job because it was the right choice for
alpine *ground* (`NOW.md` §0gs item 3). A cliff here is a smooth-shaded
ramp painted with crushed aggregate, at a slope a body cannot climb
(`movement`'s walkable threshold is the same ratio). It reads as a hill
that happens to be grey.

---

## 4 · The hierarchy, and the ore at the bottom of it

Devblog 91 (2015-12), Vince, on the ideal generation order:

> "large rock formations spawn first > then boulders in close proximity >
> then average and smaller rocks in that proximity."

Three tiers, each placed *relative to the one above it*. That is not a
density; it is a **parent–child** rule, and it is what puts a boulder field
at the foot of a formation instead of a boulder every 200 m. The cluster
work before it was tuning the same idea by hand: Devblog 50, "bigger and
more condensed rock clusters"; 54, "I also increased the number of rock
clusters and improved the way they are placed"; 55, "Ores spawn in smaller
clusters again for a more even distribution" — note that the ore was
already *clustered* in April 2015, and the number being tuned was the
cluster size, not the count.

Then the ore was hung off the rock explicitly, and the reason is stated in
player terms:

- **103** (2016-03), Andre: "Ore nodes and smaller trees should really
  spawn around rock formations to give players clear indicators for where
  they can go to gather theses types of resources."
- **105** (2016-04): "Now ores are only spawning around other rock
  formations, so when you want to find them you can look out for a huge
  rock formation from far away, go there and check it for resources." Same
  post: "Mountains let other objects spawn around their lower parts" and
  "Better rock cluster vegetation".
- **109** (2016-05): "Ores spawning around rock clusters turned out to work
  pretty well, so I'm now doing something similar for mushrooms."
- The wiki's Ore Nodes page, present tense: "They are most commonly found
  around cliffs, mountains and other rock formations."

Read those with the Topology table. `Mountain` *prevents* ores and decor
(the big prefab's body is empty) while its lower parts spawn them;
`Cliffside` *spawns* ore nodes; `Cliff` blocks everything. **The
formation is a landmark, the landmark's foot is the resource, and the
chain is built out of placement order rather than out of a map marker.**
It is the same mechanic our coast road is for barrels — a thing you can
see from far away that tells you where the loot is — and we built it for
the road and not for the ore.

`SPAWN.md` §9.6 is worth re-reading beside this: the survey-charge
deposit is the one system they made a pure function of `(seed, cell)`.
The ore *node* is not — it is a respawning population — but its
*distribution* is a filter over topology the formations painted. So the
landmark is deterministic (system A) and the ore that finds it is not
(system B), and the chain still reads to a player because the parent never
moves.

---

## 5 · Weaving: how a mesh is made to belong to its ground

The operator's actual question, and it is five mechanisms, not one.

**5.1 · The anchor searches for altitude.** Devblog 54: "The terrain
anchoring system now looks for the perfect placement altitude in every
attempt, which means it guarantees that no potential fits are missed."
`MONUMENTS.md` §9.2b already took this for the two sites (the minimax
datum, `site_floor_y`); a rock has the same problem at a smaller scale, and
`archetype_lift` + `SINK_M` (`props.rs:1206`, 6 cm) is our fixed answer to
a question they solve per instance.

**5.2 · The terrain-blend shader — the mesh's base takes the ground's
colour.** Devblog 53, Diogo: "We can now have smooth transitions from
terrain to any mesh which essentially helps make objects look naturally
integrated with the terrain." 54: "added Diogo's new terrain blending".
62: "a long overdue shader enhancement to the terrain blending quality,
resulting in crisper transitions" — and, the same post, Andre: "Our rock
shader was starting to really be a strain on performance for a lot of
people. To counteract that I switched any rocks on the procedural maps
that don't need full terrain blending to a simplified shader." 134:
"Optimized terrain and terrain-blended rock/cliff shaders; now 2x faster",
"up to 2-4 ms per frame" on a GTX 960. So it is a real per-frame cost, it
was paid for a decade, and it was *rationed* — full blending on the rocks
that need it, a cheap shader on the rest. `ART.md` rule 2 ("nothing sits
ON the ground; everything sits IN it") names the same defect; our answer
is the clutter skirt (`TERRAIN.md` stage 10b) and the 6 cm sink, and
`RENDER.md` §R lists the contact term as not built.

**5.3 · Grass and decor grow on the rock.** 54: "Grass and décor
vegetation can now grow on top of some of the rocks"; 55 fixed "some grass
placement weirdness on certain terrain blended rocks"; 62: "custom maps can
now have grass grow on procedurally spawned rocks as well". The rock's
*top* is ground for the ground population. Ours: the clutter population is
blind to props by construction (stage 10b's opening sentence) and skirts
their *feet*; nothing grows on a boulder.

**5.4 · Rock colour follows the biome.** 54: "Say goodbye to the three
static rock colors and instead enjoy smooth colour transitions based on
the biome the rock is in", and "a new material parameter system allows us
to tweak shader properties on every individual rock", plus for the ground
"a set of 4 colours per texture--one for each biome". 91, Vince: "Your
temperate biome will be white limestone cliffs and grey sands with lush
grass, tundra will tend to have darker rock and sand overall, snow with
black rock and black sand all that with a hint of blue. … Previously most
of the biomes looked like they were of the arid type." 128: "We also
enabled biome tinting on the caves". 68's changelog: "Fixed arid biome rock
terrain texture being way too dark." **The biome is a tint on one rock
mesh, not a rock mesh per biome** — which is what lets sixteen meshes
serve five biomes (§2.1). Ours: `TINT_POOL` is a four-entry *value*
multiplier keyed on the cell (`props.rs:366`, prop tint v0), grey by
design so it cannot fight the identity work; there is no term that reads
the ground under the slot.

**5.5 · Texel density is held as the rock scales.** 54: "There's also
consistently high texture resolutions no matter how big we scale the
rocks." A world-space or triplanar projection, so one rock at 3× is not a
3× blurrier rock. Our props carry object-space UVs from the generator
(`props.rs:491`, "texture tiles per metre of object space"), which is right
for a pool of fixed-size boulders and wrong the moment a tier scales.

Around those five, the ones that are about collision and cost rather than
look: Devblog 68, "Construction and deployable placement in caves and on
terrain blended rocks finally works as it should again" and "Fixed a number
of exploits that would allow players to get inside rocks" — a rock is a
place you build on, and the item-move-shaped bug class (`CLAUDE.md`'s trap
list) has a rock-shaped cousin; World Update 2.0, formations "spawn off
shore allowing for more base building possibilities than ever before";
Devblog 140, occlusion culling demonstrated "on our procedurally placed
rocks and cliffs" — the tier is big enough to occlude; Devblog 109,
"Reduced overall bush and clutter rock density" and 58's decor slider that
"blends out any decor objects that cannot be interacted with in any way and
are small enough that they don't really affect gameplay" — the clutter
stone is the tier they let the player turn down, exactly the posture our
clutter population takes (not collision, not gathered, not in
`state_hash`).

---

## 6 · The order they built it in

`WATER.md` §1 and `FORESTS.md` §5 pay off by recording sequence; this is
rock's.

1. **Clusters first, by hand** (50, 54, 55 — 2015-03/04): more, bigger,
   more condensed, then the ore's clusters made smaller again.
2. **The cliff, the same month as the blend and the tint** (54): the three
   slope-keyed placements, Diogo's terrain blending, and biome colour on
   rock all land in one post. The mesh, the seam and the palette were one
   slice.
3. **The big prefabs blended, not flattened** (55, 59, 63): mountains "no
   longer placed partially outside of the world and better blend with the
   terrain"; the blend improved "while maintaining detail in the
   transitional areas" — `MONUMENTS.md` §3, arrived at for rock first.
4. **The hierarchy stated** (91, 2015-12) — formations → boulders →
   smaller rocks.
5. **The ore hung off it** (103, 105, 109 — 2016 spring), with the player's
   reason written down.
6. **Density turned down for cost** (109) and the shader made cheaper
   (62, 134) — the tier had to be paid for before it could be thickened.
7. **More cliffs, hilltop clusters** (196/197, 2018).
8. **Everything re-drawn** (World Revamp, 2021), then **the cliff split
   into two kinds and the formations brought back** (2024), with the cliff
   still named as open work.

**Steps 2 and 4 are the two to internalize.** The cliff was not an art
pass on the mountain; it shipped with its seam and its palette in the same
breath, because a rock face that does not meet its ground is a worse frame
than no rock face. And the hierarchy was written as an *order of
placement*, which is what made the ore's landmark possible a few months
later without a marker.

---

## 7 · What ours measures, against the tree

The code first, every row with a cite, then the probe.

| fact | ours | where |
|---|---|---|
| rock tiers | **one**: `Occupant::Rock`, r 1.1145 m, top 1.5403 m, scale 0.9–1.1 | `terrain.rs:2688`, `occupant_volume` |
| cliff tier | **none** — the heightfield past tan 50° is the cliff; the cell is vetoed | `terrain.rs:28`, `scatter_in` slope veto |
| formation tier | none; `WANTED.md` has no cliff or formation row | `assets/models/WANTED.md` §2 |
| small-rock tier | none between the clutter shard (0.64 m grid, cosmetic) and the boulder | `Clutter::Shard`, `terrain.rs:3365` |
| rock weight per biome, ‰ | beach 30 · meadow 25 · forest 28 · highland 80 | `ScatterTable::alpha_default` |
| ore weight per biome, ‰ | stone 0/15/12/70 · metal 0/0/0/60 · sulfur 0/0/0/45 | same table |
| parent–child | none — one hash per cell, the row scaled by `clump` | `scatter_in`, `terrain.rs:2990` |
| mesh choice | a pool of three **by yaw**, the same three in every biome | `props.rs:1246`, `prop_models` |
| rock colour by biome | none; a 4-entry value tint keyed on the cell | `props.rs:366`, prop tint v0 |
| base blend | 6 cm sink + a clutter skirt at the footprint edge | `props.rs:1206`, `skirt_fill` |
| grass on rocks | none; the clutter grid is blind to props | `TERRAIN.md` stage 10b |
| climbable | a top is a floor only within `STEP_UP` = 0.6 m of the feet, so a 1.54 m boulder is a wall from the ground | `terrain.rs:4909`, `movement.rs:35` |
| shot | stops on a boulder (occupants are point-sampled cylinders) | `ranged.rs:340` |
| build on | a foundation seats on `terrain::ground` and tests ground slope; a rock top is not a seat | `build.rs:1219` |
| far representation | only the tree has one; a boulder draws its full mesh at every distance it draws | `props.rs:1611` |
| the cliff face's material | `rock` = `Gravel004`, scree, forced by the cliff term | `terrain.rs:3210`, `NOW.md` §0gs item 3 |
| gates on rock | none per biome; `tests/scatter.rs` holds island totals and the forest's clumping | `tests/scatter.rs`, `tests/forest.rs` |

Measured 2026-09-13 with `cargo run --release -p sim-core --example
rock_stats -- <seed>` on the three seeds `FORESTS.md` §7 used, whole-island
(the probe's tree dispersion reproduces that table's 2.925 / 3.004 / 3.071
to the digit, which is the cross-check that it stands on the same ground).

| quantity | 20260731 | `0x0047_4154_4553` | 1024 |
|---|---|---|---|
| rock / ha — beach · meadow · forest · highland | 4.5 · 4.0 · 4.7 · 9.7 | 3.8 · 3.9 · 4.0 · 11.6 | 2.3 · 3.7 · 4.2 · 10.8 |
| ore / ha — meadow · highland | 2.4 · 21.3 | 2.5 · 22.1 | 2.2 · 22.4 |
| cliff cells, ‰ of land | 17.4 (684) | 15.2 (593) | 14.9 (572) |
| cliffside cells, ‰ of land | 29.1 | 21.8 | 25.8 |
| rock on the cliff cell, per 100 | 0.44 | 0.67 | 0.87 |
| **highland only**: rock per 100, cliffside · open | **7.32 · 7.41** | **9.71 · 9.66** | **6.92 · 8.60** |
| highland only: ore per 100, cliffside · open | 14.5 · 16.8 | 15.7 · 20.7 | 17.3 · 16.8 |
| share with a rock ≤ 20 m, all-highland windows — stone · metal · sulfur | 93 · 85 · 82 % | 89 · 93 · 94 % | 94 · 94 · 95 % |
| the same for the controls — bush · tree · rock | 83 · 87 · 81 % | 83 · 88 · 88 % | 88 · 90 · 85 % |
| tree dispersion in all-forest 40 m windows (binomial null 1) | 2.925 | 3.004 | 3.071 |
| **rock dispersion**, meadow · forest · highland | **1.17 · 1.17 · 1.50** | **1.28 · 1.13 · 1.27** | **1.09 · 1.26 · 1.61** |
| shared-field prediction, `1 + mean²·CV²/null` | 1.16 · 1.18 · 1.45 | 1.15 · 1.15 · 1.54 | 1.15 · 1.16 · 1.55 |
| 3×3 windows holding ≥ 3 rocks, meadow · forest · highland | 48 · 50 · 89 | 57 · 27 · 36 | 26 · 40 · 76 |

Three sentences the table does not say on its own.

**The rock-on-the-cliff row is not zero because the mask is taken at the
cell centre and the veto at the jittered position** — a ±3 m leak, not a
rule. It is under 1 % and it is the *only* rock a cliff gets.

**The near-rock shares are what independence predicts.** Highland rock is
~7 per 100 cells; the chance of at least one in 24 neighbours at that rate
is ~82 %. The ore reads 82–95 and the controls 83–90. Island-wide the ore's
share is higher than the bush's (68 vs 56 %) only because the ore lives in
the highland and the bush does not — the row, not a coupling.

**The dispersion row is the arithmetic that decides §9.2.** A shared rate
field can only give a count dispersion of `1 + mean² · CV² / null`, and
`CV²` is the *field's* — 0.235–0.247 off the trees, the same field. At a
mean of 0.6 rocks per window that is 1.15; measured 1.09–1.28. The forest
reads 3 because there are ten times as many trees per window, not because
the field clumps trees harder. **Raising the rock weight would raise the
dispersion only in proportion to the mean** — a highland at 2 rocks per
window reads 1.5, measured 1.27–1.61 — and the cost of buying clusters that
way is a tenfold rock count. The 3×3 windows that do hold three boulders
today (26–89 per island per biome) are accidents of the draw, and there is
no place on any of the three islands where a player would say "the rocks".

---

## 8 · What we do wrong, or are missing

The operator's question, ranked by what it costs the frame and the game.
Where the answer is "by construction", the cite is the line that constructs
it.

1. **The cliff is bare, and it is a slope.** `scatter_in` returns `none`
   over `CLIFF_SLOPE_RATIO`; the reference's first tier is placed
   "wherever the terrain falls off very steeply" and its foot spawns ore.
   15–17‰ of the land, ~600 cells, is the most vertical ground on the
   island and carries nothing but scree paint. And because the cliff is the
   heightfield, it has the heightfield's smoothness: there is no overhang,
   no ledge, no shadowed face, and nothing to climb. §9.3.
2. **Rock has no hierarchy and cannot get one from the field.** §7's
   dispersion row: the grove field does to rock exactly what the arithmetic
   allows, which is nothing visible. "Large formations first, then boulders
   near them, then smaller rocks near those" is a parent–child rule and we
   have no parent. §9.2.
3. **The ore has no landmark.** Independent of rock beyond the row (§7),
   which is the state the reference found unacceptable in 2016 — "resource
   gathering mostly luck" is how the search summaries paraphrase Devblog
   103's motivation, and its own words are "clear indicators for where they
   can go". We built the landmark→loot chain for barrels (the coast road,
   the bays) and not for the ore, which is the resource the whole tier
   ladder runs on. §9.4.
4. **Rock is not seeded by biome.** 3.7–4.7/ha everywhere, the same three
   meshes everywhere (chosen by `yaw`, `props.rs:1246` — `FORESTS.md` §9.3's
   species defect, for rock), no coastal rock, no waterside rock, no
   offshore rock (`LAND_MIN_H` vetoes everything under 0.6 m of land, so the
   beach row's 30‰ draws the inland boulder on the sand and nothing stands
   in the water). The reference's beach has overhangs, beachside rocks, and
   since 2024 "many new coastal rocks close to the shore" and formations
   "off shore". §9.1.
5. **One size class.** 2.2 m ± 10 %, then nothing until the clutter shard.
   The reference runs cliff → formation → medium → small → clutter, and
   §2.1 is that the meshes are cheap and the *populations* are the work.
   `Slot::scale`'s ±10 % is not a class (`FORESTS.md` §8 gate 5, same
   sentence).
6. **Rock colour never meets the ground.** No biome term on the rock
   material (§5.4); the value tint is deliberately grey. A boulder on the
   sand and a boulder on the highland are the same albedo, which is the
   "three static rock colors" they retired in 2015.
7. **The base is a hard intersection.** A sink and a skirt (§5.2) against a
   shader that takes the ground's colour into the mesh's base. `ART.md`
   rule 2's own example is the boulder whose meeting line "is invisible:
   grass grows up over it", and ours is visible by construction until a
   base-band blend exists — which is WGSL nobody here can boot (`§LOOK`).
8. **`rock` is one identity doing three jobs** (`NOW.md` §0gs item 3):
   alpine ground, the cliff face, and the ore prop's neighbour. Scree is
   right for the first and wrong for a face. Sized there as a fifth splat
   channel and not one pass; §9.3 offers the cheaper road — a cliff
   *object* carries its own material and the ground under it stays scree.
9. **Nothing gates any of it.** `tests/scatter.rs` holds island totals and
   the forest's clumping; no test asserts a per-biome rock quantity, a
   near-cliff rate, an ore-near-rock share or a cluster count — and
   `FORESTS.md` §8's lesson applies unchanged: an island-wide total is
   blind to structure, so every gate above is new. §9.6.
10. **The asset queue does not know.** `WANTED.md` has no cliff row and no
    formation row; §0rk's re-roll prompt says "formation" and means a
    better boulder. A cliff mesh is the one rock asset that needs the
    register decided first (`WORLD.md` §9.1: art for the wrong register is
    remade).
11. **A boulder is a wall.** Its top is a floor only within 0.6 m of the
    feet (§7), so nothing rock-shaped is climbable, and the 2024 cliffs
    were reworked to be "easy to traverse" on purpose. The box-list
    occupant shape (`boxes_ground`, the shelter's plinth) already expresses
    a stepped top; no rock uses it.

**What we do right, and must not trade for any of the above:**
reproducible from the seed (`SPAWN.md` §9.1 — theirs is not, and the map
file is the price); a bit for occupancy where theirs is a physics query
(§9.5); sites resolved first and everything after them reading them as an
input (`MONUMENTS.md` §9.5); a mesh that must fit the volume the sim blocks
(`tests/greybox.rs`) and a selection step that reads its target out of
`sim-core` (`ci/measure_glb.py`) — so every tier proposed below arrives
measured; and the grove field itself, which is the right mechanism for the
one occupant dense enough to feel it.

---

## 9 · What it means for us — the plan around biome seeding

### 9.1 · Biome seeding is a kind per row, not a count per row

The reference's answer to "which rock stands here" is a population per
(biome × role × size) over a small shared mesh set, tinted by biome. In our
terms — where **the mix IS the splat** is already the law for the clutter
population and the scatter row (`TERRAIN.md` stage 9/10) — that is: the
biome row carries rock *kinds*, and the kind picks the pool. Concretely:

- `Occupant::Rock` becomes a size-classed family — a small rock, the
  boulder, a formation — each with its own volume row (`occupant_volume`),
  its own per-biome weight, and its own `measure_glb.py` band (`ART.md`
  rule 8 already splits node from formation on shape and value; the small
  rock and the cliff each need a row of their own).
- The **pool is chosen by the biome, not the yaw**: a `Slot` field drawn
  from the same cell hash the yaw comes from, so the client keeps mirroring
  for free and `sim-core` can gate it. This is the identical slice
  `FORESTS.md` §9.3 asks for species — **do both in one commit**, because
  each widens `Slot` and moves the golden, and a golden moves once or
  twice.
- The beach row draws coastal kinds, and a **waterside** kind is allowed
  below `LAND_MIN_H` down to a stated depth — the one veto exception this
  plan asks for, priced honestly: `movement`'s swim rule, the road's
  `LAND_MIN_H` reuse and `spawn_ring_lands_on_a_clear_beach` all read that
  line, so it is its own slice with its own gate and not a weight edit.
- The highland row draws formations and hilltop clusters; forest and meadow
  draw the small rock and the rockfield boulder.

None of it is a number yet. The weights are `DECISIONS.md` §open rows when
built, and `test_no_biome_row_saturates` bounds what any row may spend.

### 9.2 · The hierarchy as a pure function

The reference's rule is a *sequence* — formations first, then boulders near
them — and a sequence is state (`SPAWN.md` §9.3's whole argument). §7 says
the field cannot substitute for it at rock's density. The shape that keeps
`scatter` a function of one cell:

- **A parent draw on a coarse grid.** One hash per 64 m cell (the chunk,
  `TERRAIN.md` §6's "one grid, everywhere") decides formation-or-not and a
  jittered position, weighted by the highland row and vetoed by the sites,
  the road and the water exactly as a slot is. Bounded by construction:
  ≤ 1 formation per coarse cell, ≤ 1,024 per island.
- **Children read the parent, never the reverse.** A fine cell asks its
  four nearest coarse cells "is there a formation, and how far" — four
  hashes, no state, `in_bay`'s trick (never locate, only test against) —
  and scales its boulder, small-rock and ore weights by a kernel of that
  distance. The kernel's tail is squared (`SPAWN.md` §9.4), so a cluster's
  edge is ragged rather than a ring.
- **Totals are conserved, not raised.** The bay-slots pattern
  (`TERRAIN.md` stage 7): the same ~1,050 boulders and ~750 nodes, *moved*,
  so `CONTENT.md` §4's node bands, `tests/haven.rs`'s prize ratio and the
  live-slot band all hold without a tolerance moving. That is what keeps
  this a placement slice and not a balance pass — `RIPLIST.md` §0's threat
  frame is the reason to be careful: their ore is priced for contested
  farming, and an ore that gathers at a landmark is *more* contested, which
  is the intended direction and a spoken call.
- **The cliff foot is a parent too.** `Cliffside` — a fine cell within one
  cell of the mask — raises its ore weight from the same conserved pool.
  `ground_slope` is already resolved in `scatter_in` for the veto; the
  neighbourhood test is ≤ 8 more taps against a memo that answers most of
  them (`Lattice`).

Cost: one coarse hash channel, a golden move, a wipe
(`DECISIONS-ARCHIVE.md` 2026-08-10: a worldgen change is a wipe). Wall 2
is untouched (nothing runs in a tick), wall 1 is the usual walled float
set, wall 4 is the ≤ 4 + ≤ 8 tap bound stated at the definition.

### 9.3 · The cliff tier

The mask exists (stage 5), the cell is free (the veto empties it), and
Devblog 54's three placements are three **slope bands** on a number
`scatter_in` already holds: cliff over `CLIFF_SLOPE_RATIO`, micro-cliff in a
band below it ("fields and hillsides that fall off slightly"), overhang on
the beach. One `Occupant::Cliff` with its size from the band.

Two routes, and only one of them is honest:

- **Render-only cladding cannot protrude.** The sim's wall *is* the
  heightfield, so a drawn face proud of the slope is walk-through and a
  face inside it is invisible. What render-only can do is a *material* on
  the slope (§8 item 8's cliff face), which is worth having and is not a
  rock.
- **A slot with a volume.** `Occupant::Cliff` publishes a box list derived
  from the cell's own slope — a slab lying on the face plus one or two
  ledges — through `boxes_block` / `boxes_ground`, which already give a
  stepped top a body can stand on (the shelter's plinth). That is "easy to
  traverse" for free: a ledge within `STEP_UP` of the one below it is a
  climb. The mesh is then fitted *inside* the volume under
  `tests/greybox.rs`'s equality gate like every prop, and `measure_glb.py`
  gains a cliff row. The ground under it keeps the cliff splat, which is
  the reference's `Cliff` + `Rock` pairing arriving by construction.

Price, stated: an `Occupant` variant (the `render/verbs.rs`-shaped trap —
`cargo clippy -p client --features render --all-targets` before believing a
green workspace), the golden moves (cliff cells now return a slot), a wipe,
and one proof to keep: the slab must stay inside `CELL_SIZE` so
`OCCUPANT_PROBE_CELLS = 1` remains complete — a cliff wider than a cell is
two cliff slots, not a bigger one. **Build the slot with the greybox
massing now and source the mesh late** (`WORLD.md` §9.1) — the massing is
what the gate measures against either way.

### 9.4 · The ore at the bottom

§9.2's kernel and cliff-foot term, applied to the three node kinds from the
same conserved pool. Two things it must not do: change any node's *count*
per biome (the bands), or change what a node *pays* (`content/`). What it
changes is where a player *looks*, which is Devblog 105's sentence and is
free of both walls.

### 9.5 · The look, in the order the reference did it

1. **§0rk first.** A re-rolled node and formation on straight normal maps
   is the precondition for seeing any of this.
2. **Tint by biome** (§5.4): a term on the rock material read from the
   splat under the slot — the four identity weights already ride the
   ground as an attribute — bounded, and needing `ART.md` §3's sign-off
   because it is a hue and prop tint v0 chose value-only for a reason.
3. **The base blend** (§5.2): a base-band albedo blend toward the ground's
   colour, rationed the way they rationed it — on the formation and cliff
   tiers that need it, not on every clutter stone. WGSL, so `§LOOK`.
4. **The cliff material**: `gravel`'s bundle row is the obvious face
   (`NOW.md` §0gs item 5) and the cliff object of §9.3 is where it goes
   without a fifth splat channel.

Grass *on* rocks is not owed: the skirt answers rule 2 at the foot, and a
population on a prop's top is a new population.

### 9.6 · The gates, each with its mutant

Every one is arithmetic over `terrain::scatter`, microseconds, no clock,
no GPU — and each must be proven red under the mutant that removes the
thing it claims (`CLAUDE.md`'s `lattice.rs` entry).

1. **Per-biome rock contrast** — highland rock/ha over meadow rock/ha ≥ N,
   today 2.4–3.0. Red under a uniform weight.
2. **The cliff foot** — cliffside rock and ore per 100 cells over open
   highland ≥ N, today 1.0. Red under the kernel removed.
3. **Ore near formation, against the bush** — the node's near-parent share
   minus the bush's ≥ N points, today 0. Red under a kernel that scales
   every kind alike.
4. **Clusters exist** — 3×3 windows with ≥ 3 boulders per island ≥ N, and
   highland rock dispersion ≥ 2 (today 1.3–1.6, and §7 says no weight can
   get there). Red under the parent draw disabled.
5. **Totals conserved** — live slots, boulders and nodes per biome within
   the bands the current tree holds. Red under a raise.
6. **The cliff slot** — `greybox.rs`'s equality between the box list and
   the drawn mesh, and `OCCUPANT_PROBE_CELLS`'s completeness const-asserted
   against the slab's reach.

### 9.7 · Ranked, and what is not owed

1. **§0rk** — already queued, nothing here is worth looking at before it.
2. **Rock kind and species into `Slot`** (§9.1, `FORESTS.md` §9.3) — one
   golden move for both.
3. **Rock seeding v0** (§9.2 + §9.4 + gates 1–5) — the sim slice; the
   operator's, because it is a wipe.
4. **Cliff tier v0** (§9.3 + gate 6) — the slot and the massing; the mesh
   when the register is decided, and the generator for it already exists:
   `ci/rock_kit.py gen --kind slab` (2026-09-14), refused by the triage's
   boulder depth band until `measure_glb.py` has a slab row.
5. **Tint and base blend** (§9.5) — client, `§LOOK`.
6. **Waterside and offshore rock** (§9.1's veto exception) — its own
   slice, after 3.

**Not owed:** any number reaching `content/` or code (every weight above
is a `DECISIONS.md` §open row when it lands); a mesh purchase before the
register is spoken; a pixel gate — the cliff's *look* is a person's
(`CLAUDE.md`), and everything above it is arithmetic.

---

## 10 · Still open — logged, not queued

- **How the cliff component actually chooses** is one sentence (Devblog 54)
  and a changelog line (105). Whether it reads a slope threshold, a
  topology bit, or the terrain normal per candidate is not in anything
  fetched, and `SPAWN.md` §11.4 names the classes a later pass would open
  (`PlaceDecorUniform`, `TerrainAnchor`, `TerrainCheck`) under its own
  terms. §9.3 does not need the answer: our mask is our own.
- **Which system the clutter stone is in.** The folder says `clutter`, the
  slider says cosmetic; whether it is system A or client-side decor was not
  read. Ours is settled either way (the clutter population).
- **Whether the formation should be a site.** A buildable, climbable
  landmark with an ore footprint is closer to `SiteFootprint` than to a
  boulder. v0 is a slot because a slot is what the scatter grid can hold in
  one commit; it gains a footprint the day something reads one
  (`MONUMENTS.md` §9.2's rule).
- **Densities.** No page reached says how many formations or cliffs a map
  carries. `SPAWN.md` §11.3's answer stands: a `spawn.report` dump from a
  live server, not more source.

---

## Sources

All fetched 2026-09-13 unless noted.

- [Devblog 50](https://rust.facepunch.com/news/devblog-50) — "bigger and more condensed rock clusters", biomes by latitude and altitude
- [Devblog 53](https://rust.facepunch.com/news/devblog-53) — "smooth transitions from terrain to any mesh"; the legacy map's "new rocks"
- [Devblog 54](https://rust.facepunch.com/news/devblog-54) — cliffs, micro cliffs, overhangs and where each goes; Diogo's terrain blending; grass on rocks; biome-based rock colour; the terrain anchor searching for altitude; texel density under scale
- [Devblog 55](https://rust.facepunch.com/news/devblog-55) — mountains blend with the terrain; fewer beach overhangs; ores in smaller clusters; grass on terrain-blended rocks
- [Devblog 58](https://rust.facepunch.com/news/devblog-58) — the decor density slider
- [Devblog 59](https://rust.facepunch.com/news/devblog-59) — artist-made mountain prefabs fitted into the world; blending "while maintaining detail in the transitional areas"
- [Devblog 62](https://rust.facepunch.com/news/devblog-62) — the rock shader's cost and the simplified shader for rocks that "don't need full terrain blending"; crisper transitions
- [Devblog 63](https://rust.facepunch.com/news/devblog-63) — Procgen 8: better prefab mountain and monument blending
- [Devblog 68](https://rust.facepunch.com/news/devblog-68) — construction on terrain-blended rocks; exploits inside rocks; arid rock too dark
- [Devblog 91](https://rust.facepunch.com/news/devblog-91) — "large rock formations spawn first > then boulders in close proximity > then average and smaller rocks"; rock colour per biome
- [Devblog 103](https://rust.facepunch.com/news/devblog-103) — ore nodes and smaller trees around rock formations, "clear indicators"
- [Devblog 105](https://rust.facepunch.com/news/devblog-105) — "ores are only spawning around other rock formations"; better cliff mesh placement; mountains let objects spawn around their lower parts
- [Devblog 109](https://rust.facepunch.com/news/devblog-109) — ores around rock clusters "turned out to work pretty well"; micro cliffs off roads; cliff collider scaling; densities reduced
- [Devblog 128](https://rust.facepunch.com/news/devblog-128) — biome tinting on caves; the 4-way blend material
- [Devblog 134](https://rust.facepunch.com/news/devblog-134) — terrain-blended rock/cliff shaders 2× faster, 2–4 ms
- [Devblog 140](https://rust.facepunch.com/news/devblog-140) — occlusion culling on placed rocks and cliffs
- [Devblog 196](https://rust.facepunch.com/news/devblog-196) — more cliffs and hills; hilltop clusters of massive rocks
- [Devblog 197](https://rust.facepunch.com/news/devblog-197) — hilltop rocks and cliffs no longer interfere with monuments
- [World Revamp](https://rust.facepunch.com/news/world-revamp) (2021) — new generation cliff placement and visuals
- [World Update 2.0](https://rust.facepunch.com/news/world_update_2) (2024) — two cliff types, "easy to traverse", formations revamped and offshore, coastal rocks
- [Surviving a Decade](https://rust.facepunch.com/news/surviving-a-decade) (2024) — cliffs and rock-formation variation named as open work
- [Topology — Rust Wiki](https://wiki.facepunch.com/rust/Topology) — Cliff, Cliffside, Clutter/Decor, Mountain, Offshore
- [Terrain — Rust Wiki](https://wiki.facepunch.com/rust/Terrain) — the alpha layer, prefabs owning collision
- [Ore Nodes — Rust Wiki](https://wiki.facepunch.com/rust/Ore_nodes) — "most commonly found around cliffs, mountains and other rock formations"
- [Procedural Generation Customization — Rust Wiki](https://wiki.facepunch.com/rust/procedural_generation_customization) — `cliff` and `rock_formation_` as blacklistable prefab names
- [Cool9978/Rust-Prefab-List](https://github.com/Cool9978/Rust-Prefab-List/blob/main/prefabs.md) — the autospawn folder inventory in §2 (tier 3)
- [Rustafied, world update preview](https://www.rustafied.com/updates/2024/9/12/world-update-preview) (2024-09-12) — climbable cliffs, canyons (tier 3)
- `reference/SPAWN.md` §1–§5, §9 — the four systems, the check chain, the squared acceptance; `reference/FORESTS.md` §2–§4, §8 — the mask chain and the per-biome gate lesson; `reference/MONUMENTS.md` §3, §9.2 — blending as masks. Not re-derived here
- `crates/sim-core/examples/rock_stats.rs` — §7's numbers, re-runnable
