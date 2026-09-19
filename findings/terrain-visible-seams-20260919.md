# The near ground owns its visible surface

Follow-up to `terrain-shadow-handoff-20260918.md`, on `b859078`.

The forward splat now consumes the same resident-chunk alpha mask as the
far mesh's shadow/depth pass. It sets vertex alpha to one before asking
Bevy for material coverage: `COLOR.a` is the rock weight, so treating it as
opacity would erase distant meadow and sand. The original weights still
drive the splat.

Clipping alone would expose cracks. Each near chunk now carries paired
vertices joining its four edges to the coarse surface. Chunk boundaries
coincide with the far lattice, so the endpoint heights come from actual
coarse edges, with the existing `FAR_DROP`; no guessed skirt depth or blend
distance is introduced. Both windings cover either direction of the height
difference. Internal joins have no indices, preventing a coarse chord above
the ground from drawing a fence through adjacent near chunks.

The worker adds 520 vertices to each 4,225-vertex chunk and reserves the full
index tail. Landing/removing a chunk revisits only itself and four neighbours.
It rewrites a tail only when that chunk's exposed edges change; settled
frames modify no mesh or mask. A full 5×5 ring adds 5,120 triangles along its
perimeter, with no additional entities or draw calls. Mesh modification can
still re-upload a neighbour's vertex data: this is a bounded cost, not a
claim about frame time on hardware.

`tests/terrain_seam.rs` compares joins against separately meshed coarse
edges, checks both height orderings, samples triangle coverage across the
join, verifies unchanged original attributes and checks allocation reuse.
`tests/ground_async.rs` drives loading, walking, a disjoint teleport and
island boundaries, checking actual seam triangles against resident entities
on each streaming transition. Its settled-frame check includes mesh assets.

Three deliberate regressions were rejected: remove the joins, omit the
refresh after a chunk leaves, and put the coarse endpoint at the wrong
height. The existing ground and streaming tests remain intact.

The final shader was also opened against a local shard on seed 20260731:
desktop at `(1558, 644)`, High, 1280×720 under Xvfb/lavapipe, including a
turn to face back across the hillside; browser at `(1500, 600)`, 640×360 in
Chromium/SwiftShader after 180 seconds. Both frames show the island and
neither logs a shader validation error. The browser reported zero page
errors and zero failed game-asset requests. Its existing missing-favicon
and unregistered-AudioWorklet warnings remain; this was a rendering check.
These software-rendered frames do not measure hardware frame time or replace
the operator's appearance review.
