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

⚠ **40 MB for five props, against 6.5 MB for the entire texture set.** The 2K
maps are an operator call (*"i dont wanna nerf looks too much"*) and the real
lever is format, not resolution: Bevy is built here with
`features = ["jpeg", "wav"]`, so every one of these decompresses to full RGBA
in VRAM. `ktx2`/`basis-universal` is the unbuilt fix.

## Why this one, and why a mannequin is the right placeholder

`ART.md` §7's "real detail is allowed, and preferred" is about textures and says
meshes are "the same deal when the time comes". This is that.

**It is a rig, not a character.** That matters more than it sounds. Every other
reachable CC0 humanoid pack is *stylized low-poly*, which would commit the game
to an art direction the operator has not spoken — and the one that IS spoken is
`Rust Images/`, like-for-like (`DECISIONS.md` 2026-08-01). A featureless
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
