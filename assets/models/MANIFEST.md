# assets/models — CC0 rigged source

**Licence rail** (`DECISIONS.md` 2026-08-07): CC0 preferred, CC-BY accepted with
a `NOTICE` entry, **NC and SA refused** — the last because the game is sold, not
for any open-source reason. Everything here is CC0 and needs no notice.

| file | source | licence | note |
|---|---|---|---|
| `mannequin.gltf` + `mannequin.bin` | Quaternius [Universal Animation Library](https://quaternius.com/), glTF export mirrored at [J-Ponzo/gltf-universal-animation-library](https://github.com/J-Ponzo/gltf-universal-animation-library) | **CC0 1.0** (`LICENSE` in that repo) | One skinned `Mannequin`, 53 joints, 13,743 triangles, **46 animation clips**. 3.1 MB. |

Renamed from `AnimationLibrary_Godot_Standard.{gltf,bin}`; the only edit is the
buffer `uri`, which had to follow the rename. Nothing else in the file is
touched, so re-vendoring is a copy and one string.

⚠ **Nothing loads the mannequin since 2026-08-17** — `render/anim.rs` loads
`stumpy.glb` — and it is kept because **it is the source of that file's
animation library**. All 46 of its clips were retargeted onto the new
character's skeleton the same day (`ci/retarget_anim.py`), and a retarget is
only reproducible while both rigs are in the tree: re-running it needs this
file, its rest pose and its bone names, none of which survive in the output.
3.2 MB is the price of being able to re-derive 46 clips instead of owning them
as a black box.

## `stumpy.glb` — the player character

**Commissioned output, on the `deploy/` rail below** — Meshy, paid plan, so
private ownership with no attribution owed and no `NOTICE` entry due.

| | |
|---|---|
| file | `stumpy.glb`, **4.2 MB** |
| mesh | one skinned character, 21,360 verts / 31,252 tris, **24 joints**, split into `char1_arms` (8,184 tris) + `char1_body` for the first-person view |
| albedo | `Material_1_baseColor`, KTX2/UASTC 1024², **linear luma 0.056** — measured off the shipped file 2026-08-31 at mips 0/2/4/6 (0.0568 / 0.0564 / 0.0553 / 0.0556), 94% of texels in the darkest eighth. ⚠ **That is level with `grass` (0.054), the darkest identity in the game, and one thousandth over `ALBEDO_LUMA_BAND`'s 0.05 floor** — against granite 0.292, sand 0.178, twig 0.167 and litter 0.135. It is why the operator reads other players as dark in a frame (`NOW.md` §0dk), and it is the asset rather than the light: an A/B capture rules out SSAO at under 1% and the fill gives a shaded vertical face 14.3% of lit ground, which is physically correct. |
| clips | **53** — the delivery's 7, plus 46 retargeted off the mannequin |
| texture | one 4096² base colour, packed to **1024 UASTC** (28.9 MB → 2.8 MB) |
| stands | **1.800 m**, feet on y = 0 — the sim's own `Capsule3d::new(0.4, 1.0)` exactly, so `ANIM_RIG_H_M` is 1.8 and the scale is 1 |
| source | `Meshy_AI_Stumpy_biped.zip` → `..._Meshy_Merged_Animations.glb` |
| ⚠ owed | **the prompt, the task id and the credit count are not recorded here** — they are the operator's to supply, and the rail below asks for all three. Every other row in this file has them; this one is a gap, written down rather than invented |

Imported with **`ci/import_char.py`**, which is the prop importer's sibling and
differs from it in the one way that matters: it corrects the **scene root**
rather than the vertices, because a skinned mesh is not drawn where its
vertices are (the file's header has the whole argument). Two corrections were
needed and both are the generator's, not ours:

```
ci/import_char.py <merged>.glb stumpy_raw.glb \
    --rename Idle_11=Idle_Loop --rename Walking=Walk_Loop \
    --rename Running=Jog_Fwd_Loop --single-sided --roughness 0.85
ci/ktx_pack.py stumpy_raw.glb stumpy_packed.glb
ci/retarget_anim.py assets/models/mannequin.gltf stumpy_packed.glb stumpy_rt.glb \
    --retime Sword_Attack=1.05333
ci/split_arms.py stumpy_rt.glb stumpy_split.glb
ci/curl_hands.py stumpy_split.glb assets/models/stumpy.glb
```


1. **The up axis is Z.** The merged-animation export lies on its back; the
   character export of the *same model* is Y-up. One generator, one rig, two
   files, two answers — so this is checked per delivery, never assumed.
2. **The clip names are the generator's.** `render/anim.rs` resolves by NAME
   and a name it cannot find draws a body frozen in its bind pose, so
   `Idle_11` is an idle that never plays.

**`split_arms.py` is what makes a first-person viewmodel possible**, and it is
one line because the alternative was believed impossible: the mesh becomes
`char1_arms` + `char1_body`, two nodes on one skeleton **sharing their vertex
buffers** and differing only in their index array (+0.4 MB, no second copy of
anything). `render/viewmodel.rs` hides the body half and draws the arms on the
camera; `bodies.rs` draws both and is unchanged.

**`curl_hands.py` is the last step and it corrects a delivery, not a choice.**
The generator modelled five digits per hand — 1,048 vertices in the right one —
and rigged none of them: `RightHand` is a LEAF, so the hand's pose IS the
mesh's shape and nothing at runtime can change it. It arrived in the flat,
spread pose a rigger binds in, which is right for binding and wrong as the rest
pose of the most-seen mesh in the game. The tool bends 1,068 vertices across
both hands — nothing else in the file moves, and the shared vertex buffer means
the viewmodel and every remote body get it in the same edit. It derives the
hand frame, which digit is the thumb, which way the palm faces and where each
digit's knuckle is from the geometry; the one knob is how far, defaulting to
85° over a finger's length (a relaxed hand, not a fist) with the spread closed
45%. Both hands land at 0.13 of their reach off the palm plane against 0.03
before. **A grip pose is not this**: it would want a second baked pose selected
by what is in hand, and joints are what it would want after that (`NOW.md`
0chr).

`crates/client/tests/rig_asset.rs` gates all of it — the clip names off `Clip`
itself, the height against `ANIM_RIG_H_M`, the stand-up rotation, one material,
KTX2 textures, both halves of the split, the arms' hold clip, that the swing
clip still fits the sim's swing cadence, and that the hands are curled. Six of
the nine were watched going red under their own defect — four against the raw
file, the cadence one against the un-retimed 1.5 s swing, and the hand one
against the splayed delivery, which reports 0.032 against a floor of 0.09.

**The `--retime` on the swing is not cosmetic.** `SWING_CLIP_S` is derived
from the sim (`SWING_INTERVAL_TICKS / TICK_HZ − ANIM_BLEND_S − one frame` = 1.05333 s), and
the asset is cut to it, because a stroke longer than the cadence is cut off by
the next swing and never finishes. `Sword_Attack` is the clip despite being the
long one: on this body a punch puts a hand **15 cm inside the head**, measured
(0.147 m against a 0.295 m head radius) where the sword holds 0.490 m. A
retargeted clip inherits the source's proportions as assumptions, and an
oversized head breaks them.

### The 46 retargeted clips

The delivery carried seven. `ci/retarget_anim.py` moved the **whole**
mannequin library across in nine seconds for **+1.0 MB**, which is why the
character has a death, a sprint, a jump, a swim, a crouch and two hit
reactions without a credit being spent. Its header carries the maths; the two
things to know here are that only ROTATIONS transfer (a source bone's
translation is that skeleton's limb length, and copying it stretches the
target onto somebody else's build) and that the hips are the one exception,
converted through the hip-height ratio and this rig's centimetre units.

⚠ **It shipped once with the arms 43° low, and the cause is worth carrying.**
A rest-pose retarget transfers each rig's *deviation from its own rest*, which
silently assumes the two rests are the same pose. Measured here, they are not:
the source rests in a true T (upper arm `[1, 0, 0]`, dead horizontal) and this
character rests in an A, 43° below it. Spine 2–7°, legs 3–17°, **arms 36–43°**
— so the legs looked right, the walk looked right, and every arm arrived 43°
low, which reads as a hand passing through the torso. Reported off the bench as
*"the right arm is inside the model"*, and it is **not** a proportions problem,
which is what it looks like.

The fix anchors each bone on a *virtual* rest whose bone direction matches the
source's rather than on its own (`qbetween`, minimal-arc so no twist is
invented). `--no-align` restores the old behaviour for a pair of rigs that
genuinely share a rest pose. **The general lesson: "retarget the delta from
rest" is only as true as the claim that both rests mean the same thing, and
that claim is measurable in one command** — compare the rest bone directions
before trusting a single frame.

**On a name collision the delivery's own clip keeps the bare name** and the
retarget takes `_alt` — so `Idle_Loop`, `Walk_Loop` and `Jog_Fwd_Loop` are
still the ones authored for this character, with `Idle_Loop_alt` and friends
beside them for comparison. `--bin modelview <file> mannequin.gltf --per-clip`
puts the retarget next to its source in one frame under one clip name, which
is how this was checked: **`A_TPose` retargets to an actual T-pose**, which is
the diagnostic that isolates the rest-pose maths from everything else.

The library's own irrelevant clips came across too (`Pistol_*`, `Spell_*`,
`Driving_Loop`, `Sitting_*`). They cost ~300 KB and were kept rather than
curated, because the cheap thing to do later is delete a name and the
expensive thing is to re-derive one.

**What is still missing**: the client has no state to drive most of the 46
from. `bodies.rs` knows a remote's position, yaw, pitch, whether it is asleep
and — since wire v48 — whether it has been **killed**, and that is the whole
input, so `Jump_Loop`, `Swim_Fwd_Loop` and the crouch pair sit in the file
unplayable, each waiting on a fact on the wire. `Death01` came off that list
on 2026-08-18: the fact it was waiting for is one bit on `EntityState`, and
it plays as a one-shot that holds its fallen pose (`render/anim.rs`
`Clip::Death`). The clips are there before the states that would play them,
which is the right way round — and the death is the case that shows why: the
asset cost nothing and the wire cost everything. (`WANTED.md` §11 is closed: the gather
swing is `Sword_Attack` by operator call, and nothing here is waiting on an
asset.)

## `deploy/` — generated, and the rail they land on

**These are commissioned output, not licensed work, which is a different basis
and not a fourth flavour of the same one** (operator, 2026-08-11,
`DECISIONS.md`). Meshy's paid plan is full private ownership with no
attribution owed, so no `NOTICE` entry is due and none is written; the free
plan would instead licence them CC BY 4.0, which is why the plan they were
made under is a fact worth recording rather than a footnote. Two conditions
ride along: none of these may be published to the Meshy Community feed, and a
prompt may not launder someone else's copyright.

**The Facepunch rail is unchanged and the prompt is its new surface.** No
proper noun appears in any prompt below — each describes the object and its
real-world size, never a source — and the full prompt ships with the asset so
that claim is auditable rather than asserted. Nothing here is traced.

Pipeline, settled the same day: `nano-banana-pro` reference image →
`image-to-3d` with `model_type: smart-topology`, `ai_model: meshy-t2`, 2K PBR,
`origin_at: "bottom"`. Then `ci/import_meshy.py`, which imposes scale from
`structures.rs`'s own `DEPLOY` row and strips emission. **Sizing is ours, never
the generator's** — its vision estimate was off by 9× on a spear, and the
measurements are in `DECISIONS.md`. `crates/client/tests/deploy_assets.rs`
gates every claim in this table.

| file | arch | task id | credits | note |
|---|---|---|---|---|
| `deploy/bag.glb` | 0 bag | `019ff011-7dd5` | 24 | bedroll, 2.4:1 open on the ground. Regenerated once: the first was 1.5:1 and read as a bath mat |
| `deploy/hearth.glb` | 1 hearth | `019ff00c-c2be` | 24 | plank cupboard, iron strap hinges, ajar on a shelf. Our building-privilege deployable |
| `deploy/box.glb` | 2 box | `019fefe5-6198` | 24 | plank chest, iron brackets and hasp. Serves `box_small` AND `box_large` — one archetype, one model |
| `deploy/fire.glb` | 3 fire | `019feff4-8c6e` | 15 | stone ring, charred logs, embers. **The one asset that keeps its emissive map** (measured peak 0.24, genuine glow) |
| `deploy/workbench.glb` | 5 workbench | `019feff2-cd94` | 24 | plank worktop, vice, scattered tools |

## `site/` — the two authored places, and the fit rule they forced

Same rail and same pipeline as `deploy/` above — Meshy, paid plan, commissioned
output, no `NOTICE` entry due, and no proper noun in either prompt. Generated
2026-09-01 with `ci/meshy_gen.py`, which is the pipeline `DECISIONS.md` settled
on 2026-08-11 written down as a command instead of as prose: `nano-banana-pro`
text-to-image → `image-to-3d` with `model_type: smart-topology`,
`ai_model: meshy-t2`, 2K PBR, `origin_at: "bottom"`. It writes a JSON sidecar
carrying exactly the columns below, so this table is transcribed rather than
remembered.

| file | occupant | image task | mesh task | tris | size |
|---|---|---|---|---|---|
| `site/shelter.glb` | `HavenShelter` | `01a05e1c-5de3` | `01a05e1c-ff8d` | 4,140 | 3.6 MB |
| `site/canopy.glb` | `WaystationCanopy` | `01a05e1c-ad22` | `01a05e1d-4e95` | 3,801 | 3.3 MB |

**Credits: 72 for all three tasks** (5,247 → 5,175), and there is deliberately
no per-asset column. `ci/meshy_gen.py` reports a delta between a balance read
before its first call and after its last, which is correct for one run and
**wrong for two running at once** — these two were generated concurrently, so
each sidecar charged itself part of the other's spend and their figures sum to
111 against a true 72. The script's header now says so. The total is the number
that was actually observed; the split was not.

Image prompts, in full, because the IP rail's auditability is the reason they
are recorded at all:

> **shelter** — "A single ruined stone outpost building on a plain white
> background, three-quarter view. Square footprint 7 metres by 7 metres, 9.2
> metres tall. Thick weathered grey granite walls on three sides. One tall open
> doorway with heavy square stone jambs and a lintel across the front. Flat
> stone slab roof. Four square corner pillars rising above the roofline. One
> square watchtower standing on the roof, set back from the front, topped by a
> wide flat capstone. Moss in the joints, storm worn. Exactly one building. No
> ground, no terrain, no people, no plants."
>
> **canopy** — "A single small open timber shelter on a plain white background,
> three-quarter view. Square plank deck 5.2 metres by 5.2 metres at ground
> level, 4.1 metres tall overall. Four slender square wooden corner posts hold
> up a wide flat plank roof, with a second smaller plank roof stacked above it
> and a short square finial on top. One low knee-high plank wall along the back
> edge only; the other three sides are fully open. Weathered grey timber, iron
> nails, no paint. Exactly one structure. No ground, no terrain, no people, no
> plants."

Texture prompts are one line each — "Weathered grey granite blocks, moss in the
joints, storm worn stone, no vegetation" and "Weathered grey timber planks, iron
nails, sun bleached wood, no paint".

### These two are imported with `--fit-axes`, and the reason is not cosmetic

**A `deploy/` row is a render row; these are the sim's collision volume.**
`terrain::SHELTER_BOXES` and `WAYSTATION_CANOPY_BOXES` are what a body is
stopped by, and `OCCUPANT_R_M`/`OCCUPANT_TOP_M` are *defined* as their bounds —
so where a deployable that comes up short inside its row is "a row to
re-measure", a site that comes up short is a player stopped by air. Measured on
these two under the uniform fit every other asset here uses:

| | drawn (uniform fit) | blocked | gap |
|---|---|---|---|
| shelter | 7.00 × **7.69** × 6.92 | 7.00 × 9.20 × 7.00 | **1.51 m of blocked air above the roof** |
| canopy | **4.34** × 4.10 × **4.34** | 5.60 × 4.10 × 5.60 | **1.26 m of invisible skirt per horizontal axis** |

`ci/import_meshy.py --fit-axes` scales each axis to its own target instead, so
the drawn bound meets the blocked one: **both peaks are exact** (0.00000 m
against `OCCUPANT_TOP_M`) and neither model reaches outside its blocked radius.
The radii come up **short by 21.4 mm (shelter) and 0.2 mm (canopy)** rather
than equal, because the number that matters is the one the renderer uses —
largest per-vertex `hypot(x, z)` — and a box corner is only that if a vertex
sits on the corner, which the generator's eroded roof slab does not. The gate's
allowance for that is `SITE_SHORT_M = 0.05`, picked against the body (a fifth
of `WALL_THICKNESS_M`) rather than against today's mesh. What the fit costs is
an **aspect correction** —
`max(k) / min(k)`, how much the most-stretched axis was stretched relative to
the least:

| | x | y | z | aspect |
|---|---|---|---|---|
| shelter | 1.538 | 1.840 | 1.555 | **1.196×** |
| canopy | 1.765 | 1.367 | 1.765 | **1.291×** |

**Prompting for the aspect does not fix this, and that was measured rather than
assumed.** A second canopy was generated the same day (`01a05e20-9d03` /
`01a05e21-1775`, 24 credits) with the aspect stated three ways — "much wider
than it is tall", the two figures, "eaves overhang far past the posts". It came
back at 3.08 × 3.00 × 3.08 against the first's 3.17 × 3.00 × 3.17, i.e. an
aspect correction of **1.329×, worse than the 1.291× it was meant to fix.** It
is not shipped. The lever that would actually work is the box table, and that
is sim truth with a replay golden behind it — see `NOW.md`.

`crates/client/tests/site_assets.rs` gates every number in this section, and
each of its claims is proven red under its own mutant (uniform fit, a 5%
oversize, a kept emissive map, an unpacked JPEG).

**The box massing is not deleted.** `props::archetype_mesh` still returns it,
`greybox.rs` still measures the sim's scalars against it, and it is still what
draws if a model fails to load.

## `prop/` — the scatter, where the instance count is the argument

Same rail, same pipeline, same date-stamped audit trail. What is different is
the *reason*: a site stands once on an island, and these stand thousands of
times. Census on the shipped seed — **1,054 boulders, 609 stone nodes, 89
metal, 48 sulfur** — off exactly two meshes before this landed, because every
boulder was one `blob_mesh` at one seed and the three ore nodes shared a
second and differed only by material.

| file | occupant | image task | mesh task | tris | asked | size |
|---|---|---|---|---|---|---|
| `prop/rock_a.glb` | `Rock` (pool 0) | `01a05e71-4efe` | `01a05e73-a70e` | 2,662 | 2,400 | 3.4 MB |
| `prop/rock_b.glb` | `Rock` (pool 1) | `01a05e7a-56ee` | `01a05e7a-f87b` | 2,670 | 2,400 | 3.4 MB |
| `prop/rock_c.glb` | `Rock` (pool 2) | `01a05e7d-075b` | `01a05e7d-811d` | 2,662 | 2,400 | 3.3 MB |
| `prop/node_stone.glb` | `StoneNode` | `01a05e6e-4baa` | `01a05e6e-c5a5` | 1,439 | 1,400 | 3.5 MB |
| `prop/node_metal.glb` | `MetalNode` | `01a05e85-6c8d` | `01a05e85-e691` | 1,364 | 1,250 | 3.4 MB |
| `prop/node_sulfur.glb` | `SulfurNode` | `01a05e7c-bfc4` | `01a05e7d-3978` | 1,490 | 1,400 | 3.5 MB |
| `prop/barrel.glb` | `BarrelSlot` | `01a05ea8-6214` | `01a05ea9-0399` | 758 | 700 | 3.6 MB |
| `prop/crate.glb` | `CrateSlot` | `01a05ea1-9804` | `01a05ea2-894d` | 522 | 550 | 3.3 MB |
| `prop/cache.glb` | `CacheSlot` | `01a05ea5-0f65` | `01a05ea5-8934` | 479 | 550 | 3.2 MB |

**`target_polycount` works and overshoots by 3–11 %**, which is why the metal
node was asked for 1,250 to land under `WANTED.md` §2's 1,500 — the roll before
it came back at 1,554 and was rejected on triangles alone rather than have the
ceiling moved to fit it.

### Eleven rolls, six keepers — selection is the method

**`ci/measure_glb.py` is the step that makes this work, and it exists because
prompting alone does not.** Every row below is measured; the KEEP column is
the tool's verdict and it agrees with the six that shipped.

| roll | d/w | aspect | luma | green | verdict |
|---|---|---|---|---|---|
| rock_a | 0.425 | 1.02× | 0.179 | **47.6 %** | slab, and green |
| **rock_b → `rock_a.glb`** | 1.000 | 1.36× | 0.344 | 0.2 % | **KEEP** |
| rock_c | 0.195 | 1.08× | 0.094 | 0.2 % | wafer |
| **rock_d → `rock_b.glb`** | 1.016 | 1.36× | 0.204 | 1.3 % | **KEEP** |
| **rock_e → `rock_c.glb`** | 1.022 | 1.17× | 0.250 | 0.4 % | **KEEP** |
| rock_f | 1.032 | 1.42× | 0.234 | 12.3 % | green |
| **node_stone** | 1.000 | 1.03× | 0.303 | 0.2 % | **KEEP** |
| node_metal (1) | 0.801 | 1.22× | **0.081** | 0.2 % | 1,552 tris |
| node_metal (2) | 1.002 | 1.69× | 0.146 | 0.2 % | 1,554 tris, aspect |
| **node_metal (3) → `node_metal.glb`** | 1.000 | 1.44× | 0.190 | 3.6 % | **KEEP** |
| node_sulfur (1) | 0.531 | 1.39× | 0.269 | **28.3 %** | slab, and green |
| **node_sulfur (2) → `node_sulfur.glb`** | 0.807 | 1.48× | 0.259 | 0.4 % | **KEEP** |

What the prompt controls and what it does not, measured rather than assumed:

- **Naming an omitted axis works.** "2.2 m wide and 2.0 m tall", with no depth
  given, reconstructed to 0.425 / 1.000 / 0.195. Adding "2.2 deep, as deep as
  it is wide" took the next three to 1.016 / 1.022 / 1.032.
- **Naming a colour to avoid works.** "Lichen crusting the hollows" → 47.6 %
  green-dominant; "mottled grey with rust-brown staining" → 0.2–1.3 %. "Bright
  yellow" sulfur drifted to 28.3 % green; "warm golden-yellow, mustard, never
  green" → 0.4 %. "Dark iron ore seams" measured luma **0.081**, half of
  granite's 0.292 — a black lump in a grey world — and naming the host rock
  grey lifted it to 0.190.
- **Asking for a ratio more extreme than the object's natural one does not**,
  which is the canopy's lesson from the day before (1.329× against the 1.291×
  it was meant to fix).

