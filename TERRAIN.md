# Gates · TERRAIN.md — the island (v0.1)

> Extends `DESIGN.md` §2/§9 and rides `NETCODE.md`'s laws. No monuments in
> alpha — but the island still has to *play* like a survival map: loot
> routes, buildable flats, chokepoints, and biomes that mean something.
> Knobs marked **(knob)**.

## 0 · The contract

**The whole island is a pure function of the seed.** `h(seed, x, z)` and
every scatter decision derive from integer hashes in `sim-core`, compiled
native (server truth) and wasm (client render + prediction). Nothing about
the terrain is ever networked except *changes to it* — a harvested node, a
felled tree — which ride the chunk-epoch event stream like any structural
fact. A 2 km island costs **zero bytes** to join; the seed is in the join
bundle.

Determinism rules (the same ones that make prediction bit-exact):
restricted float ops only (`+ − × ÷ sqrt min max floor-by-cast`), gradient
directions from a fixed 8-entry table indexed by hash — no trig, no libm.
The hash is splitmix64 on `(seed, cell_x, cell_z, channel)`. CI pins a
golden grid: fixed seed → xxh3 of 64×64 sampled heights + the first 256
scatter results, asserted equal native vs wasm (rides `test_parity_wasm`).

## 1 · The generation pipeline (all in sim-core, per sample or per chunk)

Stages, in order — each cheap, each deterministic:

1. **Continent mask** — radial falloff from island center, domain-warped by
   low-frequency noise so the coastline is bays and headlands, not a
   circle. Below sea level = sea floor (walkable, shallow shelf near
   shore).
2. **Base relief** — 5-octave fBm gradient noise (quintic smoothing,
   hashed table gradients). Amplitude ~90 m **(knob)**.
3. **Domain warp** — one warp pass over the relief lookup; this is the
   single biggest "looks procedural → looks natural" purchase and costs
   two extra noise reads.
4. **Height remap curve** — a fixed LUT that flattens mid-elevations into
   **buildable shelves** and steepens the transitions. This curve *is* the
   game design: it manufactures base spots and the cliffs between them.
   Ridged-noise blend above the treeline fakes an eroded look without
   simulating erosion (hydraulic erosion is a post-alpha toy, not a need).
5. **Masks** (derived, not stored): slope from finite differences → cliff
   mask (slope > ~50° **(knob)**: unclimbable, unbuildable, distinct
   material); beach mask (height within ~2 m of sea level); moisture =
   an independent low-freq noise channel.
6. **Biomes from (height, moisture, slope)** — alpha ships four:
   **beach** (spawn zone, barrels wash up), **meadow** (buildable, sparse
   trees, hemp), **forest** (wood, cover, low visibility), **highland**
   (stone/ore nodes, exposure, weather later). Each is a material blend +
   a scatter table, nothing more — biomes are data.
7. **The coast road** — the monument-less loot route: a ring offset ~40 m
   inland from the coastline, flattened a few meters wide, dirt material,
   **barrel spawn slots along it**. It does what Rust's roads do — pulls
   players out of their bases into a circulation loop where they meet —
   with zero monument art. Junk piles at bay mouths get slightly denser
   slots **(knob)**.
