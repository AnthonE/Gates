# Branch notes — `claude/game-visual-improvements-fj1qyz`

Six visual slices, five commits, all gates green locally. This replaces the
`claude/building-placement-foundations-3gmwfk` note.

**Read `NOW.md` §0out first**, then `DECISIONS.md` §open's five new rows
(specular v0, outer tree ring v0, fire light v0, clutter contact v0's second
half, prop tint v0).

## The one thing that matters more than any of it

**Nothing here has been looked at.** No GPU has ever run this client
(`RENDER.md` §6) and this box has no Vulkan ICD, so every change below is
arithmetic, gates and Bevy's own source — which is exactly the evidence
`CLAUDE.md` says is *not* the visual gate. Five of the six slices move pixels
in ways only a person can score. The highest-value hour available now is
booting it and walking three routes:

1. **To 80 m**, for the tree LOD swap band. The hull is opaque where a canopy
   is mostly air, so it should read denser than the near tree — and the new
   outer ring multiplies whatever that looks like by four.
2. **Through a full 80-minute day**, for the specular and the fire light.
   Every frame this project has ever judged was shot at the exact peak of the
   sun's arch — 24 minutes of 80 — and the fire light has never been seen
   against either noon or midnight.
3. **Down the quality ladder.** LOW and MEDIUM are still unlooked-at from
   2026-08-20, and three of these slices add cost to the frame they tier.

## What landed

- **Frame legibility** (`b2677a0`). Coverage-preserving mip chain on the
  needle mask — the canopy had no minification filtering at all, which is a
  shimmer no still frame could show. `ui::TEXT_SHADOW` on every floating HUD
  string. Vitals icons replaced the ASCII `+ ~ *`. Build stamp and net line
  moved behind F4, off by default.
- **The horizon, and the specular** (`fd99b4f`). `OUTER_RADIUS = 5` streams a
  tree-only hull ring past `NEAR_RADIUS`, planted on `far_ground_y` rather
  than `slot.y` (0.630 m apart at worst, measured). `render/fresnel.rs` is the
  one owner of `reflectance`; every material was 8–70× under physical. Seven
  shipped AO maps bound. `ColorGrading` on the camera **at identity**.
- **Fire light** (`58ab851`). There was no dynamic light in the client at all.
  Reads `ClientCore::ovens()`, so no wire change.
- **Blade normals** (`350ef4a`). `Soup::tri_ramp`; a blade's tip stops shading
  as the ground it stands in (measured 0.9978 before).
- **Prop tint** (`d0a3189`). A four-entry mean-1 value pool per high-count
  class, keyed on the cell key.

## What is invented and therefore owed a look

`BLADE_TIP_BLEND = 0.75`, `TINT_SWING = 0.07`, and the three fire-light
numbers. Every other constant in these commits is derived from physics, from a
measurement, or from Bevy's own defaults, and says which in its doc comment.

## Corrections made to the tree's own record

- `NOW.md` §0gc and `clutter.rs` blamed `double_sided` for blades shading like
  dirt. False, checked against Bevy 0.18.1: the negation is inside
  `#ifndef VERTEX_TANGENTS` and `Soup::mesh` generates tangents. **Second**
  false mechanism recorded on that one line.
- `ground_splat.rs`'s "the cause is one constant and it is not in this file"
  was right about the constant and wrong about it being one — every material
  had it. Its measured −0.4% roughness null result **has not been re-measured**
  now that there is energy in the lobe; that needs a GPU.

## The trap this branch paid for

`tests/outer_ring.rs`'s placement assertion **passed under its own mutant** in
its first draft: the tolerance was looser than the effect and it compared
against the wrong quantity. Found by running the mutant, not by reading it —
`CLAUDE.md`'s `lattice.rs` lesson, on schedule. It now searches for the most
discriminating point on the island and asserts both that the tree is where the
far mesh draws and that it is demonstrably *not* where the naive placement
would have put it.
