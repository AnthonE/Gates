# Gates · RENDER.md — the native client's render path

**Owns**: how the Bevy desktop client draws, in what order it is built, and
the gate that watches it. **Does not own the bar** — `ART.md` is the bar and
outranks every sentence here; when a target in this file and `ART.md`
disagree, `ART.md` wins and the disagreement is the finding. `TERRAIN.md`
still owns worldgen and `CONTENT.md` still owns every number about items.

This file replaces `MIGRATION.md`, which the 2026-08-05 pivot mooted. What
it inherits from it is one rule, stated there and restated in `CLAUDE.md`:
**a render path that lands without its probes ships a client with no visual
gates at all, and that is forbidden outright.** §5 is how that debt gets
paid; §4 says which slice pays which part of it.

**The ordering principle is the operator's, spoken 2026-08-05**: visual work
is judged by *a visibly better picture in reasonable time*, and a pass that
produces tuned constants instead stops early. Every slice below is ordered by
picture-per-hour, not by architectural tidiness, and each one names the
picture it is supposed to buy.

---

## 0 · Where the picture is now — measured, both sides, one estimator

**R0–R6 have landed.** The client connects to an unmodified shard, meshes
`sim_core::terrain`, populates the near ground out of `clutter_fill` **and
`skirt_fill`**, scatters `terrain::scatter`'s occupants, samples the CC0
photograph, lights it with Bevy's Bruneton atmosphere under one owner, hangs a
procedural cloud deck in the sky, resolves with SSAO + SMAA + bloom, draws a
HUD and a viewmodel, and captures a fixed vantage list headless under Xvfb +
lavapipe.

`ci/native_bar.py` reads our captures and `Rust Images/` **in the same run,
through the same estimator** — a bar computed a different way than the frame
it judges is not a bar. Medians over the six vantages:

| statistic | first native frame | before this pass | now | reference |
|---|---|---|---|---|
| whole-frame p10 | 41.9 | 64.9 | 58.6 | 41.0 |
| whole-frame p50 | 61.0 | 85.3 | **90.2** | 91.4 |
| whole-frame p90 | 110.9 | 144.7 | **155.7** | 170.2 |
| sky band mean | 97.3 | 129.5 | 136.3 | 128.4 |
| near band mean | 54.8 | 79.5 | **79.8** | 80.5 |
| near-band saturation | 42.0% | 32.1% | **32.9%** | 33.2% |
| **near neighbour contrast** | 1.55 | 2.44 | **6.25** | 5.40 |
| chroma per unit luma | — | — | **0.163** | 0.252 |

The last two rows are the ones that matter and they have to be read together.
`ART.md` §3's contrast row is the statistic six browser passes never moved off
**0.26**; it is 6.25 now, past the reference. And §7 warns that contrast alone
cannot tell detail from aliasing — the test that can is *direction*: the
high-frequency residual resolved along the local mean colour (real relief)
versus orthogonal to it (the hue changed between neighbouring pixels). Ours
sits **below** the reference's own ratio, so the frame is carrying texture,
not noise. Both were needed; either alone would have been a number without
evidence.

Two gaps remain, both named by the table:

- **p90 155.7 against 170.2.** Closing, and the cloud deck is why (143.6 →
  155.7). What is left is cloud *form*: this deck reads as high stratus where
  `ART.md` §1 asks for cumulus with lit tops and grey bases.
- **p10 58.6 against 41.0 — our darks are still not dark.** SSAO moved it
  (64.9 → 58.6) exactly as `ART.md` §4 predicts, by removing ambient only
  where geometry occludes. The rest is the shape of the fill itself: a uniform
  ambient term buys rule 3's 0.30 floor at the price of the bottom of the
  range, and a hemisphere (sky half cool, earth half warm) is what gets both.
  Bevy's `AmbientLight` is uniform, so that is a second light or a shader.

## 1 · The rule the path hangs on: Bevy draws, it does not decide

`sim-core` keeps the walls (purity, zero-alloc tick, bounded everything,
replay determinism). `ClientCore` keeps prediction and interpolation. The ECS
**reads those and writes transforms and materials.** Gameplay state in a Bevy
component retires the determinism walls with nothing in CI to notice, which
is why this is a rule and not a preference.

Concretely, three directions of dependency:

| allowed | why |
|---|---|
| ECS reads `ClientCore` (`predict.render_position`, `interp.sample`, `PieceSet`, `DeploySet`, `BagSet`, `HarvestedSet`) | the render contract, and it is already what `draw_bodies` does |
| ECS calls pure `sim_core::terrain::*` (`height`, `slope`, `moisture`, `splat`, `scatter`, `clutter_fill`, `haven`) | worldgen is a pure function of the seed and both sides already agree on it; the browser client does exactly this through the wasm bridge |
| ECS → sim: **only** `ClientCore::set_input` | one door, the same one the browser uses |

