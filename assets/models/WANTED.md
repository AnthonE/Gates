# assets/models — WANTED

The 3D objects this game needs and does not have. **Owns nothing** — a
sourcing worklist, the shape `reference/RIPLIST.md` has for balance numbers.
`ART.md` is still the bar, `MANIFEST.md` still records what ships, and a row
here lands only when a file is in the tree with a manifest row beside it.

Every size below is **read off the code, not proposed** — the sim already
blocks these volumes and the client already draws a box of exactly these
extents. A mesh that does not fit its row is a mesh that clips a player or
leaves a gap they can walk through, so the numbers are a fit test and not a
style note. `crates/client/tests/tree.rs` is the shape of the gate that
checks one (`CLAUDE.md`: what may be gated about a frame is arithmetic).

**63 meshes and 6 texture sets.** 12 world scatter · 6 build shapes · 12
deployables · 22 held items · 3 worn · 1 animal · 3 projectile/misc · 4
deadfall (§9). The texture sets are §9 too, and they are the only thing on
this page a mesh generator cannot make.

**Two of the 63 are marked do-not-buy** (§2.1 pine, §2.6 bush) and stay listed
only so the inventory is complete — see §1.

---

## 0 · Pipeline — what a file has to be to load here

| thing | value | why |
|---|---|---|
| format | **glTF 2.0 binary (`.glb`)**, or `.gltf` + `.bin` | `bevy_gltf` is already in the dep set and load-bearing (the mannequin). Nothing else has a loader. |
| units | **metres, 1.0 = 1 m** | every number in this file is metres; the sim's are too. |
| axes | Y up, −Z forward | glTF's own convention and Bevy's, so a straight glTF export needs no fixup. |
| origin | **at the ground contact point, centred in X/Z** | see §0.1 — this is the one convention worth insisting on, and it is not what Meshy defaults to. |
| triangles | see each row; frame ceiling is **1.5 M** (`RENDER.md` §6) | a scatter ring puts ~40 boulders on screen at once. A 60 k-triangle boulder is 2.4 M on its own. |
| textures | albedo + normal + **ORM-packed** metallic-roughness | §0.2 — this is free and it fixes a shipped defect. |
| rig | none, except the pig and the player | everything else is static. Do not ship blend shapes or animation for a crate. |

### 0.1 · Origin at the base, not the centre

`props::spawn_slot` carries a per-archetype `lift` constant — 0.5 for an ore
node, 0.55 for the boulder, 0.4 for a crate — for exactly one reason: the
generated meshes are **centred on their own bounding box**, and the slot's `y`
is the ground. Every one of those constants is a hand-measured correction for
a mesh that does not know where its own feet are.

Author with the origin **on the ground plane** and every one of those goes to
0.0, which deletes a table of magic numbers rather than adding to it. Same for
`structures::DEPLOY` — `deploy_transform` puts a deployable's origin at the
cell's floor height and the current `Cuboid` meshes are centred, so a furnace
is drawn half-buried unless the table compensates.

Exceptions, both deliberate: a **door** rotates about its hinge, so its origin
is at the hinge edge on the floor, not the centre of the leaf. A **held item**
(§5) is posed in view space by `VIEWMODEL_HOLD`, so its origin is where the
hand grips it — the middle of a haft, the grip of a revolver.

### 0.2 · The ORM slot, which is currently a hole

`terrain_mesh.rs` and `props.rs` both leave `metallic_roughness_texture`
**unwired**, and say why: our sourced maps are greyscale roughness JPEGs, and
that Bevy slot is the glTF-packed ORM texture whose **B channel is metallic**.
Binding a roughness JPEG there makes every prop a half-metal, so both files
fall back to a roughness scalar and lose the map.

A glTF exporter packs ORM correctly by construction. So a Meshy `.glb` is the
first asset class in this repo that can carry real per-texel roughness, and it
arrives with no packing step and no code change. Ask for PBR textures on the
export; do not ask for a separate roughness JPEG.

