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
| file | `stumpy.glb`, **4.3 MB** |
| mesh | one skinned character, 21,360 verts / 31,252 tris, **24 joints**, split into `char1_arms` (8,184 tris) + `char1_body` for the first-person view |
| albedo | `Material_1_baseColor`, KTX2/UASTC 1024², **linear luma 0.056** — measured off the shipped file 2026-08-31 at mips 0/2/4/6 (0.0568 / 0.0564 / 0.0553 / 0.0556), 94% of texels in the darkest eighth. ⚠ **That is level with `grass` (0.054), the darkest identity in the game, and one thousandth over `ALBEDO_LUMA_BAND`'s 0.05 floor** — against granite 0.292, sand 0.178, twig 0.167 and litter 0.135. It is why the operator reads other players as dark in a frame (`NOW.md` §0dk), and it is the asset rather than the light: an A/B capture rules out SSAO at under 1% and the fill gives a shaded vertical face 14.3% of lit ground, which is physically correct. |
| clips | **55** — the delivery's 7, 46 retargeted off the mannequin, and guns v1's two (below) |
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

**Guns v1's two clips** (2026-10-18): `Rifle_Hold_Loop` (the library's
"Walk Forward While Shooting") and `Rifle_Reload` ("Standing Reload"), off
Meshy's animation library on its own rig of this mesh (rig task
`01a150ae-0851`, 5 credits; animate task `01a150ae-72ce`, 12 credits). The
library's skeleton is this one bone for bone, so no mannequin in between:

```
ci/retarget_anim.py <merged_animations>.glb assets/models/stumpy.glb out.glb \
    --identity --no-align --only Walk_Forward_While_Shooting --only Standing_Reload
```

then the two renamed. `render/anim.rs` plays the hold on the arms and spine of
a remote body shouldering a long gun, over its gait. `Rifle_Reload` rides along
unplayed: the wire does not yet say when somebody else reloads.

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