### `--fit-radius`, and why a box is the wrong shape here

These are imported `--fit-radius --center`, not `--fit-axes`, and the
difference is a defect the gate caught in production. The two authored sites
publish `OCCUPANT_R_M` as a half-**diagonal** — their footprint IS the box.
These rows were measured off `blob_mesh`, which is round in plan, so the same
number is the radius of a **cylinder**. Fitting the bounding box to `2 × R`
therefore pushes the corners outside it: measured on the first ore node at
**1.2737 m drawn against 0.9148 m blocked, 36 cm of visible rock a player
walks through**. `--fit-radius` solves for the largest per-vertex
`hypot(x, z)` instead — one shared X/Z factor, so the plan shape is preserved
and only its scale moves.

**Which fit mode a row takes is DERIVABLE, not a judgement call.** Compute the
half-diagonal of the massing the model replaces and compare it to
`OCCUPANT_R_M`: equal means the row describes a **box** and takes
`--fit-axes`; unequal means it describes a **cylinder** and takes
`--fit-radius`. Measured across the nine shipped props — crate 0.6801 ==
hypot(0.55, 0.40), cache 0.5701 == hypot(0.45, 0.35), so both are boxes; the
barrel, the boulder and the three nodes are round in plan and are not. Getting
this wrong is not cosmetic: it drew the first ore node 36 cm outside what the
sim blocks.