### 0.3 · The IP rail still applies to the prompt

`DESIGN.md`'s rail is proper nouns and traced assets. A generator prompt is a
place both can leak: do not name the reference game, its items, or its
studio in a prompt, and do not feed it screenshots from the reference set.
Describe the object — "weathered steel drum, dented, rust streaks" — which is
what `ART.md` §1 asks for anyway. Generated output is yours; it needs a
`MANIFEST.md` row (source: the generator, date, prompt) but no licence notice.

### 0.4 · Two rows need a code change before a second mesh can be drawn

Flagging these so you do not pay for a model that nothing can index:

- **`box_small` and `box_large` share archetype `box`** (index 2), one mesh at
  1.0 × 0.7 × 1.0. Two models means an eleventh archetype, which is a wire
  concern (`protocol/event.rs` widened the field to 4 bits for the recycler,
  so there is room) — not a big change, but not zero.
- **`door_wood` and `door_metal` share archetype `door`** (index 6) and today
  differ by nothing at all in the draw: `spawn_deploy` picks a material only
  on `locked`. An 800 hp metal door and a 200 hp wooden one are the same
  picture, which is a real defect independent of this list.

Everything else in §2–§8 has a slot that already indexes it.

---

## 1 · Already covered — do not spend credits here

- **The player body.** `assets/models/stumpy.glb` — commissioned, 24 joints,
  **53 clips**, 1.800 m, and `render/anim.rs` binds them.
  ⚠ **This bullet was wrong in three ways on 2026-08-17 and is worth reading
  as a correction rather than a fact.** It said the body was the mannequin,
  that a replacement *must* be rigged to the mannequin's 53-joint skeleton or
  its 46 clips would be lost, and that "generated meshes are not rigged; this
  is the one row where a generator is the wrong tool." The replacement landed
  that day: it came from a generator, it arrived **rigged and animated**, it
  is on its **own** skeleton — and the 46 clips came across anyway, by
  retarget (`ci/retarget_anim.py`), in nine seconds. Every clause was a
  reasonable inference and every one was overtaken. The mannequin stays in the
  tree as the retarget's source.
- **Every plant.** Trees, bushes, grass — **do not buy a foliage `.glb`, from
  a generator or anywhere else.** `reference/PLANTS.md` is the whole argument;
  the short form is that a plant is an alpha-card problem and a mesh generator
  emits opaque hulls, so a bought bush is the green potato we already have.
  The forest's gaps are a **species table** (ez-tree's 15 MIT presets port
  into settings we already ship, no asset), a **placement fix** (scatter is a
  uniform 8 m lattice, which `ART.md` rule 7 forbids), an **LOD**, and
  **leaf/bark textures** — three code slices and a texture hunt, no models.
  §2.1 and §2.6 stay listed as rows only so the inventory is complete.
- **Grass, pebbles, twigs, litter** (`clutter.rs`). Thousands of elements per
  16 m tile, built as one merged mesh per tile. This is a population, not an
  object; a `.glb` blade of grass would be 721 draw calls a tile.
- **Terrain, water, sky.** Generated, and not objects.
- **Icons.** All 65 UI icons exist (`assets/icons/`, game-icons.net, CC BY).
  A 3D item model does **not** replace its icon — the inventory draws the PNG.

---

## 2 · World scatter — 12

What the terrain places (`sim_core::terrain::Occupant`). These are the highest
value on the list: they are what fills the frame, they instance heavily, and
every one is currently a lump of noise or a stack of boxes.

Sizes are the **full extents at scale 1.0**; the terrain applies a per-slot
`scale` on top (roughly 0.8–1.25), so build to the nominal.