8. **The haven pad** — deterministic placement: score candidate sites on
   the road ring by flatness + coast distance, take the best, carve a
   flat pad with a smooth blend radius. (This is also the monument hook:
   later POIs are exactly "carve pad + exclusion zone + scatter table" —
   the machinery exists in alpha, the art doesn't.)
9. **Scatter pass** — per 8 m cell, one hash draw decides occupant
   (tree / stone node / metal node / sulfur node / bush / rock / barrel
   slot / nothing), plus jittered offset, yaw, and scale from the same
   hash. Densities come from the biome table. Slope and road/haven/water
   masks veto. Result: a deterministic **slot list** both sides can
   enumerate for any chunk.

## 2 · Slots: how living terrain stays cheap

A slot is potential, not state. The scatter defines *where a node can be*;
the **server owns one bit + one timer per slot**: harvested/standing, and
respawn-at-tick. Changes ship as chunk events (`slot_harvested`,
`slot_respawned` — a few bytes), and a chunk's slot bitset rides its epoch
state for late joiners. The client renders every standing slot from local
generation; a harvested node vanishes on the event. Trees fell to stumps
the same way. Respawn: 20–45 min jittered **(knob)**, never inside a
claimed building's privilege volume — no farming your own living room.

This is the same shape `NETCODE.md` §5 uses for buildings, which is the
point: **terrain life is just chunk events over a generated backdrop.**

## 3 · Collision (server truth, client prediction — same code)

- Ground: bilinear height sample under the capsule; walkable up to the
  cliff slope threshold; step-up ≤ 0.6 m; water at sea level slows to a
  swim (alpha swim = slow wade, no diving **(knob)**).
- Rocks and nodes: analytic colliders (sphere/capsule/box per archetype)
  derived from the same slot list — no mesh colliders anywhere.
- Buildings: AABB/oriented boxes per block (`DESIGN.md` §4).
- All of it lives in `sim-core`, so the wasm prediction collides with the
  exact world the server does, including the slot a node just vanished
  from (one in-flight event of skew, max).

## 4 · Rendering (three.js, per chunk)

- **Chunk meshes**: 64 m chunks; LOD0 = 1 m grid (65×65 verts) for the
  ring around the camera, LOD1 = 2 m, LOD2 = 4 m beyond, each ring with a
  **skirt** dropped at the edges (the cheap, correct crack fix — geomorph
  is a later nicety). Heights + normals generated in a **worker** from the
  shared wasm, meshes built off the main thread, uploaded once — a chunk
  is static geometry until a terrain event says otherwise.
- **Shadow casting** (the horizon casts, `DECISIONS.md` §open): both LODs
  cast, and the ground names its `shadowSide` — three casts a FrontSide
  material from its BACK face, which is right for a closed solid and
  culls a heightfield out of the depth pass entirely. The near ring casts
  its own 1 m surface; the far mesh casts everywhere else, through a
  depth material that **discards the near ring's current footprint**, so
  every XZ column of the world has exactly one caster and the two LODs
  never put disagreeing silhouettes of one hillside in one map. The
  **skirt this section wants lives there**: the far caster is sunk a
  fixed 3 m below its own surface, so where the LODs disagree at the seam
  it casts late rather than painting a band along the hole's edge.
- **Material**: one splat shader, four sets (beach/meadow/forest
  rock/highland), blended **in-shader from height, slope, and a noise
  channel recomputed in GLSL** — no splatmap textures, no extra bandwidth,
  and the blend math mirrors the biome function. Cliff mask forces rock.
  Far ring fades into fog + a low-res horizon mesh.
  Shipped (materials v0, `DECISIONS.md` §open) with **no textures at
  all**: the four sets are authored PBR identities (colour + roughness +
  bump strength), their weights ride the geometry as a 4-byte `splat`
  attribute the worker derives from the sim's own (height, moisture,
  slope) — soft ramps centred on `biome()`'s own edges — and one shared
  three-octave value-noise field, sampled in GLSL, breaks the weights up,
  mottles albedo, varies roughness and drives a footprint-faded
  surface-gradient bump. Wetness at the waterline, snow on high rock and
  cliff darkening are causal modifiers on top, each moving colour and
  roughness together. Texture sets are still the art pass; the *system*
  is not waiting on them.
  A **fourth octave — grain** (materials v1) makes the ground read at
  arm's length, where 1.7 m is already too coarse to be texture. It is
  one more tap of the same field at a wavelength the *identity* chooses
  (4 cm of sand, 12 cm of grass), with a per-identity ridge fold and
  contrast, and it drives albedo, bump and roughness like every octave
  above it. Two things about it are the parts worth keeping: it is
  retired by pixel footprint in **cycles per pixel** rather than metres,
  because four wavelengths cannot share one metre threshold — so each
  identity's grain dies at its own distance on one curve — and its fade
  reads the **world** footprint, not the horizontal one, because a pixel
  on a steep face barely moves in XZ while covering metres of surface.
  Grain is **triplanar** (materials v1) and the base maps are
  **biplanar** (materials v4): the existing top tap plus one on the
  vertical plane containing the face's own fall line, whose stretch is
  `1/sin(tilt)` where the top plane's is `1/cos(tilt)` — exact
  complements, so the worst case anywhere is 45 deg at x1.41 where they
  average. The wall turns on at the crossover (`BASE_WALL_ON`, the 45 deg
  where `sin = cos` and the wall becomes the less distorted of the two),
  so level ground pays one tap and renders bit-for-bit what it did
  before. Weights are raised to `BASE_WALL_SHARPNESS` before blending and
  the wall's footprint is `dFdx(position)` projected onto the frame, not
  `dFdx(uv)` — a rotating frame otherwise puts its own turn, times a
  world coordinate, into the mip selector. **Every octave now retires on the WORLD footprint** — the law
  stated in this paragraph since materials v1 was applied to grain alone
  for two passes, and on a 69 deg face the horizontal footprint is 1/2.8
  of the real one, so every other octave stayed at full strength three
  times past its own Nyquist and reached the image as its alias.