**The rig's hands are bent in the mesh, and the recipe is three lines**
(operator, 2026-09-01: *"we just need to close the fingers. also i can never
see the thumb"*). `stumpy.glb` arrived with five modelled digits per hand and
`RightHand` as a LEAF — the fingers were never rigged, so nothing can pose
them and the bind-pose rake was in every frame. `ci/curl_hands.py` bends the
vertices instead:

```
git show 44acd31:assets/models/stumpy.glb > /tmp/orig.glb
ci/curl_hands.py /tmp/orig.glb assets/models/stumpy.glb --degrees 150 \
    --thumb-degrees 90 --adduct 0.85 --into assets/models/stumpy.glb
```

⚠ **Run it on the ORIGINAL, never on its own output**, which is what `--into`
is for. Everything the tool derives comes off the geometry, and bent geometry
gives different answers — a second pass re-derived the LEFT hand's palm as
`+z` where the original gives `-x`, so it would bend about the wrong axis
while producing a curl number that looks right. `--into` takes the clips from
the shipped rig and only the hands from the freshly bent one, and refuses
unless every vertex neither hand moves already agrees.

The hand was at 85°/35°/0.45 (a relaxed hand) until 2026-09-01 and is now
150°/90°/0.85 (a grip). `crates/client/tests/rig_asset.rs` gates it, on a
length-to-thickness ratio rather than the fingertip offset it used to use —
that measure PEAKED at the 85° it shipped at and read a fist as an uncurled
hand. See `HAND_CURL_MAX`.

**The fingers have bones since 2026-10-06** (operator: *"we just need to work
on rigging fingers"*). `ci/rig_fingers.py` adds three per digit to both hands
(`RightHandIndex1..3` and so on, 30 in all), bound exactly where the curl
above put each point of the digit, so at rest the hand is the curled hand to
the vertex and nothing that reads the mesh moves. `render::fingers` closes the
right hand on a held row from there. Run it on the curled file, with the
original import beside it — it refuses unless bending that original
reproduces the shipped hands:

```
git fetch --depth=1 origin 44acd3139b7d4d401617c234290d0c2dc91a81c6
git show 44acd3139b7d4d401617c234290d0c2dc91a81c6:assets/models/stumpy.glb > /tmp/orig.glb
ci/rig_fingers.py /tmp/orig.glb assets/models/stumpy.glb assets/models/stumpy.glb
```

Re-curling means re-rigging: run `curl_hands.py` on an unrigged copy, then
this.

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

`crates/client/tests/prop_assets.rs` gates every number in this section, and
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

⚠ **Four of these rows are history since 2026-09-23** — `rock_a` and the three
nodes ship from `ci/rock_kit.py` now (§rock_kit below, *2026-09-23*), and a
fourth boulder, `rock_d`, joined the pool from the same tool. The rows are
kept because they say what the files WERE; `git show ecb9e21:assets/models/prop/<file>`
returns any of the four deliveries.

| file | occupant | image task | mesh task | tris | asked | size |
|---|---|---|---|---|---|---|
| ~~`prop/rock_a.glb`~~ | `Rock` (was pool 0) | `01a05e71-4efe` | `01a05e73-a70e` | 2,662 | 2,400 | 3.4 MB |
| `prop/rock_b.glb` | `Rock` (pool 2) | `01a05e7a-56ee` | `01a05e7a-f87b` | 2,670 | 2,400 | 3.4 MB |
| `prop/rock_c.glb` | `Rock` (pool 3) | `01a05e7d-075b` | `01a05e7d-811d` | 2,662 | 2,400 | 3.3 MB |
| ~~`prop/node_stone.glb`~~ | `StoneNode` | `01a05e6e-4baa` | `01a05e6e-c5a5` | 1,439 | 1,400 | 3.5 MB |
| ~~`prop/node_metal.glb`~~ | `MetalNode` | `01a05e85-6c8d` | `01a05e85-e691` | 1,364 | 1,250 | 3.4 MB |
| ~~`prop/node_sulfur.glb`~~ | `SulfurNode` | `01a05e7c-bfc4` | `01a05e7d-3978` | 1,490 | 1,400 | 3.5 MB |
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

### 2026-09-05 · two of the six keepers are the wrong object, and every map was bent

The operator read a frame: the boulder *"looks like mineable"*, the node is
*"square instead of spherical"*. Both are measurable off the files, and the
eleven-roll table above could not have caught either, because every number in
it is about SIZE and colour and the defect is shape:

| file | plan ratio | spread | luma | chart contrast | reads as |
|---|---|---|---|---|---|
| `rock_a.glb` | **1.17** | 0.12 | **0.344** | 0.139 | a pale 2 m ball — an ore node |
| `rock_b.glb` | 1.44 | 0.20 | 0.204 | 0.147 | keeps the new bands |
| `rock_c.glb` | 1.37 | 0.10 | 0.250 | 0.089 | keeps, marginally |
| `node_stone.glb` | **1.39** | 0.38 | 0.303 | **0.309** | a cube of quarried blocks |
| `node_metal.glb` | **1.39** | 0.27 | 0.190 | 0.178 | blocks |
| `node_sulfur.glb` | **1.75** | 0.20 | 0.259 | 0.143 | blocks, 0.39 m of skirt on its short axis |

Plan ratio is the widest over the narrowest direction of the footprint (1.0 a
circle, 1.39 what 36 bins make of a square); spread is how far the surface
is from a sphere; both are `ci/glbcharts.py`'s and `ci/measure_glb.py` now
rejects on them, with OPPOSITE bands for the two rows — a node must be round in
plan (the sim blocks a cylinder, and a square node's faces sit 0.26 m inside
it), a formation must not be a ball. The luma band is per row too: the node is
the pale rock and the formation the dark one, split at granite's 0.292, and
the shipped pair had that the wrong way round. `tests/prop_assets.rs` pins the
four offenders at today's numbers until their re-roll lands.

**Chart contrast is the third column and it is why the boulder wears
camouflage.** Decoded off the shipped KTX2 through Bevy's own transcoder, the
albedo's 116 UV islands disagree with each other by a texel-weighted **13.9 %**
of the map's luma (the node's 384 by 30.9 %): the generator paints each island
under its own light, and the seams follow polygon edges exactly.
`ci/flatten_charts.py` equalises them (0.139 → 0.000 on rock_a's decoded
albedo, per channel, gutter dilated); `tests/packed_maps.rs` holds what ships.

⚠ **And the larger half of the patchwork was the packer, not the generator**:
every normal map in this directory was linearised by `ci/ktx_pack.py` — see
§Textures below. Both halves are repaired by one re-pack per file.

**The two prompts, ready to roll** (`ci/meshy_gen.py … --tris 1400` for the
node, `--tris 2400` for the formation; then `ci/measure_glb.py --occupant
StoneNode|Rock` over every roll; six keepers from eleven is the rate to budget):

> **stone node** — "A single rounded weathered granite boulder on a plain white
> background, three-quarter view. A smooth water-worn dome, 1.83 metres wide,
> 1.83 metres deep and 1.65 metres tall, as deep as it is wide, sitting half
> sunk into the ground so the lower third is hidden. One continuous rounded
> mass with a few shallow fractures and small chipped facets, no separate
> blocks, no stacked stones, no cut faces, no cube. Pale grey granite with a
> light speckled crystalline surface, faint quartz veins, mottled with darker
> grey, no lichen, no moss, no green. Exactly one rock. No ground, no terrain,
> no people, no plants."  ·  texture: "Pale speckled granite, light grey with
> faint quartz veins and darker grey mottling, dry stone, no moss, no
> vegetation."
>
> **formation** (replaces `rock_a`) — "A single weathered granite outcrop on a
> plain white background, three-quarter view. An angular tilted slab of
> fractured bedrock, 2.2 metres wide, 2.2 metres deep and 2.0 metres tall, as
> deep as it is wide, leaning at a slight angle with one large flat inclined
> face, sharp broken edges, horizontal bedding lines and a fractured corner,
> partly sunk into the ground. Not rounded, not a dome, not a ball, not a
> boulder. Dark cool grey granite, matte, with rust-brown staining in the
> cracks, no moss, no lichen, no green. Exactly one rock. No ground, no
> terrain, no people, no plants."  ·  texture: "Dark cool grey weathered
> granite, matte, fractured, rust-brown staining in the cracks, dry stone, no
> moss, no vegetation."

Naming the omitted axis and naming what to avoid are the two levers this
table measured as working; "not a ball" is the second lever aimed at the
first defect. Moss is left out of the first rolls on purpose — the green
ceiling is 10 % and "lichen in the hollows" returned 47.6 % — and a mossy
variant is a second roll type once a keeper exists. The node's 1.65 m height
is for the PROPOSED lift of 0.3 (`DECISIONS.md` §open, scatter art v1): the
import height is `2 × (OCCUPANT_TOP_M − lift)`, so a lower lift buries more of
the same blocked volume and the collision does not move; at the shipped lift
of 0.5 the height is 1.25 m.

### 2026-09-16 · all 23 re-packed, and the sources were gone

The operator read a frame again: the rock texture is *"janky"*, the ore *"still
a square thing"*. The first half is repaired here; the second is a shape and is
still open below.

**The blocker nobody had hit yet: there is nothing to re-pack FROM.** §0rk item
1 says to re-pack "from their raw deliveries", which live in `assets/models/To
Examine/` — gitignored on purpose, ~600 MB, and therefore absent from every
clone. The only surviving copy of a prop's maps is the KTX2 already inside its
own `.glb`. `ci/unbake_ktx.py` is that missing step: decode level 0 back
through `ktx extract`, invert the packer's sRGB→linear on the data maps, emit a
PNG-imaged GLB for `flatten_charts.py` and `ktx_pack.py` to take from there.

**The inversion is exact, and two readings say so without being fitted:**

| | shipped | repaired | why that is the right number |
|---|---|---|---|
| normal X/Y | 0.212 | 0.498 | a tangent-space map centres on 0.500 |
| roughness | 0.411 | 0.673 | §prop above, written three weeks earlier: "~0.67 was delivered" |

**And one check that is not arithmetic at all.** A tangent-space normal map is
a field of UNIT vectors — a property of what the file means, which a subtly
wrong curve cannot satisfy. Across the 23: **0.00–0.93 % of texels decoded to
unit length before, 99.42–100 % after.** That is `--verify`'s bar and it is
`packed_maps.rs`'s own 90 %.

Chart contrast fell everywhere in the same pass, worst residual the crate at
0.045 against a `CHART_CONTRAST_MAX` of 0.06 (rock_a 0.139 → 0.000, node_stone
0.309 → 0.033, barrel 0.328 → 0.041). Both lists in `packed_maps.rs` are empty
now, kept as named empties because an empty list cannot rot.

⚠ **Geometry is byte-identical** — the accessor blocks hash the same before and
after on every file spot-checked, so no collision volume, no shape pin and no
`prop_assets.rs` row moved. This repair is textures and nothing else. The cost
is one extra UASTC generation (worst texel error 0.0026, pure 8-bit
quantisation), which is the price of having thrown the sources away and is far
below the 41° bend it removes.

⚠ **Nobody has seen it in a frame** (`§LOOK`). Every number here is a decode.

## `ci/rock_kit.py` — the Blender stage, and the rail a generated piece lands on

**Our own work, on no licence at all.** A piece `rock_kit.py gen` makes is
authored by a script in this tree from a seed; Blender's GPL covers the
program and not what it exports, so no vendor, no plan, no `NOTICE` entry —
the provenance is the sidecar. Landed 2026-09-14 with nothing shipped; **five
props ship from it since 2026-09-23** — the three ore nodes, `rock_a` and
`rock_d` (§*2026-09-23* at the end of this section). The 2026-09-14 table
below stayed in a scratch directory and was measured there.
`WANTED.md` §0.4 says when to reach for it instead of Meshy;
`reference/ROCKS.md` §9 is why the cliff and formation tiers need it.

The pipeline is the prop pipeline with one command swapped:
`rock_kit.py gen` (or `meshy_gen.py` → `rock_kit.py edit`) → `measure_glb.py`
→ `import_meshy.py --fit-radius --center` (a no-op on a kit piece: measured
`x1.000 y1.000 z1.000`, origin kept) → `ktx_pack.py`. The sidecar
`<file>.glb.json` carries what a row here needs — `tool`, `script_sha256`,
`seed`, `kind`, `target_m` and where it was read from, every derived noise
parameter, `blender`, `measured` (the triage's own numbers) and `verdict` —
so a row is transcribed, not remembered.

**Measured on the Rock row** (`occupant_volume`: r 1.1145, top 1.5403, lift
0.55, so a 2.229 × 1.981 × 2.229 m target), 1024² maps, 2,352 triangles, one
box of four cores, ~50 s a piece with three preview renders:

| piece | plan | spread | luma | chart | MB | `measure_glb.py` |
|---|---|---|---|---|---|---|
| `gen --seed 1` | 1.820 | 0.133 | 0.210 | 0.006 | 1.2 | KEEP |
| `gen --seed 2` | 1.452 | 0.114 | 0.209 | 0.004 | 0.9 | KEEP |
| `gen --seed 3` | 1.452 | 0.109 | 0.209 | 0.008 | 1.2 | KEEP |
| `edit` of seed 1 to 882 tris, rebaked | 1.830 | 0.124 | 0.210 | 0.012 | 0.4 | KEEP |
| `gen --kind formation --size 6 4 6` (5,428 tris) | 2.412 | 0.307 | 0.210 | 0.009 | 1.5 | KEEP |
| `gen --kind slab --size 8 5 3` (1,574 tris) | 3.051 | 0.252 | 0.209 | 0.001 | 1.1 | **reject: depth/width 0.38** — the boulder's band; a slab has no row yet |

Every boulder sits on the row exactly (radius 1.1145, height 1.9806, the
importer's factors 1.000), the normal maps decode to X/Y means 0.499 / 0.502
with 100 % of texels unit length, roughness reads 0.84 over the rock, and the
same seed regenerates a **byte-identical GLB** (three maps and the blob
compared). Nobody has looked at one.

**Three things it found, each now in the script and one in `CLAUDE.md`'s
trap list.** Blender's `Image.pixels` starts at the **bottom**-left, so a map
written from that buffer is upside down against the albedo Blender saves
itself; its glTF exporter writes `v' = 1 − v`; and a whole-image mean over a
baked map is a mean over the empty space outside the UV islands (roughness
read 0.52 for a map of 0.72–0.96 at 60 % coverage). The kit builds its
coverage mask with `glbcharts.rasterize`, calibrates the albedo's luma over
the covered texels only, and fills the rest with the rock's own mean — so
`measure_glb.py`'s whole-image luma and the surface's mean are one number by
construction. A Meshy delivery's background is whatever the generator left.

### 2026-09-23 · five pieces ship: the three ore nodes and a boulder pair

**The first kit output in the tree.** The node needed a kind the kit did not
have — every existing kind's squash is unequal in plan, which is right for a
boulder and exactly what a node's round band forbids — so `--kind node` was
added (equal squash, 12–18 upward-leaning **chips** that shear a cap flat, the
bottom third cut and buried), and three **looks** over one shared rock:
`stone` (pale speckled granite, quartz veins, glossy white crystals), `metal`
(grey rock, dark METALLIC seams that swell and pinch, a rust halo — the only
texels in the kit with a non-zero ORM blue channel) and `sulfur` (a cool grey
rock under a crystalline yellow crust packed into its cavities). `rock_a` and
the new `rock_d` are the 2026-09-14 `boulder` kind's geometry — the same seeds
rebuild the same meshes — under a fourth, opt-in look, `formation`: the dark
rock on the same machinery, because the first cut's `granite` read as clean
clay beside the generated pair it is pooled with. `granite` stays the default,
so a documented seed still builds the rock it was measured as — on this box the
2026-09-14 seeds rebuild their plan and spread to three decimals, and their
bake reads a few thousandths off (seed 1: luma 0.209 against 0.210, chart 0.004
against 0.006), which is a different CPU under the same Cycles seed and is why
byte-identity is only ever claimed per box. `props.rs` pools the two kit
boulders first so each rock family draws two silhouettes. Numbers and reasons:
`DECISIONS.md` §open, scatter art v2.

```
ci/rock_kit.py gen --occupant StoneNode --kind node --seed 31 --tris 1400 --out node_stone.glb
ci/rock_kit.py gen --occupant MetalNode --kind node --seed 43 --tris 1400 --out node_metal.glb
ci/rock_kit.py gen --occupant SulfurNode --kind node --seed 53 --tris 1400 --out node_sulfur.glb
ci/rock_kit.py gen --occupant Rock --kind boulder --look formation --seed 1 --out rock_a.glb
ci/rock_kit.py gen --occupant Rock --kind boulder --look formation --seed 3 --out rock_d.glb
# then, per file: import_meshy.py <in> <out> W H D --fit-radius --center ; ktx_pack.py <out> assets/models/prop/<file>
```

| file | occupant | kind · look · seed | tris | plan | spread | luma | chart | metal texels | packed |
|---|---|---|---|---|---|---|---|---|---|
| `prop/node_stone.glb` | `StoneNode` | node · stone · 31 | 1,372 | 1.139 | 0.127 | 0.341 | 0.039 | — | 1.7 MB |
| `prop/node_metal.glb` | `MetalNode` | node · metal · 43 | 1,372 | 1.194 | 0.111 | 0.260 | 0.025 | 24.9 % | 1.7 MB |
| `prop/node_sulfur.glb` | `SulfurNode` | node · sulfur · 53 | 1,372 | 1.099 | 0.098 | 0.299 | 0.012 | — | 1.5 MB |
| `prop/rock_a.glb` | `Rock` (pool 0) | boulder · formation · 1 | 2,352 | 1.820 | 0.133 | 0.210 | 0.025 | — | 1.7 MB |
| `prop/rock_d.glb` | `Rock` (pool 1) | boulder · formation · 3 | 2,352 | 1.452 | 0.109 | 0.210 | 0.049 | — | 1.7 MB |

Measured by the triage off each piece before packing (it cannot decode KTX2);
`tests/packed_maps.rs` re-reads the packed albedo through Bevy's own transcoder
and gets the same chart contrast to three decimals, and every normal map 100 %
unit length at X/Y 0.500–0.502. **Five files, 8.4 MB, where the four they
replace were 13.5 MB.** All five rebuild **byte-identical** from the committed
script (`ci/rock_kit.py` sha256 `f9e5ddbe12445481…`, bpy 4.5.0) with the
commands above — checked on the box they were made on, before packing.

**Selected, not accepted.** On the final shape the plan band took two of six
stone and metal rolls out (seed 33 at 1.215, seed 42 at 1.227), and chart
contrast took two sulfur rolls (0.070, 0.071) until the crust's contrast
against its rock was narrowed — a kit bake is lit uniformly, so a chart reject
here always means the content is spread unevenly across islands, never
lighting. The keepers are the roundest that passed, except metal: seed 43
(1.194) over 41 (1.175), whose chips met in a tent-shaped ridge. Every keeper
was then looked at in a lineup rendered the way the game seats it
(`archetype_lift − SINK_M`, yawed, on turf, a sun and a cool sky fill, Khronos
PBR Neutral — Blender's nearest view transform to the game's TonyMcMapface),
beside the files it replaces. Three rounds of that looking changed the tool
rather than the bands: a voronoi crack is a net and reads as a cracked egg
(cracks are warped noise isolines now); a bright metal mirrors the sky and
reads as blue paint (the seams are a dark metal that flashes); and the first
sulfur graph had a cycle in it, which Cycles resolves by dropping links — it
baked at luma 0.48 against a ramp of 0.24 and every number downstream looked
plausible. A constant-colour control (0.2 in, 0.191 out) is what told the graph
from the bake.

⚠ **Nobody has seen these in the game** (`NOW.md` §LOOK, 2026-09-23). The
metal node's glint depends on Bevy's environment light, which Blender does not
reproduce; and a node on a slope floats on its downhill edge exactly as the
cubes did, because the lift still buries 0.13 m (scatter art v1's 0.3 is the
fix, and `greybox.rs` holds the fallback blob to the same lift).

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
| `held/stone_hatchet.glb` | Stone Hatchet | `019fefe7-e4b8` | 24 | **stood up** 2026-09-01 |
| `held/stone_pickaxe.glb` | Stone Pickaxe | `019fefe9-a24e` | 24 |
| `held/hammer.glb` | Hammer | `019feffa-b068` | 24 |
| `held/building_plan.glb` | Building Plan | `019feffc-45fe` | 24 |
| `held/wooden_spear.glb` | Wooden Spear | `019feffe-2c4b` | 24 |
| `held/hunting_bow.glb` | Hunting Bow | `019feff1-37f6` | 24 | **stood up** 2026-09-01 |

**Two of these ship a second baked transform, and the reason is a defect that
was in every frame.** `ci/import_meshy.py` centres the BOUNDING BOX on X/Z,
and `ui::hold` spends the grip as one number up the model's own +Y — two
statements that agree only when the thing the hand closes on is itself on +Y.
The hatchet was authored leaning **32°** with a heavy head off one side, so
the box centred on the head and the haft sat **121 mm** from the axis at the
grip height; `viewmodel::pose` then slid thin air into the palm and the axe
hung a hand's width beside the fist, pointing across the frame. The bow was
the same at **165 mm** across a 45° diagonal. Both were stood up by
`ci/stand_grip.py` — a rigid rotation and a slide, no resampling, no rescale,
so the axe and the bow are exactly the size they always were:

```
ci/stand_grip.py assets/models/held/stone_hatchet.glb <out> --shaft 0.02 0.45 --grip 0.25
ci/stand_grip.py assets/models/held/hunting_bow.glb   <out> --shaft 0.02 0.98 --grip 0.50
```

`height_m` in `HELD_MODELS` moved with them — 0.500 → 0.562 and 1.191 → 1.687
— because a diagonal's shadow is shorter than the diagonal, and
`crates/client/tests/held_assets.rs::the_fist_closes_on_the_model_and_not_on_air`
is the gate that now refuses the whole class.

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

## `held/` — the prop kit's version 2, batch 1 (Meshy, 2026-10-15)

Ten items that drew the code kit's primitives (`render/heldkit.rs`) and now
draw these, in the hand, on the ground and as their icon (`iconbake`
photographs the file). `WANTED.md` §12 is the worklist and its routing.
Meshy paid plan, the `deploy/` rail's terms: no `NOTICE` entry, no proper
noun in any prompt (below, in full).

Pipeline, every step a script in `ci/`: `meshy_gen.py --tris 3000`
(nano-banana-pro image → meshy-t2 smart topology; `--multiview` → meshy-7.1
multi-image) → `measure_glb.py --size` → `flatten_charts.py` where it helps
→ `import_meshy.py` to a CUBE the size of the item's longest side, so the
object keeps its own proportions → `ktx_pack.py --size 512` (a hand-sized
thing, and the browser downloads each one at boot: ~1 MB a file).

| file | item | image task | mesh task | credits | size (m) | note |
|---|---|---|---|---|---|---|
| `held/wood.glb` | Wood | `01a13df9-132f` | `01a13df9-92fc` | 24 | 0.46 × 0.27 × 0.47 | charts flattened |
| `held/stone.glb` | Stone | `01a13e01-8e0e` | `01a13e02-6222` | 39 | 0.24 × 0.22 × 0.13 | 3 views; remeshed after delivery, see below |
| `held/metal_ore.glb` | Metal Ore | `01a13e06-1cc1` | `01a13e06-9b16` | 29 | 0.23 × 0.14 × 0.22 | charts flattened; credits overlap the stone's remesh |
| `held/sulfur_ore.glb` | Sulfur Ore | `01a13e07-fd79` | `01a13e08-a1c2` | 39 | 0.23 × 0.19 × 0.17 | as delivered (flattening turned its rock olive); 3 views |
| `held/cloth.glb` | Cloth | `01a13e0b-8292` | `01a13e0c-54b5` | 24 | 0.30 × 0.14 × 0.29 | as delivered (chart contrast 0.012 already) |
| `held/metal_fragments.glb` | Metal Fragments | `01a13e0d-6c57` | `01a13e0d-c1cc` | 24 | 0.25 × 0.10 × 0.25 | charts flattened |
| `held/charcoal.glb` | Charcoal | `01a13e0e-ab7d` | `01a13e0f-2a0c` | 24 | 0.27 × 0.14 × 0.27 | charts flattened |
| `held/low_grade_fuel.glb` | Low Grade Fuel | `01a13e0f-e93e` | `01a13e10-6864` | 24 | 0.26 × 0.33 × 0.12 | charts flattened |
| `held/junk.glb` | Junk | `01a13e11-508e` | `01a13e11-cfb8` | 24 | 0.34 × 0.18 × 0.30 | charts flattened |
| `held/leather.glb` | Leather | `01a13e12-b919` | `01a13e13-600e` | 24 | 0.34 × 0.11 × 0.11 | as delivered (flattening washed it out) |

**275 credits for the batch** (balance 3,124 → 2,849, measured). The
per-row figures are balance deltas, and two ran concurrently with the
stone's remesh, so the metal ore's 29 holds part of that.

What was learned, measured:

- **`--multiview` dropped `target_polycount`**: the stone came back at
  **580,271 triangles, 26 MB**. `multi-image-to-3d` only honours the budget
  when told to remesh, which `ci/meshy_beast.py` already did and
  `meshy_gen.py` now does (`should_remesh`). The sulfur ore, made after the
  fix, landed at 3,113 with its normal map. The stone was rescued with the
  Remesh API (3,135 triangles), which re-bakes base colour and metal/rough
  only, so it ships **without a normal map** (`tests/packed_maps.rs`
  `NO_NORMAL_MAP` names it) and read the same in every render.
- **`--multiview` can return one view.** The metal ore asked for several
  and got one, so it went the single-view way; it reconstructed fine, 0.93
  deep per unit wide, because the prompt names the depth.
- **A pile asked for multiple views came back STACKED**: the stone is a
  three-rock cairn, not the low pile the prompt describes. It reads as stone
  and shipped; a re-roll would say "lying side by side, none on another".
- **Chart flattening is per item, not a default.** It equalised the wood,
  ore, fragments, charcoal, can and junk with no visible loss, turned the
  sulfur ore's grey rock olive (it pulls each island toward the mean colour,
  and the mean is part yellow) and washed the leather's browns out. Look,
  then choose.
- **`measure_glb.py`'s depth band is a rock's.** The can (0.45) and the
  leather roll (0.33) "fail" it by being what they are.
- **`import_meshy.py` now prunes** the textures no material reads. Meshy's
  multi-image path ships an emissive map; the import strips the slot, and
  until now `ktx_pack.py` then packed the orphan anyway. The held items
  shipped before this each carry one, small on disk (0.8-44 KB packed,
  mostly black) and harmless.

Prompts, in full, for the IP rail:

> **wood** — "A single small stack of split firewood on a plain white background, three-quarter view. Five split logs lying side by side in two loose layers, each piece about 45 centimetres long and 10 centimetres thick; the whole stack 47 centimetres long, 38 centimetres deep and 24 centimetres tall, as deep as it is long. Wedge-shaped and half-round pieces from splitting: rough grey-brown pine bark on the rounded sides, pale cream split faces with long grain and drying cracks, chopped ends showing growth rings. Real, weathered, slightly dirty, soft daylight. Exactly one stack. No ground, no terrain, no axe, no people, no hands, no plants, no text, no labels, no scale bar."  ·  texture: "Split pine firewood: rough grey-brown bark, pale cream split wood with long grain and checking cracks, chopped ends with growth rings, dry and dusty, no moss, no green, no paint"
>
> **stone** — "A single small pile of three fist-sized broken stones on a plain white background, three-quarter view. Three rough angular rocks of pale grey granite lying together, the pile 24 centimetres wide, 22 centimetres deep and 12 centimetres tall, as deep as it is wide. Fresh fracture faces with sharp edges, a light speckled crystalline surface with faint quartz flecks, mottled with darker grey, dry and dusty. Real, photographed in soft daylight. No moss, no lichen, no green. Exactly one pile. No ground, no terrain, no people, no hands, no plants, no text, no labels, no scale bar."  ·  texture: "Pale speckled granite, light grey with faint quartz flecks and darker grey mottling, fresh fracture faces, dry dusty stone, no moss, no green"
>
> **metal_ore** — "A single small pile of four fist-sized chunks of iron ore on a plain white background, three-quarter view. Heavy angular lumps of mid grey rock shot through with dark metallic iron seams and rust-brown and ochre staining, a few small dull metallic glints, the pile 23 centimetres wide, 23 centimetres deep and 13 centimetres tall, as deep as it is wide. Real, rough, dusty, photographed in soft daylight. The host rock is mid grey, not black. No moss, no green. Exactly one pile. No ground, no terrain, no people, no hands, no plants, no text, no labels, no scale bar."  ·  texture: "Mid grey rock with dark metallic iron seams, rust-brown and ochre staining, small dull metal glints, rough dusty ore, not black, no green"
>
> **sulfur_ore** — "A single small pile of four fist-sized lumps of sulfur ore on a plain white background, three-quarter view. Rough grey rock chunks crusted with warm golden-yellow mustard sulfur crystals in the cracks and over one face of each lump, the pile 23 centimetres wide, 23 centimetres deep and 14 centimetres tall, as deep as it is wide. Real, dusty, photographed in soft daylight. The yellow is warm golden mustard, never green, never lime. Exactly one pile. No ground, no terrain, no people, no hands, no plants, no text, no labels, no scale bar."  ·  texture: "Grey rock crusted with warm golden-yellow mustard sulfur crystals, dusty, matte, never green, never lime"
>
> **cloth** — "A single neat stack of three folded pieces of rough undyed cloth on a plain white background, three-quarter view. Coarse plain-woven fabric, off-white to pale oatmeal, folded into rectangles and stacked slightly askew, frayed edges and a few loose threads, faintly dirty and creased. The stack 30 centimetres wide, 22 centimetres deep and 14 centimetres tall. Real, photographed in soft daylight. Exactly one stack. No ground, no table, no people, no hands, no plants, no text, no labels, no scale bar."  ·  texture: "Coarse undyed plain-woven cloth, off-white to pale oatmeal, visible weave, frayed edges, faint dirt and creases, matte, no pattern, no print"
>
> **metal_fragments** — "A single small heap of torn scrap metal fragments on a plain white background, three-quarter view. Jagged bent pieces of steel sheet, twisted strips and broken metal chunks lying together, mostly rust-brown and dull grey steel, with bright silver fresh cuts along the torn edges. The heap 25 centimetres long, 21 centimetres wide and 10 centimetres tall. Real, dirty, photographed in soft daylight. Exactly one heap. No tools, no nails, no screws, no ground, no people, no hands, no plants, no text, no labels, no scale bar."  ·  texture: "Torn scrap steel: rust-brown oxide and dull grey steel, bright silver fresh cuts on the torn edges, dirty, oily, no paint"
>
> **charcoal** — "A single small heap of lump charcoal on a plain white background, three-quarter view. Irregular chunks of charred hardwood, deep charcoal grey with a silvery sheen on the cracked faces and pale grey ash dust in the cracks, the wood grain and growth rings still visible on the broken ends. The heap 27 centimetres wide, 25 centimetres deep and 15 centimetres tall, as deep as it is wide. Real, photographed in soft daylight. Exactly one heap. No fire, no flames, no glowing embers, no ground, no people, no hands, no plants, no text, no labels, no scale bar."  ·  texture: "Lump charcoal: deep charcoal grey charred wood with a silvery sheen on cracked faces, pale grey ash in the cracks, visible wood grain, matte, no glow, no fire"
>
> **low_grade_fuel** — "A single steel jerry can standing upright on a plain white background, three-quarter view. A flat rectangular fuel can 33 centimetres tall, 22 centimetres wide and 10 centimetres deep, three bar handles across the top, a capped spout on one corner, pressed X-shaped ribs on its broad sides. Dented, its dull red paint chipped and worn through to bare grey steel on the edges and handles, rust at the seams, grimy. Real, photographed in soft daylight. Exactly one can. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Dull red painted steel worn through to bare grey metal on the edges and handles, chipped paint, rust at the seams, grime, dents, no text, no logos"
>
> **junk** — "A single small heap of rusty scrap metal junk on a plain white background, three-quarter view. A large hex nut, a long threaded bolt, a broken cog wheel, a bent steel bracket and a short coil spring lying together, mostly rust-brown and dark grey steel with oily dirt. The heap 34 centimetres long, 27 centimetres wide and 21 centimetres tall. Real, photographed in soft daylight. Exactly one heap. No tools, no wood, no ground, no people, no hands, no plants, no text, no labels, no scale bar."  ·  texture: "Rusty scrap steel: rust-brown oxide, dark grey bare steel, oily dirt, worn threads and teeth, no paint"
>
> **leather** — "A single rolled-up tanned animal hide on a plain white background, three-quarter view. A cylinder of rough brown leather 34 centimetres long and about 13 centimetres thick, tied around with two thin leather cords, the loose edge of the hide lying along the roll, the paler suede underside showing at both ends of the roll. Worn, creased, uneven natural edges. Real, photographed in soft daylight. Exactly one roll. No fur, no ground, no people, no hands, no plants, no text, no labels, no scale bar."  ·  texture: "Rough tanned brown leather with creases and natural grain, paler suede underside at the ends, thin leather cords, worn, matte, no fur, no print"

## `held/` — the prop kit's version 2, batch 2 (Meshy, 2026-10-15)

Ten more, the same pipeline and terms as batch 1 above: the powders, the
fat, the bones, the tarp, both waterskins and the three gun bodies. Piles and
folded things are `borne` rows, the skins and bodies `upright`, each keeping
its kit row's grip and scale (`iconbake --hand`, judged against batch 1's
stone and wood); the powders went from 0.50 to 0.60, the files being lower
mounds than the kit's cones.

| file | item | image task | mesh task | credits | size (m) | note |
|---|---|---|---|---|---|---|
| `held/sulfur.glb` | Sulfur | `01a13e37-c4b7` | `01a13e38-43f9` | 24 | 0.34 × 0.09 × 0.34 | as delivered (chart contrast 0.019 already) |
| `held/gunpowder.glb` | Gunpowder | `01a13e39-0593` | `01a13e39-8504` | 24 | 0.34 × 0.10 × 0.34 | as delivered (0.028) |
| `held/animal_fat.glb` | Animal Fat | `01a13e3a-708d` | `01a13e48-d49f` | 24 | 0.26 × 0.18 × 0.19 | as delivered (0.051); the mesh failed once, see below |
| `held/bone_fragments.glb` | Bone Fragments | `01a13e3b-f8c2` | `01a13e3c-78f0` | 24 | 0.23 × 0.11 × 0.27 | charts flattened |
| `held/tarp.glb` | Tarp | `01a13e3d-dd49` | `01a13e3e-32ce` | 24 | 0.36 × 0.15 × 0.29 | as delivered (0.039) |
| `held/waterskin.glb` | Waterskin | `01a13e3f-1fe5` | `01a13e3f-a695` | 24 | 0.18 × 0.31 × 0.08 | charts flattened |
| `held/full_waterskin.glb` | Full Waterskin | `01a13e40-691c` | `01a13e40-bf45` | 24 | 0.20 × 0.32 × 0.15 | charts flattened |
| `held/semi_automatic_body.glb` | Semi Automatic Body | `01a13e41-d335` | `01a13e42-54aa` | 24 | 0.38 × 0.21 × 0.05 | charts flattened |
| `held/smg_body.glb` | SMG Body | `01a13e4e-2c45` | `01a13e4e-ac50` | 48 | 0.32 × 0.16 × 0.07 | second roll, charts flattened; see below |
| `held/rifle_body.glb` | Rifle Body | `01a13e44-4cbd` | `01a13e44-cc0d` | 24 | 0.46 × 0.16 × 0.16 | charts flattened |

**264 credits for the batch** (balance 2,849 → 2,585, measured): eleven
rolls at 24, the ten items and the SMG body's second. The fat's failed first
mesh cost nothing; its 24 is the image and the mesh built again from it (15).

What was learned, measured:

- **A gun part asked for as "a receiver" can come back as a gun.** The SMG
  body's first roll (`01a13e43-0aee` / `01a13e43-89d4`, kept in
  `To Examine/kit_v2/smg_body_roll1_*`) was a chunky pistol with a round
  breech. The second prompt names the shape instead of the part — "a long
  straight rectangular tube of stamped sheet steel", the magazine well under
  its front half, the grip under its back half — and lists what is not there
  (barrel, stock, magazine, sights, muzzle); it reads as a receiver.
