# Terrain shadows follow the resident ring

`NOW.md` §0gfx, measured against `ff4cf97`, seed 20260731.

The coarse island mesh and the detailed near ring both cast shadows. A
0.15 m vertical offset cannot keep an 8 m chord below a curved surface, so
the far mesh can shadow the near mesh from above. Disabling the whole far
caster would also remove terrain shadows outside the near ring.

The client now gives the far material a binary alpha mask in its standard
base-colour slot. Each texel covers one 64 m chunk: opaque until that near
chunk lands, transparent while it is resident, opaque again on despawn.
Bevy's standard prepass consumes this alpha in both shadow and camera views.
The custom forward splat shader replaces base colour and alpha outright,
so this changes shadow/depth coverage, not the visible mesh overlap.

The mask is 32×32 RGBA8, 4 KiB, derived from island size / chunk size. It
uses nearest filtering and no mip chain; UV scaling derives from the same
`UV_PER_M` that the mesh writes. Alpha is binary with a midpoint cutoff,
not a new visual tuning parameter. Updating a texel also marks the far
material modified: image extraction replaces its GPU view and a material
left untouched would keep its old binding (`mipmap::retouch`'s lesson).
Settled frames modify neither asset. Meshes and their streaming budgets
are unchanged.

## Measurement

A temporary client test evaluated `far_ground_y(seed, &haven, x, z)`
(`crates/client/src/render/terrain_mesh.rs:91`) minus
`terrain::ground(seed, &haven, x, z)` (`crates/sim-core/src/terrain.rs:4633`)
over `[0, 2048)` on both axes at a 2 m pitch, excluding points at or below
sea level. Both functions used `terrain::haven(seed)`. The measurement
helper is not a shipped test.

| quantity | measured |
|---|---:|
| land samples | 630,424 |
| far surface above ground | 62,236 (9.8721%) |
| mean positive separation | 0.19997 m |
| largest positive separation | 2.30740 m at (1558, 644) |
| `World::spawn_pos(1)` | (657.69214, 1798.4929) |
| largest separation within 90 m of that spawn | 0.23547 m |

This is the **bilinear estimate**, not a rasterized shadow measurement:
`far_ground_y` documents its difference from the mesh's two triangles.
The whole island was sampled; the spawn is the sim's player-1 spawn, not
the capture harness's overridden position. No frame-time claim follows.

## Regression coverage and limits

`ground_async.rs` runs the real streamer through loading, a one-chunk walk,
a disjoint teleport, and both island edges. Every changed mask is checked
against actual chunk entities, including pending builds and budgeted
teardown. It checks UV lookup, far-material assignment, retained distant
casting, material refresh, and no asset churn while settled.

Three mutations were run and rejected: omit material refresh; never mask a
resident chunk; never restore its caster after teardown.

A native client also connected to a local shard and reached the island under
Xvfb/lavapipe at 640×360 with the High preset. No shader validation errors
were logged. The camera was placed at (1558, 644), the maximum-overlap sample;
the resulting frame still shows the coarse surface covering the view, which
is evidence of the geometry limitation below, not an appearance pass.

A fresh browser build also reached the island in Chromium/SwiftShader at
640×360, viewed after 240 seconds at (1500, 600). No page errors, failed
game-asset requests, or shader validation errors were recorded. The console
reported a missing favicon and an unregistered AudioWorklet; this was a
rendering check, not an audio validation.

The coarse visible surface can still occlude finer ground where it is
higher. Removing that overlap requires a geometry/seam treatment and remains
in §0gfx. This pass does not claim to fix it or to validate appearance on a
hardware GPU.
