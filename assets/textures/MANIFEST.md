# assets/textures — CC0 PBR source set

**Every file here is CC0 / public domain.** Source: [Poly Haven](https://polyhaven.com),
which releases all assets under CC0 — no attribution required, no license file to
carry, no restriction on commercial use or modification. Recorded here anyway
because provenance is worth keeping.

Operator call, 2026-08-03 (`DECISIONS.md`): *"if its CC0 im fine to pull in
whatever helps us. then we can replace later."* These are a working set, not a
final art pass — the point is that the renderer finally has real measured
detail to sample. Replacing one is a file swap, not a code change.

Fetched at 1K, re-encoded for the browser: albedo 1024 q82, normal 1024 q90,
roughness 512 greyscale q80. **9 materials, 27 files, 6.0 MB total.**
Displacement and AO maps were deliberately not taken — the terrain has its own
height and the ground gets AO from the light rig.

| role | Poly Haven asset | note |
|---|---|---|
| `bark` | [bark_brown_02](https://polyhaven.com/a/bark_brown_02) | vertical fissures + moss — the reference asks for exactly this |
| `grass` | [forrest_ground_01](https://polyhaven.com/a/forrest_ground_01) | turf with dirt wear — matches ART §3 lit-grass band once tinted |
| `gravel` | [bicolour_gravel](https://polyhaven.com/a/bicolour_gravel) | fine scree; slope/scree identity and path scuff |
| `litter` | [brown_mud_leaves_01](https://polyhaven.com/a/brown_mud_leaves_01) | forest floor, red-leaning; forest identity |
| `metal` | [green_metal_rust](https://polyhaven.com/a/green_metal_rust) | MARGINAL: flat painted green, not rusty corrugated. Building tier 3 placeholder. |
| `rock` | [cliff_side](https://polyhaven.com/a/cliff_side) | MARGINAL: layered sandstone, hue ~25° and far more saturated than ART §3's granite (35–43°, 10–19% sat). Pull it into band with the per-identity tint the shader already has, or replace. |
| `sand` | [coast_sand_01](https://polyhaven.com/a/coast_sand_01) | fine coastal sand, close to ART §3's 42°/10% sample |
| `stone` | [castle_brick_01](https://polyhaven.com/a/castle_brick_01) | MARGINAL: old brick, not stacked field stone. Building tier 2 placeholder. |
| `wood` | [brown_planks_03](https://polyhaven.com/a/brown_planks_03) | weathered grey planks; building tier 1 |

**The three marginal picks are marked on purpose.** They are inside the working
set because a placeholder that carries real high-frequency detail beats a
procedural field that carries none, and because the per-identity tint and
chroma machinery in `materials.js` can pull an off-band albedo toward `ART.md`
§3's measured targets without touching the file. When a better source is
found, drop it in with the same name.

**Not here yet:** meshes. Trees are still four primitives
(`terrain.js`, `pineGeometry`); the vegetation upgrade is its own NOW item and
may stay procedural — `.claude/skills/threejs-procedural-vegetation` covers
trunks, recursive branches, leaf cards and species presets, which is a long way
above a stack of cones without shipping a single binary.