- **A failed mesh keeps its image.** The fat's first image-to-3d came back
  `server_error` ("Failed to generate preview images"); the image task was
  fine, so `meshy_gen.py --resume-image 01a13e3a-708d-74c9-810a-f694116d92ed`
  built the mesh from it for 15 credits instead of a fresh 24.
- **The generated guns are authored muzzle to -X**, the kit's revolver to
  +X. `iconbake`'s `side()` turns a file round so every gun in the bag points
  the same way; in the hand they are parts and point where they were made.
- **`measure_glb.py`'s depth band rejects every gun body** (a receiver is a
  fifth as deep as it is long) and its aspect gate the sulfur heap (a mound
  is wide and low). Both are what they should be; the gates are a rock's.

Prompts, in full, for the IP rail:

> **sulfur** — "A single small heap of yellow sulfur powder on a plain white background, three-quarter view. Fine crumbly powder in a low rounded mound with a few small broken lumps on top, 34 centimetres wide, 34 centimetres deep and 17 centimetres tall, as deep as it is wide. Warm golden-yellow mustard sulfur, dusty, matte. Real, photographed in soft daylight. Never green, never lime. Exactly one heap. No container, no bag, no ground, no people, no hands, no plants, no text, no labels, no scale bar."  ·  texture: "Warm golden-yellow mustard sulfur powder, fine and crumbly, dusty, matte, a few small lumps, never green, never lime"
>
> **gunpowder** — "A single small heap of black gunpowder on a plain white background, three-quarter view. Fine dark grey-black granular powder in a low rounded mound, a few tiny grains scattered at its foot, 34 centimetres wide, 34 centimetres deep and 16 centimetres tall, as deep as it is wide. Matte charcoal grey with a faint graphite sheen on the grains, not pure black. Real, photographed in soft daylight. Exactly one heap. No container, no bag, no fuse, no ground, no people, no hands, no plants, no text, no labels, no scale bar."  ·  texture: "Fine dark grey-black granular gunpowder, matte charcoal grey with a faint graphite sheen on the grains, not pure black, no glow"
>
> **animal_fat** — "A single slab of raw animal fat on a plain white background, three-quarter view. A thick irregular chunk of white suet with a waxy surface, faintly yellow and creamy, uneven torn edges and a few thin pink streaks, 26 centimetres long, 20 centimetres deep and 17 centimetres tall. Real, photographed in soft daylight. Exactly one piece. No plate, no board, no knife, no ground, no people, no hands, no plants, no text, no labels, no scale bar."  ·  texture: "Raw white animal fat, waxy creamy suet, faintly yellow, thin pink streaks, slightly glossy, no plate, no blood pool"
>
> **bone_fragments** — "A single small pile of broken animal bones on a plain white background, three-quarter view. Two snapped leg bones with knobbed joint ends and a few sharp splinters lying together, weathered off-white and pale ivory with faint brown staining in the cracks, the pile 27 centimetres long, 23 centimetres wide and 9 centimetres tall. Real, photographed in soft daylight. Exactly one pile. No skull, no meat, no blood, no ground, no people, no hands, no plants, no text, no labels, no scale bar."  ·  texture: "Weathered off-white and pale ivory animal bone, porous ends, faint brown staining in the cracks, dry, matte, no blood"
>
> **tarp** — "A single folded canvas tarpaulin on a plain white background, three-quarter view. Heavy olive-drab waterproof canvas folded into a flat rectangular bundle, two brass grommets at one corner, creased, dusty and stained, 36 centimetres wide, 26 centimetres deep and 10 centimetres tall. Real, photographed in soft daylight. Exactly one bundle. No rope, no ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Heavy olive-drab waterproof canvas, visible weave, creases, dust and stains, brass grommets, matte, no text, no logos"
>
> **waterskin** — "A single empty leather waterskin on a plain white background, three-quarter view. A flat teardrop-shaped bag of dark tanned leather, slack and creased because it is empty, stitched along its edges, a short leather neck with a wooden stopper and a cord tied around the neck, standing upright, 31 centimetres tall, 18 centimetres wide and 9 centimetres deep. Real, worn, photographed in soft daylight. Exactly one waterskin. No water, no cup, no ground, no people, no hands, no plants, no text, no labels, no scale bar."  ·  texture: "Dark tanned leather, creased and slack, stitched seams, worn edges, wooden stopper, cord tie, matte, no print"
>
> **full_waterskin** — "A single full leather waterskin on a plain white background, three-quarter view. A plump teardrop-shaped bag of dark tanned leather, full and taut with water, darker and slightly glossy where it is damp, stitched along its edges, a short leather neck with a wooden stopper and a cord tied around the neck, standing upright, 32 centimetres tall, 20 centimetres wide and 15 centimetres deep. Real, worn, photographed in soft daylight. Exactly one waterskin. No cup, no puddle, no ground, no people, no hands, no plants, no text, no labels, no scale bar."  ·  texture: "Dark tanned leather, full and taut, darker and slightly glossy where damp, stitched seams, wooden stopper, cord tie, no print"
>
> **semi_automatic_body** — "A single bare semi-automatic rifle receiver on a plain white background, three-quarter side view. Only the central metal body of the gun: the receiver with its trigger guard, trigger and a pistol grip, an ejection port and a short stub where the barrel would mount. No stock, no barrel, no magazine, no scope. Worn black and gunmetal-grey finish with scratches and bare steel on the edges, 38 centimetres long, 14 centimetres tall and 8 centimetres wide. Real, photographed in soft daylight. Exactly one part. No ground, no people, no hands, no ammunition, no text, no labels, no logos, no scale bar."  ·  texture: "Worn black and gunmetal-grey steel, scratches, bare metal on the edges, oil grime, matte, no text, no logos"
>
> **smg_body** — "A single bare submachine gun receiver on a plain white background, seen from the side, slightly from above. A long straight rectangular tube of stamped sheet steel lying horizontally, 32 centimetres long, about 5 centimetres tall and 5 centimetres wide, with a narrow magazine well hanging under its front half, a pistol grip and a trigger guard under its back half, a slot for the cocking handle and rivets along its side. Only the receiver: no barrel, no stock, no magazine, no sights, no muzzle. Worn black paint over steel, scratched to bare metal on the edges. Real, photographed in soft daylight. Exactly one part. No ground, no people, no hands, no ammunition, no text, no labels, no logos, no scale bar."  ·  texture: "Worn black paint over stamped steel, scratched to bare metal on the edges, rivets, oil grime, matte, no text, no logos"
>
>
> **smg_body**, first roll (not shipped) — "A single bare submachine gun receiver on a plain white background, three-quarter side view. Only the compact boxy stamped-steel body of the gun with a magazine well underneath, a pistol grip, a trigger and trigger guard and a cocking handle on the side. No stock, no barrel, no magazine, no sights. Worn black finish with scratches and bare steel on the edges, 32 centimetres long, 17 centimetres tall and 6 centimetres wide. Real, photographed in soft daylight. Exactly one part. No ground, no people, no hands, no ammunition, no text, no labels, no logos, no scale bar."
>
> **rifle_body** — "A single bare bolt-action rifle action on a plain white background, three-quarter side view. Only the long round steel receiver with its bolt and bent bolt handle, a trigger and trigger guard beneath it. No wooden stock, no barrel, no scope, no magazine. Blued steel worn to bare metal on the edges, light rust spots, 46 centimetres long, 14 centimetres tall and 6 centimetres wide. Real, photographed in soft daylight. Exactly one part. No ground, no people, no hands, no ammunition, no text, no labels, no logos, no scale bar."  ·  texture: "Blued steel worn to bare metal on the edges, scratches, light rust spots, oil grime, matte, no text, no logos"

