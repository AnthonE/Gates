# Gates · TERRAIN.md — the island (v0.1)

> Extends `DESIGN.md` §2/§9 and rides `NETCODE.md`'s laws. No monuments in
> alpha — but the island still has to *play* like a survival map: loot
> routes, buildable flats, chokepoints, and biomes that mean something.
> Knobs marked **(knob)**.

## 0 · The contract

**The whole island is a pure function of the seed.** `h(seed, x, z)` and
every scatter decision derive from integer hashes in `sim-core`, compiled
native (server truth) and wasm (client render + prediction). Nothing about
the terrain is ever networked except *changes to it* — a harvested node, a
felled tree — which ride the chunk-epoch event stream like any structural
fact. A 2 km island costs **zero bytes** to join; the seed is in the join
bundle.

Determinism rules (the same ones that make prediction bit-exact):
restricted float ops only (`+ − × ÷ sqrt min max floor-by-cast`), gradient
directions from a fixed 8-entry table indexed by hash — no trig, no libm.
The hash is splitmix64 on `(seed, cell_x, cell_z, channel)`. CI pins a
golden grid: fixed seed → xxh3 of 64×64 sampled heights + the first 256
scatter results, asserted equal native vs wasm (rides `test_parity_wasm`).

## 1 · The generation pipeline (all in sim-core, per sample or per chunk)

Stages, in order — each cheap, each deterministic:

1. **Continent mask** — radial falloff from island center, domain-warped by
   low-frequency noise so the coastline is bays and headlands, not a
   circle. Below sea level = sea floor (walkable, shallow shelf near
   shore).
2. **Base relief** — 5-octave fBm gradient noise (quintic smoothing,
   hashed table gradients). Amplitude ~90 m **(knob)**.
3. **Domain warp** — one warp pass over the relief lookup; this is the
   single biggest "looks procedural → looks natural" purchase and costs
   two extra noise reads.
4. **Height remap curve** — a fixed LUT that flattens mid-elevations into
   **buildable shelves** and steepens the transitions. This curve *is* the
   game design: it manufactures base spots and the cliffs between them.
   Ridged-noise blend above the treeline fakes an eroded look without
   simulating erosion (hydraulic erosion is a post-alpha toy, not a need).
5. **Masks** (derived, not stored): slope from finite differences → cliff
   mask (slope > ~50° **(knob)**: unclimbable, unbuildable, distinct
   material); beach mask (height within ~2 m of sea level); moisture =
   an independent low-freq noise channel.
6. **Biomes from (height, moisture, slope)** — alpha ships four:
   **beach** (spawn zone, barrels wash up), **meadow** (buildable, sparse
   trees, hemp), **forest** (wood, cover, low visibility), **highland**
   (stone/ore nodes, exposure, weather later). Each is a material blend +
   a scatter table, nothing more — biomes are data.
7. **The coast road** — the monument-less loot route: a ring offset ~40 m
   inland from the coastline, flattened a few meters wide, dirt material,
   **barrel spawn slots along it**. It does what Rust's roads do — pulls
   players out of their bases into a circulation loop where they meet —
   with zero monument art. Junk piles at bay mouths get slightly denser
   slots **(knob)**.
   **Landed** as `terrain::road_band`, and the constraint block below turned
   out not to bind: the road needs no memo at all, because the ring is never
   located, only tested against. A sample is on the road iff the shoreline
   crossing lies in a window around the point `ROAD_INLAND_M` seaward along
   its own outward radial — three `height` taps, one for most of the island,
   and it tracks the wobble exactly rather than approximating it with control
   points. Scatter clears the carriageway and draws barrels on the shoulder.
   **The denser bay slots landed, as a redistribution rather than a raise.**
   `terrain::in_bay` reuses stage 7's own trick — never locate the coastline,
   only test against it: probe `height` at the sample's own shoreline radius
   on the two bearings `BAY_SPAN_YAW` either side, and both land means the
   coast curves around water here. Two taps, no march, no memo, paid only on
   the shoulder. The shoulder then draws at `ROAD_BAY_BARREL_PERMILLE` in a
   bay and `ROAD_OPEN_BARREL_PERMILLE` outside it, set so the island-wide
   mean stays on `ROAD_BARREL_PERMILLE`: **239 barrels islandwide under the
   old flat rate, 236 under the split**, so the route pays what it always
   paid and only *where* moved. Conservation is the point, not thrift —
   `tests/haven.rs`'s `HAVEN_PRIZE_RATIO_MIN` prices the pad against the
   shoulder it replaces, so inflating the road to make bays interesting
   would have spent the destination's lead to buy it. Measured over four
   seeds: **25–39% of the ring sheltered, in 2–5 contiguous arcs**, bays
   carrying **2.46–2.76×** the open coast's barrels. `tests/road.rs` gates
   the coherence separately from the density, because a classifier that
   answered in speckle would still pass a density ratio and would still be
   unlearnable; per-cell parity substituted for `in_bay` gives 14–17 arcs
   against the cap of 8.
   **Still open**: the flattening (it needs a mask inside `height` — that is
   the representation decision the block defers, and nothing forced it yet)
   and the dirt material. `DECISIONS.md` §open "coast road v0" and "bay slots
   v0" have the knobs and the measured numbers.
