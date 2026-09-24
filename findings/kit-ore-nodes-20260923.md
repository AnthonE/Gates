# The ore nodes and a boulder pair, made by the rock kit, 2026-09-23

The operator's two frames said it first — the boulder *"looks like mineable"*
and the node is *"square instead of spherical"* (2026-09-05), then the ore
*"still a square thing"* (2026-09-16). A lineup rendered the way the game seats
them said it again: three nodes that are stacks of quarried blocks, and a pale
dome where the boulder should be, which reads as more of a node than the nodes.
`ci/rock_kit.py` could not make a node (`NOW.md` §0rk: its boulder squash is
never equal in plan), and there was no generator key on the box. So the kit
learned one.

## What landed

A `node` kind: an ico sphere squashed EQUAL in plan (the one thing the other
kinds forbid), gentle lobes, 12–18 **chips** — planes that shear a cap flat,
leaning upward so they cut facets without flattening the footprint — and a
flat cut a third of the way up so the widest ring is the buried one. And three
**looks** over one shared rock: pale speckled granite with quartz veins and
glossy white crystals (stone); grey rock with dark, METALLIC seams that swell
and pinch in a rust-orange halo, baked into the ORM's blue channel (metal);
and a cool grey rock under a crystalline yellow crust packed into its cavities
(sulfur). The shared rock has cracks as warped noise isolines, rims worn paler
and cavities darker off Cycles' `Pointiness`. `rock_a` became a kit boulder and
a fourth, `rock_d`, joined it, so each rock family has two silhouettes.

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
replace were 13.5 MB.**

## What the kit learned

- **A node graph with a cycle is dropped silently.** The first sulfur look read
  its crevice mask off the same AO node that later took the finished colour as
  input. Cycles resolves the loop by discarding links: the bake came back at
  linear luma 0.48 against a ramp of 0.18–0.29, and the calibration then dimmed
  the yellow to reach its target, so every number downstream was plausible. A
  constant-colour control (0.2 in, 0.191 out) is what separated "the bake is
  wrong" from "the graph is wrong". Two AO nodes now.
- **Measure an input before banding it.** Off a node's high-poly, `Pointiness`
  reads p5 0.498 / p50 0.506 / p95 0.549, so a textbook 0.44 cavity threshold
  never fired. The bands are centred on the measurement.
- **A bright metal mirrors the sky.** Seams at an iron-like 0.53 read as blue
  paint flakes under a sky fill; at 0.32 they read as a seam that flashes where
  it catches the sun. That is the glint `ART.md` rule 8 asks of a node.
- **A voronoi edge is a net, not a crack.** Closed cell edges read as a cracked
  egg even through a zone mask; a warped noise isoline is an open curve that
  wanders and ends.
- **Selection still does the work, and one band reads differently here.** On
  the final shape the plan band rejected two of six stone and metal rolls
  (1.215, 1.227). Chart contrast rejected two sulfur rolls (0.070, 0.071): the
  band was written for a generator that lights each UV island from its own
  view, but a kit bake is lit uniformly, so for the kit a chart reject always
  means CONTENT spread unevenly across islands — here a crust twice as bright
  as its rock. Narrowing that to ~1.6× took the same seed to 0.039.

## What remains

- **§LOOK: nobody has seen these in the game.** Every render here is Blender
  Cycles under Khronos PBR Neutral, the nearest built-in to the game's
  TonyMcMapface, on a flat plane; Bevy's metallic response to `fill.rs`'s
  environment map is the unverified part of the metal node's glint.
- **A node on a slope floats on its downhill side**, like every node before it:
  the lift buries 0.13 m and scatter admits slopes to 1.19. The proposed lift
  of 0.3 (`DECISIONS.md` §open, scatter art v1) is the fix and needs the
  fallback blob's height to move with it, which `greybox.rs` holds.
- **The texture is procedural.** It reads as rock at a player's distance and
  cleaner than a photograph up close; a scanned granite detail map blended in
  at the grain scale is the upgrade, and a licence-rail question first.
- **The kit boulders are doughier than the generated pair they pool with.**
  The `formation` look closed most of the surface gap; the rest is shape —
  the `boulder` kind's facets are dissolved at 7°, so its rims are soft where
  `rock_b`'s are broken. The node's chips are the obvious tool to try on it.
