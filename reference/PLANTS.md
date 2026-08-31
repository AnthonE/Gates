# reference/PLANTS.md — how games grow a forest

**Owns nothing** — research, not law, the same posture `reference/ANIMALS.md`
holds and for the same reason: it surveys how several games and one body of
graphics literature solve vegetation, and §9 is what that means here.
`ART.md` is still the bar and `TERRAIN.md` still owns placement.

**Source posture: clean, and better than `ANIMALS.md`'s.** Nothing decompiled
and nothing from the reference game. The sources are a SIGGRAPH paper, four
MIT-licensed repositories, and the source of a crate this repo already
depends on — all publicly readable. One caveat in `DOORS.md`'s shape: **this
box's egress proxy blocks `docs.rs`**, so the crate's API was read off
`raw.githubusercontent.com` instead, which worked. Asset hosts (Poly Haven,
OpenGameArt, itch.io, Sketchfab) are blocked too — noted because it decides
who fetches what in §9.4, not because it changed a finding.

Written because the operator asked whether our trees are good. They are not,
and the reason is not the tree.

---

## 1 · The finding, first

**Our forest's problem is arithmetic, not art.** Three measurements, all off
this tree:

1. **One species.** `tree.rs`'s own doc comment: *"the pool is one species at
   three seeds rather than three species."* `CONIFER_POOL = 3`, all
   `TreeType::Evergreen`, all `PINE_H = 6.6 m`. Every tree on the island is
   the same 6.6 m conifer at one of three seeds and a yaw.
2. **The placement has a density CEILING, set by the grid.** `terrain::scatter`
   draws **one occupant per 8 m cell** (`CELL_SIZE = 8.0`), jittered ±3 m — so
   the maximum density of anything, anywhere, is **one stem per 64 m²** and two
   trees can never be closer than 2 m. A thicket at 1–3 m spacing is not
   something the weights can ask for; the grid cannot express it.

   ⚠ **This row said "the placement is a lattice, a forest cannot have a
   thicket or a clearing" and that was wrong — corrected 2026-08-10, same day,
   before anything was built on it.** Clearings and groves already exist:
   `terrain::clump` is a low-frequency fBm field that `scatter` multiplies the
   whole weight row by, squared per `SPAWN.md` §9.4 so a grove edge is a
   ragged gradient rather than a contour line. It is gated
   (`sim-core/tests/scatter.rs`) against a closed-form independent-draw null,
   and the measurement that motivated it is recorded on the function: variance
   of the tree count in a 40 m window was 1.05× the null before it existed.
   **So the density field is done and only the ceiling is open.** The error
   was reading `CELL_SIZE` and not reading forty lines further; the lesson is
   that "our terrain does not do X" needs a grep for X, not an inference from
   a constant.
3. **It is knowingly over budget past ~80 m.** `CONIFER_MAX_TRIS = 6_000` × a
   p90 of 328 trees in the draw ring = 1.97 M against `DESIGN.md` §9's 1.5 M.
   `tests/tree.rs` prints the arithmetic so it cannot be forgotten, and
   `NOW.md` §0t item 1 has queued the billboard LOD that fixes it.

Point 1 is the one to act on, and §6.1 is why it is nearly free.

Point 2 is real but **much narrower than the first cut of this doc claimed**,
and raising the ceiling is not cheap: a second occupant in a cell breaks
`gather::cell_key`, which packs one slot per cell and is read by the wire
(`EV_SLOT_HARVESTED`), the save (`worldsave.rs`) and the client's mirror.
Halving `CELL_SIZE` is the cheaper lever and is not free either — 4× the
cells, 4× the live `SlotLives` rows against `TERRAIN.md` §6's budget. Neither
is a rendering change and neither should be attempted as one.

---

## 2 · A forest has five layers and we have two

The standard forest-structure decomposition, and the one every vegetation
system in games reproduces whether or not it names it. Against ours:

| layer | height | what it is | ours |
|---|---|---|---|
| **Canopy** | 6–25 m | mature trees, closed or open | one 6.6 m conifer |
| **Sub-canopy** | 2–6 m | young trees, suppressed stems | **nothing** |
| **Shrub** | 0.5–2 m | bushes, saplings, ferns | one lumpy green sphere (`blob_mesh(0.7, …)`, 80 tris) |
| **Herb** | 0.05–0.5 m | grass, forbs, flowers | grass tufts — **this layer is good** |
| **Floor** | 0 m | litter, deadfall, moss, logs | pebbles, twigs, shards |

The herb layer is genuinely done: `clutter.rs` bakes 721 elements per 16 m
tile into one mesh per tile, 25 tiles in the ring, blades at 0.34 m inside
`ART.md` §1's measured 20–40 cm band. It exists because the measured gap it
closed was enormous — reference near-ground neighbour contrast 6.3 luma/px
against the browser's 0.26 — and the mechanism was geometry, not shader.

**The two missing layers are the two that make a forest read as a forest.**
A canopy over bare ground reads as an orchard. The sub-canopy and shrub layers
are what break sightlines, what makes a treeline opaque, and what makes moving
through woods feel different from crossing a meadow — which for a survival game
is a *gameplay* property, not a visual one. `TERRAIN.md`'s four biomes (Beach,
Meadow, Forest, Highland) currently differ by which of seven occupants roll,
and Forest vs Meadow is mostly "more trees".

---

## 3 · The three separate problems people call "trees"

Worth splitting, because they have different answers and only one of them
wants bought assets.

### 3.1 · Generation — solved, and we already ship the solver

Two families in the literature:

- **L-systems** (Lindenmayer, and Prusinkiewicz's *Algorithmic Beauty of
  Plants*): a grammar rewrites a string, the string drives a turtle. Total
  control, botanically principled, and it produces obviously self-similar
  trees — the symmetry is inherent to the rewrite.
- **Space colonization** (Runions et al., 2007, at algorithmicbotany.org):
  seed a crown envelope with attraction points, grow branches toward
  whichever points are nearest, delete points as they are reached.
  Competition for space picks the branching, **which breaks the symmetry
  L-systems bake in** — that is the whole reason it looks better, and the
  paper says so.

`bevy_procedural_tree` — already a dependency, MIT OR Apache-2.0 — is a Rust
port of `@dgreenheck/ez-tree`, and `tree.rs` calls exactly one function of it
(`generate_tree_meshes`) as a pure settings-in-meshes-out call, with the
crate's ECS plugin deliberately untouched. **This half is done.** What we do
with it is the gap:

- `TreeType` has **two** variants, `Evergreen` and `Deciduous`. We use
  `Evergreen` only. A broadleaf is a settings change, not an asset.
- ez-tree ships **15 preset JSON files** — `ash`, `aspen`, `oak`, `pine` in
  small / medium / large, plus `bush_1..3` and a `trellis` — under MIT. They
  are *parameter sets*, and the crate's `TreeMeshSettings` is the same
  parameter space. **Porting numbers out of a JSON file is not vendoring an
  asset**, it needs no file in the tree, and it turns a 3-seed pool into a
  12-preset one for the cost of a table.
- The crate has `BarkType { Birch, Oak, Pine, Willow }` and
  `LeafType { Ash, Aspen, Pine, Oak }` **commented out in its own source** —
  the Rust port dropped ez-tree's texture selection. So the port gives us
  geometry and no leaf art, which is precisely where §4 says the money goes.

### 3.2 · Placement — mostly built, and the open half is the ceiling