8. **The haven pad** — deterministic placement: score candidate sites on
   the road ring by flatness + coast distance, take the best, carve a
   flat pad with a smooth blend radius. (This is also the monument hook:
   later POIs are exactly "carve pad + exclusion zone + scatter table" —
   the machinery exists in alpha, the art doesn't.)
   Landed as `terrain::haven(seed) -> Haven`, the memoized argmax the
   constraint block below anticipated: `HAVEN_CANDIDATES` bearings, each
   marched seaward to the *first* shoreline crossing and stepped back
   `ROAD_INLAND_M` — the road's own center-line definition inverted, so the
   site is on the ring by construction. Resolved at `World::new`, passed
   into `scatter` rather than resolved there. **The exclusion zone is built,
   and as of 2026-08-16 the carve's SEAM is built while the cut itself is
   not** — `SITE_STAMP_STRENGTH = 0.0`, so the ground has not moved and no
   golden did either. What changed is that arming it is now one constant
   instead of a cross-lane change: the split is by ROLE rather than by call
   site — solvers (`haven`, `road_band`, `spawn_pos`, the determinism probe)
   keep reading `height`, consumers (`movement`, `terrain_mesh`, `ranged`,
   `build`/`deploy`, the placement ghost, and the `y` an authored object is
   seated at) read `terrain::ground`, and `sim-core/tests/height_roles.rs`
   holds that rule as a scrape. The counts this paragraph used to quote were
   wrong in both directions and the real shape is smaller than either: 65
   `height` reads in all, 31 of them inside `terrain.rs`, and only ~18
   consumers to convert.
   ⚠ **Two things arming still waits on, one of them found by building it.**
   The strength is the operator's (a worldgen change is a wipe), and
   `WAYSTATION_RADIUS_M` **must widen first** — the canopy is footed out to
   10.46 m against an 11.0 m mask, so carving that site puts most of the
   structure on the blend ramp and makes its footing *worse* (measured
   1.795 m → 1.889 m over 16 seeds, against the haven shelter's
   1.374 m → 0.063 m). `terrain.rs`'s const block turns an armed carve into
   a compile error while that holds.
   ⚠ **`Haven::relief` is not the number the carve fixes**, which this file
   implied for months by quoting the two together. It is a rosette at
   `HAVEN_RADIUS_M` — exactly `HAVEN_FOOTPRINT.scatter_m`, where the stamp
   has faded to nothing by construction — so it stays near 3.76 m however
   deep the cut is. The carve's own measure is the spread over the floor it
   makes (`SiteFootprint::stamp_m`); `Haven::relief` still publishes how flat
   the site was *found*, which is what the argmax selected on.
   **The scatter table — the third of the hook — is now built, and it is the
   part the carve does not block.** `HAVEN_CRATES = 5` containers stand on a
   `HAVEN_CRATE_R_M = 10.0` ring, at authored positions rather than drawn
   ones: `reference/SPAWN.md` §6 records that the reference hangs monument
   loot on hand-placed child spawn points with no spacing rule, so a
   destination reads as arranged where scatter reads as weather. They are
   `Occupant::CrateSlot`, whose container is `content/loot.toml`'s
   `loot.crate` — a table that until now had no spawn site in the world at
   all. That is what makes the pad outpay the route: **2.64× the container
   density of the road shoulder, holding a container worth 2.31× a barrel in
   expected items, five of them reachable nowhere else.**
   **Placing something on the pad made the pad's own selector stricter,
   which is the general lesson for every later POI.** Two checks joined the
   candidate chain, both because the gate caught them and neither by review:
   a site's whole ring must be on land (seed 555555 centred a pad at 0.69 m
   on a shore shelf whose ring is under the land line at *every* radius), and
   no container may stand on the carriageway (the pad is on the road by
   construction, so `HAVEN_PHASE_TRIES = 16` rotations of the ring are tried
   and the site is refused if none is clear). `SPAWN.md` §5's placement-check
   chain, arrived at from the other direction: **refuse the position, never
   patch the object.** The carve's seam is built and the cut is dark; see
   above and `DECISIONS.md` §open "site carve v0".
   **A greybox now stands on it, and it stands BESIDE the road rather than
   on it.** The clearing stopped being self-evidently made the moment stage 9
   gave the island natural clearings — 2.2–4.1% of forest windows are empty
   by design now — so arrival needed a positive silhouette rather than an
   absence of trees. One `Occupant::HavenShelter` slot carries the whole
   structure (a cell holds one slot, and five containers on a 10 m ring
   already spend the pad's packing budget at the 11.31 m cell diagonal), and
   the sim blocks it as fourteen boxes: a 6.2 m room with a 2.4 × 2.8 m
   doorway, corner posts overrunning the roof, and a tower to 9.2 m against a
   6.6 m pine. It stands `HAVEN_SHELTER_R_M` off center in a gap in the ring,
   on a bearing searched against `road_band` — because the pad's center IS
   the road's center line, and `tests/road.rs` caught the first draft
   blocking the loop it exists to serve.
   **The sim owns those fourteen boxes too** — `terrain::SHELTER_BOXES`.
   ⚠ **It is no longer a checked mirror of the client's.** `ci/haven_shelter.mjs`
   held all 14 × 6 fields equal across the language seam and **was deleted
   with the browser client with nothing replacing it**; the drawn mesh is now
   nine hand-written rows in `render/props.rs` against the sim's fourteen.
   See §7.1 for what that costs and `NOW.md` §0x for the gate that is owed.
   The plinth is floor rather
   than the fourteenth wall (`SHELTER_FLOOR_IX`), or the building seals from
   outside. **It is called now** — `movement.rs:158` → `occupy.rs`'s `blocks`
   → `terrain::slot_blocks`, so these walls stop a body. What is still missing
   is narrower than the sentence that stood here: `combat.rs` carries no
   occupant term, so a shot passes through a wall that stops a body; and
   `collide::piece_ground` (`collide.rs:339`) reads built pieces only, so the
   plinth is a kerb you sink into rather than a floor. `DECISIONS.md` §open
   "haven pad v0", "haven crates v0", "haven shelter v0" and "shelter volume
   v0" have the knobs and the measurements.
   **The hook is now used, and the second tier cost no new search.** The
   argmax above scores `HAVEN_CANDIDATES` bearings and keeps one; the other 63
   passed the same land and road checks and lost on flatness by centimetres.
   `WAYSTATIONS = 2` **waystations** are the best of those losers — recorded
   by the scan that was already running, so no extra shoreline march, no extra
   bisect and no extra `height` fan — each standing `WAYSTATION_CRATES = 2`
   containers on a 6.5 m ring inside an 11 m exclusion zone, at least
   `WAYSTATION_MIN_SEP_M = 600 m` from the pad and from each other.
   **Its containers are their own kind, `Occupant::CacheSlot`, and that is
   what lets the tier have its own price.** They were the pad's `CrateSlot`
   for a day and therefore drew `loot.crate`: per container the lesser tier
   paid exactly what the destination paid, and only geometry separated them.
   A container's KIND is the only thing a loot table is selected by
   (`bake.rs`: the name is content, the index is `loot::LOOT_*`), so the tier
   needed its own kind before it could have its own price —
   `content/loot.toml`'s `loot.cache`, between the shoulder's barrel and the
   pad's crate on rolls, on expected items and on swings to open.
   **What makes it a tier rather than more crates is a gradient, and it is
   const-asserted both ways**: the lesser tier in aggregate pays less than the
   one destination (`WAYSTATIONS × WAYSTATION_CRATES < HAVEN_CRATES`, which is
   what fixes the count at two), *and* it pays less per square metre. The
   second half is not decoration — the first draft used a 10 m zone and two
   crates in 314 m² beat five in 804 m², so the site with fewer containers was
   the better square metre and a player optimizing loot-per-walk would have
   skipped the haven. Every count was right; only the assert found it. The
   radius is now derived from the inequality (`> √(2·256/5) = 10.12 m`) rather
   than chosen. `tests/waystation.rs` measures all three tiers in one unit —
   pad > waystation > shoulder — and `DECISIONS.md` §open "waystations v0" has
   the knobs.
   **Density is not the quantity a player collects, though, and gating only
   density was the hole.** `ci/haven_prize.mjs` now states the same gradient
   in **expected items per site** — container count × what that container
   pays — against the shoulder each site stands on and therefore replaces:
   the pad 15.4× that opportunity cost, a waystation 5.7×, and the whole
   lesser tier still under the one destination. A yields edit moves that
   number; it could not move containers per m².
9. **Scatter pass** — per 8 m cell, one hash draw decides occupant
   (tree / stone node / metal node / sulfur node / bush / rock / barrel
   slot / nothing), plus jittered offset, yaw, and scale from the same
   hash. Densities come from the biome table, **scaled by the grove field**
   (below). Slope and road/haven/water masks veto. Result: a deterministic
   **slot list** both sides can enumerate for any chunk.

   **The grove field — why the draw is per-cell and the forest still is
   not.** One hash per cell decides that cell alone, and independent draws
   are white noise: no groves, no clearings, uniform speckle at whatever the
   biome weight says. Stage 6 asks forest for "wood, cover, low visibility"
   and an orchard is not cover. Measured, conditioning on the forest biome
   so biome structure could not be mistaken for clumping: the variance of
   the tree count in a 40 m window was **1.05 / 1.03 / 0.98× the
   independent-draw null** on seeds 0 / 1 / 7, with **3 windows in 10,000**
   empty. `reference/SPAWN.md` §9.3 names this the highest-value defect in
   that file, and the reference's own fix — clusters drawn from one quadtree
   leaf, braked by a local density cap — needs a stateful sampler we cannot
   have without giving up the property the constraint block below is about.

   So the clumping moves out of the sampler and into the **weight**: a cell
   still decides alone, but it decides against `biome_weight × clump(seed,
   x, z)`, a 3-octave field at a 96 m base wavelength shared with its
   neighbours. Groves where it is high, clearings where it is low, one extra
   fBm read, still O(1) per cell and still no state. The factor is squared
   (`SPAWN.md` §9.4) so a grove edge is a soft tail rather than a contour
   line, and normalized to island-mean 1 so the field **redistributes**
   density without spending §6's live-slot budget. It scales the whole biome
   row, not the tree entry — a clearing is a clearing, not a clearing with
   the rocks left standing — and it sits below the road and pad branches,
   because a shoulder barrel is drawn at the road's own rate and a pad crate
   is authored. Neither is weather.

   Result in the same units as the defect: dispersion **2.90–3.34**, empty
   forest windows **37–52×** the null, live slots within 2.3% of the counts
   they replaced. Gated by `tests/scatter.rs`, which computes the binomial
   null in closed form rather than remembering a number. Knobs and the full
   measurement set: `DECISIONS.md` §open "scatter clumping v0".

   **That last line is now closed, and the fix was to stop classifying.** The
   residual here read "`biome()` is still a hard classifier, so a biome
   boundary is still a step in *composition* even though density now ramps
   across it" — a pine forest ended on the `moisture > 0.05` contour while
   the turf under it faded across `SPLAT_MOIST_BAND`, so the props and the
   ground they stood on disagreed about where the forest was, and the props
   were the half that stepped. `scatter` no longer picks a row: it blends all
   four by the ground's own splat weights (`terrain::scatter_row`), because
   `splat_from`'s channels are sand · grass · forest-litter · rock and
   `Biome` is Beach · Meadow · Forest · Highland — the same four identities
   in the same order. Stage 10's sentence for clutter, **the mix IS the
   splat**, is now true of the prop population too: one law, three
   populations.

   It costs no taps (`h`, moisture and slope are all already resolved by
   `scatter`'s own vetoes) and no density — live slots moved by at most 35
   of ~9,800 across the four gate seeds, because a blend redistributes what
   a classifier partitioned. Measured: the worst per-sample jump in the tree
   weight across a moisture sweep is **4 per-mille against the classifier's
   190**, the full Meadow→Forest span it used to move at one sample; and
   **10.2–11.8% of land cells** sit in a band where no single splat channel
   owns the cell, which is the share of the island the change can reach.
   Interiors are bit-identical by construction, and the blend is convex, so
   `test_no_biome_row_saturates` still bounds the blended row without
   knowing it exists. Gated by `tests/scatter.rs`
   (`test_scatter_mix_is_identity_in_the_interior`,
   `..._ramps_where_it_used_to_step`, `..._is_convex_and_the_island_uses_it`);
   `GOLDEN_TERRAIN_HASH` moved in the same commit and its doc says why.

**Stages 7–9 share one constraint, stated before either of the first two
exists because it decides how they get built.** Everything in `terrain.rs`
today is `(seed, x, z) → value` with **no state at all** — which is why
`scatter` runs identically in the server, the wasm client, the chunk worker
and the golden with no setup, and why stage 9 costs nothing to call 65 k
times. Stages 7 and 8 break that property: the road ring needs distance to a
domain-warped coastline, and the pad is explicitly a **global argmax** over
candidate sites. Neither is computable per sample, so both need something
derived once from the seed and then queried — still a pure function of the
seed, just memoized.

Three walls bound what that thing may be, and they are the design rather than
a review note: built at **world init, never in a tick** (wall 2); **bounded,
with its cap in `limits.rs`** (wall 4); **bit-identical native and wasm**
(wall 1 → `test_parity_wasm`), so integer or walled-float math only and no
iteration whose order a map could perturb.

What it must not be *assumed* to be is a raster. The reference game bakes a
topology bitmask into its map file and samples that, and it is right to —
for a game that already has a map file and dozens of monuments to keep out
of each other's way (`reference/SPAWN.md` §4). We have one ring and one pad,
so the small shape — a handful of seed-derived control points plus a width,
distance-tested per sample — is almost certainly cheaper, has no resolution
to choose, and keeps §0's "nothing about the terrain is ever stored or
networked" intact. **Decide it when stage 7 is built. Do not pre-build a
channel for producers that do not exist yet.**

**Decided, and the block asked the wrong question.** Stage 7 needed no memo
at all (the ring is never located, only tested against), and stage 8's memo
came out smaller than the smallest shape anticipated here: one site, four
floats, no width and no control points. Shape was never the constraint. The
constraint is the **signature** — a memo is only free while nothing inside
`height` needs it, and the moment stage 8's *carve* does, it has to reach
~50 `height` call sites across four crates. So the rule this block was
reaching for is narrower and more useful than "do not pre-build a channel":
**a worldgen stage that only vetoes can take its memo as a parameter and
cost one signature; a stage that writes `height` costs every reader of the
terrain, and that is a different size of decision.** Placement is cheap,
displacement is not. Later POIs inherit both halves.

## 2 · Slots: how living terrain stays cheap

A slot is potential, not state. The scatter defines *where a node can be*;
the **server owns one bit + one timer per slot**: harvested/standing, and
respawn-at-tick. Changes ship as chunk events (`slot_harvested`,
`slot_respawned` — a few bytes), and a chunk's slot bitset rides its epoch
state for late joiners. The client renders every standing slot from local
generation; a harvested node vanishes on the event. Trees fell to stumps
the same way. Respawn: 20–45 min jittered **(knob)**, never inside a
claimed building's privilege volume — no farming your own living room.

This is the same shape `NETCODE.md` §5 uses for buildings, which is the
point: **terrain life is just chunk events over a generated backdrop.**

### 2.1 · An authored site publishes masks, not a radius

Landed 2026-08-10; research `reference/MONUMENTS.md` §3, knob row
`DECISIONS.md` §open "site footprints v0".

The haven pad and the waystations used to carry exactly one number each
(`HAVEN_RADIUS_M`, `WAYSTATION_RADIUS_M`) answering exactly one question —
*does the scatter grid stand anything here*. Every other world system either
asked that same circle or was never told the sites exist. Ground clutter was
the second kind: `clutter_fill` had no `Haven` parameter, so grass and litter
grew straight across both tiers while the carriageway through them was
correctly grit.

`SiteFootprint` is now the site's published table — `scatter_m` (the grid
veto, asserted equal to the radius it replaced) and `swept_m` (the made
floor, derived as the container ring plus one clutter cell). Between them
`site_sweep` is a **smoothstep profile, not a circle**: consumers dither each
element against it with a hash byte they had already drawn, so the edge of a
destination is a thinning population rather than a ring on the ground. That
distinction is the whole of `MONUMENTS.md` §3 — the reference game shipped
monuments on visible circular plateaus for a decade because a footprint was a
radius — and `tests/clutter.rs` §S refuses a hard circle explicitly.

Rows this struct gains when a reader exists: build-block (open for the
operator), a height stamp (there is no carve — §1 stage 8 finds flat ground
rather than making it), nav, water.

## 3 · Collision (server truth, client prediction — same code)

- Ground: bilinear height sample under the capsule; walkable up to the
  cliff slope threshold; step-up ≤ 0.6 m; water at sea level slows to a
  swim (alpha swim = slow wade, no diving **(knob)**).
- Rocks and nodes: analytic colliders (sphere/capsule/box per archetype)
  derived from the same slot list — no mesh colliders anywhere.
- Buildings: AABB/oriented boxes per block (`DESIGN.md` §4).
- All of it lives in `sim-core`, so the wasm prediction collides with the
  exact world the server does, including the slot a node just vanished
  from (one in-flight event of skew, max).

## 4 · Rendering (per chunk)

**Written for the three.js client; the shapes carry, the threading does not.**
The native client reaches the same worldgen by direct call rather than through
a worker and a wasm view — `RENDER.md` §3 records that trade, and it deleted a
whole class of detached-buffer bug along with the worker. Where a bullet below
says "worker", read "off the main schedule" for the native path.

- **Chunk meshes**: 64 m chunks; LOD0 = 1 m grid (65×65 verts) for the
  ring around the camera, LOD1 = 2 m, LOD2 = 4 m beyond, each ring with a
  **skirt** dropped at the edges (the cheap, correct crack fix — geomorph
  is a later nicety). Heights + normals generated in a **worker** from the
  shared wasm, meshes built off the main thread, uploaded once — a chunk
  is static geometry until a terrain event says otherwise.
- **Shadow casting** (the horizon casts, `DECISIONS.md` §open): both LODs
  cast, and the ground names its `shadowSide` — three casts a FrontSide
  material from its BACK face, which is right for a closed solid and
  culls a heightfield out of the depth pass entirely. The near ring casts
  its own 1 m surface; the far mesh casts everywhere else, through a
  depth material that **discards the near ring's current footprint**, so
  every XZ column of the world has exactly one caster and the two LODs
  never put disagreeing silhouettes of one hillside in one map. The
  **skirt this section wants lives there**: the far caster is sunk a
  fixed 3 m below its own surface, so where the LODs disagree at the seam
  it casts late rather than painting a band along the hole's edge.
- **Material**: one splat shader, four sets (beach/meadow/forest
  rock/highland), blended **in-shader from height, slope, and a noise
  channel recomputed in GLSL** — no splatmap textures, no extra bandwidth,
  and the blend math mirrors the biome function. Cliff mask forces rock.
  Far ring fades into fog + a low-res horizon mesh.
  Shipped (materials v0, `DECISIONS.md` §open) with **no textures at
  all**: the four sets are authored PBR identities (colour + roughness +
  bump strength), their weights ride the geometry as a 4-byte `splat`
  attribute the worker derives from the sim's own (height, moisture,
  slope) — soft ramps centred on `biome()`'s own edges — and one shared
  three-octave value-noise field, sampled in GLSL, breaks the weights up,
  mottles albedo, varies roughness and drives a footprint-faded
  surface-gradient bump. Wetness at the waterline, snow on high rock and
  cliff darkening are causal modifiers on top, each moving colour and
  roughness together. Texture sets are still the art pass; the *system*
  is not waiting on them.
  A **fourth octave — grain** (materials v1) makes the ground read at
  arm's length, where 1.7 m is already too coarse to be texture. It is
  one more tap of the same field at a wavelength the *identity* chooses
  (4 cm of sand, 12 cm of grass), with a per-identity ridge fold and
  contrast, and it drives albedo, bump and roughness like every octave
  above it. Two things about it are the parts worth keeping: it is
  retired by pixel footprint in **cycles per pixel** rather than metres,
  because four wavelengths cannot share one metre threshold — so each
  identity's grain dies at its own distance on one curve — and its fade
  reads the **world** footprint, not the horizontal one, because a pixel
  on a steep face barely moves in XZ while covering metres of surface.
  Grain is **triplanar** (materials v1) and the base maps are
  **biplanar** (materials v4): the existing top tap plus one on the
  vertical plane containing the face's own fall line, whose stretch is
  `1/sin(tilt)` where the top plane's is `1/cos(tilt)` — exact
  complements, so the worst case anywhere is 45 deg at x1.41 where they
  average. The wall turns on at the crossover (`BASE_WALL_ON`, the 45 deg
  where `sin = cos` and the wall becomes the less distorted of the two),
  so level ground pays one tap and renders bit-for-bit what it did
  before. Weights are raised to `BASE_WALL_SHARPNESS` before blending and
  the wall's footprint is `dFdx(position)` projected onto the frame, not
  `dFdx(uv)` — a rotating frame otherwise puts its own turn, times a
  world coordinate, into the mip selector. **Every octave now retires on the WORLD footprint** — the law
  stated in this paragraph since materials v1 was applied to grain alone
  for two passes, and on a 69 deg face the horizontal footprint is 1/2.8
  of the real one, so every other octave stayed at full strength three
  times past its own Nyquist and reached the image as its alias.