| # | object | full size (m) | tris | what it is |
|---|---|---|---|---|
| 2.1 | **Pine** | 3.4 ⌀ × **6.6** tall | ≤ 8 k | Generated (`tree.rs`), and staying generated. **Do not buy — §1, and `reference/PLANTS.md` §4.** Listed so the inventory is complete. |
| 2.2 | **Pine stump** | 0.64 ⌀ × 0.34 tall | ≤ 500 | What a felled tree leaves. Bark ring, and a **fresh cut face** lighter than the bark — that contrast is the whole reason a stump reads as a stump. Drawn by `apply_fell` the instant a tree drops. ⚠ **Buy this AFTER the verb, not before** (`NOW.md` §0stump): the operator's point is that a stump is collected for wood, and ours is not — it is not an `Occupant`, has no `gatherables.toml` row and no collision, so today a model here is an ornament on something you cannot click. |
| 2.3 | **Stone node** | 1.83 ⌀ × 1.25 | 1,439 | ⚠ **RE-ROLL QUEUED 2026-09-05** — what shipped 2026-09-02 (`prop/node_stone.glb`) is a **1.39 cube** of blocks; a node is round in plan because the sim blocks a cylinder (`ci/measure_glb.py`, `MANIFEST.md` §prop for the prompt). ✅ Covered 2026-09-02 — `assets/models/prop/node_stone.glb`. ⚠ The size column said 2.0 × 2.0 × 2.0; the sim blocks a **cylinder** r=0.9148 by 1.2538 tall (`OCCUPANT_R_M`/`_TOP_M`, measured off the blob), so it is a low wide outcrop and not a cube. Granite outcrop, not a boulder — it should read as *bedrock breaking the surface*. Sits 0.5 m into the ground. |
| 2.4 | ~~**Metal node**~~ | 1.83 ⌀ × 1.25 | 1,364 | ✅ **COVERED 2026-09-02** — `assets/models/prop/node_metal.glb`. ⚠ The size column said 2.0 × 2.0 × 2.0; the sim blocks a **cylinder** r=0.9148 by 1.2538 tall (`OCCUPANT_R_M`/`_TOP_M`, measured off the blob), so it is a low wide outcrop and not a cube. Same rock, with dark ore seams and a metallic glint. `ART.md`: a node's identity is the glint its reflectance gives it. |
| 2.5 | ~~**Sulfur node**~~ | 1.83 ⌀ × 1.25 | 1,490 | ✅ **COVERED 2026-09-02** — `assets/models/prop/node_sulfur.glb`. ⚠ The size column said 2.0 × 2.0 × 2.0; the sim blocks a **cylinder** r=0.9148 by 1.2538 tall (`OCCUPANT_R_M`/`_TOP_M`, measured off the blob), so it is a low wide outcrop and not a cube. Same rock, yellow crystalline crust in the fissures. |
| 2.6 | **Berry bush** | 1.4 × 1.4 × 1.4 | ≤ 800 | A green sphere today, and the fix is ez-tree's three `bush_*` presets, not a model. **Do not buy — §1.** |
| 2.7 | **Boulder** | 2.23 ⌀ × 1.98 | 2,662–2,670 | ⚠ **RE-ROLL QUEUED 2026-09-05 for `rock_a`** — a 1.17 ball at luma 0.344, which is an ore node's silhouette and value; it re-rolls as a *formation* (`MANIFEST.md` §prop). ✅ Covered 2026-09-02 — a **pool of three**, `prop/rock_{a,b,c}.glb`, indexed by yaw. One mesh was 1,054 identical boulders on the shipped seed. Same cylinder caveat as 2.3: the sim blocks r=1.1145 by 1.5403, not a 3 m cube. |
| 2.8 | ~~**Loot barrel**~~ | 0.585 ⌀ × 0.88 | 758 | ✅ **COVERED 2026-09-02** — `prop/barrel.glb`. The size column said 0.9 ⌀ × 0.95; the sim blocks 0.585 × 0.88 (the measured 55-gallon drum, `DECISIONS.md` barrel proportions v1). |
| 2.9 | ~~**Supply crate**~~ | 1.1 × 0.8 × 0.8 | 522 | ✅ **COVERED 2026-09-02** — `prop/crate.glb`. A BOX row: `OCCUPANT_R_M` 0.6801 is exactly hypot(0.55, 0.40), so it imports `--fit-axes`. |
| 2.10 | ~~**Cache box**~~ | 0.9 × 0.55 × 0.7 | 479 | ✅ **COVERED 2026-09-02** — `prop/cache.glb`. Prompted as *visibly poorer* than 2.9 — mismatched split planks and rope where the crate has iron banding — because a loot table is chosen by which of the two you opened. |
| 2.11 | ~~**Haven shelter**~~ | 7.0 × 9.2 × 7.0 | 4,140 | ✅ **COVERED 2026-09-01** — `assets/models/site/shelter.glb` (`MANIFEST.md`). ⚠ The size column above was WRONG and this row is the correction: it read 7.2 × 5.6 × 7.2 where `SHELTER_BOXES` bounds are 7.0 × 9.2 × 7.0 — **3.6 m short on height**, which is the axis the tower lives on, so it was a spec nobody could have hit. Read the box table, never this file, for a volume the sim collides with. |
| 2.12 | ~~**Waystation canopy**~~ | 5.6 × 4.1 × 5.6 | 3,801 | ✅ **COVERED 2026-09-01** — `assets/models/site/canopy.glb`. Same correction as 2.11: this row said 3.8 × 2.1 × 3.8 against `WAYSTATION_CANOPY_BOXES`' 5.6 × 4.1 × 5.6 — wrong on every axis, by 1.8 / 2.0 / 1.8 m. The ≤ 2 k triangle target is exceeded and deliberately not enforced — `tests/site_assets.rs` gates 12 k, because a structure that stands twice on an island and never instances is not what presses `RENDER.md` §6's frame ceiling. |