The canonical treatment is Deussen et al., *Realistic Modeling and Rendering
of Plant Ecosystems* (SIGGRAPH '98, Stanford/Calgary/ZKM). Its two placement
techniques are painted density fields and an **individual-based population
model** — plants compete, shade each other, and die, and the surviving
distribution is the output. Its third contribution is *approximate
instancing*: replace similar plants with instances of a representative before
rendering, which is the same idea as our shared mesh pool one level up.

**We already took the takeable half.** `terrain::clump` is the density-field
answer: a low-frequency fBm shared between neighbouring cells, multiplied
into the whole scatter weight row and squared so an edge is a gradient. Its
own doc comment explains why the reference's approach was NOT copied — theirs
is a stateful sampler drawing `ClusterSizeMin..Max` objects out of a quadtree
leaf with a 20 m local-density brake, and `scatter` has to stay a pure
function of one cell or every on-demand caller resolves the island. So a cell
still decides alone, against a field its neighbours can see.

What is left is the **ceiling**, and the three ways to raise it are all
sim-core with real costs:

1. **More than one occupant per cell.** Breaks `gather::cell_key` — one slot
   per cell is baked into the wire, the save and the client mirror. Large.
2. **A smaller `CELL_SIZE`.** 8 → 4 m quadruples max density and also
   quadruples the cell count and the live `SlotLives` rows, against
   `TERRAIN.md` §6's 8–12 k budget. Also re-derives the clutter skirt's
   `SKIRT_TILE_CELLS * 8 == CLUTTER_TILE_M` assert.
3. **Leave it.** One stem per 64 m² is a real forest density for mature
   conifers; what it cannot draw is a young dense stand.

**All three are wall-1 territory** — no `HashMap` iteration, no trig, no libm,
floats restricted — which is what rules out a relaxation loop or a
dart-throwing Poisson sampler with a live candidate list. Determinism is not
negotiable: the scatter is replayed and the client mirrors it, so a client
that rolled placement differently would draw trees that are not there.

### 3.3 · Rendering at scale — the budget, and it is queued already

The problem is stated in `NOW.md` §0t: full-detail trees are affordable to
~80–100 m and the ring is 320 m across. Two techniques:

- **Impostors.** The modern form is the **octahedral impostor**: bake the
  object from a grid of view directions over a (hemi)octahedron into an
  atlas, then draw one quad and pick/blend the nearest captured views in the
  shader. `wojtekpil/Godot-Octahedral-Impostors` is the best-documented open
  implementation, with a baker and an automatic LOD node; a three.js port
  exists demoing 200 k trees. This is strictly better than the two crossed
  cards `SeedThree`'s `impostor.js` uses (already cited in `CLAUDE.md`),
  because crossed cards fail when you look along a card and octahedral views
  do not.
- **Instancing that does not upload per-instance data.**
  `pinkponk/bevy_efficient_forest_rendering` is the Bevy-specific datum:
  chunked custom render pipelines, **8 M grass straws at 60 fps**, and a
  measured **3–4× over general GPU instancing** by *randomising in the shader
  instead of sending per-instance buffers*. Same idea our clutter bake uses
  (one mesh per tile, not one entity per blade), one level further.

Other Bevy grass crates surveyed — `warbler_grass`, `bevy_procedural_grass`,
`frosty_grass` — are all plugins that own placement and spawn entities. **None
is adoptable here**: placement is `sim-core`'s (it is gated, replayed, and the
client mirrors it), and `RENDER.md` §1 says Bevy draws and does not decide. We
would be importing a decider. Read them for the shader, not for the plugin —
the same posture `CLAUDE.md` takes on the three.js skill packs.

### 3.4 · Wind

Owed since the port (`NOW.md` §0t item 2) and unchanged by this research:
`StandardMaterial` cannot read a custom vertex attribute, so `aWind` needs the
custom material `RENDER.md` already lists. The design is `SeedThree`'s and is
already recorded in `CLAUDE.md` — one per-vertex cantilever weight rooted at
the trunk base, phase from the instance's world position so a gust crosses the
forest, two sine octaves. A billboard has four vertices to hang a weight on,
so LOD1 sways too; do the material once and both layers get it.

---

## 4 · Why a mesh generator is the wrong tool for foliage

Stated plainly because the question that prompted this doc was which models to
buy.

**A plant is an alpha card problem, not a mesh problem.** `tree.rs`'s own
header carries the browser's conclusion after three passes of building pines
out of cones: *"a conifer's canopy is made of ALPHA CARDS, and an opaque hull
with a polygon edge cannot get there from any amount of geometry."* A leaf is
one or two triangles wearing a cut-out texture; a canopy is a few hundred of
them. What decides whether it reads is the **texture's alpha silhouette**,
which is why `ART.md` rule 6 puts silhouette before surface.

Text-to-3D generators produce watertight opaque hulls with baked colour. That
is the right output for a barrel, a furnace and a revolver — every row in
`WANTED.md` §2 §4 §5 — and the wrong output for anything with leaves. A
generated bush is a green potato: it is exactly the defect our current
`blob_mesh` bush already has, bought again.