- **Scatter rendering**: per chunk, one `InstancedMesh` per archetype
  filled from the slot list (minus harvested), frustum-culled per chunk.
  Trees get two LODs (mesh / billboard cross) **(knob: distances)**; each
  archetype carries an authored PBR response, baked vertex colours and a
  per-instance tint hashed from its own cell, so variation costs no draw
  calls (materials v0).
  Grass: cheap camera-ring patches, purely cosmetic, off on low tier
  **(knob)**. The tier idea is browser-era — it existed because a WebGL page
  had to run on whatever opened it — and a native client with a system
  requirement may not need one at all; unresolved, not decided.
- **Water** (shipped as water v0, `RENDER.md` §R8): one eye-centred mesh
  rather than a plane — a 2 m uniform core with a geometric skirt to 2.6 km,
  a four-wave swell with analytic normals whose amplitude goes to zero as the
  water shallows, per-channel colour and alpha from `exp(-depth · extinction)`
  off this file's own `height`, and shore foam banded by depth and weighted by
  the *land's* slope. A tiling ripple normal map carries everything below the
  shortest wave. **Nothing simulates**: the sim's only fact about water is
  `SEA_LEVEL` and §3's swim rule, and the drawn surface is not consulted by
  either. `reference/WATER.md` is the research.
  The land side of the same seam is here too: the ground darkens and saturates
  within `WET_BAND_M` of sea level, which is the reference's shoreline wetness
  and `ART.md` §5's wet sand.