---

## 3 · The build kit — 6 shapes

`content/building.toml` has 18 rows (6 shapes × 3 materials) but the material
is a **texture set, not a mesh**: `structures.rs` builds one mesh per shape and
swaps `TIER[0..3]`. So this is 6 models × 3 texture variants, and the wood /
stone / metal split is best done as three material sets over the same geometry.

Grid: cell 3.0 m, level height 3.0 m, seam 0.04 m (so a spanning piece is
2.96 m, deliberately — the gap is what stops z-fighting between neighbours).

| # | shape | full size (m) | tris | notes |
|---|---|---|---|---|
| 3.1 | **Foundation** | 2.96 × 0.3 × 2.96 | ≤ 800 | Slab whose **top** is the level plane — players stand on it. Origin at the top face. |
| 3.2 | **Floor** | 2.96 × 0.3 × 2.96 | ≤ 800 | Same volume as 3.1, different read: planking vs footing. May share a mesh if you would rather spend the budget elsewhere. |
| 3.3 | **Roof** | 2.96 × 0.3 × 2.96 | ≤ 800 | Same volume again. Currently the fallback slab. |
| 3.4 | **Wall** | 0.24 × 3.0 × 2.96 | ≤ 1 k | Thin. 0.24 m is `collide::WALL_THICKNESS_M` and the sim's collision uses it — do not thicken. |
| 3.5 | **Doorway** | 3 parts, see below | ≤ 1.5 k | Two posts, each 0.24 × 3.0 × 0.9, hugging the ends; a lintel 0.24 × 0.9 × 1.16 whose **underside is at 2.1 m** — that is the height the sim lets a player through, so the lintel is not decorative. Opening: 1.16 m wide × 2.1 m tall. |
| 3.6 | **Stairs** | 2.96 × 0.3 × 4.15 | ≤ 1.5 k | A ramp, pitched 45° (−π/4 about X), rising toward +Z. Steps cut into the top face are welcome; the collision is the ramp plane. |

---

## 4 · Deployables — 12