⚠ **Stating a dimension can put the dimension IN the picture.** Every prompt
here names metric sizes, and on the boar the image came back with a scale bar
and the figures "1.5 metres / 0.78 metres" drawn under the animal — text that
a reconstruction can bake into an albedo. None of the nine shipped props show
it, and it is not something a bounding-box check can see; look at the
reference image, not only at the numbers.

**One roll was rejected for a colour the prompt itself asked for**: the first
barrel measured **16.0 % green-dominant** on "flaking grey-green paint over
rust". Re-rolled as "bare rusted steel, orange-brown oxide, no paint at all"
→ 0.0 % and luma 0.248. The lesson from the boulders holds — the generator
does what the prompt says, including the parts of it nobody thought about.

`--center` is the other mode they need: `archetype_lift` is 0.55 for a boulder
and 0.5 for a node against half-extents of 0.99 and 0.63, so these sit 0.44 m
and 0.13 m **into** the ground (`ART.md` rule 2). Every asset before them stood
on its base.

**The greybox massing is not deleted** — `archetype_mesh` still returns it,
`greybox.rs` still measures the sim's scalars against it, and `spawn_slot`
still draws it wherever a model is absent, which is every row on a headless
build.

## `held/` — what the viewmodel puts in your hand

**The surface these were waiting on turned out to already exist.** They were
held out of the tree on the belief that the client could not know *what* was in
the selected hotbar cell; `ClientCore.inv` has in fact carried every slot's
`ItemStack` since the container slice, and `catalog` the display names. No wire
change was needed and none was made — `RENDER.md` §6 carries the correction.