- **Budgets** (within DESIGN §9's 300 draw calls / 1.5 M tris — **both
  browser-era and neither re-derived for the native client**, see there):
  terrain ≈ 40–60 draw calls and ~250 k tris at LOD; scatter instancing
  keeps the rest. Chunk build ≤ 4 ms **in the browser's worker**; natively
  it is a main-schedule system amortized one chunk per frame, which is the
  same amortization without the thread.

## 5 · What the island gives the game (the part that has no code)

The reads a survival map must produce, and which stage buys each:

- **"Where do I build?"** — the remap curve's shelves, visible from a
  distance as flat benches. Scarcity of *great* spots (flat + water +
  road-adjacent + node-adjacent) is the land-value engine.
- **"Where do I go?"** — the coast road. Barrels respawn there, so the
  route circulates, so players collide. One ring is enough for 100.
- **"Where's the risk?"** — highlands: the good nodes, no cover,
  silhouetted on ridgelines. Forest: resources + ambush shadow.
- **"Where am I?"** — biome color, coastline shape, and the one or two
  distinctive headlands the warp gives every seed. A client-rendered map
  (drawn from the same wasm heights — free) ships in alpha; no minimap,
  map is a held item you stop to read **(knob)**.

## 6 · Numbers of record (alpha)

| thing | value |
|---|---|
| island | 2,048 × 2,048 m **(knob)**, sea ring beyond |
| height grid | 1 m authoritative; server caches sampled chunks LRU |
| relief amplitude | ~90 m, sea level at 0 |
| chunks | 64 m, aligned with the netcode grid — one grid, everywhere |
| scatter cells | 8 m (≈ 65 k cells; ~8–12 k live slots per seed) |
| clutter cells | 0.64 m (≈ 10 M cells; total coverage on land, streamed in 16 m tiles) |
| clutter richness | 2nd stratum, rate `RICH_ACCEPT_MAX` = 32 in 256 by splat×clump; ≤ 96 per tile (frame-budget-bound, not design); dispersion 1.40 @ 3.2 m → 8.51 @ 12.8 m |
| prop skirts | annulus from the footprint edge out `SKIRT_BAND_M` = 0.45 m; 3–16 elements by reach; ≤ 256 per tile (measured max 40) |
| biomes | 4 (beach/meadow/forest/highland) |
| roads | 1 coast ring, ~4 m wide |
| authored sites | 3 — one haven pad + 2 waystations, all on the ring |
| pad containers | 5 `crate` on a 10 m ring, 2.64× the shoulder's density |
| waystation containers | 2 `cache` on a 6.5 m ring, ≥ 600 m from every other site |
| greyboxes | 2 kinds, one per tier: the pad's enclosed 7 m block to a 9.2 m tower, and the waystation's open canopy — 4 posts, one knee-high parapet, 4.1 m — standing in a gap in that 6.5 m ring rather than at the site centre, which is the road. **These are the numbers the sim blocks** (`terrain::WAYSTATION_CANOPY_BOXES`, gated by `sim-core/tests/{waystation,solid}.rs`); the mesh that draws them is no longer held to them — see §7 |
| tier prices | E[items] per container barrel 14.3 < cache 20.8 < crate 33.1; per site pad 165 > waystation 42 (`ci/haven_prize.mjs`) |
| node respawn | 20–45 min jittered, privilege-vetoed **(knob)** |

