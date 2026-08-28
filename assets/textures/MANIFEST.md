# assets/textures — CC0 PBR source set

**This file records what SHIPS. The queue of what does not exist yet is
`CANDIDATES.md` beside it** — 84 rows for the six foliage/bark sets, 80 CC0 and
4 CC-BY, with the fetch script, the CSV and the sheet in this directory and the
CC-BY notice drafts in `CANDIDATES_CC_BY.md`. Same split as
`assets/models/{MANIFEST,WANTED}.md`. Fetched candidates land in
`candidates/`, which is **gitignored** (~1.3 GB of other people's pristine
source); a candidate earns a line here only once it is measured, packed and its
licence recorded.

**Every file here is CC0 / public domain.** Sources: [Poly Haven](https://polyhaven.com)
and [ambientCG](https://ambientcg.com), both of which release all assets under CC0
— no attribution required, no license file to carry, no restriction on commercial
use or modification. Recorded here anyway because provenance is worth keeping.
Both are named in `ART.md` §7, so neither needs a new operator call.

Operator call, 2026-08-03 (`DECISIONS.md`): *"if its CC0 im fine to pull in
whatever helps us. then we can replace later."* These are a working set, not a
final art pass — the point is that the renderer finally has real measured
detail to sample. Replacing one is a file swap, not a code change.

Fetched at 1K, re-encoded: albedo 1024 q82, normal 1024 q90, roughness and AO
512 greyscale q80. **10 materials, 38 files, 7.1 MB total.** (`twig` is the
tenth, 2026-08-22, encoded to that same spec — a file joining a documented set
matches the set, and re-spec'ing is the separate call the ⚠ below describes.)

⚠ **That encode spec was chosen for a first-visit browser download and the
constraint is gone** (the browser client was cut 2026-08-06; the client
installs a depot and reads from disk). `ART.md` §7 retires the 12 MB payload
ceiling with it and says re-sourcing at 2K/4K is unblocked — what is real
natively is VRAM and disk, and nothing has measured either. **Treat every
size and quality number below as a browser-era artifact, not a budget.**

**`rock` is the one exception to that encode spec — albedo q74, normal q82.** It
holds the swap at **553 KB against `cliff_side`'s 562 KB**, so the only identity
here that costs client boot time does not cost more of it than what it replaced.
(⚠ This parenthetical used to say only the four ground identities were bundled
and `stone`/`metal`/`wood`/`bark` were "not loaded by the client at all today".
That went stale twice: the props bound all four on 2026-08-11 — the **bundled**
paragraph below has said so since — and the build pieces bound the last three
on 2026-08-16. Eight of nine roles are loaded; `gravel` is the one that is not.)
It is cheap — span 1.03 and keep 0.97 are
unchanged, albedo sd moves 0.0942 → 0.0933 — so it is kept as margin.

**It is margin, not a fix, and the record should say so.** The trim was first made
on the theory that the spec-quality granite (749 KB) had reddened `browser_smoke`
by pushing tab B past its 60 s join clock. **That theory was measured and is
false**: baseline `cliff_side` at 561,587 B joined in 54.7 s, while the *smaller*
552,915 B granite build joined in 59.2 s and passed. Join time on this box is
dominated by contention, not payload — three runs gave a timeout, a browser crash
at 54.7 s, and a pass at 59.2 s, at ~830–894 ms/frame under software GL. The real
defect is that `JOIN_TIMEOUT_MS` is the clock-based assertion `CLAUDE.md`
forbids; when it moves onto observable state (`inWorld && snapshots > n`) this can
go back to q82/q90 and nothing is lost.

**Displacement is still deliberately not taken** — the terrain has its own height.
**AO now is**, and the earlier line here saying the light rig supplies it was
wrong. A light rig supplies *large*-scale occlusion; the scale between a
surface's own features is a different one and no rig reaches it. Filament's
material doc puts it plainly — an AO map "recreate[s] the natural shadowing that
occurs between the different tiles. Without ambient occlusion, both materials
appear too flat" — and flat is the complaint the visual judge has filed
repeatedly. Lagarde & de Rousiers (*Moving Frostbite to PBR* §4.10.3) is the
source for the three scales and for how the terms combine; `ART.md` §4 carries
the application rules. The files are here ahead of the shader work that consumes
them, so **`*_ao.jpg` is currently fetched but unread** — see `NOW.md` item 3.

**Licence rail, since 2026-08-07: CC0 preferred, CC-BY accepted with a `NOTICE`
entry, NC and SA refused** (`DECISIONS.md`) — the last because the game is sold,
not for any open-source reason. Everything currently here is CC0 and needs no
notice.

`PH` = Poly Haven, `aCG` = ambientCG. Both CC0. `ao` marks a role that also
ships an occlusion map. **bundled** marks a role the client actually loads —
**eight of nine now, where it was four.** `bark`, `wood`, `stone` and `metal`
were fetched, manifested and then read by nothing for four days: a
`StandardMaterial` has one base-colour slot and **no procedural mesh in the
client carried a UV**, so no prop could sample a map however many shipped. The
UV half is solved on the CPU (`props::Soup` box-projects per triangle, which a
triangle soup makes free), and those four are bound now.

⚠ **`gravel` is still the one unbundled role, and the reason recorded here was
wrong.** This said it was "waiting on the splat material". The splat material
landed 2026-08-15 (`render/ground_splat.rs`) and gravel is no closer, because
what it actually waits on is a **classifier slot**: `terrain::splat` resolves
exactly four identities — sand · grass · litter · rock — and gravel is not one
of them. Binding it is a `sim-core` change (a fifth weight on the wire the mesh
carries, and a fifth band in `splat_from`), not a client one. The single-map
limitation it was blamed on is gone and gravel did not move, which is the
evidence that the diagnosis was wrong.

✅ **The four ground roughness maps are read (2026-08-16)** — `sand`, `grass`,
`litter`, `rock` at shader bindings 110–113, sampled per texel. They had been
loaded, uploaded and resident since the day the set landed and nothing sampled
them, so this cost four bindings, zero samplers and **zero new VRAM**. Measured
raw (a roughness map is data, loaded `is_srgb = false`): **sand 0.9631 · grass
0.9364 · litter 0.9197 · rock 0.6108**, and `tests/ground_splat.rs` re-measures
all four off the files, so swapping a source changes the island's specular
loudly instead of silently. The reason recorded against them for four days —
the glTF-packed ORM slot whose B channel is metallic — was a constraint of that
slot and never of these files; **the same false reason is still recorded in
`render/props.rs` against the other five**, where it is false for a second
reason as well (Bevy multiplies `metallic` by that channel, and it defaults to
0.0). `NOW.md` §0gp item 6.

⚠ **The maps had no detectable effect on the frame** (contrast −0.4% over six
vantages, inside the harness's own ~0.3% run-to-run spread — `RENDER.md` §5)
and that is not about the files: the ground material's
`reflectance: 0.18` puts specular F0 at 0.52% where a dielectric is ~4%, so
roughness has almost nothing to shape. Recorded because it is the next thing
and it belongs to the coupled lighting owner — `DECISIONS.md` §open "ground
specular v0".

**What the splat material DID change here:** the four ground identities each
sample their own albedo and normal now, instead of all four sharing
`ground_detail.jpg` and `grass`'s normal map. So `sand`, `litter` and `rock`
went from *bundled but only as a colour* to bundled as surfaces. Measured at a
pinned spawn with the mix stated (litter 611‰, rock 329‰): near-ground
neighbour contrast **6.43 → 8.53, +32.8%**. `ground_detail.jpg` is the
casualty — it is grass's baked luminance field, the shader computes the same
thing from `grass_albedo.jpg`, and **nothing loads it now**; it still ships and
is still gated as a file.

| role | source | ao | bundled | note |
|---|---|---|---|---|
| `bark` | PH [bark_brown_02](https://polyhaven.com/a/bark_brown_02) | | ✓ | vertical fissures + moss — the reference asks for exactly this, **on a tree**. It wore the twig building tier as well until 2026-08-22; see the `twig` row. Authored at 1000 mm, and the shared 0.55 drew it over 1.82 m, so a twig base was a photograph of a trunk at 1.8× life size. |
| `grass` | PH [forrest_ground_01](https://polyhaven.com/a/forrest_ground_01) | ✓ | ✓ | **Authored at 2000 mm** → `terrain_mesh::GROUND_TILE_M[1]` is 2.0 m. It was drawn over the shared 4 m ground reference until 2026-08-28, i.e. at twice life size. turf with dirt wear — matches ART §3 lit-grass band once tinted. Its AO is the strongest of the set (mean 0.477, sd 0.162) and it owns ~99% of the near ring. |
| `gravel` | PH [bicolour_gravel](https://polyhaven.com/a/bicolour_gravel) | ✓ | | fine scree; slope/scree identity and path scuff |
| `litter` | PH [brown_mud_leaves_01](https://polyhaven.com/a/brown_mud_leaves_01) | ✓ | ✓ | **Authored at 1300 mm** → `terrain_mesh::GROUND_TILE_M[2]` is 1.3 m. At the old shared 4 m it drew at 3.1× life size — leaves the size of a hand. forest floor, red-leaning; forest identity |
| `metal` | aCG [CorrugatedSteel009](https://ambientcg.com/view?id=CorrugatedSteel009) | ✓ | ✓ | replaced `green_metal_rust` 2026-08-04. Photoscanned ribbed steel: albedo sd 0.0090 → **0.0709**, i.e. the old one was a flat swatch. Grey industrial rather than rusty — ambientCG's rusty corrugated sheets are all `PBRProcedural` and their albedos are flat paint with screw dots, the very defect being replaced, so the rust has to come from the wear layer. |
| `rock` | aCG [Gravel004](https://ambientcg.com/view?id=Gravel004) | ✓ | ✓ | CC0. **No authored size published** — `dimensionX/Y/Z` are all `0`, ambientCG's sentinel for unknown and the same answer `CorrugatedSteel009` gives `structures::TIER`. `GROUND_TILE_M[3]` keeps the 4 m reference on the counted cross-check that row's shape asks for: the median-energy wavelength of the shipped albedo is 14.4 texels, so a clast draws 5.6 cm at 4 m, inside real crushed-aggregate grading (20–63 mm). Replaced `Rock023` 2026-08-27, which replaced `cliff_side` 2026-08-04 — **both of those were cliffs**; see below. |
| `sand` | PH [coast_sand_01](https://polyhaven.com/a/coast_sand_01) | ✓ | ✓ | **Authored at 15000 mm and it is the one ground row that refuses its own size**: 15 m is over the 4 m ceiling `tests/pieces.rs` holds piece surfaces to, and drawing it there costs `ART.md` rule 1's near-field grain — the share of its linear-luma variance under 5 cm falls **66.3% → 22.5%**. `GROUND_TILE_M[0]` is 4.0 m, and `terrain_mesh.rs` carries the measurement. fine coastal sand, close to ART §3's 42°/10% sample |
| `stone` | aCG [Bricks089](https://ambientcg.com/view?id=Bricks089) | ✓ | ✓ | replaced `castle_brick_01` 2026-08-04. Photoscanned medieval stacked field stone — the identity ART asks for, not brick. sd 0.0947 → **0.1253**, anisotropy 1.34 → **1.09** (less row-banding). |
| `ground_detail` | **derived** from `grass` (PH [forrest_ground_01](https://polyhaven.com/a/forrest_ground_01)) | | ✓ | Rec.601 luma of the source's LINEAR albedo, re-encoded to sRGB greyscale, 1024 q88, 342 KB. The ground's near-field grain: `ART.md` §7 asks a modifier that sets a colour to multiply the surface's own **mean-1 luminance field**, and a luminance field has gain span **1.000 by construction** where the four colour sources measure 2.454 / 2.073 / 3.586 / 1.054 (grass / sand / litter / rock) against a ×1 ceiling. Linear luma mean 0.2464, sd 0.0762. Derived, never edited: the source stays pristine and swappable, and regenerating is a luma convert. |
| `twig` | PH [bamboo_wall](https://polyhaven.com/a/bamboo_wall) | | ✓ | lashed vertical poles; building tier 0 — **the straw/lashed-pole set the `wood` row below had been waiting for since 2026-08-12**, landed 2026-08-22 because the tier nobody chooses is the tier everybody sees (`build.rs`: every piece enters the world as twig) and it was wearing a photograph of a living tree, moss and all. Authored at 2000 mm, so `structures::Tier::tiles_per_m` is 0.5 and its poles draw ~4 cm — a sapling. Linear mean rgb 0.242 0.153 0.091, luma **0.167**, sd 0.0551, measured on the shipped file: in `ALBEDO_LUMA_BAND` on its own, so it ships its colour whole at gain 1.0 and *is* the paler of the first two tiers without the ×1.6 `bark` needed to fake it. |
| `wood` | PH [brown_planks_03](https://polyhaven.com/a/brown_planks_03) | | ✓ | weathered grey planks; building tier 1 — **and it is that tier's map since 2026-08-16**, four days after this note was written as an intention. `stone` is tier 2, `metal` tier 3. (⚠ This row also said `bark` doubled as tier 0 "for want of a straw/lashed-pole set". It did, for six days short of a fortnight; the `twig` row above is the want being filled, and `bark` is a tree's map again and nothing else's.) Authored at 1000 mm → `tiles_per_m` 1.0, which puts a plank at ~11 cm; it drew at 20 cm under the shared 0.55 every tier used until 2026-08-22. `structures::TIER` |

### ⚠ `rock` was a wall twice over, and the gate that would have said so was measured and not shipped

**2026-08-27, from an operator screenshot: *"the texture has a pattern to it,
like rows and lines"*.** It did. `Rock023` is a photograph of a stratified
cliff face — horizontal bedding, edge to edge — and ambientCG tags it
`cliff`, `stone`, **`wall`**. The ground lays it flat on a planar XZ
projection, so the bedding became rows running across the terrain, and above
60 m the alpine band saturates so the worst identity dressed every summit.

**The selection below is not wrong about its own numbers; it is wrong about
one of them, and that one was never gated.** It records "anisotropy (>1.3 =
strata)" at **1.05** for `Rock023` and rejects `Rock022` at 1.58 for exactly
this defect — then says in as many words that the figure "is not a shipped
gate". Measured now under the repo's own convention (`DECISIONS.md` materials
v4: a ratio of `|dx|` to `|dy|`), `Rock023` is **0.762** on a 256² reduction,
where the other three ground identities sit at 0.957 / 1.040 / 1.003. Nobody
can reproduce 1.05, the estimator was never published, and nothing in CI could
see the disagreement. `client/tests/ground_splat.rs::
no_ground_identity_is_a_photograph_of_a_wall` is that gate now, proven red on
the file it describes.

**The replacement is `Gravel004`** — crushed angular aggregate, which is what
alpine scree is — chosen off the same 74-candidate method re-run over 87
ground-tagged CC0 aCG materials with the estimator validated against this
file's own published numbers first (it reproduces `Rock023`'s span 1.054 and
linear means 0.273/0.269/0.259 to the digit). Four passed every rail; two of
those landed inside ART §3's granite band.

| | Rock023 (out) | **Gravel004 (in)** | Gravel005 (runner-up) |
|---|---|---|---|
| anisotropy @256 (1.0 = isotropic) | **0.762** | **0.953** | 1.022 |
| tile visibility (sd of an 8×8 box, % of mean) | **4.1%** | **2.2%** | 1.9% |
| albedo sd (measured detail) | 0.083 | **0.129** | 0.084 |
| gain span → chroma keep | 1.054 → 0.97 | 1.109 → 0.90 | 1.050 → 0.97 |
| albedo hue / luma | 43.5° / 140 | 48.7° / 129 | 35.2° / 141 |

`Gravel005` wins every published rail and was passed over on the one the rails
do not carry: it reads as fine asphalt, and `rock` also dresses cliff faces
(the slope veto forces it) and ore nodes. `Gravel004` buys 55% more measured
detail and a real scree identity for 7% of the chroma and 5.7° of hue — and the
hue band, as this file already argues for `Rock023`'s saturation miss, was
measured on *lit* reference frames rather than on albedo.

**Two things this swap moved, both re-measured by their own gates rather than
by hand:** `GRAIN_GAIN[3]` 3.7128 → 4.0820 (the map's mean linear luma fell
0.2693 → 0.2450) and `ROUGH_MEAN[3]` 0.6108 → 0.5359. Sizes grew — albedo 239 →
328 KB, normal 279 → 613 KB — because scree carries far more high-frequency
normal detail than a smooth face; the ⚠ above retires the browser-era size
spec as a budget, so this is recorded rather than corrected.

**The normal convention is `NormalGL`, and it was undocumented until now.**
Determined by measurement rather than by guessing: the shipped `rock_normal.jpg`
differs from aCG's `Rock023_NormalGL` by a mean |ΔG| of **1.64** (JPEG
re-encode noise) and from `NormalDX` by **38.98**. Picking the wrong one
inverts every bump on the ground and nothing in this repo would have said so.

**The three marginal picks are gone, and `rock` is why the others went with it.**
`cliff_side` was layered sandstone standing in for granite. Its gain span was
**5.72**, so `materials.js` kept **0.17** of its measured colour — 83% of a
photograph discarded to hold it in band. `Rock023` was chosen by measurement,
not by eye: 74 CC0 photogrammetry candidates were scored with the repo's own
estimator (`baseGainSpan` in `materials.js`, replicated exactly — it reproduces
all four published gain vectors to the digit), on three axes.

| | cliff_side | Rock023 |
|---|---|---|
| gain span → chroma keep | 5.72 → **0.17** | 1.03 → **0.97** |
| albedo sd (measured detail) | 0.0534 | **0.0933** |
| anisotropy (>1.3 = strata) | 1.23 | **1.05** |
| albedo hue / luma | ~25° / — | **42.3° / 142** |

Hue and luma land inside ART §3's granite band (35–43°, 127–167) off the raw
file. Saturation does not — 2.5% against a 10–19% band — but that band was
measured on *lit* reference frames, not on albedo, and mineral hue is what the
per-identity tint octave exists to add. **Nothing in `materials.js` was edited
for this**: the keep is derived from the file's own span, exactly as that
constant's comment promises, and `browser_smoke` asserted the derivation
rather than the value — that gate went with the browser client and **the
derivation is unasserted natively**. The anisotropy figure is not a shipped
gate — it was added for
this selection because "streaking" was the defect, and a layered source is how
you buy it back. Rock022 scored an identical span of 1.03 and was rejected on
it: 1.58, the same stratification defect as the incumbent.

When a better source is found, drop it in with the same name.

**Measured for the prop bind, 2026-08-07** — linear means, Rec.709 luma,
against `ALBEDO_LUMA_BAND = [0.05, 0.55]`. A prop has one identity, so the map
IS its colour and no mean-placing gain is applied; the gain is 1, so §7's
"deviation may not be stretched more than ×1" is satisfied by construction
rather than by a correction. All six clear the band off the raw file:

| role | linear mean rgb | luma | albedo sd | gain span |
|---|---|---|---|---|
| `rock` | 0.273 0.269 0.259 | 0.269 | 0.0933 | 1.054 |
| `bark` | 0.128 0.105 0.064 | 0.107 | 0.0676 | 2.000 |
| `twig` | 0.242 0.153 0.091 | 0.167 | 0.0551 | 2.659 |
| `wood` | 0.161 0.139 0.112 | 0.142 | 0.0661 | 1.442 |
| `stone` | 0.237 0.202 0.106 | 0.203 | 0.1139 | 2.223 |
| `metal` | 0.230 0.228 0.228 | 0.228 | 0.0689 | 1.009 |

(`twig` joined 2026-08-22, measured the same way and off the SHIPPED file. The
five above it were re-derived at the same time as a check on the method rather
than trusted: all five reproduce to the digit, span included — `max(mean_rgb) /
min(mean_rgb)`, which is what `materials.js`'s `baseGainSpan` computes.)

The span column is recorded but **not binding for any of these**, because nothing
divides by their means — it is what the correction WOULD have cost had one been
needed, and it is why the ground (which does need one) can only take `rock`.

**One pick was wrong on identity rather than on measurement, which is a
failure mode §7 does not cover.** `stone` scored well and was bound to the
stone ORE NODE because the names matched; `Bricks089` is coursed field stone
with mortar joints, so a 2 m boulder wore masonry and read as a buried castle
wall. It is the right map for something a player BUILT and the wrong one for an
outcrop. Ore nodes take `rock` — a node is granite whatever the mineral in it.
**Sourcing note:** there is no sulfur or ore-specific albedo here, so the three
node types differ by roughness and reflectance only.

**Not here yet:** meshes. Trees are still four primitives
(`terrain.js`, `pineGeometry`); the vegetation upgrade is its own NOW item and
may stay procedural — `.claude/skills/threejs-procedural-vegetation` covers
trunks, recursive branches, leaf cards and species presets, which is a long way
above a stack of cones without shipping a single binary.
