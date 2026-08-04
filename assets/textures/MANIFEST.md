# assets/textures — CC0 PBR source set

**Every file here is CC0 / public domain.** Sources: [Poly Haven](https://polyhaven.com)
and [ambientCG](https://ambientcg.com), both of which release all assets under CC0
— no attribution required, no license file to carry, no restriction on commercial
use or modification. Recorded here anyway because provenance is worth keeping.
Both are named in `ART.md` §7, so neither needs a new operator call.

Operator call, 2026-08-03 (`DECISIONS.md`): *"if its CC0 im fine to pull in
whatever helps us. then we can replace later."* These are a working set, not a
final art pass — the point is that the renderer finally has real measured
detail to sample. Replacing one is a file swap, not a code change.

Fetched at 1K, re-encoded for the browser: albedo 1024 q82, normal 1024 q90,
roughness and AO 512 greyscale q80. **9 materials, 34 files, 6.5 MB total.**

**`rock` is the one exception to that encode spec — albedo q74, normal q82.** It
holds the swap at **553 KB against `cliff_side`'s 562 KB**, so the only identity
here that costs client boot time does not cost more of it than what it replaced.
(Only the four ground identities are bundled; `stone`/`metal`/`wood`/`bark` are
not loaded by the client at all today.) It is cheap — span 1.03 and keep 0.97 are
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

`PH` = Poly Haven, `aCG` = ambientCG. Both CC0. `ao` marks a role that also
ships an occlusion map. **bundled** marks the four the client actually loads.

| role | source | ao | bundled | note |
|---|---|---|---|---|
| `bark` | PH [bark_brown_02](https://polyhaven.com/a/bark_brown_02) | | | vertical fissures + moss — the reference asks for exactly this |
| `grass` | PH [forrest_ground_01](https://polyhaven.com/a/forrest_ground_01) | ✓ | ✓ | turf with dirt wear — matches ART §3 lit-grass band once tinted. Its AO is the strongest of the set (mean 0.477, sd 0.162) and it owns ~99% of the near ring. |
| `gravel` | PH [bicolour_gravel](https://polyhaven.com/a/bicolour_gravel) | ✓ | | fine scree; slope/scree identity and path scuff |
| `litter` | PH [brown_mud_leaves_01](https://polyhaven.com/a/brown_mud_leaves_01) | ✓ | ✓ | forest floor, red-leaning; forest identity |
| `metal` | aCG [CorrugatedSteel009](https://ambientcg.com/view?id=CorrugatedSteel009) | ✓ | | replaced `green_metal_rust` 2026-08-04. Photoscanned ribbed steel: albedo sd 0.0090 → **0.0709**, i.e. the old one was a flat swatch. Grey industrial rather than rusty — ambientCG's rusty corrugated sheets are all `PBRProcedural` and their albedos are flat paint with screw dots, the very defect being replaced, so the rust has to come from the wear layer. |
| `rock` | aCG [Rock023](https://ambientcg.com/view?id=Rock023) | ✓ | ✓ | replaced `cliff_side` 2026-08-04 — see below. |
| `sand` | PH [coast_sand_01](https://polyhaven.com/a/coast_sand_01) | ✓ | ✓ | fine coastal sand, close to ART §3's 42°/10% sample |
| `stone` | aCG [Bricks089](https://ambientcg.com/view?id=Bricks089) | ✓ | | replaced `castle_brick_01` 2026-08-04. Photoscanned medieval stacked field stone — the identity ART asks for, not brick. sd 0.0947 → **0.1253**, anisotropy 1.34 → **1.09** (less row-banding). |
| `wood` | PH [brown_planks_03](https://polyhaven.com/a/brown_planks_03) | | | weathered grey planks; building tier 1 |

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
constant's comment promises, and `browser_smoke` asserts the derivation rather
than the value. The anisotropy figure is not a shipped gate — it was added for
this selection because "streaking" was the defect, and a layered source is how
you buy it back. Rock022 scored an identical span of 1.03 and was rejected on
it: 1.58, the same stratification defect as the incumbent.

When a better source is found, drop it in with the same name.

**Not here yet:** meshes. Trees are still four primitives
(`terrain.js`, `pineGeometry`); the vegetation upgrade is its own NOW item and
may stay procedural — `.claude/skills/threejs-procedural-vegetation` covers
trunks, recursive branches, leaf cards and species presets, which is a long way
above a stack of cones without shipping a single binary.