### Stage 10 · Ground clutter — the layer below the scatter grid

`ART.md` §1 calls the near ground the single largest structural difference
between our frames and the references, and rule 4 makes it a number: no
visible ground patch larger than ~3 m² inside 15 m without scatter. Stage 9
cannot answer that at any weight — two adjacent 8 m cells at full occupancy
still leave ~60 m² of bare ground between their two props.

So there is a second population on a **0.64 m jittered grid**, resolved by
`terrain::clutter_cell`: four kinds (pebble · tuft · twig · shard), full-cell
jitter, yaw and scale off one hash draw. It is **potential, not state** — like
a `Slot`, and less: no volume, no harvest, no wire, nothing in `state_hash`.

Two properties carry it, and both are gated rather than argued:

- **The mix IS the splat.** The ground material's four identity weights live
  in `terrain::splat_from` — moved there out of the terrain worker, which now
  calls the bridge — so the population and the surface under it evaluate one
  function. A tuft stands where the ground is grass because it is the same
  number that made the ground grass.
- **Coverage is total by construction.** Those weights normalize to 255 on
  land, so every land cell yields an element, and rule 4 becomes a property of
  the grid alone: a disc of radius `CLUTTER_CELL_M`·√2 contains a whole cell
  wherever it is centred. `tests/clutter.rs` measures it instead of trusting
  it — worst bare disc **1.50 m² of the 3 m² cap** over 33,852 land points.