`content/deployables.toml`, drawn from `structures::DEPLOY`. Every one is a
flat-coloured `Cuboid` today. Sizes are full extents and are what the sim
blocks, so they are hard.

| # | object | full size (m) | tris | notes |
|---|---|---|---|---|
| 4.1 | **Sleeping bag** | 1.2 × 0.25 × 0.7 | ≤ 500 | Rolled-out canvas bedroll on the ground. A respawn point — it must be findable from a distance. |
| 4.2 | **Hearth** | 0.9 × 0.9 × 0.9 | ≤ 1 k | The base's claim stone (`sim-core/lock.rs`, `reference/BUILDING.md`). Stone plinth with a fire bowl — reads as *permanent*, where 4.4 reads as temporary. |
| 4.3 | **Small box** | 1.0 × 0.7 × 1.0 | ≤ 600 | Wooden storage chest, hinged lid, iron banding. |
| 4.4 | **Fire pit** | 0.7 × 0.4 × 0.7 | ≤ 600 | Ring of stones, logs, ash. A cooking station — `cooking.toml` turns raw meat into cooked meat on it. |
| 4.5 | **Furnace** | 1.1 × 1.5 × 1.1 | ≤ 1.5 k | Stacked-stone smelter, chimney, glowing mouth. Tall. |
| 4.6 | **Workbench** | 1.6 × 0.9 × 0.9 | ≤ 1.5 k | Wide, low, cluttered bench top. Stands beside 4.9 and must be tellable apart across a room by silhouette alone. |
| 4.7 | **Wooden door** | 0.12 × 2.1 × 0.9 | ≤ 500 | Planked leaf, iron strap hinges. **Origin at the hinge edge on the floor** — it swings. |
| 4.8 | **Metal door** | 0.12 × 2.1 × 0.9 | ≤ 500 | Riveted sheet steel. Must read as four times the door 4.7 is; today they are the same picture (§0.4). |
| 4.9 | **Research table** | 1.5 × 0.8 × 0.8 | ≤ 1.2 k | Waist-high, deep. Papers, calipers, a vice. |
| 4.10 | **Recycler** | 1.3 × 1.15 × 0.9 | ≤ 1.2 k | Metal, squat, industrial hopper. Sits next to 4.5 in a base — *squat metal* against the furnace's *tall stone* is the whole read. |
| 4.11 | **Code lock** | 0.2 × 0.3 × 0.12 | ≤ 400 | Keypad, bolted on. **Not drawn at all today** — a lock mints no deploy record, so this needs a draw path as well as a mesh. Mounts on a door's outer face at chest height. |
| 4.12 | **Large box** | ~1.4 × 1.0 × 1.0 | ≤ 800 | Blocked on §0.4's archetype split. Size is a proposal, not read off code — nothing draws it yet. |

---

## 5 · Held items — 22

**There is exactly one held-item model in the game and it is a hatchet**
(`viewmodel.rs`: `handle_mesh` + `head_mesh`). A revolver, a torch, a medkit
and a cooked steak are all drawn as that hatchet. This is the largest gap on
the list by count and the one a player meets every second of play.

First person, so these are seen at **~0.5 m** — the closest anything gets to
the camera, and the only place where geometry the eye can resolve is worth
paying for. Origin at the grip. Length along −Z (into the screen).

Deployables carried in hand (bag, boxes, furnace, doors, …) reuse their §4
model; they are not repeated here.

**Tools & weapons — 12**