- **Scatter rendering**: per chunk, one `InstancedMesh` per archetype
  filled from the slot list (minus harvested), frustum-culled per chunk.
  Trees get two LODs (mesh / billboard cross) **(knob: distances)**; each
  archetype carries an authored PBR response, baked vertex colours and a
  per-instance tint hashed from its own cell, so variation costs no draw
  calls (materials v0).
  Grass: cheap camera-ring patches, purely cosmetic, off on low tier
  **(knob)**.
- **Water**: a single animated plane at sea level with depth-fade alpha
  and shore foam from the beach mask. Nothing simulates.
- **Budgets** (within DESIGN §9's 300 draw calls / 1.5 M tris): terrain
  ≈ 40–60 draw calls and ~250 k tris at LOD; scatter instancing keeps the
  rest. Chunk build ≤ 4 ms in the worker, amortized one chunk per frame.

## 5 · What the island gives the game (the part that has no code)

The reads a survival map must produce, and which stage buys each:

- **"Where do I build?"** — the remap curve's shelves, visible from a
  distance as flat benches. Scarcity of *great* spots (flat + water +
  road-adjacent + node-adjacent) is the land-value engine.
- **"Where do I go?"** — the coast road. Barrels respawn there, so the
  route circulates, so players collide. One ring is enough for 100.
- **"Where's the risk?"** — highlands: the good nodes, no cover,
  silhouetted on ridgelines. Forest: resources + ambush shadow.
- **"Where am I?"** — biome color, coastline shape, and the one or two
  distinctive headlands the warp gives every seed. A client-rendered map
  (drawn from the same wasm heights — free) ships in alpha; no minimap,
  map is a held item you stop to read **(knob)**.

## 6 · Numbers of record (alpha)

| thing | value |
|---|---|
| island | 2,048 × 2,048 m **(knob)**, sea ring beyond |
| height grid | 1 m authoritative; server caches sampled chunks LRU |
| relief amplitude | ~90 m, sea level at 0 |
| chunks | 64 m, aligned with the netcode grid — one grid, everywhere |
| scatter cells | 8 m (≈ 65 k cells; ~8–12 k live slots per seed) |
| biomes | 4 (beach/meadow/forest/highland) |
| roads | 1 coast ring, ~4 m wide |
| monuments | 0 — haven pad only; the pad carver is the future hook |
| node respawn | 20–45 min jittered, privilege-vetoed **(knob)** |

## 7 · Gates

- `test_terrain_golden`: seed → pinned hash of heights + first 256 slots,
  native = wasm = the checked-in value.
- `test_terrain_gameplay`: for 16 random seeds assert invariants — haven
  pad exists and is flat, road ring is closed and walkable, ≥ N buildable
  shelf area, ≥ N slots per biome, spawn ring has ≥ 24 valid spawns. A
  seed that fails is a bug in the generator, not a reroll: wipes must be
  able to trust any seed.
  The spawn-ring half of it has landed early as sim-core
  `world::tests::spawn_ring_lands_on_a_clear_beach`: 32 seeds × 64 joins,
  each spawn asserted beach biome, above the wade line, off a cliff, and
  4 m clear of every scatter slot. The rest waits on the road and the pad.
- Chunk-build time and instancing counts ride the client perf harness.