Water is the only exclusion (the population's own `LAND_MIN_H`). The road
carriageway is the one splat override: grit, never grass.

**And a second stratum on top, because total coverage is also uniform
coverage.** One element per cell everywhere on land makes rule 4 provable and
makes the ground read as evenly dusted — sand carries exactly what a meadow
does. `findings/pass-20260804-173640-01-visual.md` gap 1 asked for the other
half in its own words ("so **density** follows the biome"), and
`reference/SPAWN.md` §9.3 names the same defect from the other side: the
reference's scatter clusters, ours is per-cell independent, which is why ours
reads as white noise. `terrain::clutter_rich_cell` is the answer — a SECOND
element a cell may earn, at a rate the ground sets:

- **Rate, not table.** Grass + forest litter weight (the two growing splat
  channels) scaled by `clump`, the same grove/clearing field stage 9 scales
  its whole row by. So the ground thickens where the trees thicken, one cause
  drives both layers, and there is no third law to drift.
- **It clusters, and that is measured rather than asserted.** Index of
  dispersion **1.40** at 3.2 m rising to **8.51** at 12.8 m; an independent
  coin holds 1.0 at every scale, and the RISE is what a constant rate cannot
  fake. Growing ground carries **0.088** extra elements per cell against bare
  ground's **0.005** — 18×.
- **It is frame-budget-bound, and that is the finding.**
  `CLUTTER_RICH_PER_TILE = 96` of 625 cells is not a design number: it is what
  the deleted `ci/clutter_shape.mjs` §4's 20%-of-1.5 M triangle share left after the
  coverage stratum and the skirts. **Note what that makes it downstream of**:
  1.5 M is `DESIGN.md` §9's browser-era ceiling, so this number inherits a
  WebGL constraint. If the native budget is ever re-derived, this is one of
  the things that moves with it — and the direction is up, toward `ART.md`
  rule 4's "empty ground is a defect", which is the bar it is currently
  rationed against. The first draft asked for 256 and a gate
  refused it. `RICH_ACCEPT_MAX = 32` then keeps the budget a **backstop**
  — a rate the ground can afford, rather than a truncation of one it cannot,
  which would have banded every tile's first rows and left the rest bare.

The wall that carries all of this **used to stand in one place.** Its four
bearings all walked outward from the island centre and the centre qualifies on
every seed, so twelve origins were three, and 33,852 query points hid it. It
now takes 24 golden-angle stances per seed from the centre to the shore, and
asserts its own spread. **It bought a real number:** the worst bare disc it
measures is **1.73 m² of the 3 m² cap** over 201,651 land points across 72
vantages, where the one-place stance reported 1.50 m² — the old gate was
under-reporting the island's worst ground by 15% because it never stood on it.

### Stage 10b · Prop-base skirts — the grid's blind spot

The grid answers rule 4 and is blind to props by construction: 0.64 m cells
that do not know a boulder stands in them. `ART.md` rule 2 is the other half —
*nothing sits ON the ground, everything sits IN it* — and a uniform grid cannot
give it, because what breaks a contact line is clutter crowded AT the base, not
clutter distributed evenly past it.

`terrain::skirt_fill` rings every scatter prop with the same four kinds in a
stratified annulus starting at the prop's footprint edge. Reach is
`occupant_volume`'s published radius (floored, since a pine's is its 0.26 m
trunk and a bush's is 0), so a prop that changes size drags its skirt with it.

It is deliberately not a second system:

- **Same population.** Same `ClutterElem`, same four pools, arriving in the
  same `terrain_fill_clutter` buffer behind the grid — no new material, no new
  program, no new draw call.
- **One kind law.** `clutter_kind_at` is extracted, not copied, so grid and
  skirt cannot drift at the one place a player's eye is: the foot of a prop,
  where a grid tuft and a skirt tuft stand 20 cm apart.
- **Tile-owned.** Elements are clipped half-open to the emitting tile, so a
  prop on a tile edge is skirted once however its neighbours stream.

Angular stratification with full-stratum jitter — the same discipline the grid
uses on position, turned 90°. Free jitter over the circle clumps and leaves a
bald arc; `test_a_skirt_is_spread_not_clumped` gates the quadrant occupancy.
No trig: `yaw_lut::yaw_dir`, per wall 1.

Measured over 1,875 tiles × 3 seeds: **max 40 elements per tile against the 256
bound**, peak 5×5 ring 15,930 of the 22,025 the pools size for, worst triangle
fleet 264 k = 17.6% of `DESIGN.md` §9's frame budget, inside the 20% share this
population declared — a percentage of a **browser-era** 1.5 M, so the share is
firmer than the denominator. **None of it has been seen** — no frames this
pass; the renderer tier and `browser_smoke` were deleted with the browser
client (`DECISIONS.md` 2026-08-06), so nothing photographs this at all now.

## 7 · Gates

- `test_terrain_golden`: seed → pinned hash of heights + first 256 slots,
  native = wasm = the checked-in value.
- `test_terrain_gameplay`: for 16 random seeds assert invariants — haven
  pad exists and is flat, road ring is closed and walkable, ≥ N buildable
  shelf area, ≥ N slots per biome, spawn ring has ≥ 24 valid spawns. A
  seed that fails is a bug in the generator, not a reroll: wipes must be
  able to trust any seed.
  The spawn-ring half of it has landed early as sim-core
  `world::tests::spawn_ring_lands_on_a_clear_beach`: 32 seeds × 64 joins,
  each spawn asserted beach biome, above the wade line, off a cliff, and
  4 m clear of every scatter slot.
  The road half has landed as sim-core `tests/road.rs`: 4 seeds × 64
  bearings, asserting the ring is **closed on every bearing**, that 0 slots
  stand on the carriageway (re-derived from the slot list, so a veto that
  stopped firing reddens), that the shoulder carries the barrel route, that
  under 10% of the road is over the cliff ratio, and that the sea is
  `ROAD_INLAND_M` ± the shoulder width seaward.
  The pad half has landed as sim-core `tests/haven.rs`: 16 seeds, asserting
  the site is deterministic and distinct per seed, stands **on the road** and
  on land inside the ring bracket (both of `haven`'s fallbacks asserted
  unreachable), is flat — re-measured on a 48-tap footprint the selector
  never scored, worst 3.76 m of relief and 0.21 rim slope — that it really is
  the **argmax**, re-derived by an independent 0.05 m march with no candidate
  allowed to score better, and that the exclusion zone is non-vacuous against
  a control haven parked off-island. "Exists and is flat" is now a number.
  What it cannot assert yet is that the pad is *carved* flat: v0 finds a flat
  site rather than making one (§1 stage 8), so this suite measures the
  generator's best natural ground, and the 3.76 m is the argument for the
  carve rather than evidence it happened.
- `tests/clutter.rs` §S: the authored sites sweep their own floor, measured
  against the same seed rendered with the site list parked offshore, so all
  three claims are exact rather than statistical — the floor is grit and
  carries no understory, **the wilderness is bit-identical**, and the band
  between the two masks contains both outcomes. Each is proven red under its
  own mutant (sweep disabled · sweep as a hard circle · the band collapsed to
  zero width). §2.1 has the design.
- Chunk-build time and instancing counts ride the client perf harness.

### 7.1 · The greybox mirror: one list now, and a gate over the rest

**Was a live defect; fixed 2026-08-10.** The two authored structures were
declared twice — once in `terrain.rs` as the volume the sim blocks, once in
`render/props.rs` as the mesh the client draws — and the gate that held the
lists equal was `ci/haven_shelter.mjs` + `ci/waystation_canopy.mjs`, both
deleted with the browser client. Nothing replaced them and they had drifted:

| | sim blocked (centre + **full size**) | client drew (centre + **half extent**) |
|---|---|---|
| haven shelter | `SHELTER_BOXES`, 14 rows, peak 9.2 m | 9 rows, peak 5.6 m — no corner posts, no tower-cap |
| waystation canopy | `WAYSTATION_CANOPY_BOXES`, 9 rows, finial 4.1 m | 6 rows, top 2.09 m |

A player was stopped ~0.7 m outside posts they could see. Note the units: full
size against half extent, which is the transcription hazard the deleted gate
existed to cover.

**The design call the old text left open is made: the sim's list is
authoritative** — `ART.md` §6 and the tier-silhouette argument were written
against those numbers, `OCCUPANT_R_M`/`OCCUPANT_TOP_M` are *defined* as the
tables' own bounds, and the drawn list was the one that had lost rows.

**And the fix is derivation rather than a second gate.** `props::authored`
builds the mesh from the sim's rows, converting full size to half extent in
one place against a length the type system pins; only the colours are the
client's. There is one list, so this particular drift cannot recur.

What `crates/client/tests/greybox.rs` gates is what derivation cannot make
true by construction, and it reaches further than the two structures:

- the unit conversion, row for row, on bit patterns;
- every row reaching the mesh, by vertex count (36 a box, measured);
- the authored pair's drawn bounds **equalling** the published broad phase in
  both directions — a gap either way means the scalar and the table came apart;
- **every other archetype fitting the volume the sim blocks**, which closes
  `OCCUPANT_R_M`'s own admission that "nothing in the Rust workspace can see a
  triangle, so the asserts below prove only that this file agrees with itself";
- a coverage check, so a new occupant arrives measured or explicitly excused.

**Closed the same day, on the operator's call**: the *generated* props blocked
wider than they drew — a boulder reaching 1.1145 m inside a 1.5 m blocked
cylinder, because `blob_mesh` displaces vertices inward from its nominal
radius and the row had been written off the nominal. `OCCUPANT_R_M` and
`OCCUPANT_TOP_M` carry the measured bounds now (rounded outward at four
decimals) and the gate's ratchet is an equality check at a one-millimetre
rounding allowance. Prop skirts tightened with it for free, because
`skirt_base_r` already reached off `occupant_volume`. `DECISIONS.md`
2026-08-10 has the call and what it touched.
