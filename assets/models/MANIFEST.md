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
ci/split_arms.py stumpy_rt.glb assets/models/stumpy.glb
```


1. **The up axis is Z.** The merged-animation export lies on its back; the
   character export of the *same model* is Y-up. One generator, one rig, two
   files, two answers — so this is checked per delivery, never assumed.
2. **The clip names are the generator's.** `render/anim.rs` resolves by NAME
   and a name it cannot find draws a body frozen in its bind pose, so
   `Idle_11` is an idle that never plays.

**The last step is what makes a first-person viewmodel possible**, and it is
one line because the alternative was believed impossible: the mesh becomes
`char1_arms` + `char1_body`, two nodes on one skeleton **sharing their vertex
buffers** and differing only in their index array (+0.4 MB, no second copy of
anything). `render/viewmodel.rs` hides the body half and draws the arms on the
camera; `bodies.rs` draws both and is unchanged.

`crates/client/tests/rig_asset.rs` gates all of it — the clip names off `Clip`
itself, the height against `ANIM_RIG_H_M`, the stand-up rotation, one material,
KTX2 textures, both halves of the split, the arms' hold clip, and that the
swing clip still fits the sim's swing cadence. Five of the eight were watched
going red under their own defect — four against the raw file, and the cadence
one against the un-retimed 1.5 s swing.

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
from. `bodies.rs` knows a remote's position, yaw, pitch and whether it is
asleep, and that is the whole input — so `Death01`, `Jump_Loop`,
`Swim_Fwd_Loop` and the crouch pair sit in the file unplayable, each waiting
on a fact on the wire. The clips are there before the states that would play
them, which is the right way round. (`WANTED.md` §11 is closed: the gather
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