**Enforcement, honestly labelled.** The dependency direction is mechanical and
gated: `crates/client` must never depend on `server`, and `cargo tree` says so
in one line — add it to the code tier. The *no-gameplay-state* half is a review
wall with no gate today, exactly like the `EV_*` payload comments in `world.rs`
that `CLAUDE.md`'s trap list names. The cheapest real check is a shape
assertion rather than a grep: **a headless run must produce a byte-identical
state hash with the renderer attached and detached, on the same seed and WAL.**
If the ECS ever decides anything, that equality breaks. It is not built; it is
the right gate, and it is listed in §5 as R-G4.

---

## 2 · What we do NOT re-derive — paid-for lessons, mapped

Every item here was learned at cost in `web/` or in research. They are
arithmetic or discipline, so they cross the language boundary intact. A slice
that rediscovers one of these has failed, not learned.

- **Tonemap, sky, exposure and fog are ONE owner.** Measured elsewhere: three
  parallel passes over the coupled set worsened defects 60→66; one sequential
  owner cut them to 26. In Bevy that set is `Atmosphere` + `DirectionalLight`
  + `Tonemapping` + `Exposure`/`AutoExposure` + `DistanceFog`/volumetrics.
  **One module, one file, one lane, one iteration.** Nothing else in the tree
  creates a light or sets an exposure (`ART.md` rule 5).
- **A pixel statistic cannot see whether the frame is a picture of anything.**
  On 2026-08-05 `ci/vantages.mjs` passed all 36 checks on a beige smear with
  no sky, no horizon and no object in it. Every assertion in it was contrast,
  chroma or luma neutrality, and a featureless wash satisfies all three.
  **Structural assertions run BEFORE any statistic** (§5), and a ranked visual
  gap is not evidence about shading until someone has looked at the frame.
- **Median fps hides shader-compile stalls.** Lazy pipeline specialization is
  Bevy's version of the lazy WebGL program link that cost 700 ms+ worst-frames
  while the benchmark read 90 fps. The gate is a **COUNT** — no new pipelines
  created after the world is up — never a frame-time threshold.
- **A gate that waits on a clock is not a gate on this box.** Assert on
  observable state (`inWorld`, `snapshots > n`, frames rendered), never on
  elapsed milliseconds. Under lavapipe this is not a nicety: a CPU rasterizer
  makes every wall-clock budget a lie.
- **Biplanar projection has two rules and both were shipped wrong here first.**
  (a) Differentiate the *position* and project it onto the frame — never
  `dFdx(uv)` of a per-fragment frame, which picks up the frame's rotation times
  a world coordinate and mips several levels too coarse. (b) The blend needs an
  exponent: linear weights hand a third of the sample to the worse plane, and
  `pow(w, 8)` hands it 0.05%. Both are Quilez's. They are WGSL now, unchanged.
- **A modifier multiplies; it never replaces.** `mix(albedo, SNOW_COLOR, 1)`
  reverted whole hillsides to a flat value with every amplitude gate green,
  because none of them was measured up there. Wetness, snow, cliff darkening,
  wear: multiply the surface's own mean-1 luminance field, or carry the detail
  through explicitly. There is no third option.
- **A sourced map's colour deviation may not be stretched more than ×1 by the
  correction that places its mean** — the ×13.45-on-blue rainbow speckle. The
  estimator that scores this already exists (`materials.js` `baseFacts`), and a
  WGSL port must be scored with the *shipped* estimator, never a
  re-implementation that might disagree.
- **Capture is bit-identical or it is not evidence**: fresh process per shot,
  engine clock rather than wall clock, fixed seed, fixed spawn, fixed vantage.
- **The near-ground contrast target is 6.3 luma/px and ours was 0.26** — a 24×
  gap that no shader has ever moved. Rules 1 and 4 of `ART.md` (albedo variation
  at two scales; no bare ground inside 15 m) are the two mechanisms that close
  it, and the second one is geometry.

---

## 3 · What Bevy gives that three.js did not, and the one it unblocks

Verified against Bevy 0.18.1's own example set. This is a *shopping list of
things not to hand-build*, and the reason the pivot can move faster than the
six visual passes it follows.