Resolved by **normalised display name** and nothing else, which is
`ui::hold`'s existing rule and the reason a rename in `content/items.toml`
breaks the icon, the mouse modality and the model in one place instead of
silently breaking one. `crates/client/tests/held_assets.rs` reads
`items.toml` and fails if any key here has no live item behind it.

| file | item | task id | credits |
|---|---|---|---|
| `held/rock.glb` | Rock | `019fefe3-2c88` | 24 |
| `held/stone_hatchet.glb` | Stone Hatchet | `019fefe7-e4b8` | 24 |
| `held/stone_pickaxe.glb` | Stone Pickaxe | `019fefe9-a24e` | 24 |
| `held/hammer.glb` | Hammer | `019feffa-b068` | 24 |
| `held/building_plan.glb` | Building Plan | `019feffc-45fe` | 24 |
| `held/wooden_spear.glb` | Wooden Spear | `019feffe-2c4b` | 24 |
| `held/hunting_bow.glb` | Hunting Bow | `019feff1-37f6` | 24 |

Three of these were regenerated once, and the two prompt failures are worth
keeping because they are the failure mode of the *prompt*, not the tool: the
building plan came back with a **disembodied hand** modelled into it (the size
note said "held in one hand"), and the spear came back as **two crossed
spears**. The hammer came back a sledgehammer. All three were fixed by saying
EXACTLY ONE and naming what not to draw.

