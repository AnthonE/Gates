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