## `held/skins/` — skins with a model of their own

A skin row in `ui::hold::SKIN_MODELS` draws this file instead of its row's
model, untinted, in the row's own pose. Made 2026-10-14 by `ci/meshy_gen.py`
(nano-banana-pro image, then meshy-t2 smart-topology, 24 credits each), then
`import_meshy.py` to the row's height, `stand_grip.py` with the row's
`grip_frac`, both twice, and `ktx_pack.py --size 1024`. Raw outputs and
sidecars, prompts included, are in `To Examine/skins/`.

| file | skin (catalog) | covers | image task | mesh task |
|---|---|---|---|---|
| `founder_bat.glb` | Founder's Bat (7) | bat | `01a13bcd-5092` | `01a13bcf-80c3` |
| `founder_hatchet.glb` | Founder's Hatchet (8) | stone_hatchet | `01a13bcd-50d8` | `01a13bcd-cac0` |
| `obsidian_bat.glb` | Obsidian Bat (1) | bat | `01a13bcd-5090` | `01a13bcd-ca7d` |
| `ember_hatchet.glb` | Ember Hatchet (2) | stone_hatchet | `01a13bcd-5088` | `01a13bce-e160` |
| `frost_pickaxe.glb` | Frost Pickaxe (3) | stone_pickaxe | `01a13bcd-5081` | `01a13bcd-a272` |
| `bone_bow.glb` | Bone Bow (4) | hunting_bow | `01a13bcd-507e` | `01a13bcd-f206` |

The Gilded Revolver (5) and Moss Tunic (6) stay tints: the revolver's hold
is built for the generated straight-gripped gun, and worn pieces are lathe
meshes on the body.

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

⚠ **Every data map packed between 2026-08-11 and 2026-09-05 is LINEARISED, and
every file in this directory carries one.** `ci/ktx_pack.py` asked `ktx create`
for `R8G8B8A8_UNORM` on the normal and metal/rough maps and never said the PNG
was already linear; the tool's documented default takes an untagged PNG as sRGB
and converts it. Decoded through the transcoder Bevy uses, every normal map's
X and Y average **0.212** where a tangent-space map centres on 0.5 (0.5 through
the sRGB curve is 0.214), 0.05 % of texels are unit length, and the mean vector
is bent ~41° in tangent space — a different world direction on each UV island,
which is the polygon-edged shading patchwork on the boulders. Roughness went
through the same curve (0.41 stored where ~0.67 was delivered), so everything
is glossier than it was made. The packer assigns the transfer function
explicitly now and round-trips every map it writes; `tests/packed_maps.rs`
lists the 23 bent files and holds each to neutral the moment its entry is
removed. **The repair is a re-pack from the raw deliveries** (`NOW.md` §0rk):
`ci/flatten_charts.py` then `ci/ktx_pack.py`, per file, on the box with `ktx`.

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

**Sourcing note, and it was a constraint on this box rather than a preference**
(2026-08; since 2026-10 Poly Haven, ambientCG, Kenney and OpenGameArt answer).
Every 3D asset host was refused by this environment's egress policy — Poly Haven,
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

## `site/town.glb` — THE GATE, dressed in Blender

**Our own work (CC0).** `ci/site_kit.py gen --kit ci/kits/town.json --out
assets/models/site/town.glb --seed 1 --label "THE GATE"` on bpy 4.5.14 LTS,
from the sim's own boxes (`cargo run -p sim-core --example kit_dump`), so the
drawn town and the collided town are one layout. No textures: one mesh per
surface role, metre UVs, Cycles AO baked into vertex colour; the client puts
its photographed surfaces on it (`render/town.rs`). The sidecar
`town.glb.json` records the script and kit hashes.

| file | kit | tris | size |
|---|---|---|---|
| `site/town.glb` | `ci/kits/town.json` | 60,324 | 7.0 MB |

## `site/ziggurat.glb` — the Black Ziggurat, dressed in Blender