**Nothing held emits light** and `tests/held_assets.rs` enforces it — the
generator ships `emissiveFactor = [1,1,1]` on nearly everything, and this
spear's map peaked at **0.53** before the import stripped it, i.e. a stick
that glows in the dark. A torch would be the first exception and would grow
that list rather than retire the rule.

## Textures are KTX2/UASTC at 1024, and the reason is VRAM

**Every `.glb` here carries GPU-compressed textures since 2026-08-11**
(`ci/ktx_pack.py`). The problem it solves is video memory, not disk: a JPEG is
decompressed to full RGBA8 on the GPU, so a 2048² map costs **16.8 MB of VRAM
whatever it weighs on disk**, and twelve props at three maps each was ~600 MB
of texture alone. Only a compressed-in-VRAM format moves that number.

Four options were encoded and measured on one prop rather than argued about:

| | disk/prop | VRAM/prop | albedo PSNR |
|---|---|---|---|
| 2K JPEG (what shipped) | 7.9 MB | ~50 MB | source, **no mipmaps** |
| 2K ETC1S | 1.4 MB | 8.4 MB | **28.0 dB — visibly lossy** |
| 2K UASTC | 11.8 MB | 16.8 MB | ~lossless, but disk *grows* |
| **1K UASTC — chosen** | **3.2 MB** | **4.2 MB** | **46.9 dB** |