| # | item | length (m) | tris | notes |
|---|---|---|---|---|
| 5.1 | Rock | 0.15 | ≤ 400 | The naked spawn's first tool. A hand-sized stone, one chipped edge. |
| 5.2 | Torch | 0.5 | ≤ 500 | Bound rag head. Needs a flame — that is a VFX slice, not this mesh. |
| 5.3 | Wooden spear | 2.0 | ≤ 600 | Fire-hardened point, no head. |
| 5.4 | Metal spear | 2.0 | ≤ 700 | Leaf-blade head, bound socket. |
| 5.5 | Stone hatchet | 0.5 | ≤ 700 | Lashed stone bit. |
| 5.6 | Metal hatchet | 0.55 | ≤ 700 | Forged head, socketed haft. *Replaces the current one — the only existing model.* |
| 5.7 | Stone pickaxe | 0.6 | ≤ 700 | Two-pointed stone head. |
| 5.8 | Metal pickaxe | 0.65 | ≤ 700 | Forged, one pick one adze. |
| 5.9 | Hunting bow | 1.2 | ≤ 800 | **Needs a drawn and an at-rest string** — `sim-core/ranged.rs` has a draw. The string is the part a generator will skip; expect to add it as two triangles. |
| 5.10 | Crossbow | 0.8 | ≤ 1 k | Stock, prod, trigger. |
| 5.11 | Revolver | 0.28 | ≤ 1 k | Six-shot, worn blueing. The one item where 0.5 m viewing distance really bites. |
| 5.12 | Satchel charge | 0.35 | ≤ 800 | Cloth bag of explosive, fuse, taped bundle. Thrown — so it needs to read in the air too. |

**Utility — 4**

| # | item | size (m) | tris | notes |
|---|---|---|---|---|
| 5.13 | Hammer | 0.4 | ≤ 600 | The building tool. Claw or mallet, your call. |
| 5.14 | Building plan | 0.4 | ≤ 400 | Rolled/held blueprint. Held while the shape wheel is open. |
| 5.15 | Bandage | 0.12 | ≤ 300 | Rolled cloth strip. |
| 5.16 | Medkit | 0.25 | ≤ 500 | Canvas pouch, cross marking. Do not use a real organisation's mark. |

**Food — 6** — small, seen in hand while eating, and also the contents of a
corpse bag.

| # | item | size (m) | tris | notes |
|---|---|---|---|---|
| 5.17 | Berries | 0.1 | ≤ 300 | Handful/cluster. |
| 5.18 | Mushrooms | 0.12 | ≤ 300 | Two or three caps. |
| 5.19 | Corn | 0.2 | ≤ 300 | Cob, husk partly peeled. |
| 5.20 | Raw meat | 0.2 | ≤ 400 | The one food you cannot eat — it exists to be carried to a fire. |
| 5.21 | Cooked meat | 0.2 | ≤ 400 | Same cut, seared. Must be tellable from 5.20 and 5.22 in hand. |
| 5.22 | Burnt meat | 0.2 | ≤ 400 | Blackened. |

---

## 6 · Worn — 3

`content/armor.toml`. These fit the **player rig's skeleton**, so unlike
everything else on this list they are not standalone props — they are either
skinned to the same rig or rigid pieces parented to a joint. Rigid is fine for
the hood and the vest; the tunic is not.

⚠ **Which skeleton that is changed on 2026-08-17 and this section said the
wrong one until then.** The rig is `stumpy.glb` — **24 joints**, named
`Hips / Spine / Spine01 / Spine02 / neck / Head / Left|Right + UpLeg, Leg,
Foot, ToeBase, Shoulder, Arm, ForeArm, Hand`. The mannequin's 53 Rigify
`DEF-*` names are no longer what anything loads. Two consequences for this
section: a parented piece names a joint from *that* list, and **there are no
finger joints at all**, so anything that wanted to fit a hand fits `LeftHand`
or `RightHand` as one rigid piece.

| # | item | slot | notes |
|---|---|---|---|
| 6.1 | Burlap hood | head | Sackcloth, eye slit. Parent to the head joint. |
| 6.2 | Burlap tunic | body | Sackcloth over the torso. **Wants skinning**, or it will not follow the run cycle. |
| 6.3 | Roadsign vest | body | Cut-and-bent sheet metal plates lashed over the chest — scavenged, not fitted. Rigid plates over a skinned strap is the honest build. |