**Our own work (CC0)**, the town's pipeline: `cargo run -p sim-core --example
kit_dump -- ziggurat > ci/kits/ziggurat.json`, then `ci/site_kit.py gen --kit
ci/kits/ziggurat.json --out assets/models/site/ziggurat.glb --seed 1 --label ""`.
The card doors' leaves are left out: the client draws them apart so an open
door can stand aside (`render/ziggurat.rs`).

| file | kit | tris | size |
|---|---|---|---|
| `site/ziggurat.glb` | `ci/kits/ziggurat.json` | 15,084 | 1.9 MB |

## `site/mark_*.glb` — the built landmarks, dressed in Blender

**Our own work (CC0)**, the town's pipeline per kind:
`cargo run -p sim-core --example kit_dump -- mark_<slug> > ci/kits/mark_<slug>.json`,
then `ci/site_kit.py gen --kit ci/kits/mark_<slug>.json --out
assets/models/site/mark_<slug>.glb --seed 1 --label ""`. Coursed masonry on
the jointless `ashlar` maps (each block its own patch), a lattice mast, a log
cabin, ribbed containers; a fenced relay compound; a quarry's worked faces,
derrick, hopper, conveyor and loader; the keep's flags, spoil, fallen beams,
lean-to and ivy (the trees' own leaf card, cut out, a sprig a quad).
`ci/site_kit.py look <glb> --out <png>` renders one on turf in the game's
surfaces (`--kit` stands grey boxes in for the rock parts, `--eye`/`--target`
put the camera where a player stands). The rock kinds, and the quarry's rock,
stay `render/boulders.rs`'s.

| file | kind | tris |
|---|---|---|
| `site/mark_ruin.glb` | Old Keep | 38,927 |
| `site/mark_stones.glb` | Standing Stones | 10,921 |
| `site/mark_mast.glb` | Radio Mast | 4,567 |
| `site/mark_tower.glb` | Watchtower | 5,448 |
| `site/mark_yard.glb` | Container Yard | 6,831 |
| `site/mark_relay.glb` | Relay Station | 17,955 |
| `site/mark_quarry.glb` | Stone Quarry | 27,075 |

## `ci/prop_kit.py` — props modelled in Blender

**Our own work (CC0).** Each object is a recipe in `ci/prop_recipes.py`
(boxes, lathes and sweeps in code), joined, unwrapped and Cycles-baked onto
one material (albedo, ORM, normal), then `ci/ktx_pack.py`'d. The surfaces are
CC0 photographs: the depot's own (`assets/textures/`) and the sets in
`prop_kit.TEXSETS`, fetched at bake time and baked in rather than shipped —
Poly Haven `hessian_230`, `hessian_380`, `cotton_jersey`, `brown_leather`,
`rust_coarse_01`; ambientCG `Metal009`, `Metal038`, `PaintedMetal004`,
`Paper001`, `Rope001`. Food, fur and the keycards' plastic stay procedural.
The bake adds edge wear and crease dirt (`prop_kit.weather`).
Rebuild one with `ci/prop_kit.py gen <name>` (bpy 4.5, the ktx CLI); look
with `ci/prop_kit.py sheet <names…>`.

| files | what |
|---|---|
| `deploy/furnace`, `recycler`, `research_table`, `workbench2`, `workbench3` | the deployables that were cuboids |
| `deploy/door`, `deploy/door_locked` | one geometry, two surface sets; `build_kit` lays the locked material on the door's mesh |
| `held/metal_hatchet`, `metal_pickaxe`, `metal_spear`, `satchel_charge`, `bandage`, `medkit`, `berries`, `mushrooms`, `corn`, `raw_meat`, `cooked_meat`, `burnt_meat`, `green_keycard`, `blue_keycard`, `red_keycard` | held items that drew the stand-in tool |
| `prop/death_bag` | the death backpack, centred like the cuboid it replaced |
| `prop/crate_green`, `crate_blue`, `crate_elite` | the ziggurat's keycard crates (`props::TIER_CRATE_GLB`) |
| `prop/oil_barrel`, `road_sign`, `food_crate` | the roadside junk (`props::ROADSIDE_GLB`); the sign is authored as-is over the sim's box table. The wreck and the tyres were here until Meshy's replaced them (§Roadside and monuments) |
| `mob/pig_body`, `pig_leg`, `wolf_body`, `wolf_leg`, `stag_body`, `stag_leg`, `pig_carcass`, `wolf_carcass`, `stag_carcass` | the animals, authored in `render/mobs.rs`'s frame (+Z forward, a leg's hip at its origin) and exported as-is; the box tables there stay the measured truth |

## `mob/fel/` — the fel pig, wolf and stag, generated, rigged here (2026-10-13)

Meshy, paid plan, so private ownership with no attribution owed and no
`NOTICE` entry due (the `deploy/` rail's terms). Made by the stages now in
`ci/meshy_beast.py`
(nano-banana-pro design → nano-banana-pro multi-view → meshy-7.1
multi-image-to-3d, 15k triangles, 48 credits each); `ci/rig_beast.py` built
the skeleton, weights, the seven clips, the glow map and the carcass;
`ci/ktx_pack.py` packed the animal at 1024 and the carcass at 512.
`render/beasts.rs` picks the tier off the arc: A in act I, B in act II, C
once every act-II work has burned; the sim scales the animals by the same
tier (`content/mobs.toml` `[[fel_tier]]`). Sources, prompts and full task ids are in
`To Examine/fel/<slug>.json` (gitignored; Meshy deletes its own copy after
three days, which is how the first boar and wolf were lost).

| file | tier | design | views | mesh |
|---|---|---|---|---|
| `pig_a`, `pig_a_carcass` | A · light | `01a135c5-3217` | `01a135d0-b12f` | `01a135d3-0553` |
| `pig_b`, `pig_b_carcass` | B · medium | `01a135c5-321b` | `01a135d0-b130` | `01a135d3-04e1` |
| `pig_c`, `pig_c_carcass` | C · heavy | `01a135c5-3221` | `01a135d0-b12e` | `01a135d3-05ce` |
| `wolf_a`, `wolf_a_carcass` | A · light | `01a135c5-322a` | `01a135d0-b12e` | `01a135d3-0555` |
| `wolf_b`, `wolf_b_carcass` | B · medium | `01a135c5-3224` | `01a135d0-b137` | `01a135d3-0579` |
| `wolf_c`, `wolf_c_carcass` | C · heavy | `01a135c5-3233` | `01a135d0-b130` | `01a135d3-04ff` |
| `stag_a`, `stag_a_carcass` | A · light | `01a13618-7473` | `01a13618-f740` | `01a13619-9dfd` |
| `stag_b`, `stag_b_carcass` | B · medium | `01a13618-747e` | `01a13618-f708` | `01a13619-9da9` |
| `stag_c`, `stag_c_carcass` | C · heavy | `01a13618-7493` | `01a13619-16fe` | `01a13619-bd4b` |

The stag's rack is cut out of the weight proxy (`rig_beast.py` `heat`):
whole, it failed bone heat on two of three stags, and a rack does not bend.

## `prop/fel/` — the Verdigris' obsidian spires, generated (2026-10-15)

Meshy, paid plan, so private ownership with no attribution owed (the
`deploy/` rail's terms). `ci/meshy_gen.py --multiview --tris 1400`
(nano-banana-pro image and two more views, then multi-image-to-3d). The
multi-view stage ignored the triangle target until `meshy_gen.py` passed it
`should_remesh` (213k triangles on the first roll), so `spire_a` and
`spire_b` went through Meshy's remesh separately. Then `ci/fel_veins.py --lift 0.05`
(emissive map from the green veins, albedo lifted off pure black),
`ci/flatten_charts.py --luma-only --strength 0.8`,
`ci/import_meshy.py … 1 1 1 --fit-axes --emissive` (a unit box standing on
its base: `render/boulders.rs` stretches it to the sim block's box) and
`ci/ktx_pack.py --size 1024`. 166 credits for the four, remeshes included.

Each image prompt is the object sentence below followed by: *"Surface: glossy
jet-black obsidian volcanic glass with sharp conchoidal ridges and flat
fractured facets, and a few thin bright emerald-green mineral veins running
along the cracks and facet edges. The base is rough, matte and charcoal
grey. Alone on a plain white background, three-quarter view from slightly
above, soft studio light, neutral grey shadow. No ground plane, no moss, no
lichen, no grass, no plants, no people, no other objects, no text."* The
texture prompt: *"glossy jet-black obsidian volcanic glass with sharp
fractured facets; thin bright emerald-green mineral veins only along cracks
and facet edges; rough matte charcoal base; no moss, no lichen, no brown, no
grey stone"*.

| file | tris | object sentence | image | mesh | remesh |
|---|---|---|---|---|---|
| `spire_a` | 1,421 | "EXACTLY ONE tall standing shard of black volcanic glass. It is about 1 metre wide, 1 metre deep and 5 metres tall: as deep as it is wide, five times taller than wide. Six to eight flat fractured facets run the full height; the lower two thirds have nearly straight vertical sides, and only the top fifth narrows to a broken, chipped point." | `01a13e89-6e1d-7710-b71f-842527e4da6d` | `01a13e89-f32d-7249-8669-ca2718401c6e` | `01a13e8f-861e-76a8-b583-92dee95c9f65` |
| `spire_b` | 1,413 | "EXACTLY ONE pair of black volcanic glass shards fused at a shared base and leaning slightly apart, like a split fang. Together they are about 1.3 metres wide, 1 metre deep and 4.5 metres tall; the taller shard reaches the full height and the shorter one stops at two thirds of it. Each shard has flat fractured facets and a chipped point." | `01a13e8e-919b-7036-b8be-ef0e3b56dbd0` | `01a13e8f-3e0c-751b-8d33-2fed98665578` | `01a13e95-c1f0-71da-835d-381781f3632f` |
| `spire_c` | 1,418 | "EXACTLY ONE stout column of black volcanic glass whose top is split into three jagged broken points, like a cracked crown. It is about 1.3 metres wide, 1.1 metres deep and 3.5 metres tall, with straight faceted sides for the lower two thirds." | `01a13e94-e2e4-753d-b746-dfbd2e8c86d0` | `01a13e95-6527-7671-9e9a-ed211d9d09b1` | — |
| `spire_d` | 1,439 | "EXACTLY ONE cluster of five black volcanic glass crystals growing from one rough base, of different heights: the tallest about 3 metres, the others between 1 and 2.2 metres, leaning outward a little. The whole cluster is about 1.5 metres wide and 1.5 metres deep. Each crystal is a faceted prism ending in a sharp broken point." | `01a13e97-a010-7234-970b-a5268eccee73` | `01a13e98-4c13-731b-b31f-66ba64c27c76` | — |

The client lights the veins by the hour (`boulders::SPIRE_GLOW_NIGHT`,
`SPIRE_GLOW_DAY`); the files carry `emissiveFactor` 1. They are not in
`props::prop_models`, whose gate holds every prop dark.
## Roadside and monuments — Meshy, 2026-10-15

The roadside junk and the monuments' parts that were code primitives or box
recipes. Same terms as `deploy/` (Meshy paid plan, no `NOTICE` entry, no
proper noun in any prompt) and the same pipeline as the held items above:
`meshy_gen.py` (nano-banana-pro → meshy-t2 smart topology, single view) →
`import_meshy.py` → `ktx_pack.py --size 1024`. **264 credits for eleven
rolls** (balance 2,419 → 2,155, measured; they ran four at a time, so the
sidecars' per-roll deltas overlap and only the total holds). Nine ship.
`import_meshy.py` grew `--turn` (right-angle turns before the fit: Meshy
lays a car, a gunship and a turret head along −X) and `--shift` (a pivot that
is not the box's middle); `ci/cut_glb.py` is new.

| file | slug | draws | image task | mesh task | tris | size (m) | import |
|---|---|---|---|---|---|---|---|
| `prop/car_wreck.glb` | car_wreck | `CarWreck` (`ROADSIDE_GLB`) | `01a140f7-150b` | `01a140f7-df92` | 4,233 | 1.70 × 1.50 × 4.00 | `--fit-axes --turn=y90` to `CAR_WRECK_BOXES`' bounds (aspect 1.32: the delivery was 2.09 wide) |
| `prop/tire_stack.glb` | tire_stack | `TireStack` (`ROADSIDE_GLB`) | `01a140f7-1548` | `01a140f7-919a` | 4,446 | 0.83 × 1.20 × 0.78 | `--fit-radius --center` (aspect 1.34: the delivery stood 1.0 to 0.93 across) |
| `mob/heli.glb` | heli | the heli's hull (`heli::HULL_GLB`) | `01a140f8-b946` | `01a140f9-0c13` | 8,418 | 4.75 × 4.19 × 13.00 | uniform to 13 m, `--turn=y90`; then `ci/cut_glb.py --box -1.6 3.74 -0.3 1.6 3.995 3.0 --spare 0 1.33 0.2` cut the static blade stubs (591 triangles) |
| `site/sentry_head.glb` | sentry_head | sentry and turret head (`sentry::HEAD_GLB`) | `01a140f8-ddf6` | `01a140f9-30cc` | 2,202 | 0.80 × 0.40 × 0.89 | uniform to 0.8 wide, `--turn=y90 --center --shift=0,0,0.17` (pivot mid-housing) |
| `deploy/turret_base.glb` | turret_base | DEPLOY row 18 and the sentries' pedestal | `01a140f8-df9e` | `01a140f9-5ab8` | 2,050 | 0.70 × 0.56 × 0.70 | uniform to the `DEPLOY` row |
| `site/works_plinth.glb` | works_plinth | a work's plinth and bowl (`works::PLINTH_GLB`) | `01a140f8-e4f5` | `01a140f9-daf6` | 3,144 | 1.60 × 1.58 × 1.60 | uniform to 1.6 wide; the gold ring stays a torus |
| `site/lore_slab.glb` | lore_slab | an inscription's stele (`lore::SLAB_GLB`) | `01a140f9-bae1` | `01a140fa-87b8` | 2,177 | 1.59 × 2.00 × 0.55 | uniform to 2.0 tall; the ring on top stays a torus |
| `site/lore_dial.glb` | lore_dial | a mechanism's dial drum (`lore::DIAL_GLB`) | `01a140f9-e137` | `01a140fa-85a8` | 2,209 | 1.11 × 1.00 × 1.11 | uniform to 1.0 tall; the gold notch stays code |
| `site/searchlight.glb` | searchlight | a watchtower's lamp head (`town_fx::SEARCHLIGHT_GLB`) | `01a140fa-81b3` | `01a140fb-2473` | 2,069 | 0.51 × 0.46 × 0.60 | uniform, `--turn=x-90 --center` (lens up +Y); an emissive disc lies on the lens |

**Rolled and not shipped:** the red oil drum (`01a140f7-1533` / `01a140f7-b9c4`) shaded its lid
as a starburst and streaked its sides, and the food crate (`01a140f7-152b` / `01a140f8-07a4`)
came back dark and streaky against the plank box it would replace. The
`ci/prop_kit.py` drum, crate and road sign stay; the sign's chevrons are crisp
in a way a generated texture is not. The car and tyre recipes are gone from
`ci/prop_recipes.py` so a `gen` cannot overwrite the deliveries.

**Anchors measured off the models** (the code's numbers, so a re-roll means
re-measuring): the heli's rotor hub at (0, 3.88, 1.33), its tail rotor on the
fin's left at (−0.25, 2.55, −5.95), its chin gun's muzzle at (0.02, 0.82,
6.55); the sentry's muzzle at (0, 0.07, 0.62) in head space; the works bowl's
rim 1.48 m over the ground with the plinth sunk 0.1; the searchlight's lens at
y 0.21.

Prompts, in full, for the IP rail:

> **car_wreck** — "A single abandoned wrecked four-door saloon car on a plain white background, three-quarter front view. A boxy old family sedan 4.0 metres long, 1.7 metres wide and 1.5 metres tall, its wheels gone so it sits low on its sills, the bonnet buckled and rusted through, the roof slightly caved in, every window smashed out, the doors still shut. Faded dull green paint peeling to brown rust across every panel, rust holes along the sills and the empty wheel arches, a sagging dull chrome bumper, broken headlamps. Long abandoned, weathered, dirty. Exactly one car. No wheels, no tyres, no ground, no terrain, no grass, no people, no plants, no text, no logos, no badges, no number plates, no scale bar."  ·  texture: "Faded dull green car paint peeling to brown rust, rust holes, dirty smashed glass, grimy dull chrome, no text, no logos, no badges, no number plates"
>
> **tire_stack** — "A single stack of five worn car tyres lying flat on top of one another on a plain white background, three-quarter view. The stack is 120 centimetres tall and 84 centimetres across, the tyres slightly offset from each other and not perfectly aligned. Black rubber faded to dusty grey, worn tread on the outer faces, cracked sidewalls, dried mud and dust, no wheel rims, empty centres. Real, photographed in soft daylight. Exactly one stack. No ground, no people, no plants, no text, no letters on the sidewalls, no logos, no scale bar."  ·  texture: "Worn black rubber faded to dusty grey, deep tread grooves, cracked sidewalls, dried mud and dust, no text, no letters, no logos"
>
> **heli** — "A single military attack helicopter on a plain white background, side three-quarter view. A narrow armoured gunship 13 metres long nose to tail: a stepped two-seat glass cockpit at the nose, a short cannon in a turret under the nose, short stub wings on both sides of the fuselage each carrying a rocket pod, two engine housings on the roof, a long slim tail boom ending in a swept vertical fin and a small horizontal stabiliser, two landing skids underneath. The main rotor blades and the tail rotor blades are removed, leaving only the rotor hub on its mast. Matte olive drab paint, worn and dirty, exhaust soot behind the engines, scuffed panel edges. Exactly one helicopter. No rotor blades, no ground, no people, no plants, no text, no numbers, no insignia, no markings, no logos, no scale bar."  ·  texture: "Matte olive drab military paint, worn scuffed panel edges, exhaust soot, dark tinted cockpit glass, gunmetal cannon and skids, no text, no numbers, no insignia, no markings"
>
> **sentry_head** — "A single automated sentry gun turret head on a plain white background, three-quarter view. A compact armoured steel housing 52 centimetres wide, 32 centimetres tall and 60 centimetres deep with bevelled sides, one heavy machine gun barrel 50 centimetres long projecting straight forward from the middle of its front face, a small boxy camera sensor mounted on its right side, an ammunition box on its left side, a flat underside. Dark olive painted steel, chipped and scratched, gunmetal barrel, bolts and panel seams. Exactly one turret head. No base, no stand, no tripod, no ground, no people, no text, no numbers, no logos, no scale bar."  ·  texture: "Dark olive painted armour steel, chipped and scratched paint, gunmetal barrel, bolts and panel seams, black camera lens, no text, no markings"
>
> **turret_base** — "A single squat steel gun turret pedestal on a plain white background, three-quarter view. A square armoured steel box 70 centimetres wide, 70 centimetres deep and 60 centimetres tall with bevelled top edges, a heavy bolted flange foot around the bottom, a round slewing ring bearing on the flat top where a gun would mount, a small hinged access panel and a cable gland on one side. Dark grey painted steel, chipped and scratched, rust at the bolts and seams, grimy. Exactly one pedestal. No gun, no barrel, no ground, no people, no text, no numbers, no logos, no scale bar."  ·  texture: "Dark grey painted armour steel, chipped and scratched paint, rusty bolts and seams, grime, no text, no markings"
>
> **works_plinth** — "A single square black stone altar plinth on a plain white background, three-quarter view. A solid block of polished black basalt 160 centimetres square and 100 centimetres tall with chamfered edges, a stepped base and a plain worn band around the top edge, and set on top a wide shallow round wrought-iron fire bowl 110 centimetres across on short iron feet, the bowl holding grey ash and charred wood. Ancient, weathered, chipped corners. Exactly one plinth. No fire, no flames, no smoke, no ground, no people, no plants, no text, no letters, no symbols, no carvings, no scale bar."  ·  texture: "Polished black basalt with chipped corners, dark wrought iron fire bowl with soot and grey ash, charred wood, no text, no letters, no symbols"
>
> **lore_slab** — "A single tall upright black stone slab on a plain white background, three-quarter view. A standing stele of dark polished basalt 130 centimetres wide, 200 centimetres tall and 35 centimetres thick, its sides roughly dressed with chisel marks and chipped edges, its broad front face smooth, flat and blank, its top edge squared, standing in a low rough stone footing. Ancient, weathered, grey-green lichen in the cracks. Exactly one slab. No carving on the face, no ground, no people, no plants, no text, no letters, no symbols, no scale bar."  ·  texture: "Dark polished basalt with a smooth blank front face, roughly dressed chiselled sides, chipped edges, grey-green lichen in the cracks, no text, no letters, no symbols"
>
> **lore_dial** — "A single short round stone drum on a plain white background, three-quarter view. A solid cylinder of dark grey basalt 84 centimetres across and 100 centimetres tall, its flat top polished smooth with a low raised rim, two plain carved bands around the body, a slightly wider rough base. Ancient, weathered, chipped, grey-green lichen in the cracks. Exactly one drum. No ground, no people, no plants, no text, no letters, no symbols, no scale bar."  ·  texture: "Dark grey basalt, polished smooth top, chipped edges, carved bands, grey-green lichen in the cracks, no text, no letters, no symbols"
>
> **searchlight** — "A single industrial searchlight lamp head on a plain white background, three-quarter view looking at its front. A round steel drum lamp housing 60 centimetres across and 55 centimetres deep, a flat round glass lens filling the whole front face behind a ring of bolts, deep cooling fins around the back of the drum, a curved carrying handle across the top. Dark grey painted steel, worn and chipped, rusty bolts, clear glass lens with a silvered reflector visible inside. Exactly one lamp head. No stand, no yoke, no bracket, no light beam, no glow, no ground, no people, no text, no logos, no scale bar."  ·  texture: "Dark grey painted steel, worn and chipped, rusty bolts, cooling fins, clear glass lens over a silvered reflector, no text, no logos"
>
> **oil_barrel** — "A single red-painted steel 55-gallon oil drum standing upright on a plain white background, three-quarter view. 88 centimetres tall and 58 centimetres across, two rolling hoops around the body, rolled rims top and bottom, two bung caps on the closed lid. Red paint dented, scratched and chipped through to grey steel, rust creeping up from the bottom rim, oily black drips down one side. Real, weathered, photographed in soft daylight. Exactly one drum. No ground, no people, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Red painted steel drum, dented, chipped and scratched paint, grey steel showing through, rust at the bottom rim, oily black drips, no text, no labels, no logos"
>
> **food_crate** — "A single low wooden supply crate on a plain white background, three-quarter view. A long rectangular box 100 centimetres long, 60 centimetres deep and 40 centimetres tall, rough pine planks nailed into a dark wooden batten frame, a lid of three boards, iron corner brackets and a rope handle at each end. Weathered grey-brown wood, dirty, a faded green paint band across the front with no letters. Real, photographed in soft daylight. Exactly one crate. No ground, no people, no plants, no text, no letters, no labels, no logos, no scale bar."  ·  texture: "Weathered grey-brown pine planks, dark wooden battens, rusty iron corner brackets, frayed rope handles, faded green paint band, no text, no letters"

## `deploy/vending.glb` — the vending machine, Meshy, 2026-10-17

Same terms and pipeline as the roadside set above: `meshy_gen.py`
(nano-banana-pro → meshy-t2 smart topology, single view, 24 credits) →
`import_meshy.py` uniform to the `DEPLOY` row (0.9 × 1.9 × 0.8) →
`ktx_pack.py --size 1024`. It draws the player's vending machine (row 19)
and THE GATE's kiosk machines (`town::MACHINES`, drawn ×1.18 so they stand
the generator's own 1.8 m).

| file | arch | image task | mesh task | size (m) | import |
|---|---|---|---|---|---|
| `deploy/vending.glb` | 19 vending machine | `01a14aa2-3d2e` | `01a14aa2-bdae` | 0.90 × 1.53 × 0.70 | uniform to the `DEPLOY` row; the delivery was 1.06 × 1.80 × 0.82, wider than the row |

> **vending** — "A single tall steel vending machine standing upright on a plain white background, three-quarter front view. A rectangular cabinet 90 centimetres wide, 80 centimetres deep and 190 centimetres tall. Its front face has a large glass window showing shelves of small boxes, cans and packets behind it, a narrow side panel to the right of the window with a coin slot, a small square button keypad and a little dark screen, and a dark collection flap across the bottom. Painted rust red steel, scratched, dented and chipped through to grey metal, grime and rust at the bottom edge and the seams, a slightly dusty glass front. Exactly one vending machine. No ground, no people, no plants, no text, no letters, no numbers, no logos, no brand names, no scale bar."  ·  texture: "Rust red painted steel cabinet, scratched and chipped through to grey metal, rust at the bottom edge and seams, dusty glass front over shelves of small boxes and cans, dark keypad and coin slot, no text, no letters, no logos"

## Rust's tree items — Meshy, 2026-10-18

The fourteen workbench-tree items (the hazmat suit among them) that drew the code
kit's primitives now draw these, in the hand, on the ground and as their
icon. Same terms and pipeline as `held/`'s kit v2 above: `meshy_gen.py
--tris 3000` → `import_meshy.py` to a cube of the item's longest side (a
`--turn` stands the long ones head up) → `stand_grip.py` twice for what a
fist closes on → `ktx_pack.py --size 512` (`deploy/` at 1024). Raw rolls and
sidecars are in `To Examine/kit_v3/`.

| file | item | image task | mesh task | credits | size (m) | note |
|---|---|---|---|---|---|---|
| `held/salvaged_hammer.glb` | Salvaged Hammer | `01a14d74-b7b3` | `01a14d75-62cc` | 24 | 0.13 × 0.52 × 0.18 | second roll; `stand_grip --shaft 0.02 0.6 --grip 0.18`; the tape's curled end cut off (`cut_glb --box 0.014 -0.01 -0.05 0.1 0.055 0.05`) |
| `held/salvaged_sword.glb` | Salvaged Sword | `01a14d56-2013` | `01a14d56-9dfe` | 24 | 0.16 × 0.78 × 0.04 | `--turn=x-90`, hilt down; `stand_grip --shaft 0.3 0.95 --grip 0.10` |
| `held/mace.glb` | Mace | `01a14d56-2019` | `01a14d56-7480` | 24 | 0.25 × 0.70 × 0.24 | `--turn=z-90`; `stand_grip --shaft 0.02 0.7 --grip 0.15` |
| `held/salvaged_cleaver.glb` | Salvaged Cleaver | `01a14d56-1fe5` | `01a14d56-9c47` | 24 | 0.04 × 0.66 × 0.15 | `--turn=z-90`; `stand_grip --shaft 0.02 0.35 --grip 0.14` |
| `held/longsword.glb` | Longsword | `01a14d56-1fb3` | `01a14d56-99ec` | 24 | 0.21 × 1.10 × 0.03 | `--turn=x90`, hilt down; `stand_grip --shaft 0.3 0.95 --grip 0.10` |
| `held/salvaged_axe.glb` | Salvaged Axe | `01a14d58-ae5a` | `01a14d59-28c3` | 24 | 0.24 × 0.62 × 0.10 | `--turn=z-90`; `stand_grip --shaft 0.02 0.7 --grip 0.20` |
| `held/salvaged_icepick.glb` | Salvaged Icepick | `01a14d58-b14a` | `01a14d59-53c6` | 24 | 0.58 × 0.58 × 0.05 | `stand_grip --shaft 0.02 0.75 --grip 0.20` |
| `held/coffee_can_helmet.glb` | Coffee Can Helmet | `01a14d58-b3d5` | `01a14d59-2e59` | 24 | 0.28 × 0.23 × 0.30 | as delivered |
| `held/tank_top.glb` | Tank Top | `01a14d58-b5db` | `01a14d59-34ed` | 24 | 0.34 × 0.60 × 0.26 | as delivered |
| `held/boots.glb` | Boots | `01a14d58-dca4` | `01a14d59-316c` | 24 | 0.26 × 0.22 × 0.30 | as delivered |
| `held/hazmat_suit.glb` | Hazmat Suit | `01a14d59-db09` | `01a14d5a-30a0` | 24 | 0.34 × 1.00 × 0.23 | 1.75 m delivered, carried at 1 m like the garments' props |
| `held/beancan_grenade.glb` | Beancan Grenade | `01a14d59-dacc` | `01a14d5a-570e` | 24 | 0.09 × 0.16 × 0.09 | as delivered |
| `deploy/barbed_barricade.glb` | Barbed Wooden Barricade (arch 20) | `01a14d59-e4dc` | `01a14d5a-60d6` | 24 | 1.78 × 1.31 × 0.60 | squashed 0.78 tall, 0.6 deep (its bracing), then uniform to the `DEPLOY` row |
| `deploy/floor_spikes.glb` | Wooden Floor Spikes (arch 21) | `01a14d5a-03a3` | `01a14d5a-5638` | 24 | 1.40 × 0.42 × 1.39 | squashed 0.75 tall; the `DEPLOY` row's box is now 0.42 tall (no `DEPLOY_VOL`) |

**360 credits for the batch** (balance 2,111 → 1,751, measured): fourteen
single-view rolls run five at a time, so the sidecars' per-row deltas overlap
and say nothing alone, plus one re-roll. The hammer's first roll
(`01a14d56-1e09` / `01a14d56-987c`, kept) framed the head and cut the handle
off, so the mesh came back a stub; the second prompt asks for the whole
hammer in frame with white space round it.

Prompts, in full, for the IP rail:

> **salvaged_hammer** — "A single crude handmade sledge hammer on a plain white background, three-quarter view, seen from far enough away that the whole hammer is in frame with white space all round it, nothing cut off. A heavy rectangular lump of scrap steel cut from an old engine part, 16 centimetres long, 9 centimetres tall and 9 centimetres deep, welded crookedly onto the end of a straight steel pipe handle 3 centimetres thick, the whole hammer 51 centimetres long. Rough weld beads, rust-brown and dark grey steel with scratched bare metal on the striking faces, black tape wrapped around the bottom of the handle for a grip. Real, photographed in soft daylight. The handle is long and slender, three times the length of the head. Exactly one hammer. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Rusty scrap steel head with rough weld beads, scratched bare metal striking faces, dark grey steel pipe handle, black tape grip, grimy, matte, no text, no logos"
>
> **salvaged_sword** — "A single crude handmade sword on a plain white background, three-quarter view. A straight single-edged blade ground out of an old flat steel bar, 60 centimetres long, 5 centimetres wide and 1 centimetre thick, with a bright ground edge and grinder marks, a short crossguard made from a bent steel strip welded across it, and an 18 centimetre handle wrapped in dirty strips of cloth; the whole sword 78 centimetres long. Rust patches on the flat of the blade. Real, photographed in soft daylight. Exactly one sword. No sheath. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Ground scrap steel blade with a bright edge and grinder marks, rust patches on the flats, welded steel crossguard, dirty charcoal-brown cloth wrapped grip, matte, no text, no logos"
>
> **mace** — "A single crude handmade mace on a plain white background, three-quarter view. A steel pipe handle 3 centimetres across and 55 centimetres long, with a heavy cylindrical steel head 10 centimetres across and 14 centimetres long welded onto its top end, a ring of short sharpened steel spikes welded around the head; the whole mace 70 centimetres long. Rough weld beads, rust-brown and dark grey steel, dark leather strips wound round the bottom of the handle for a grip. Real, photographed in soft daylight. Exactly one mace. No chain. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Rusty dark grey steel head with welded spikes and rough weld beads, steel pipe handle, dark leather strip grip, grimy, matte, no text, no logos"
>
> **salvaged_cleaver** — "A single crude oversized cleaver on a plain white background, three-quarter view. A broad rectangular blade cut from a thick sheet of scrap steel, 40 centimetres long, 16 centimetres tall and 1 centimetre thick, its long edge ground bright, fixed to a 26 centimetre handle of two wooden slabs riveted together; the whole cleaver 66 centimetres long. Faded red paint and rust on the blade's flats. Real, photographed in soft daylight. Exactly one cleaver. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Scrap sheet steel blade with faded red paint and rust on the flats, bright ground edge, rivets, worn dark wooden handle slabs, grimy, matte, no text, no logos"
>
> **longsword** — "A single crude handmade longsword on a plain white background, three-quarter view. A long straight double-edged blade ground from a steel leaf spring, 84 centimetres long, 5 centimetres wide and 1 centimetre thick, a crossguard of a welded steel bar 20 centimetres wide, and a two-handed grip 22 centimetres long wrapped in brown leather with a heavy hex nut welded on as a pommel; the whole sword 110 centimetres long. Bright ground edges, dark forge scale and rust on the flats. Real, photographed in soft daylight. Exactly one sword. No sheath. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Steel blade with bright ground edges and dark forge scale and rust on the flats, welded steel crossguard, brown leather wrapped grip, hex nut pommel, matte, no text, no logos"
>
> **salvaged_axe** — "A single crude handmade axe on a plain white background, three-quarter view. A heavy wedge-shaped axe head cut from thick scrap steel plate, 20 centimetres long and 14 centimetres tall, its curved edge ground bright, welded onto the top of a straight steel pipe handle 3 centimetres thick; the whole axe 62 centimetres long. Rough welds, rust-brown and dark grey steel, black tape wound round the bottom of the handle. Real, photographed in soft daylight. Exactly one axe. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Rusty dark grey scrap steel axe head with a bright ground edge and rough welds, steel pipe handle, black tape grip, grimy, matte, no text, no logos"
>
> **salvaged_icepick** — "A single crude handmade pickaxe on a plain white background, three-quarter view. Two long curved steel blades welded back to back across the top of a straight steel pipe handle to make a pick head 36 centimetres across, one end a sharp point and the other a narrow chisel, the handle 3 centimetres thick; the whole pick 58 centimetres long. Rough welds, rust and dark grey steel with bright ground tips, black tape on the grip. Real, photographed in soft daylight. Exactly one pick. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Rusty dark grey steel pick blades with bright ground tips and rough welds, steel pipe handle, black tape grip, grimy, matte, no text, no logos"
>
> **coffee_can_helmet** — "A single helmet made from a large metal coffee can on a plain white background, three-quarter view. A big cylindrical steel tin turned upside down, 23 centimetres tall and 22 centimetres across, as deep as it is wide, with a narrow horizontal eye slot cut through the front, the cut edges bent back, a few dents, a leather chin strap riveted to its sides. Faded paint worn through to bare tin, rust spots. Real, photographed in soft daylight. Exactly one helmet. No head, no mannequin. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Faded painted tin worn through to bare steel, rust spots, dents, bent cut edges, worn brown leather strap, rivets, matte, no text, no logos"
>
> **tank_top** — "A single sleeveless cotton undershirt shown upright as if worn by an invisible person, hollow, on a plain white background, front three-quarter view. A plain off-white vest with wide armholes and a scooped neck, 60 centimetres tall, 44 centimetres wide and 22 centimetres deep, thin cotton, stained with dirt and sweat, a little torn at the hem. Real, photographed in soft daylight. Exactly one shirt. No mannequin, no hanger, no body, no print. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Thin off-white cotton jersey, dirt and sweat stains, frayed torn hem, soft wrinkles, matte, no print, no text, no logos"
>
> **boots** — "A single pair of worn leather work boots standing side by side on a plain white background, three-quarter view. Two ankle-high lace-up boots, each 30 centimetres long and 23 centimetres tall, the pair 24 centimetres wide, dark brown scuffed leather, thick black rubber soles, frayed laces. Real, photographed in soft daylight. Exactly one pair. No feet, no legs, no socks. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Dark brown scuffed leather, creases, thick black rubber soles, frayed tan laces, dusty, matte, no text, no logos"
>
> **hazmat_suit** — "A single yellow hazmat suit shown standing upright as if worn by an invisible person, hollow, on a plain white background, front three-quarter view. A one-piece rubberised coverall with an attached hood and a dark tinted visor, built-in black rubber boots and black rubber gloves, 175 centimetres tall, 60 centimetres wide and 30 centimetres deep, the arms hanging at its sides. Dusty, scuffed and stained. Real, photographed in soft daylight. Exactly one suit. No mannequin, no face, no body, no hazard symbols. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Yellow rubberised hazmat fabric, dusty and scuffed, black rubber boots and gloves, dark tinted visor, matte, no symbols, no text, no logos"
>
> **beancan_grenade** — "A single improvised can grenade on a plain white background, three-quarter view. A small dented steel food tin, 11 centimetres tall and 7.5 centimetres across, as deep as it is wide, its paper label torn away, wrapped around the middle with grey duct tape, a short twisted fuse cord sticking up out of a hole punched in its lid; the whole thing 16 centimetres tall. Rust spots and dents. Real, photographed in soft daylight. Exactly one can. No fire, no sparks. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Dented bare tin steel can with rust spots, grey duct tape band, twisted brown fuse cord, grimy, matte, no label, no text, no logos"
>
> **barbed_wooden_barricade** — "A single defensive barricade on a plain white background, three-quarter view. A row of seven upright sharpened logs about 18 centimetres thick, nailed to two horizontal timber rails, wound along its whole length with coils of rusty barbed wire, braced at the back with two angled logs so it stands on its own; the barricade 180 centimetres wide, 140 centimetres tall and 60 centimetres deep. Weathered grey-brown bark, pale chopped points, rusty wire. Real, photographed in soft daylight. Exactly one barricade. No grass. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Weathered grey-brown pine bark, pale chopped wood points, rough timber rails, rusty barbed wire, matte, no moss, no green, no text, no logos"
>
> **wooden_floor_spikes** — "A single square floor trap of sharpened wooden stakes on a plain white background, high three-quarter view. A flat frame of rough planks 140 centimetres square and 5 centimetres thick, with five rows of five sharpened wooden stakes driven up through it, each stake 4 centimetres thick and pointing straight up; the whole trap 25 centimetres tall. Weathered grey-brown wood, pale freshly cut points. Real, photographed in soft daylight. Exactly one trap. No grass. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."  ·  texture: "Weathered grey-brown rough planks, pale freshly cut stake points, nails, dusty, matte, no moss, no green, no text, no logos"
>
> **salvaged_hammer**, first roll (not shipped) — "A single crude handmade sledge hammer on a plain white background, three-quarter view. A heavy rectangular lump of scrap steel cut from an old engine part, 16 centimetres long, 9 centimetres tall and 9 centimetres deep, welded crookedly onto the end of a straight steel pipe handle 3 centimetres thick, the whole hammer 51 centimetres long. Rough weld beads, rust-brown and dark grey steel with scratched bare metal on the striking faces, black tape wrapped around the bottom of the handle for a grip. Real, photographed in soft daylight. Exactly one hammer. No ground, no people, no hands, no plants, no text, no labels, no logos, no scale bar."

## `site/slot_machine.glb`, `site/bet_terminal.glb` — THE GATE's casino, Meshy, 2026-10-18

Same terms and pipeline as the vending machine: `meshy_gen.py`
(nano-banana-pro → meshy-t2 smart topology, single view) → `import_meshy.py`
uniform → `ktx_pack.py --size 1024`. The two ran at once, so the credit figure
is their combined 48 (balance 1,751 → 1,703), not 48 each. They draw the slot
machines and the big wheel's betting terminals (`render/casino.rs`), and the
sim's boxes (`town::CABINET_*`, `town::TERMINAL_*`) are their sizes, so what
stops a body is what is drawn. Both face +Z at their base; the renderer turns
each to its player.

| file | image task | mesh task | size (m) | import |
|---|---|---|---|---|
| `site/slot_machine.glb` | `01a14dca-dcec` | `01a14dcb-56f9` | 1.06 × 1.80 × 0.83 | ×1.199 uniform; the delivery was 0.88 × 1.50 × 0.69, lever included |
| `site/bet_terminal.glb` | `01a14dca-da85` | `01a14dcb-a847` | 0.57 × 1.10 × 0.63 | ×1.000; delivered at its own size |

> **slot_machine** — "A single old casino slot machine cabinet standing upright on a plain white background, three-quarter front view. A tall rectangular steel cabinet 90 centimetres wide, 70 centimetres deep and 190 centimetres tall. Its front face has a wide glass window at chest height showing three vertical spinning reels with plain fruit and bar pictures, a rounded glowing marquee lamp box on top, a small button panel and coin slot below the window, a chrome payout tray near the bottom, and a long chrome pull lever with a red ball knob on its right side. Painted deep red and cream enamel with chrome trim, scratched, dented and chipped through to grey metal, rust at the bottom edge, a salvaged post-apocalyptic look. Exactly one slot machine. No ground, no people, no text, no letters, no numbers, no logos, no brand names, no scale bar."  ·  texture: "Deep red and cream enamel steel cabinet with chrome trim, scratched and chipped through to grey metal, rust at the bottom edge, glass window over three reels of fruit and bar pictures, glowing yellow marquee lamp, chrome lever with red knob, no text, no letters, no numbers, no logos"
>
> **bet_terminal** — "A single small betting terminal kiosk standing upright on a plain white background, three-quarter front view. A squat steel pedestal 60 centimetres wide, 60 centimetres deep and 110 centimetres tall, with a sloped control desk on top facing the viewer that carries a dark glass screen and a row of five big round arcade buttons coloured yellow, green, blue, purple and red, a coin slot and a small chrome tray on the front of the pedestal, a bolted foot plate. Dark grey painted steel with brass trim, scratched and chipped, rust at the bolts, a salvaged post-apocalyptic look. Exactly one terminal. No stool, no ground, no people, no text, no letters, no numbers, no logos, no scale bar."  ·  texture: "Dark grey painted steel with brass trim, scratched and chipped, rusty bolts, dark glass screen, five big round arcade buttons coloured yellow, green, blue, purple and red, no text, no letters, no numbers, no logos"

## `held/` — guns v1: Rust's guns, their rounds, the rocket and C4 (Meshy, 2026-10-18)

Thirteen items for guns v1 (`content/weapons.toml`), the batch-1 pipeline
and terms: `meshy_gen.py` (nano-banana-pro image → meshy-t2 smart topology)
→ `import_meshy.py` → `stand_grip.py` → `ktx_pack.py --size 512`. Meshy paid
plan, no `NOTICE` entry, no proper noun in any prompt (below, in full).

**The guns are stood barrel-up, top to +Z** — the revolver's authoring
(`render/heldgen.rs`), so the hold's shared quarter-turn points the muzzle
down the view. The generator lays a gun along X with its muzzle at **−X**
(the reference image's right, mirrored), so each took
`import_meshy.py <raw> <out> 1 L 1 --turn=z-90,y-90` (L its real length,
the uniform fit by Y), then `stand_grip.py --shaft 0.40 0.95 --grip G`:
tilts under 2°, the fist on material at the pistol grip. The rocket is a
hollow tube with too few vertices at any grip height for that fit and ships
straight from the import. The charge and the two piles were fitted to a
cube of their longest side, as batch 1's piles were.

**1,703 → 1,391 credits, 312 for the batch**, run three at a time, so the
per-row figures below are balance deltas that overlap (the first three
report 72 each against a true ~24).

| file | item | image task | mesh task | size (m) | note |
|---|---|---|---|---|---|
| `held/assault_rifle.glb` | Assault Rifle | `01a15089-fcf7` | `01a1508a-7756` | 0.06 × 0.89 × 0.26 | barrel-up, grip 0.32 |
| `held/semi_automatic_rifle.glb` | Semi-Automatic Rifle | `01a15089-fd1f` | `01a1508a-7e7e` | 0.06 × 0.96 × 0.25 | barrel-up, grip 0.36 |
| `held/thompson.glb` | Thompson | `01a15089-fd5a` | `01a1508a-7d6a` | 0.05 × 0.87 × 0.28 | barrel-up, grip 0.34 |
| `held/custom_smg.glb` | Custom SMG | `01a1508b-2b27` | `01a1508b-a82c` | 0.05 × 0.62 × 0.35 | barrel-up, grip 0.42 |
| `held/bolt_action_rifle.glb` | Bolt Action Rifle | `01a1508b-2c86` | `01a1508b-ad02` | 0.07 × 1.09 × 0.29 | barrel-up, grip 0.33 |
| `held/pump_shotgun.glb` | Pump Shotgun | `01a1508b-5951` | `01a1508b-ae13` | 0.06 × 0.95 × 0.22 | barrel-up, grip 0.30 |
| `held/semi_automatic_pistol.glb` | Semi-Automatic Pistol | `01a1508c-59e8` | `01a1508c-d457` | 0.03 × 0.20 × 0.14 | barrel-up, grip 0.22 |
| `held/python_revolver.glb` | Python Revolver | `01a1508c-5bf1` | `01a1508c-ae7a` | 0.04 × 0.33 × 0.13 | barrel-up, grip 0.17 |
| `held/rocket_launcher.glb` | Rocket Launcher | `01a1508c-5e32` | `01a1508c-da60` | 0.20 × 1.06 × 0.40 | barrel-up, grip 0.45 |
| `held/rocket.glb` | Rocket | `01a1508e-b00b` | `01a1508f-0269` | 0.20 × 0.60 × 0.20 | nose-up; also drawn in flight |
| `held/timed_explosive_charge.glb` | Timed Explosive Charge | `01a1508d-85ca` | `01a1508d-daf4` | 0.24 × 0.12 × 0.15 | cube fit |
| `held/5_56_rifle_ammo.glb` | 5.56 Rifle Ammo | `01a1508d-ab02` | `01a1508e-2608` | 0.13 × 0.04 × 0.13 | cube fit |
| `held/12_gauge_buckshot.glb` | 12 Gauge Buckshot | `01a1508e-53e1` | `01a1508e-a726` | 0.13 × 0.06 × 0.13 | cube fit |

Prompts, in full, for the IP rail:

> **assault_rifle** — "A single assault rifle lying horizontally on a plain white background, seen exactly from the side, muzzle pointing right. A classic wood-furniture assault rifle: a stamped steel receiver, a brown hardwood stock, pistol grip and handguard, a long curved steel box magazine hanging under the receiver, a gas tube over the barrel, a front sight post and a slanted muzzle brake. 88 centimetres long, 24 centimetres tall including the magazine and 7 centimetres wide. Battle-worn: scratched black paint worn to bare steel on the edges, dented, dirty, the wood chipped and oiled. Real, photographed in soft daylight. Exactly one object. No ground, no people, no hands, no text, no labels, no logos, no scale bar."  ·  texture: "Worn gunmetal and blued steel scratched to bare metal on the edges, oil grime, matte, brown chipped hardwood stock and handguard, no text, no logos"
>
> **semi_automatic_rifle** — "A single homemade semi-automatic rifle lying horizontally on a plain white background, seen exactly from the side, muzzle pointing right. Built from salvage: a rough steel receiver welded from scrap plate with visible weld beads and bolts, a straight 10-round box magazine below, a scrap wooden stock bound with wire and tape, a plain round barrel with a simple front sight. 95 centimetres long, 20 centimetres tall and 6 centimetres wide. Rusty, scratched, dirty, crude but solid. Real, photographed in soft daylight. Exactly one object. No ground, no people, no hands, no text, no labels, no logos, no scale bar."  ·  texture: "Rusty scrap steel with weld beads and bolts, worn grey paint, scratched bare metal edges, rough wooden stock wrapped in wire and black tape, grime, no text, no logos"
>
> **thompson** — "A single vintage 1920s-style submachine gun lying horizontally on a plain white background, seen exactly from the side, muzzle pointing right. A blued steel receiver with a cocking knob on top, a finned barrel with a muzzle compensator, a walnut wooden butt stock, a walnut vertical foregrip under the barrel and a walnut pistol grip, a straight 20-round stick magazine below the receiver. 80 centimetres long, 25 centimetres tall and 7 centimetres wide. Worn bluing scratched to bare steel, the walnut scuffed. Real, photographed in soft daylight. Exactly one object. No ground, no people, no hands, no text, no labels, no logos, no scale bar."  ·  texture: "Worn blued steel scratched to bare metal, scuffed oiled walnut wood, grime, matte, no text, no logos"
>
> **custom_smg** — "A single crude homemade submachine gun lying horizontally on a plain white background, seen exactly from the side, muzzle pointing right. Made of welded steel pipe and folded sheet metal: a tube receiver, a short barrel shroud with drilled holes, a long straight magazine sticking down, a bent steel rod for a stock, a pistol grip wrapped in cloth tape, bolts and rivets. 60 centimetres long, 26 centimetres tall and 6 centimetres wide. Rusty, scratched, crude and dirty. Real, photographed in soft daylight. Exactly one object. No ground, no people, no hands, no text, no labels, no logos, no scale bar."  ·  texture: "Rusty welded steel pipe and sheet metal, scratched bare metal edges, drilled holes, cloth tape grip, grime, no text, no logos"
>
> **bolt_action_rifle** — "A single bolt-action hunting rifle with a telescopic sight lying horizontally on a plain white background, seen exactly from the side, muzzle pointing right. A long walnut stock with a cheek rest, a blued steel action with a bent bolt handle, a long slim barrel, a black steel rifle scope on two rings above the action, a small internal magazine. 112 centimetres long, 22 centimetres tall and 7 centimetres wide. Worn: scuffed wood, bluing worn to bare steel on the edges. Real, photographed in soft daylight. Exactly one object. No ground, no people, no hands, no text, no labels, no logos, no scale bar."  ·  texture: "Worn blued steel, black matte scope, scuffed oiled walnut stock, scratches, grime, no text, no logos"
>
> **pump_shotgun** — "A single crude pump-action shotgun lying horizontally on a plain white background, seen exactly from the side, muzzle pointing right. A thick steel pipe barrel over a tube magazine, a wooden pump fore-end that slides, a boxy steel receiver held with bolts, a rough wooden stock bound with steel straps. 95 centimetres long, 20 centimetres tall and 7 centimetres wide. Salvaged, rusty, scratched, dirty, sturdy. Real, photographed in soft daylight. Exactly one object. No ground, no people, no hands, no text, no labels, no logos, no scale bar."  ·  texture: "Rusty steel pipe and bolted receiver, scratched bare metal edges, rough worn wooden pump and stock, steel straps, grime, no text, no logos"
>
> **semi_automatic_pistol** — "A single compact semi-automatic pistol lying on its side on a plain white background, seen exactly from the side, muzzle pointing right. A matte black steel slide with rear serrations, an exposed hammer, a black polymer grip with a textured panel, a trigger guard. 19 centimetres long, 14 centimetres tall and 3.5 centimetres wide. Worn: the black finish rubbed to bare steel on the slide edges, dusty. Real, photographed in soft daylight. Exactly one object. No ground, no people, no hands, no text, no labels, no logos, no scale bar."  ·  texture: "Matte black steel slide worn to bare steel on the edges, black textured grip, grime, no text, no logos"
>
> **python_revolver** — "A single large-frame double-action revolver lying on its side on a plain white background, seen exactly from the side, muzzle pointing right. A long six-inch barrel with a ventilated rib on top and a full-length lug underneath, a fluted six-shot cylinder, a large hammer, a brown wooden grip. 30 centimetres long, 15 centimetres tall and 4 centimetres wide. Polished blued steel worn to bare metal on the edges, the wood scuffed. Real, photographed in soft daylight. Exactly one object. No ground, no people, no hands, no text, no labels, no logos, no scale bar."  ·  texture: "Polished blued steel worn to bright bare metal on the edges, scuffed brown wooden grip, grime, no text, no logos"
>
> **rocket_launcher** — "A single crude shoulder-fired rocket launcher lying horizontally on a plain white background, seen exactly from the side, front pointing right. A long thick steel tube 12 centimetres across, with a flared rear exhaust, a pistol grip and trigger under the front half, a second grip behind it, a padded shoulder rest, a simple folding iron sight and riveted steel bands around the tube. 105 centimetres long, 26 centimetres tall and 14 centimetres wide. Olive drab paint, chipped and scorched black at the rear, rusty, dirty. Real, photographed in soft daylight. Exactly one object. No ground, no people, no hands, no text, no labels, no logos, no scale bar."  ·  texture: "Chipped olive drab paint over steel, scorched black soot at the rear exhaust, rust, riveted steel bands, grime, no text, no logos"
>
> **rocket** — "A single crude rocket projectile lying horizontally on a plain white background, seen exactly from the side, nose pointing right. A cone-shaped steel warhead, a long steel pipe body, four flat stabilising fins at the tail, a black nozzle. 60 centimetres long, 12 centimetres across the fins. Olive drab and rusty steel, scratched, dirty. Real, photographed in soft daylight. Exactly one object. No ground, no people, no hands, no text, no labels, no logos, no scale bar."  ·  texture: "Olive drab paint over rusty steel, black nozzle, scratches, grime, no text, no logos"
>
> **timed_explosive_charge** — "A single timed explosive charge on a plain white background, three-quarter view from above. A rectangular brick of grey-white plastic explosive wrapped in olive drab cloth tape, with a small black digital timer box strapped on top, red and yellow wires running into the brick, and a short loop of black duct tape. 24 centimetres long, 14 centimetres wide and 8 centimetres tall. Dirty, worn, real. Real, photographed in soft daylight. Exactly one object. No ground, no people, no hands, no text, no labels, no logos, no scale bar."  ·  texture: "Grey-white plastic explosive brick, olive drab cloth tape, black plastic timer box with a dark display, red and yellow wires, black duct tape, grime, no text, no logos"
>
> **5_56_rifle_ammo** — "A single small pile of loose brass rifle cartridges on a plain white background, three-quarter view. About twelve long bottlenecked brass cases with copper pointed bullets lying together, some crossed over others, the pile 12 centimetres long, 10 centimetres wide and 4 centimetres tall. Slightly tarnished, real. Real, photographed in soft daylight. Exactly one object. No ground, no people, no hands, no text, no labels, no logos, no scale bar."  ·  texture: "Tarnished brass cartridge cases, copper bullet tips, slight grime, metallic, no text, no logos"
>
> **12_gauge_buckshot** — "A single small pile of shotgun shells on a plain white background, three-quarter view. About eight red plastic shotgun shells with brass bases lying together, some crossed over others, the pile 12 centimetres long, 10 centimetres wide and 5 centimetres tall. Scuffed, real. Real, photographed in soft daylight. Exactly one object. No ground, no people, no hands, no text, no labels, no logos, no scale bar."  ·  texture: "Scuffed red plastic shotgun shell hulls, tarnished brass bases with primers, slight grime, no text, no logos"

## `deploy/abyss_rug.glb` — the Abyss Rug, painted by Meshy (2026-10-18)

Meshy paid plan, commissioned output, same terms as the rows above. **Except
the mark**, which is Abyss's logo (the DEX the JUNK coin launches on) and is
pending their permission. `ci/meshy_gen.py --ref` sends abyss.trading's own
`abyss-icon.png` as the reference, so `nano-banana-pro` (image-to-image)
paints their mark rather than something logo-shaped.

**image-to-3d does not make a rug.** The three-quarter reference came back
as a 0.37 × 0.50 × 0.47 m box (mesh task `01a150b9-e842`, 15 credits),
the mark smeared into a "Y" over its sides: one view of a flat thing reads
as a block. So Meshy paints and the geometry is ours: a second image-to-image
pass (references: the first painting and the icon) shot the rug straight
down on magenta, saved as `ci/icons/abyss_rug_painting.png`, and
`ci/icons/abyss_rug.py` keys it, squares it up by four fitted corners, and
puts it on a 1.56 × 0.02 × 0.97 m slab with 7 cm alpha-cut tassels
(`alphaMode: MASK`) lying off each short end, 1.70 m overall. 0.97 rather
than the row's 1.1 because that is the painting's aspect, and stretching it
squashes the mark. Maps: the albedo, a normal map off its luminance and a
pile grain, and metal/rough so the gold thread is satin (metal 0.45, rough
0.48) against matte wool (rough 0.93); `ktx_pack.py --size 1024`.
`--drawn` still builds the earlier rug drawn in code, whose mark is the one
function `abyss_mark`. 69 credits all told across six images and the
rejected mesh.

| file | item | image tasks | size (m) | note |
|---|---|---|---|---|
| `deploy/abyss_rug.glb` | Abyss Rug (arch 22) | `01a150b9-09b4` (three-quarter), `01a150bd-e819` (top-down) | 1.70 × 0.02 × 0.97 | 32 vertices; the slab's sides fold the binding's outer 2 cm over the edge |

> **three-quarter** (ref: `abyss-icon.png`) — "A single woven rug lying perfectly flat, viewed from directly above at a slight angle, on a plain white background. Rectangular, 170 by 110 centimetres, two centimetres thick, with short cream tassel fringe on the two short ends. Deep dark teal wool (#0e3b43) with a subtle tonal pattern of thin wavy lines like light rippling far underwater. Framed by a metallic gold Art Deco border: an outer gold band and an inner thin gold line, with stepped zigurrat corners and small gold chevron fans at the middle of each side. In the centre, the gold emblem from the reference image woven in gold thread on the teal: a triangle 'A' whose left half is four slanted stripes. Match the emblem exactly. Rich, plush, slightly worn at the edges. Exactly one rug. No people, no objects, no text, no letters, no words."
>
> **top-down** (refs: the three-quarter painting, `abyss-icon.png`) — "The exact rug from the first reference image, scanned flat on a large-format scanner: perfectly orthographic top-down view, no perspective, no tilt, edges straight and parallel to the frame, all four corners square. Landscape orientation: the rug's long sides run left to right along the top and bottom, and the cream wool tassel fringe hangs off the left and right short ends. Same design as the reference: plush deep teal wool (#0e3b43) with thin tonal darker wave lines like light rippling underwater, evenly coloured across the whole rug; a metallic gold Art Deco frame of one wide outer band and one thin inner line, stepped ziggurat corners, and small gold chevron fans centred on all four sides. In the exact centre, woven in gold thread and standing upright with its apex toward the top edge, the emblem from the second reference image reproduced precisely: a triangle A whose lower left is cut into four parallel slanted stripes. The rug fills most of the frame with a margin all round. Solid flat bright magenta (#ff00ff) background. Bright, even, shadowless lighting with true colours, no vignette, no dark corners. Exactly one rug. No text, no letters, no words."

## Cacti — Meshy, 2026-10-19

Replace `plants::cactus`'s generated saguaro (which swayed on the bush stems'
wind). Same terms and pipeline as §Roadside: `meshy_gen.py` (nano-banana-pro →
meshy-t2, single view) → `flatten_charts.py --luma-only --strength 0.8` →
`import_meshy.py` (uniform, standing on its base) → `ktx_pack.py --size 1024`.
96 credits for the two. A cactus slot of `terrain::BALL_CACTUS_SPECIES` is the
ball; the rest are saguaros (`props::CACTUS_GLB`).

| file | tris | size | image | mesh |
|---|---|---|---|---|
| `prop/cactus_saguaro.glb` | 2,559 | 1.41 × 3.60 × 0.49 (trunk ≈ `CACTUS_R_M`) | `01a1522a-31db-73e8-97e8-d30f66f0a68e` | `01a1522a-fbfe-72e0-b34d-dd672d3c2b67` |
| `prop/cactus_ball.glb` | 1,212 | 0.62 × 0.60 × 0.62 (`BALL_CACTUS_TOP_M`) | `01a1522a-320a-7087-97ba-b7168bad2779` | `01a1522a-ae4a-7228-b011-275372bead61` |

## `site/works_crucible.glb`, `site/works_shard.glb` — the Crucible's furnace and the works' floating stones, Meshy, 2026-10-22

The works leave the black plinth for ancient machine ruins: weathered sandstone, verdigris
bronze and brass, empty glass housings the client lights (`render/works.rs` `LOOKS`). Same
terms as `deploy/` (Meshy paid plan, no `NOTICE` entry, no proper noun in any prompt) and the
same pipeline: `meshy_gen.py` (nano-banana-pro → meshy-t2 smart topology, single view) →
`import_meshy.py` → `ktx_pack.py --size 1024`. **120 credits for five rolls** (balance 2,175 →
2,055, measured, run serially). Two ship. `measure_glb.py` rejects the furnace's first roll for
green and chart contrast, which is the verdigris and the stone-against-bronze it was asked for.

| file | draws | image task | mesh task | credits | tris | size (m) | import |
|---|---|---|---|---|---|---|---|
| `site/works_crucible.glb` | THE CRUCIBLE's terminal (`works::LOOKS`, the anvil) | `01a1617a-9c22` | `01a1617b-1aed` | 24 | 10,011 | 3.19 × 2.28 × 3.20 | third roll; uniform into 4.0 × 3.5 × 3.2 (the room before the anvil rock); spout and mould on +Z |
| `site/works_shard.glb` | a work's floating stones (`works::SHARD_GLB`) | `01a1617d-4e71` | `01a1617d-cd11` | 24 | 1,664 | 0.40 × 0.41 × 0.45 | second roll; uniform into 0.6 × 0.45 × 0.45 |

**Rolled and not shipped:** the furnace's first roll (`01a16177-9821` / `01a16178-1702`)
is plainer, one buttress and a looser crucible; the second (`01a16179-1d2b` /
`01a16179-9c8c`) carved its frieze in pseudo-glyphs a player would try to read against
the real alphabet. The stone's first (`01a1617c-1b8e` / `01a1617c-9c95`) came back
grey and mossy against the furnace's warm stone.

**Anchors measured off the model** (the code's numbers, so a re-roll means re-measuring): the
crucible's rim 2.28 m up, centred at (−0.13, −0.31), 0.44 m inside radius, floored at 1.53; the
glass dome 0.42 m round at (1.15, 0.80, −0.30).

Prompts, in full, for the IP rail:

> **works_crucible** — "A single ancient machine-furnace ruin on a plain white background, three-quarter front view. A squat block of weathered warm sandstone masonry 400 centimetres wide, 320 centimetres deep and 240 centimetres tall, with a stepped base, chipped carved ornamental bands around its top edge and two heavy buttresses at the back. Set into the top, a large round brass crucible 150 centimetres across held in a geared bronze cradle with big toothed gear wheels on both sides. A thick bronze pipe runs from the crucible down the front face to a stepped stone spout over a shallow rectangular ingot mould at the foot. On the right side, bronze heat pipes with valve wheels run into an empty round glass-domed housing 70 centimetres across ringed with riveted bronze bands. Bronze and brass weathered to green verdigris patina, soot stains, cold and long abandoned. Ancient, ornate, weathered. Exactly one furnace. No fire, no flames, no glow, no light, no molten metal, no crystals inside the dome, no smoke, no ground, no people, no plants, no text, no letters, no symbols, no scale bar."  ·  texture: "Weathered warm sandstone masonry with chipped carved bands and soot stains, brass crucible, bronze gears, pipes and riveted bands with green verdigris patina, clear empty glass dome, no glow, no text, no letters, no symbols"
>
> **works_shard** — "A single fallen carved stone block fragment on a plain white background, three-quarter view. A broken chunk of weathered warm sandstone 60 centimetres long, 40 centimetres wide and 35 centimetres tall. One face is carved with a plain ornamental band and a thin inlaid bronze strip with green verdigris; the other faces are rough fractured stone with chipped edges. Ancient and weathered. Exactly one fragment. No glow, no ground, no people, no plants, no text, no letters, no symbols, no scale bar."  ·  texture: "Weathered warm sandstone, rough fractured faces, chipped edges, carved ornamental band, thin bronze inlay with green verdigris, no glow, no text, no letters, no symbols"

## `site/works_exchange.glb` — the Exchange's gateway, Meshy, 2026-10-22

Same terms and pipeline as the Crucible above. **72 credits for three rolls**, run serially. One
ships, uniform into 3.0 × 3.1 × 2.6 (the street's room short of the guards' beat; the string
lights cross 3.5 m over it).

| file | draws | image task | mesh task | credits | tris | size (m) | import |
|---|---|---|---|---|---|---|---|
| `site/works_exchange.glb` | THE EXCHANGE's terminal (`works::LOOKS`, the town) | `01a1620c-6873` | `01a1620d-17ea` | 24 | 10,398 | 3.00 × 2.99 × 2.12 | first roll; counter on +Z, turned about in the code to face the gate |

**Rolled and not shipped:** the second (`01a1620e-0391` / `01a1620e-8386`) filled
its porthole with sandstone where the veil glows; the third (`01a1620f-639a` /
`01a16210-0fb0`) drew dimension lines and centimetre labels into its reference image.

**Anchors measured off the model:** the porthole 0.7 m round about (0.02, 1.39), the panel's
faces at z −0.45 and −1.0; the counter z 0.35–1.06, 0.9 m tall.

Prompts, in full, for the IP rail:

> **works_exchange** — "A single ancient free-standing stone gateway ruin on a plain white background, three-quarter front view. Two square weathered warm sandstone pillars 50 centimetres wide joined by a heavy carved lintel across their tops, the whole gateway 300 centimetres wide, 300 centimetres tall and 80 centimetres deep. The opening between the pillars is closed by a recessed flat panel of deep blue glazed stone with one plain round recess 180 centimetres across in its middle, framed by a riveted bronze ring mount. Bronze and brass machinery on the pillars: toothed gear wheels, a small piston, pipes running up into the lintel, valve wheels. In front of the right pillar stands a waist-high sandstone counter 100 centimetres tall with a brass top, a narrow slot and a shallow brass tray, and a small gear-driven sorting mechanism with brass chutes on its side. Bronze and brass weathered to green verdigris patina. Ancient, ornate, weathered. Both faces of the gateway finished alike. Exactly one gateway. No door, no ring inside the recess, no glow, no light, no crystals, no coins, no ground, no people, no plants, no text, no letters, no symbols, no scale bar."  ·  texture: "Weathered warm sandstone pillars and carved lintel, deep blue glazed stone panel, bronze ring mount, gears and pipes with green verdigris patina, brass counter top and tray, no glow, no text, no letters, no symbols"