UASTC over ETC1S because 28 dB is a drop you can see and the standing
instruction is not to nerf the look. 1024 over 2048 because that is what makes
UASTC *cheaper* than the JPEG it replaces instead of 1.5× dearer. **The
mipmaps are the quiet win**: the JPEGs had none at all, so every prop aliased
at distance, and a mipmapped 1K generally reads better in motion than a
shimmering 2K. Whole set: **95 MB → 42 MB.**

⚠ **These files are deliberately not spec-clean glTF, and it is not an
oversight.** The standard way to say "this texture is KTX2" is
`KHR_texture_basisu`; **Bevy 0.18 does not implement it** — its own support
table marks it ❌ (bevyengine#19104) — and its validator rejects any unknown
entry in `extensionsRequired` outright. Measured: twelve models, twelve
`Unsupported extension` errors and a white fallback on every one. What Bevy
*does* support is ktx2 dispatched off the **mime type**, so `textures[].source`
stays and only `images[].mimeType` is unusual (`image/ktx2`, where core glTF
allows jpeg and png). A third-party glTF tool may refuse these. Revisit when
#19104 lands.

**Re-encoding needs a tool a headless box has no reason to carry.** The
KTX-Software CLI is not in apt; fetch the release tarball from GitHub and point
`KTX_BIN`/`LD_LIBRARY_PATH` at it. Same class as the three `-dev` packages
`ci/gates.sh` names — install it rather than skipping the step. **The 2K
sources are kept out of tree**, so re-encoding at any resolution is a script
run and never a regeneration: `ci/import_meshy.py` then `ci/ktx_pack.py`.

## Why this one, and why a mannequin is the right placeholder

`ART.md` §7's "real detail is allowed, and preferred" is about textures and says
meshes are "the same deal when the time comes". This is that.

**It is a rig, not a character.** That matters more than it sounds. Every other
reachable CC0 humanoid pack is *stylized low-poly*, which would commit the game
to an art direction the operator has not spoken — and the one that IS spoken is
the reference set, like-for-like (`DECISIONS.md` 2026-08-01). A featureless
mannequin at human proportions reads as "player, untextured" rather than as the
wrong style, so it can be replaced by a clothed survivor later without anything
around it changing. The clips are the durable half; the mesh is scaffolding.

**Sourcing note, and it is a constraint on this box rather than a preference.**
Every 3D asset host is refused by this environment's egress policy — Poly Haven,
ambientCG, Quaternius's own site, poly.pizza, itch.io, Sketchfab, OpenGameArt.
GitHub is reachable and `git clone` works, which is the only reason this is here
at all. Mixamo was the operator's first suggestion and is **not** used: it needs
an Adobe account, so it can never be fetched by a loop, and its licence is
Adobe's own rather than CC0/CC-BY, so it would need its own spoken row.

## What the client actually uses

`render/anim.rs` names the clips it binds. The library carries 46 and this game
is not a driving game or a spellcaster, so most go unused — they cost nothing
but the 3.1 MB, and a later slice that adds swimming or a sword has them
already. **Do not trim the file to the used set**: re-vendoring is a copy, and a
trimmed copy is an edited asset that no longer matches its source.