> **The replacement character landed, and it did exactly what the note that
> used to stand here warned about.** `stumpy.glb` ships on its own 24-joint
> skeleton, not the mannequin's, so the 46 clips did not come with it — the
> game runs on the seven the delivery carried. The warning was right; it was
> also not a veto, because the character is the product and an animation set
> is a thing you can add to a rig. §11 is the bill.

---

## 11 · The player's missing clips — 0

**Opened and closed the same day.** This section listed seven clips the
character did not have; `ci/retarget_anim.py` moved the mannequin's whole
46-clip library onto its skeleton and six came across — `Sprint_Loop`,
`Death01`, `Hit_Chest`, `Jump_Loop`, `Swim_Fwd_Loop`, `Crouch_Idle_Loop` +
`Crouch_Fwd_Loop`. The two routes named here were "commission them" and
"retarget them, nobody has tried it"; the second was tried, took nine seconds
and cost 1 MB.

The rig is `stumpy.glb`, 24 joints (§6), **53 clips**. A name must be
**exactly** right: `render/anim.rs` resolves by name and
`crates/client/tests/rig_asset.rs` fails on a mismatch.

**Closed 2026-08-17 — nothing on this list is outstanding.** The last item was
a `Gather_Swing`, since 46 clips contain no chopping motion; the operator took
`Sword_Attack` instead (*"accept sword attack because tbh thats rust lol you
just swing whatever pretty much"*, `DECISIONS.md`). ⚠ **It shipped as `Punch_Cross` for a day and is `Sword_Attack` again.**
A parallel lane measured that `Sword_Attack`'s 1.5 s does not fit the 1.267 s
swing cadence and took the only clip short enough; the operator then rejected
the punch on sight, because it puts a hand 15 cm inside this body's oversized
head (0.147 m against a 0.295 m head radius, where the sword holds 0.490 m).
The clip is retimed to 1.087 s at import instead — see the 2026-08-18 row. The reference game swings
one animation at everything, so a dedicated chop was a distinction it does not
draw either.

**Nothing here is now waiting on an asset.** What third-person swinging is
waiting on is a fact on the wire — `bodies.rs` sees a remote's position, yaw
and sleeping flag and nothing else — which is `NOW.md` §0sw's problem and not
this file's.

---

## 7 · Animals — 1

| # | object | size (m) | tris | notes |
|---|---|---|---|---|
| 7.1 | **Pig** | 1.5 long × 0.78 tall | ≤ 2 k | `content/mobs.toml`, 80 hp. Ships as **body + one leg mesh**, because `mobs.rs` spawns four leg children and swings them per-leg (`leg_swing_rad`). So: one body with four hip sockets, and one leg with its origin at the hip. Not a rigged animal — the client's animation is four transforms. |

`crates/client/tests/` measures `PIG_LEN_M` / `PIG_H_M` off the shipped mesh,
so a replacement is gated on fitting 1.5 × 0.78.

---

## 8 · Projectiles & misc — 3

| # | object | size (m) | tris | notes |
|---|---|---|---|---|
| 8.1 | **Wooden arrow** | 0.7 long | ≤ 300 | In flight and lodged in what it hit. `reference/PROJECTILES.md`: the arrow is an item three times over — fired, lodged, recovered. Drawn today as a cylinder tracer. |
| 8.2 | **Metal arrow** | 0.7 long | ≤ 300 | Broadhead. Tellable from 8.1 when lodged. |
| 8.3 | **Death backpack** | 0.6 × 0.35 × 0.45 | ≤ 600 | A low canvas bundle where a body fell — also what a killed pig leaves. It is how you find your own corpse, so it must read at range against grass. |

---

## 9 · Plants — 4 meshes and 6 texture sets

`reference/PLANTS.md` is the research; this is its §6.4 in buying order. The
split matters: **the meshes below are opaque solids, which is what a mesh
generator is good at. The textures are alpha cut-outs, which is what it is
not** — and the plants themselves stay generated, so they appear nowhere here.

**Deadfall — 4 meshes.** The cheapest thing that makes a forest floor read as
a forest floor and not as ground with trees standing on it. No alpha, no
leaves, so a generator is exactly right.

| # | object | size (m) | tris | notes |
|---|---|---|---|---|
| 9.1 | **Fallen log, long** | 0.5 ⌀ × 6.0 | ≤ 1.5 k | Bark sloughing off one side, broken at both ends. Lies flat — a player walks over it. |
| 9.2 | **Fallen log, short** | 0.45 ⌀ × 2.5 | ≤ 1 k | Same identity, different footprint so a clearing does not repeat. |
| 9.3 | **Root plate** | 2.5 × 2.2 × 0.8 | ≤ 1.5 k | The disc of roots and soil an uprooted tree tears up. Stands near-vertical; the single most legible "a tree fell here" silhouette. |
| 9.4 | **Rotting stump** | 0.7 ⌀ × 0.5 | ≤ 600 | Older and softer than §2.2's fresh cut — no clean face, collapsed centre. |

**Textures — 6 sets**, and this is where the actual gap is. Alpha cut-out
atlases unless noted; `ART.md` §7's sourcing rules apply and are not a
formality — score candidates on gain span with the shipped estimator *before*
committing, since that lever took `rock` from keep 0.17 to 0.97 on a file swap
with no code change.

| # | set | why | priority |
|---|---|---|---|
| 9.5 | **Conifer sprig atlas** | Replaces `tree::needle_image`, which is *generated* and is the weakest link in the canopy today. Highest-value texture on this page. | 1 |
| 9.6 | **Broadleaf atlas** | Needed the moment `TreeType::Deciduous` is switched on. Ash / aspen / oak read very differently — one sheet with all three is fine. | 2 |
| 9.7 | **Grass blade atlas** | Blades are vertex-coloured today with **no map at all**, on the layer `ART.md` calls the largest structural gap. | 3 |
| 9.8 | **Fern / frond atlas** | The shrub layer's one irreducible texture — a branch generator has no grammar for a frond. | 4 |
| 9.9 | **Bark ×2–3** (not alpha) | We ship one. Birch and a dead/weathered bark carry most of the species read at trunk level. | 5 |
| 9.10 | **Flower / forb atlas** | Meadow-biome variety. Small, cheap, last. | 6 |

Sources to start from: [`madjin/awesome-cc0`](https://github.com/madjin/awesome-cc0)
indexes the CC0 collections; ambientCG and Poly Haven are the usual bark
sources. Every asset host is blocked by this box's egress proxy, so these are
the operator's to fetch — a loop cannot pull them.

---

## 10 · If you are only doing fifteen

Ranked by what a player looks at most, weighted by how bad the current
stand-in is:

1. §5.6 metal hatchet, §5.5 stone hatchet — the starting tools, in view always
2. §5.9 bow, §5.11 revolver — same, and today they are a hatchet
3. §2.3–2.5 the three ore nodes — the gather loop's whole surface
4. §2.7 boulder, §2.8 barrel — the scatter's most-instanced solids
5. §4.5 furnace, §4.6 workbench, §4.3 small box — the base you look at while crafting
6. §4.7 / §4.8 the two doors — every base has several and they are identical today
7. §7.1 pig — the only animal
8. §8.3 death backpack — the object a player hunts for after every death
9. §9.1–9.3 deadfall — cheapest per-unit gain on the page: three logs and a
   root plate turn "ground with trees on it" into a forest floor, and unlike
   everything else here they need no code change to be placeable

And separately from the fifteen, **§9.5 the conifer sprig atlas** — it is a
texture rather than a model, so it does not compete for the same budget, and
it replaces a canopy card the client is currently *generating*.

Everything in §5's food and utility block is small, cheap, and rarely the
thing on screen; do it last.