| we hand-built, badly or not at all | Bevy 0.18 ships |
|---|---|
| a gradient sky shader with a hand-fitted fog seam (`scene.js`, ~200 lines of comment about one horizon step) | procedural atmosphere that **also lights the scene** — `examples/3d/atmosphere.rs`; 0.18 adds `ScatteringMedium` so haze is authored, not fitted |
| fog, aerial perspective, and the seam between fog and sky, tuned against each other by hand | `atmospheric_fog`, `volumetric_fog`, `fog_volumes`, `scrolling_fog` |
| a shadow clipmap with light-space texel snapping and texel-scaled normal bias, 899 lines | cascaded shadow maps, `pcss` (percentage-closer soft shadows), `shadow_biases` |
| nothing — occlusion at the medium scale is fetched-but-unread (`assets/textures/*_ao.jpg`) | `ssao` for the large scale; the AO maps supply the medium scale, per `ART.md` §4 |
| nothing — no AA beyond the browser's | `anti_aliasing` (TAA/FXAA/MSAA) |
| a hand-metered exposure constant | `auto_exposure`, and `tonemapping` with several transfers to measure rather than argue about |
| an instancing pool per archetype, hand-managed, 4096/archetype | automatic batching for same-mesh/same-material (`automatic_instancing`), and `custom_shader_instancing` when that is not enough |
| a terrain worker + wasm views + a detached-buffer class of bug | **direct calls into `sim_core::terrain`** on `AsyncComputeTaskPool` — no worker, no serialization, no bridge |
| a browser capture harness with 43 `readPixels` sites | `headless_renderer` (render to texture, read back on CPU, no window at all) and `screenshot` |

**The unblock, and it is the headline finding of this plan.** `web/src/scene.js`
records, with a measured table, that the sun could not rise from 0.36 rad
(20.6°) to the 30–40° band `ART.md` §1 asks for: the ground's entire relief was
a *bump field*, and a normal perturbation δ changes `N·L` on flat ground by
`cot(elevation)·δ`, so raising the sun to 45° would have cost 96% of the
frame's relief (12.81% of the frame moved at cot 2.66, 0.47% at cot 1.00). The
constant was blocked "on the ground's structure moving from bump into albedo."

**The native path moves it out of bump by construction**: real normal maps on
a real mesh, plus a *populated* ground (grass geometry, §4 R3) whose relief is
occlusion and silhouette rather than a perturbed flat normal. So the midday
sun `ART.md` asks for is available on this path from R2 onward, and the blocked
row's exit condition is met by building the world rather than by tuning the
rig. **Do not carry `SUN_ELEVATION = 0.36` across.** It is an artifact of a
renderer we are replacing.

---

## 4 · The slices, in order, each with the picture it buys

Each slice is one iteration: branch → build → gates green → merge. Each names
its probe, and **no slice merges without one** (§5). Slices are ordered so the
frame gets visibly better every time, not so the architecture gets tidy.

### R0 · Input, and the capture harness — **LANDED**

The two things every later slice needs and neither is art.

- Keyboard/mouse → `ClientCore::set_input(buttons, yaw, pitch, move_x,
  move_z, sel)`. First-person camera at eye height 1.6 m (the cosmetic already
  registered), yaw/pitch owned by the renderer and quantized on the wire —
  **quantize both sides or prediction drifts by rounding.**
- `--capture <dir>` mode on the `gates` binary: connect, wait on *observable
  state* (welcome received, `snapshots > n`, world meshed), teleport the camera
  to each named vantage, render N frames, read back, write PNG, exit non-zero
  on any failure. Uses Bevy's headless render-to-texture path so it needs
  neither a window nor `xwd` — which this container does not have.
- **Probe (R-G0)**: `cargo clippy -p client --features render --all-targets
  -D warnings` and a `--capture` run that writes a non-empty PNG. That is the
  gate that stops the render path from silently not compiling.
- **Where the world is built, since 2026-08-06.** The rig, the water, the HUD
  and the sky hang off `OnEnter(Screen::Loading)`, not `InWorld` — the loading
  screen is the state that owns the ~25 frames of ring filling, and a capture
  run passes through it like every other connected start. Two consequences for
  anything that schedules against this: the streamers and `place_eye` run under
  `render::world_running` (the world exists) rather than under one state, and
  `input::gather` is the only system still gated on `InWorld` alone, because it
  is the only one that writes what the sim reads. The overlay is opaque and the
  3D pass runs behind it, which is where lazy pipeline specialization should be
  paying its cost — see the prewarm trap below.

*Picture bought*: none. It is the only slice allowed to say that, and it is
first because everything after it is unmeasurable without it.

### R1 · The island — mesh `sim_core::terrain` — **LANDED**

Not design; meshing. The heightfield is a pure function of the seed and both
sides already agree on it.

- Chunked heightfield around the camera, built on `AsyncComputeTaskPool`, one
  build in flight and one teardown per frame (**stream-in AND stream-out are
  budgeted — the teardown spike is the half everyone forgets**). `web/src`'s
  shipped shape is the starting point: a near ring of 64 m chunks at 1 m
  resolution, a far mesh at 8 m dropped 0.15 m to hide the seam.
- Vertex attributes carry what the material needs: normal from the analytic
  gradient (not from the triangulation — `ci/bump_basis.mjs` holds that
  arithmetic and it is language-agnostic), and `splat()`'s four identity
  weights.
- One translucent plane at sea level for water. It does not simulate
  (`TERRAIN.md` §4) and it is not this slice's subject.