So for plants the purchase is **textures**, and the geometry stays generated.
That also keeps `ART.md` rule 7 satisfiable — a static `.glb` tree is one
silhouette repeated, and unlimited seeds are not.

The one exception is **deadfall**: a fallen trunk, a root plate, a rotting log
are opaque solids with no alpha in them, and a mesh generator is the right
tool. They are also the cheapest thing that makes a forest floor read as a
forest floor rather than as ground with trees on it.

---

## 5 · What we have, measured

Anchors for anyone re-checking this doc:

| thing | value | where |
|---|---|---|
| species | 1 (`TreeType::Evergreen`) | `tree.rs::settings` |
| tree variants | 3 seeds | `tree::CONIFER_POOL` |
| tree height / max radius | 6.6 m / 1.7 m | `props::PINE_H`, `PINE_MAX_R` |
| triangles per tree | ≤ 6,000 | `tree::CONIFER_MAX_TRIS` |
| trees in ring, p90 / max | 328 / 446 | `tests/tree.rs` |
| frame ceiling | 1.5 M tris | `DESIGN.md` §9 |
| scatter grid | 8 m cell, one occupant, ±3 m jitter | `terrain::CELL_SIZE`, `terrain::scatter` |
| occupant kinds | 7 rolled | `terrain::OCCUPANT_KINDS` |
| biomes | Beach · Meadow · Forest · Highland | `terrain::Biome` |
| clutter | 721 elements / 16 m tile, 5×5 ring | `terrain::CLUTTER_PER_TILE`, `clutter::CLUTTER_RING` |
| blade height | 0.34 m | `clutter::TUFT_H` |
| leaf/needle art | **generated** | `tree::needle_image` |

---

## 6 · What it means for us

### 6.1 · Trees do not need bought models

The single highest-value change on this page costs no assets: **port ez-tree's
preset parameters and turn on `TreeType::Deciduous`.** That takes the pool
from one species at three seeds to four species at three sizes, inside a
generator we already ship, with the fit-to-bounds and vertex-colour post-passes
`tree.rs` already applies. `PINE_H`/`PINE_MAX_R` become per-preset rather than
global, and `tests/tree.rs` grows a row per preset.

Do this *before* buying any tree art. It changes what the leaf textures have
to cover.

### 6.2 · The lattice is the real defect and it is a sim-core slice

§3.2's option 1 — clump centres plus members — is the cheapest thing that lets
a forest have thickets and clearings, and it is a pure function of another
`cell_hash` channel. It touches `terrain::scatter`, which means
`test_terrain_golden` and `test_replay` both move, and the client's mirror
comes along for free because it already calls `terrain::scatter` itself.

**This should be ranked above the billboard LOD**, which is the opposite of
`NOW.md` §0t's current order, and the reason is that clumping makes the
budget problem *worse* — more stems in the near ring — so doing LOD first
means doing it against the wrong distribution. Sequence: distribution, then
measure, then LOD against what the measurement says.

### 6.3 · The shrub and sub-canopy layers are content, not code

Once §6.1 lands, ez-tree's `bush_1..3` presets are three more generated
shrubs, and a small-size tree preset at 40 % scale is a sapling. Both are new
`Occupant` variants (the enum has room; slot 8 is skipped and 13+ are free)
plus rows in the scatter table — which is `TERRAIN.md`'s business and follows
the pattern `HavenShelter` and `WaystationCanopy` already set.

Ferns and forbs are the one shrub-layer thing the generator will not give,
because a frond is a shape a branch generator has no grammar for. That is a
texture-plus-quad job in `clutter.rs`'s existing bake, not a new system.

### 6.4 · The shopping list, split by who fetches it

⚠ **This said the proxy blocks every asset host from this box. It was true
where it was written and it is not a property of the hosts** — `SOURCES.md`
§0's own warning, arrived at independently: reachability belongs to the
container, so PROBE rather than trusting either claim. Measured 2026-08-31 on
this box, `ambientcg.com`, `polyhaven.com` and `api.polyhaven.com` all answer
200, and `assets/textures/fetch_gates_texture_candidates.py` pulled sets 9.7,
9.8 and 9.10 without an error. Two of the rows below are consequently no
longer wanted:

- ~~**Grass blade atlas**~~ — taken. Poly Haven `grass_medium_01`, baked by
  `ci/bake_grass_atlas.py`, drawn by `clutter::card`.
- The **bush** half of the shrub layer (§6.3) — taken. Poly Haven `shrub_01`,
  composed into leaf clusters by `ci/bake_bush_atlas.py`, drawn by
  `props::bush_card_mesh`.

The rest of the list stands, and the fetcher is the way to get it: `WANTED.md` §10
carries the same list in buying order.

**Textures — the real gap, and none of it is a mesh generator's job:**

- **Leaf atlases**, alpha cut-out, one per species §6.1 lands — broadleaf
  (ash/aspen/oak read very differently) and a conifer sprig. The conifer one
  replaces `tree::needle_image`, which is generated and is the weakest link
  in the canopy today.
- **Fern / frond atlas** — the shrub layer's one irreducible texture.
- **Grass blade atlas.** Blades are vertex-coloured today with no map at all.
- **Flower / forb atlas**, small, for meadow biome variety.
- **2–3 more bark maps** — we ship one. Birch and a dead/weathered bark carry
  most of the species read at trunk level.

`ART.md` §7's rules apply unchanged and they are not a formality: score
candidates on gain span with the shipped estimator before committing, because
that is the lever that took `rock` from keep 0.17 to 0.97 on a file swap.

**Meshes — worth generating, because they are opaque solids:**

- Fallen log / deadfall, 2–3 lengths
- Root plate / upturned stump
- Rotting stump variants (we have one stump at 0.64 ⌀ × 0.34)
- Everything already in `WANTED.md` §2 §4 §5 — unchanged by this doc

**Do not buy:** tree models, bush models, grass models, or any foliage
`.glb`. §4 is why.

### 6.5 · What stays open

- Whether clumping is worth its golden churn before the alpha. It reddens
  `test_terrain_golden` and `test_replay` by design — that is a regenerate,
  not a break, but it is a wipe of every existing world.
- Whether the impostor is octahedral or crossed-cards. Octahedral is better
  and is more code; the crossed-card version is `SeedThree`'s and is already
  written down. Measure at the ring's p90 before choosing.
- Nothing here is a spoken knob. Every number in §6 is a proposal and belongs
  in `DECISIONS.md` §open before it lands in code.

---

## Sources

- Runions et al., [*Modeling Trees with a Space Colonization Algorithm*](https://algorithmicbotany.org/papers/colonization.egwnp2007.large.pdf) (EGWNP 2007)
- Deussen et al., [*Realistic Modeling and Rendering of Plant Ecosystems*](http://www.graphics.stanford.edu/papers/ecosys/) (SIGGRAPH '98)
- [`dgreenheck/ez-tree`](https://github.com/dgreenheck/ez-tree) — MIT, the 15 presets, and [eztree.dev](https://www.eztree.dev/) as its live editor
- [`Affinator/bevy_procedural_tree`](https://github.com/Affinator/bevy_procedural_tree) — MIT OR Apache-2.0, our dependency; `src/enums.rs` for the commented-out `BarkType`/`LeafType`
- [`wojtekpil/Godot-Octahedral-Impostors`](https://github.com/wojtekpil/Godot-Octahedral-Impostors) — the impostor baker worth copying the method from
- [`pinkponk/bevy_efficient_forest_rendering`](https://github.com/pinkponk/bevy_efficient_forest_rendering) — 8 M straws, 3–4× over general instancing
- [`EmiOnGit/warbler_grass`](https://github.com/EmiOnGit/warbler_grass), [`jadedbay/bevy_procedural_grass`](https://github.com/jadedbay/bevy_procedural_grass), [`DavidHospital/FrostyGrass`](https://github.com/DavidHospital/FrostyGrass) — surveyed, all own placement, none adoptable; read for shaders
- [`madjin/awesome-cc0`](https://github.com/madjin/awesome-cc0) — the CC0 index to start §6.4's texture hunt from