- **Probe (R-G1)**: structural. From the spawn vantage, the bottom third of
  the frame is ground and not sky; the camera's feet are within ε of
  `terrain::height(seed, x, z)`; the horizon line exists and sky is above it.

*Picture bought*: a cube in a void becomes a person standing on an island.
This is the single largest step in the document, and it was.

### R2 · The light rig v0 — one owner, one file — **LANDED**

Atmosphere, sun, exposure, tonemap, fog. **One module. One iteration. One
lane.** (§2, first bullet — this is the rule that was measured, not a style.)

- `Atmosphere` on the camera, sun as a `DirectionalLight` in the 30–40° band
  (§3's unblock), shadows on with cascades, `pcss` if it is cheap enough on
  the gate box.
- Tonemapper chosen by *measurement* against `ci/reference_bar.mjs`'s frames,
  not by name. The paid-for datum: a transfer with a quadratic toe over the
  shaded range (Khronos PBR Neutral, `x - 6.25x²` under 0.08) squares the
  shadows and is why a shaded face arrived at 8/255. `ART.md` rule 3's floor —
  no shaded face below 0.30 of its lit face — is a property of the transfer and
  the fill together, so it is decided here or nowhere.
- Ambient is not one number: a hemisphere lands `mix(ground, sky, 0.5+0.5·N·y)`,
  so the **sky half lights up-facing ground in shadow** and the **ground half
  lights every down-facing prop face**. Moving them together is the mistake
  that cost a pass; they are two knobs.
- **Probe (R-G2)**: `ART.md` rule 3 as an assertion (shaded/lit ratio ≥ 0.30 on
  a probe object), plus sky-brighter-than-ground, plus the far third lighter,
  bluer and less saturated than the near third.

*Picture bought*: outdoors instead of a viewport. Cheap, because Bevy owns the
hard parts now.

**Three things this slice measured, all of them cheap to have got wrong:**

  1. **Rule 3's floor is arithmetic and the first cut missed it by 10×.** A
     35° sun at 100,000 lux puts ~57,000 on flat ground, so the fill that
     reaches 0.30 of it is ~17,000 — not the 3,500 that shipped first. The
     tell was a boulder with a NAVY shaded face.
  2. **Exposure was most of a stop under the bar**, measured rather than
     eyeballed: p50 61 against 91. `Exposure::SUNLIGHT` minus 0.8 stop.
  3. **Air density is not a sky knob**, and this is `ART.md` rule 5 catching a
     hand in the till. Thickening the medium 6× to buy aerial perspective put
     the sky mean exactly on the bar (120 → 135 vs 128) and simultaneously
     dropped the ground from 84 to 63 and pushed saturation 31% → 40%: the
     medium extinguishes the sun on its way DOWN as well as scattering it
     sideways, so midday became a hazy sunset. 1.6× is what the coupled set
     tolerates. Nothing but a measurement catches this — the sky looked better
     the whole time.

**Do not meter the tonal bar here.** p10/p50/p90 against `Rust Images/` is R5's
job, after there is content in the frame. A bar measured on an empty world is
the beige-smear trap with a different file name.

### R3 · The population — the largest structural gap in `ART.md` — **LANDED**

"The ground is not a surface, it is a population." No shader fixes this and
six passes proved it.

- **Scatter** (`terrain::scatter`, 7 occupant kinds): pines, stumps, rocks,
  the haven and waystation structures. Instanced, one mesh+material per
  archetype so Bevy's automatic batching collapses them to a draw each.
  `ART.md` rule 6: **silhouette before surface** — a smooth cone is wrong at
  any texture budget; the pine is tall, thin and ragged-edged, and
  `ci/pine_shape.mjs` already holds that shape as arithmetic.
- **Clutter** (`terrain::clutter_fill`, 721 elements/tile over a 5×5 tile
  ring): tufts, pebbles, shards, twigs. This is `ART.md` rule 4 — no bare
  ground over ~3 m² inside 15 m — and it is already *placed* natively and
  gated (`crates/sim-core/tests/clutter.rs` measures the largest bare disc).
  It has never been *drawn* by this client.
- Contact: nothing sits ON the ground, everything sits IN it (rule 2) — an AO
  darkening, a dirt skirt, or scatter crowding the base. In the reference the
  boulder's meeting line with the turf is invisible.
- Wind: one per-vertex cantilever weight rooted at the trunk base, phase from
  the instance's world position so a gust crosses the forest, two sine octaves.
  The design is `SeedThree`'s, credited in `CLAUDE.md`, and it is a vertex
  shader either way.
- **Probe (R-G3)**: structural first — N distinct instances framed, largest
  bare disc inside 15 m under the sim's own bound — then the statistic that
  matters: near-band neighbour contrast against `ART.md` §3's 6.3 luma/px.

*Picture bought*: the difference between a terrain demo and a place, and the
24× contrast gap moved here exactly as predicted — 0.26 → 2.44 (§0).

**What the first population capture got wrong, both structural:** three blades
to a tuft is a sprig, not turf, at `clutter_fill`'s ~2.4 elements per square
metre (now seven, wider, shorter); and a blade's two triangles wind opposite
ways, so blending only 0.72 toward vertical left a facet normal in and every
tuft had one lit blade and one black one. Blades are also `NotShadowCaster`:
two triangles a few centimetres wide against a cascade sized for 200 m is not
a shadow, it is acne, and the first capture was full of it.

### R4 · Materials — the photograph, on the surface — **LANDED (single-map)**

The CC0 set in `assets/textures/` already exists, is manifested, and its
*selection* was already measured (gain span, albedo sd, anisotropy). None of
that work is lost; it moves to WGSL.

- Terrain: 4-way splat blend from the vertex weights, biplanar projection with
  §2's two rules, per-identity tint bounded by the deviation rule, macro
  break-up at 0.5–1 m and near-field grain under 5 cm (rule 1).
- AO maps become `indirectDiffuse *= ao` — indirect only, medium scale — and
  `min(bakedAO, ssAO)` where SSAO also runs, never a sum or a product. Micro
  occlusion stays baked in albedo and *does* apply to direct light. Specular
  occlusion is its own term. (`ART.md` §4, from Frostbite §4.10.3.)
- Value separation is the point, not hue: granite ~2× turf's value; grass
  shadows go cool (hue 70° → 170°).
- **Probe (R-G4)**: the shipped estimators, ported and cross-checked against
  the JS ones on the same input — along-colour vs orthogonal chroma residual in
  the 0.077–0.193 band, `ALBEDO_LUMA_BAND` respected, and the biplanar identity
  `ci/prop_photo.mjs` already asserts in closed form.

### R5 · The light rig, metered — the tonal bar

Now that the frame has content, meter it. `ci/reference_bar.mjs` reads the six
outdoor-daylight reference frames with the same code path that reads ours;
port that discipline, not the numbers. Targets are `ART.md` §3's.

### R6 · HUD and viewmodel — **LANDED**

`ART.md` §6 and §8: a frame with no viewmodel and no HUD reads as a
flythrough, and the blind reader had named it on every capture so far. Landed
as the reference's shape — bottom-centre hotbar with the selected cell lit,
right-side vitals stack, a held item entering from the lower right. Every
number on it is the server's (`hp`/`hp_max`, `food`, `water` off `ClientCore`),
and the zero-max rule `core.rs` states is honoured: a shard whose content
disarms combat draws no bar rather than an empty one.

Still owed: item icons in the cells (the hotbar knows only which cell is
selected, not what is in it), status chips (`WET 36%`), and a viewmodel that
is the held item rather than a stand-in.

### R8 · Clouds — **LANDED**

`ART.md` §4 states it outright: a cloudless gradient cannot reach the
reference's spread. Two cheaper answers were checked against the arithmetic
first and **both are impossible, not merely weak**:

  · **The sun disk cannot move a p90.** `SunDisk::EARTH` is 9.31 mrad; at 75°
    over 1280 px that is ~9 px across, about 65 px, 0.007% of the frame. A p90
    needs ~92,000 pixels. Raising its intensity — which this pass did before
    checking — is measuring the wrong thing.
  · **Bloom cannot add light.** `Bloom::NATURAL` is `EnergyConserving`, whose
    composite is `{src: Constant, dst: OneMinusConstant}` — a lerp between the
    scene and its blur. It redistributes energy. Only `Additive` adds, and the
    docs warn that a non-default prefilter without it is physically wrong.

**A top stop must come from AREA.** The deck is a procedural cloud cubemap
handed to `Skybox`, generated at boot from the world seed — no asset, no
shader, no download. The reason it is a `Skybox` and not a dome is ownership:
`Skybox` draws at the end of `MainOpaquePass`, which is *before*
`AtmosphereNode::RenderSky`, so **the atmosphere composites the clouds itself**
as `dst = inscattering + transmittance·dst`. `ART.md` rule 5 holds by
construction. An `AlphaMode::Blend` dome draws in `MainTransparentPass`, after
the sky is resolved, so it would need its own `DistanceFog` to sit right — a
second owner of haze, which is the coupled-lighting failure already paid for
once. `FullscreenMaterial` binds no depth and no view uniform, so it cannot
tell sky from geometry. And `AtmosphereNode` is private in `bevy_pbr`, so no
custom node can be ordered against it at all.

Still owed: the deck reads as high stratus, not cumulus. Vertical structure
and a real light march are the difference.

### R7 · What is deliberately not in this plan

Water that simulates, billboard/impostor LOD for distant trees, meshlets,
Solari (hardware raytracing — the gate box has no GPU at all), decals, and
any texture compression work. Each is a real want; none of them is the reason
the current frame does not look like the reference. `TERRAIN.md` §4 holds the
impostor design and `CLAUDE.md` credits its source.

---

## 5 · The native visual gate

The pivot's real debt. It is built **incrementally with the slices** rather
than as one item at the end — that is how the rule "no render path without its
probes" is actually satisfied.

**The harness verifies its own output now, and that was not free.**
`save_to_disk` handles an IO error with `error!("Cannot save screenshot, IO
error: {e}")` and then returns — there is no error path back to the caller —
so the capture counted screenshot entities SPAWNED, not files landed. Two
layers of silent success were found by mutating the run (pre-creating a
directory where a PNG should go):

  1. the finish check accepted a non-empty `metadata()`, and a directory
     reports non-zero length, so it needed `is_file()` as well;
  2. and even once it printed "1 of 6 vantages did not reach disk", the
     process **exited 0**, because `App::run()` returns an `AppExit` that
     implements `Termination` and `main` was discarding it.

Both are fixed and both directions are measured: exit 1 with a frame missing,
exit 0 with six verified on disk. A gate reading that exit code would have
called the first one a pass.

**Landed: the harness and the measurement. NOT landed: the assertions.**
`gates --capture <dir>` settles on observable state (all three rings full —
25 chunks, 25 scatter parents, 25 clutter tiles, reported at the frame it
happens), warms 30 frames, shoots six vantages and exits; `ci/native_bar.py`
reads those captures and `Rust Images/` through one estimator. What neither
does yet is FAIL. Nothing in `ci/gates.sh` runs either, and until it does the
render path's coverage is `cargo clippy -p client --features render` and a
human looking at a PNG. That is the top of §8's list, it is the pivot's stated
debt, and calling it anything other than open would be the "pass it didn't
earn" this repo names as its worst bug class.

Also landed, and it is the cheapest thing in the document: **the render
feature now compiles under a lint gate.** `cargo clippy -p client --features
render --all-targets -- -D warnings` is green and it caught three real
findings on its first run — before it, cargo skipped `gates.rs` entirely and
a bin containing `this is not rust at all !!!` would have passed.

**The tier.** `ci/gates.sh` grows a native renderer tier beside the browser
one, scheduled the same way `renderer_touched` schedules today's: a diff
touching `crates/client/**` or `assets/**` runs it. Bevy is several hundred
crates and minutes of build; that cost belongs in the tier that owns it, never
in the ~106 s code tier. Use `bevy/dynamic_linking` for local iteration.

**The capture protocol**, and every clause is a trap already paid for:

- Fresh process per shot. Fixed seed, fixed dev spawn, one shard per vantage,
  **one live renderer at a time** (two was the browser tier's whole problem).
- Settle on observable state — welcome received, `snapshots > n`, chunk queue
  drained — and never on elapsed time. Budget in **frames**, not milliseconds.
- Render off-screen and read back (Bevy's headless renderer path). No Xvfb, no
  `xwd`, no window server dependency.
- A missing Vulkan ICD is a **loud failure**, not a skip: this container has
  `libvulkan` and no `icd.d` entry, so the gate would find no adapter, and
  `CLAUDE.md`'s rule for exactly this class is that a wall which cannot run is
  not a wall — install `mesa-vulkan-drivers` (lavapipe) rather than skipping.
- Vantages, from `ci/vantages.mjs`'s hard-won list: not one frame from one
  spawn at one bearing on one seed. A design view, a near/detail view, a
  far/silhouette view, a steep face, above the snow line, the waterline, and a
  second seed. Three defects shipped green through the beach's blind spot.

**The assertion order is the gate's whole design.**

1. **Structural, first, always.** Sky occupies the top band and ground the
   bottom; a horizon exists between them; ≥ N distinct objects are framed
   (count connected components above a size floor, or count draw batches — the
   point is *is this a picture of anything*); the camera's feet are on the
   terrain. **A beige smear must fail here before any statistic is read.**
2. **Then `ART.md`'s checklist**, one assertion per §8 bullet, in that file's
   own units.
3. **Then the counts**: pipelines created after the world is up (must be 0),
   draw calls, triangles against `DESIGN.md` §9's 1.5 M.

**R-G4, the boundary gate** (§1): a headless run's state hashes must be
byte-identical with and without the renderer attached, same seed, same WAL. It
is the only mechanical answer to "did Bevy start deciding," and it is worth
more than the four pixel assertions above it.

**What this does NOT do** is claim the browser gates' result. `web/` keeps its
gates until the native client can do what it does; a run that skipped a tier
prints its own final line and never "ALL GATES GREEN" — the existing script's
discipline, extended, not loosened.

---

## 6 · Budgets, and where each number comes from

**Three of these were chosen for a browser and are inherited, not re-derived.**
`DESIGN.md` §9 now says which is which; the short version is that the frame
target is a hardware floor and survives, while the triangle, draw-call and
payload ceilings were WebGL and download shaped. A native measurement that
exceeds one of those three is **evidence about the budget** and not
automatically a defect — and the fix is a proposal in `DECISIONS.md` §open,
never a number quietly edited into this table.

| budget | value | source |
|---|---|---|
| triangles | < 1.5 M | `DESIGN.md` §9 — **browser-era, not re-derived** |
| draw calls | < 300 | `DESIGN.md` §9 — **browser-era, not re-derived** |
| frame | 60 fps on a mid laptop iGPU | `DESIGN.md` §9 — survives the move; **measured on a GPU, never on the gate box** |
| texture payload | < 12 MB before compression | `ART.md` §7 — **retired**: it was a first-visit download, and a depot install is not one |
| clutter ring | 5×5 tiles of 16 m, 721 elements/tile peak | `sim-core::terrain`, and it is frame-budget-bound, not design-bound |
| eye height | 1.6 m | `DECISIONS.md` §open, client cosmetics |

The first thing to actually press on the triangle ceiling is the generated
conifer: a full 328-tree scatter ring at 5.9 k tris a tree is 1.9 M.
`crates/client/tests/tree.rs` asserts the affordable ~80 m band and *prints*
the ring rather than asserting it, precisely because 1.5 M is the number this
table is unsure of. The billboard LOD (`TERRAIN.md` §4) closes it either way;
which of the two is the real fix is a measurement nobody has taken.

Every constant this path invents beyond these is **PROPOSED** and goes to
`DECISIONS.md` §open in the commit that ships it — as a `` `NAME = value` ``
declaration, which `ci/knob_registry.mjs` then holds equal to the code. Not
before: the registry gate fails on a declaration that resolves to no constant,
and correctly so.

---

## 7 · Definition of done for the path

The frame this is trying to produce, and it is `ART.md` §8 verbatim — a
capture passes only if all of it holds:

- materials read as distinct substances, separated by **value**, not only hue;
- visible contact shadowing or crowding where every object meets the ground;
- colour and value variation within each surface at both scales, with
  near-ground neighbour contrast on the way to 6.3;
- no unlit face below 0.30 of its lit face;
- the ground inside 15 m populated, not bare;
- the sky the brightest thing in the frame, with structure in it;
- the far third lighter, bluer and less saturated than the near third;
- nothing reading as procedural — no tiling, no uniform spacing, no repeated
  identical instances;
- evidence a person is playing: viewmodel, HUD, or both.

R1 through R6 exist to satisfy that list. When a capture passes it, the
browser client can start being deleted (`NOW.md` §1), and not one slice
earlier.

---

## 8 · What is next, in the order the measurements rank it

1. **The gate asserts.** Structural claims first, then `ART.md` §8's
   checklist, then the counts. Wire `--capture` + `native_bar` into a native
   renderer tier in `ci/gates.sh`, scheduled off a `crates/client/**` or
   `assets/**` diff. Three things are known about how to write it, from a
   probe that calibrated candidates in numpy against the six reference frames
   and seven synthetic washes:
   - **No single statistic separates a picture from a wash.** Edge density is
     defeated by σ6 noise, block-mean spread by a pure gradient, coarse
     structure by a blur. The structural tier has to be a SET whose members
     fail on complementary washes; at fine/mid/coarse detail the references
     score 3.88–8.10 / 7.11–15.44 / 28.3–58.1 while every wash fails at least
     two with 3–20× margin.
   - **The horizon row is arithmetic, not a heuristic.** `PerspectiveProjection
     ::fov` is VERTICAL, so at 75° over 720 rows the horizon sits at row 289
     for the four −0.15 vantages, off the top for `near`, and row 531 for
     `sky`. The expectation table is keyed on the vantage label.
   - **Three counts cannot ride in a PNG** — pipelines created after the world
     is up, draw batches, triangles — so the client has to print them. Note
     `RenderDiagnosticsPlugin` works on this box but half its output is wall
     clock and must never be asserted.

2. **Harvested state — LANDED and gated.** A felled tree is a stump, a mined
   node or a smashed barrel disappears, and both reverse on respawn.
   `crates/client/tests/fell.rs` is the gate: five assertions, headless, no
   GPU and no socket, driven through a predicate so the transport is not in
   the way. Eight mutants were run against it and the ones that matter go red.

   Three things it settled that the next draw path inherits:
   - **Poll the authority, never the change feed.** `on_stream` zeroes its
     change slice at the top of every call and `Session::pump` drains every
     queued message before a frame runs, so a frame that received two messages
     sees only the last one's changes. `push_change` also drops silently past
     64 entries. `HarvestedSet::contains` cannot miss an edge.
   - **The join seed is drip-fed** — ≤256 entries scanned and ≤64 cells per
     tick — so a node is spawned STANDING and corrected several ticks later.
     The first convergence after join must be a hard swap, never an animation,
     or a new player watches the forest collapse around them.
   - **State the renderer keeps must be absolute, not accumulated.** The first
     cut moved the stump with `y += lift` / `y -= lift`, which is right only
     while every transition is observed; anything that re-seeds an entity
     without resetting the flag drifts it 0.17 m per missed pair, invisibly,
     because each single step looks correct.

   Still owed: the fall animation. The browser's is 33 ticks of rotation then
   a 60-tick sink, on `core.clock.client_tick` and NOT on `Time`, with the
   bearing from `fellBearing`'s own hash so two clients agree which way a tree
   went down, and the tilt PREMULTIPLIED onto the instance yaw.

3. **What is still not drawn, with the spec to draw it.** Pieces, deployables
   and bags are mirrored in `ClientCore` and have no draw path. The arithmetic,
   from the sim rather than from the browser:

   - **Pieces.** `PieceRec { cx, cz, level, loc, row }` — `hp`/`uh` are
     sim-only and always 0 on the client. Kind and tier come from
     `piece_defs.pieces[row]`, and **that read must be gated on
     `piece_defs_have`**: an undripped row is `INERT`, whose shape is
     `SHAPE_FOUNDATION`, so an ungated read draws a foundation slab in mid-air
     where a wall belongs. Base height is resampled locally —
     `terrain::height(seed, cx*3+1.5, cz*3+1.5) + PIECE_LIFT_M + level*3` —
     at the CELL CENTRE for every `loc`, including edge pieces, so the two
     cells an edge adjoins cannot disagree. A plane's slab hangs BELOW its
     walk surface (centre at `base_y - 0.15`, top at `base_y`); getting that
     sign wrong sinks the player into every floor. A doorway's opening is
     1.2 m × 2.1 m because `collide::edge_hit` blocks exactly `t` in
     `[0, 0.9]` and `[2.1, 3.0]` — draw it elsewhere and the frame lies about
     where a player can walk. Reconcile the whole `entries()` slice keyed on
     `(cx, cz, level, loc)`: order is not stable (removal swap-removes), and
     an upgrade re-rows an address with no new message kind, so comparing set
     membership alone keeps drawing a wood wall that became stone.
   - **The door leaf is a deployable, not a piece.** `PieceSet` never carries
     open/closed; the doorway piece draws the same either way.
   - **Deployables and bags** carry their own sync and their own removal
     events; a bag's position is quantized on the wire and a bag is how a
     player recovers a death, so its dequantization is not cosmetic.

4. **The sim state nothing draws.** A probe compared every public `ClientCore`
   surface against what `render/` calls, and the native client draws **nothing
   from the sim except remote body capsules**: no `pieces` (built bases), no
   `deploys` (doors, boxes, campfires), no `bags` (death backpacks), and no
   `harvested` — so **a felled tree never disappears and gathering has no
   visible effect at all.** That is a gameplay-visible gap, not a cosmetic
   one, and it outranks the remaining polish. The one worldgen gap left with
   it: `terrain::road_band` is never drawn, so the coast ring exists only as a
   side-effect inside `clutter_kind_at` — visible in the 40 m clutter ring and
   invisible on the far mesh, which is the range it would actually read at.
2. **Clouds.** The p90 gap is 25 luma and `ART.md` §4 says where it comes
   from. Bevy has no cloud layer; this is a real slice, not a knob.
3. **R4, the near-field grain.** Contrast 2.44 → 5.40 needs the photograph on
   the ground, which is the CC0 set already in `assets/textures/` plus the
   biplanar rules §2 carries.
4. **A hemisphere fill.** Rule 3's floor and the p10 both, instead of one at
   the other's expense (§0).
5. **Per-instance tint.** `ART.md` rule 7 — the forest is four meshes at many
   yaws and scales, which is variation but not colour variation.

---

## 9 · Open, and deliberately unanswered here

- **Which tonemapper.** Measured in R2, argued nowhere.
- **Whether Bevy's atmosphere runs usefully on lavapipe.** It is compute-shader
  driven; the gate box is a CPU rasterizer. R0 answers it, and if the answer is
  no, the gate's light rig runs at a reduced tier with the structural claims
  intact — never with the assertions dropped.
- **Instancing route for clutter**: automatic batching until measured
  insufficient, then a custom instanced pipeline. Measure before writing the
  second one.
- **Bevy's default feature set** pulls gltf and more that this client does not
  draw. Trimming is a build-time and payload win, not a picture win — it
  happens when it is in the way. **`bevy_audio` is no longer on that list**:
  the client makes sound as of 2026-08-06, and `wav` had to be *added* to the
  feature set (the defaults enable `bevy_audio` and `vorbis` only, so a
  generated WAV would have panicked with `UnrecognizedFormat` at the moment it
  played). Audio's own boundary rule is this document's rule one surface over
  — **Bevy plays, it does not decide** — with the model in
  `crates/client/src/sound/` (pure, code tier) and `render/audio.rs` owning
  nothing but the bank, the listener and the voices.
