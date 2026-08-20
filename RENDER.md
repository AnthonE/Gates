# Gates · RENDER.md — the native client's render path

**Owns**: how the Bevy desktop client draws, in what order it is built, and
the gate that watches it. **Does not own the bar** — `ART.md` is the bar and
outranks every sentence here; when a target in this file and `ART.md`
disagree, `ART.md` wins and the disagreement is the finding. `TERRAIN.md`
still owns worldgen and `CONTENT.md` still owns every number about items.

This file replaces `MIGRATION.md`, which the 2026-08-05 pivot mooted and the
2026-08-06 deletion removed from the tree. The one rule it used to inherit
from it — *a render path may not land without its probes* — is **retired**
(operator, 2026-08-06). `ci/vantages.mjs` passed all 36 checks on a beige
smear, so the probe that rule protected did not work; the operator boots the
game and looks instead. **Do not write a replacement pixel gate.** §5's
remaining gates are the ones that are arithmetic rather than photographic.

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

`ci/native_bar.py` reads our captures and the reference set **in the same run,
through the same estimator** — a bar computed a different way than the frame
it judges is not a bar. Medians over the six vantages:

| statistic | first native frame | before props v1 | now | reference |
|---|---|---|---|---|
| whole-frame p10 | 41.9 | 58.2 | **71.0** | 41.0 |
| whole-frame p50 | 61.0 | 90.1 | **93.1** | 91.4 |
| whole-frame p90 | 110.9 | 155.7 | 155.7 | 170.2 |
| sky band mean | 97.3 | 136.3 | 136.4 | 128.4 |
| near band mean | 54.8 | 79.8 | **83.5** | 80.5 |
| near-band saturation | 42.0% | 32.9% | **32.5%** | 33.2% |
| **near neighbour contrast** | 1.55 | 6.25 | **6.36** | 5.40 |
| chroma per unit luma | — | 0.163 | **0.171** | 0.252 |

The last two columns are one before/after pair taken on one box through one
estimator, which the earlier columns are not — those came off a different
machine and are kept for shape, not for arithmetic.

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
- **p10 71.0 against 41.0 — our darks are not dark, and props v1 made it
  worse, by 13.** Reported rather than buried, because the cause is understood
  and it is not the prop work being wrong. The old props were flat `base_color`
  surfaces sitting well under their own materials' reflectance — a boulder with
  a near-black facet was holding the tenth percentile down for the wrong
  reason. They now carry photographs whose means are measured in band, so the
  frame lost its accidental darks and what is left is the fill's true shape: a
  uniform ambient term buys rule 3's 0.30 floor at the price of the bottom of
  the range. A hemisphere (sky half cool, earth half warm) is what gets both.
  ✅ **LANDED 2026-08-15 — `crates/client/src/render/fill.rs`**, and the
  sentence that used to end this bullet ("Bevy's `AmbientLight` is uniform, so
  that is a second light or a shader") was **true about `AmbientLight` and
  wrong about the conclusion**, which is worth keeping because it is the kind
  of error that costs a whole slice. `pbr_ambient.wgsl` really does ignore its
  `world_normal` argument — but `environment_map.wgsl` samples its diffuse
  cubemap **by the world normal**, so an `EnvironmentMapLight` holding
  `fill_at(n)` is a hemisphere exactly, with no second light, no
  `AsBindGroup` and no WGSL. Gated by `crates/client/tests/fill.rs`.
  **What is NOT claimed: that this moved the p10.** The sky half is carried
  across unchanged so up-facing ground does not move, which is what made the
  change safe to land without a frame in front of anyone; the darks it
  restores are on down-facing faces. The p10 gap is still open and its
  remaining half is the transfer, not the fill.

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

### 1.0 · The front door is four states, and the window comes first

The path a player walks before any of the slices below draw a pixel:

```text
  Boot ──ready──▶ Menu ──pick──▶ Connecting ──welcome──▶ Loading ──▶ InWorld
   └──chosen──────────────────────▲
```

**`Boot` exists because a double-click used to show nothing.** On the launcher
path the client did a blocking scry handshake and a blocking QUIC connect
*before the window existed*, and `exit(1)`'d on failure — into a terminal a
double-clicked game does not have. Both are states now (`render/boot.rs`), so
the window is the first thing that happens and everything slow is drawn while
it happens. It ends on observable state — every `Startup` asset handle
settled, the handshake answered — never on a clock, which is §1.1's rule one
screen earlier and `CLAUDE.md`'s rule generally.

**`--capture` is the one door that skips it** and still connects before the
window. The probe harness is a gate: a client that draws a world it is not
connected to lies for its first few frames, and a harness that could
photograph a half-finished handshake is a harness whose frames depend on the
network.

What the splash still cannot cover is its own first ~3 s — wgpu adapter
enumeration and window creation precede the first Bevy frame (measured under
llvmpipe on this box). Covering that needs a second process; not taken, and
`DECISIONS.md` §open says so.

The screens themselves share one **shell** — wordmark, nav column, tinted
control panel, content pane — owned by `render/ui.rs`, because the five
reference frames the operator handed over are one screen with five payloads.
Nothing in it decides: the browser's filtering, sort and favourites are
`crate::ui::servers`, gated headless, and the launcher-backed entries are
`crate::ui::hub`.

**The backdrop is footage, and that is a correction.** The reference plays a
video behind its menu (operator, 2026-08-10), so the note that used to sit
here — render the island live behind the shell — was the expensive way to buy
the cheap thing. A backdrop has no camera to drive, no ring to feed and no
`WorldId` to insert and tear down; it costs one texture under a scrim, and it
is absent-tolerant. Motion is a frame sequence and a size trade, not a
renderer feature; `DECISIONS.md` §open "menu backdrop v0" has the numbers.
`--capture --no-hud` shoots a clean plate, which is how the shipped still was
made — the island is a pure function of the seed, so its title art is
reproducible like everything else here.

### 1.1 · The corollary: nothing is loaded until the server says what

"Bevy reads `ClientCore`" has a precondition that went unwritten until it bit,
and it is worth stating separately because the failure has no symptom.

**The welcome carries `player_id`, `seed` and `tick` — and no position.** A
seed is an island; it is not a place on one. Where the player stands arrives
one snapshot later, when the first packet carrying our own entity lets
`Predictor::adopt` take the authoritative spawn.

Between those two the client used to run the whole `Stream` set anyway, and
every streamer in it reads `Eye::pos`. An unplaced `Predictor` reports
`Body::default()`, whose position is the **world origin** — and the world
origin is a real place on this island, not a sentinel. So nothing faulted,
nothing logged, and the rings built a perfectly good neighbourhood of
somewhere the server never named.

**What that cost, stated at two different confidences.** Unconditionally: the
first frames of every connect built chunks at the origin and threw them away
when the eye jumped — small, since the rings fill one cell a frame and the
first snapshot is normally a frame or three behind the welcome. The severe
outcome is a **race**, not the common case: it needs the first own-entity
snapshot to be later than the whole ring build (~25 frames), and then the bar
reaches full at the origin and `InWorld` opens around a player the server has
not placed. Packet loss on the first snapshot, a server hitch, or a distant
shard is all it takes. A defect whose severity depends on an RTT is the worst
shape for one to have, which is why the fix is a precondition rather than a
tuned wait.

The rule, and where each half lives:

| | |
|---|---|
| `Eye::placed` mirrors `Predictor::started` | `render::input::place_eye` — and it writes **only** the flag when unplaced, never a position |
| the `Stream` set stands down until placed | `render::world_placed`, a `run_if` on the set |
| `place_eye` itself is **never** gated | pumping is how the placing snapshot arrives; a condition on it deadlocks |
| the bar reads 0.0 and is never `done()` while unplaced | `ui::load::Progress` — a gate in front of the mean, not a term in it |
| the screen says which of the two waits it is in | `WAITING FOR THE SERVER TO PLACE YOU`, rather than three zeroes for work no ring has been asked for |
| the capture harness bounds the wait | `capture::PLACE_FRAMES` — exits nonzero with the reason, because a hung gate reports nothing at all |

**Gated, and in the code tier**: `crates/client/tests/ui.rs` §E, three tests,
no GPU and no `--features render`. That is the point of `ui::load` being a
pure module — this decides *when a player enters the world*, and the version
of it that lived inside a Bevy system could only be tested by a windowed run
against a live shard.

**And measured once against a live one anyway**, because a gate on the
arithmetic cannot tell you the run-condition is wired to the right set. Seed
20260731, Xvfb + lavapipe: the shard placed us at `1001.6, 1.2, 1935.3` —
**2,179 m from the origin on a 2,048 m island**, which is the whole diagonal,
so the old path's first chunks were off the opposite corner rather than
merely nearby. World built at frame 27, six vantages written, exit 0.

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
- **Median fps hides shader-compile stalls, and the native symptom is a POP.**
  Lazy pipeline specialization is Bevy's version of the lazy WebGL program link
  that cost 700 ms+ worst-frames while the benchmark read 90 fps — but
  `synchronous_pipeline_compilation` is false by default, so a draw whose
  pipeline is not ready is skipped rather than waited for (measured 2026-08-20;
  `CLAUDE.md`'s trap entry said "a bigger stall" and was corrected). The gate is
  still a **COUNT** — no new pipelines created after the world is up — never a
  frame-time threshold, and it still needs a GPU. `render/prewarm.rs` closes
  most of it by drawing every `StandardMaterial` once, tiny, when it is
  created; skinned meshes are a different key and are not covered.
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
  gradient (not from the triangulation — `ci/bump_basis.mjs` **held** that
  arithmetic and is deleted with the browser client; the derivation is
  language-agnostic and readable from git, and has no native gate), and
  `splat()`'s four identity weights.
- Water is **not** this slice's and no longer a plane — `render/water.rs`
  owns it, and R1's only remaining relationship to it is that the seabed is
  the same heightfield (`TERRAIN.md` §4, R8 below).
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
- Tonemapper chosen by *measurement* against the reference set's frames,
  not by name. The paid-for datum: a transfer with a quadratic toe over the
  shaded range (Khronos PBR Neutral, `x - 6.25x²` under 0.08) squares the
  shadows and is why a shaded face arrived at 8/255. `ART.md` rule 3's floor —
  no shaded face below 0.30 of its lit face — is a property of the transfer and
  the fill together, so it is decided here or nowhere.
- Ambient is not one number: a hemisphere lands `mix(ground, sky, 0.5+0.5·N·y)`,
  so the **sky half lights up-facing ground in shadow** and the **ground half
  lights every down-facing prop face**. Moving them together is the mistake
  that cost a pass; they are two knobs. ✅ **Landed 2026-08-15** as `fill.rs`,
  in that exact form — and the two halves went in at different standings, which
  is the "two knobs" rule being obeyed rather than sidestepped: the sky half is
  the shipped uniform term carried across untouched, the ground half is the
  island's own measured albedo under its own irradiance. Only the *split* is
  new, so exactly one thing moved.
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

**Do not meter the tonal bar here.** p10/p50/p90 against the reference set is R5's
job, after there is content in the frame. A bar measured on an empty world is
the beige-smear trap with a different file name.

### R3 · The population — the largest structural gap in `ART.md` — **LANDED**

"The ground is not a surface, it is a population." No shader fixes this and
six passes proved it.

- **Scatter** (`terrain::scatter`, 7 occupant kinds): pines, stumps, rocks,
  the haven and waystation structures. Instanced, one mesh+material per
  archetype so Bevy's automatic batching collapses them to a draw each.
  `ART.md` rule 6: **silhouette before surface** — a smooth cone is wrong at
  any texture budget; the pine is tall, thin and ragged-edged.
  `ci/pine_shape.mjs` held that shape as arithmetic and is deleted;
  **this one did get a native successor** — `crates/client/tests/tree.rs`,
  which gates the generated conifer against the volume the sim blocks and the
  frame's triangle ceiling.
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

### R4 · Materials — the photograph, on the surface — **LANDED (4-way splat, 2026-08-15)**

The CC0 set in `assets/textures/` already exists, is manifested, and its
*selection* was already measured (gain span, albedo sd, anisotropy). None of
that work is lost; it moves to WGSL.

**The first WGSL in the tree is `assets/shaders/ground_splat.wgsl`**, bound by
`render/ground_splat.rs` as an `ExtendedMaterial<StandardMaterial, GroundSplat>`
whose extension deliberately does not declare `#[bindless]` — which is what
forces the whole material non-bindless and retires the blocker `terrain_mesh.rs`
had recorded against exactly this slice. Four albedo and four normal maps, one
shared sampler, per-identity roughness. Landed with `tests/ground_splat.rs`.

Three things it settled that are worth not re-deriving:

  · **The weights ride `ATTRIBUTE_COLOR`, not a packed `UV_1`.** Packing two
    `u8` per `f32` was scouted and is wrong — the rasterizer interpolates the
    packed value and `floor(p/256)` mixes the low byte into the high one. Exact
    at both vertices, 50% wrong mid-triangle, i.e. at identity boundaries.
    `UV_1` carries the two scalar modifiers (break-up, waterline) instead.
  · **Each map contributes LUMINANCE, never colour**, which is the §7 deviation
    rule satisfied by construction rather than by a correction: a mean-1
    luminance field has gain span 1.000, where only `rock` clears the rule as
    colour (grass 2.454, sand 2.073, litter 3.586).
  · **The height blend is a tie-breaker and measured as a no-op.**
    `splat_from` is near-binary (92.2% of samples over 0.8) so the contested
    band is a sliver. Kept as insurance, not as a live setting.

- Terrain: 4-way splat blend from the vertex weights, biplanar projection with
  §2's two rules, per-identity tint bounded by the deviation rule, macro
  break-up at 0.5–1 m and near-field grain under 5 cm (rule 1). **The blend and
  the grain landed; the projection is still planar XZ, not biplanar** — a
  vertical face still stretches, and that is what R4 has left.
- **Built pieces were the last flat surface, and landed 2026-08-16** (piece
  surface v0, `DECISIONS.md` §open · `NOW.md` §0ps). A wall is the largest
  flat thing a player stands in front of and it drew as a `base_color` until
  now, while props and the viewmodel had sampled the same maps since
  2026-08-11. Four tiers × (albedo + normal), no new file: the paths are
  `MapSet::load`'s, so the handles are `PropMaps`' own. The three
  prerequisites are the transferable part — **0..1 UVs became metre-scaled**
  (a stretched tile reads as no texture), **meshes gained tangents** (Bevy
  drops `normal_map_texture` without them, silently), and a **mean-1 per-face
  vertex tint** carries the cap-vs-side separation a second draw call would
  otherwise cost. It also found a defect nothing could see: the tier table had
  three rows against the sim's four materials, so every piece drew one rung
  off and twig had no look at all. Gate `client/tests/pieces.rs`.
- AO maps become `indirectDiffuse *= ao` — indirect only, medium scale — and
  `min(bakedAO, ssAO)` where SSAO also runs, never a sum or a product. Micro
  occlusion stays baked in albedo and *does* apply to direct light. Specular
  occlusion is its own term. (`ART.md` §4, from Frostbite §4.10.3.)
- Value separation is the point, not hue: granite ~2× turf's value; grass
  shadows go cool (hue 70° → 170°).
- **Probe (R-G4)**: the shipped estimators, ported and cross-checked against
  the JS ones on the same input — along-colour vs orthogonal chroma residual in
  the 0.077–0.193 band, `ALBEDO_LUMA_BAND` respected, and the biplanar identity
  `ci/prop_photo.mjs` asserted in closed form — that gate is deleted with the
  browser client and the closed form is readable from git, so porting it is
  part of this slice rather than a cross-check against a live gate.

### R5 · The light rig, metered — the tonal bar

Now that the frame has content, meter it. `ci/native_bar.py` reads the
outdoor-daylight reference frames with the same code path that reads ours;
port that discipline, not the numbers. Targets are `ART.md` §3's. (The browser
tier's `ci/reference_bar.mjs` did this first and is deleted — it needed a page
this repo no longer opens. The frames it read are out of the tree too, so
`native_bar` wants `GATES_REFERENCE_DIR`; `ART.md` §0 has the posture.)

### R9 · Bodies and the held item — **LANDED**

Two halves of "evidence a person is playing", one first-person and one not.

The viewmodel is `render/viewmodel.rs`: a textured tool with bob off distance
travelled, sway as a frame-rate-independent lag on the look rate, and a swing
triggered off `Feed` rather than off the input buttons — the swing worth
drawing is the one that LANDED.

Remote bodies are a CC0 skinned mannequin (`render/anim.rs`, 46 clips,
`assets/models/MANIFEST.md`) instead of `Capsule3d`. Three things this settled:

  · **The wire already had `yaw` and `pitch` and nothing read them.** Bodies
    faced +Z whatever they were doing. A capsule hides that; a figure cannot.
  · **The rig's origin is its FEET; the capsule's was its middle**, so the
    inherited `+ 0.9` floated every player a metre off the ground.
  · **Clips resolve by NAME through `Gltf::named_animations`, never by index.**
    `GltfAssetLabel::Animation(i)` is positional, and a re-vendor that inserts
    one clip would renumber every one after it — every body playing the wrong
    animation with all gates green. That is the trap list's positional-payload
    entry, and it is the one place in this path where it applies.

Still owed, and all four are the same shape — the CLIENT is ready and the WIRE
is not: no grounded bit, no crouch bit, and no per-remote action event, so
crouch, jump, swim and attack clips sit in the file unreachable. `NOW.md` §0v.

### R6 · HUD and viewmodel — **LANDED**

`ART.md` §6 and §8: a frame with no viewmodel and no HUD reads as a
flythrough, and the blind reader had named it on every capture so far. Landed
as the reference's shape — bottom-centre hotbar with the selected cell lit,
right-side vitals stack, a held item entering from the lower right. Every
number on it is the server's (`hp`/`hp_max`, `food`, `water` off `ClientCore`),
and the zero-max rule `core.rs` states is honoured: a shard whose content
disarms combat draws no bar rather than an empty one.

**The viewmodel is the held item since 2026-08-11, and the sentence that used
to stand here was wrong about why it was not.** It read "the hotbar knows only
which cell is selected, not what is in it", which sounded like a wire gap and
was not one: `ClientCore.inv` has carried every slot's `ItemStack` since the
container slice, filled from `EventMsg::Inv`, and `catalog` carries the display
names. Nothing needed to be added to the wire — the data was already arriving
and no reader had asked for it. `ui::hold::held_model` resolves the selected
slot to a model by the same normalised display name the icons key off, and
`render::viewmodel::swap` puts it in the hand.

Three pictures, deliberately distinct: a modelled item draws its model; an item
with no model yet draws the generic stand-in tool; an **empty hand draws
neither**, because a tool that appears when you are carrying nothing is a lie
about your own inventory.

Still owed: item icons in the cells — same data, same lookup, and now clearly a
UI slice rather than a wire one — status chips (`WET 36%`), and per-item pose
tuning, since one grip offset serves a 20 cm rock and a 1.8 m spear.

**The face landed 2026-08-07 and `render/ui.rs` owns it**, the same way it
owns the palette and for a worse-founded reason: nothing owned it before, so
all 42 `TextFont` sites here and across `render/panels/` were `..default()`
and every screen drew in Bevy's embedded debug mono. Roboto Condensed, bold
by default and regular for prose, embedded rather than loaded — an unresolved
`Handle<Font>` draws nothing at all, and `OnEnter(Loading)` runs before
`Startup`, which is the trap `audio::build_bank` already documents.
`DECISIONS.md` "ui type v0" has the sources and what was deliberately left
(the size scale, blocked on nothing being able to photograph a panel).

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

### R8 · The sea — `render/water.rs` — **LANDED (water v0)**

Research is `reference/WATER.md`; every number is `DECISIONS.md` §open,
"water v0". Built in **the reference's own published order** — surface,
optics, motion, foam — which puts waves third rather than first.

- **One eye-centred mesh**, uniform 2 m core to 64 m then a geometric skirt
  out to 2.6 km. One rather than a near grid plus a far plane: two translucent
  surfaces that overlap blend twice, and the sea would darken along a seam
  that moves with the player.
- **The optics are a volume.** Colour is `S·(1 − e^{-d·σ})` per channel and
  alpha is `1 − mean(e^{-d·σ})` — the reference's "depth-based colour
  extinction" and "thickness-based visibility" are one arithmetic because they
  are one fact. `AlphaMode::Premultiplied`, so the sky's Fresnel reflection
  survives in shallow water where the alpha is near zero.
- **The swell** is four directional waves with analytic gradients, each
  retired where the mesh under it is coarser than its own quarter wavelength —
  the ground material's octave-retirement law applied to geometry. Below the
  shortest, a tiling ripple normal map with its own mip chain, scrolled by
  `uv_transform`.
- **The waterline is a band, not a line**, and it is worked from four sides:
  the wash *stands off* the water's edge (peaking at 0.6 m of depth, zero at
  the edge itself — foam that peaks at the seam outlines it), its contour is
  displaced by world-space noise so its edges are lobes rather than iso-depth
  curves, it surges with the swell so the edge moves, and on the land side
  `terrain_mesh::wet_factor` damps a few metres of *ground* inland, bounded by
  a run as well as a height.
- **What is still a hard edge**: the alpha ramp is a vertex quantity read off
  `terrain::height`, so it fades correctly against the terrain and not at all
  against a boulder, a foundation or a player standing in the shallows. The
  fix is a depth-prepass fade in the fragment — an `ExtendedMaterial` and the
  first WGSL in the tree. `NOW.md` §0y.
- **Budget**: 7,921 vertices, one mesh, one draw. Per snap (an 8 m cell
  crossing) ~7.9 k `terrain::height` taps; per frame four sines a vertex and
  three attribute writes, no allocation.
- **Not built, and §5/§6 of the research is the licence**: screen-space
  reflections (their expensive half; the payoff they name is the sky, which
  the atmosphere's specular already gives us), rivers and lakes, and any
  underwater colour grade — the last refused because haze has one owner.

*Picture bought*: the island stops sitting on a blue plate.

### R7 · What is deliberately not in this plan

Water that simulates *the player back* (a wave that pushes a body is sim state
in the ECS), water reflections, billboard/impostor LOD for distant trees,
meshlets,
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

**The probe photographs a scene now, not just terrain** (2026-08-20). One
process is the whole population, so every frame the harness had ever taken was
a picture of ground — and the two things anybody actually asks to see, another
player and a building, were the two it could not reach. Nothing new was
needed in the sim: `population = N` seats bots that build a twig base over the
shard's own wire, `dev_spawn` makes them the camera's neighbours instead of
scattering them round the coast by id hash, and `dev_spawn_kit` pays for the
wood a foundation costs. `render/capture.rs` grew a **scene pass** after the
verb pass — nearest body at eye height as `7-player.png`, nearest base
(centroid of one cluster, from `BUILD_STANDOFF_M` back) as `8-build.png` —
and `ci/scene.sh` arranges both halves. Both shots are conditional and skip
loudly; the tail check verifies them beside the vantages.

Four things it cost, and three of them are already-paid traps re-collected:

- **Animals are on the body lane.** The first run of the scene pass, against a
  shard with a population of ZERO, reported *"player at 67.3 m"* and
  photographed a wolf — `bodies.rs`'s own scar (`mob::slot_of_id`), reproduced
  exactly, in the file next to it.
- **The probe dies in the pile.** `dev_spawn` puts every body on one point,
  which is what makes the scene and also what puts the camera inside six bots.
  Measured: dead at frame 32 with the HUD reading `CHARGE · 2s · N 11M`. So
  the rig does not arm the raiders by default, and lets the world run first —
  the bots walk continuously and are spread within a couple of minutes.
- **`world_running` is true for a corpse**, so a dead probe used to shoot six
  vantages of the death wash and exit 0. It now says so and exits nonzero.
  This is the §"wolves kill it" failure below, finally made loud.
- **Aiming at a body is not seeing one, and it took four runs to believe it.**
  Bodies at 9.9 m, 25.9 m and 5.6 m were each aimed at correctly and each
  photographed as foliage or a wall — which reads exactly like "remote bodies
  do not render", and is not. The camera now RANKS candidates by whether the
  line to them is clear (`sight_is_clear`), and the fifth run put three
  mannequins in frame with one at the crosshair. Two things worth keeping:
  a tree's `occupant_volume` radius is **0.2398 m — the bark, not the
  canopy**, so a sight test against it passes straight through a tree that
  fills the frame (the proxy is trunk-to-line distance at `CANOPY_CLEAR_M`);
  and a piece is a blocker only when `loc != LOC_PLANE`, because the
  horizontal one is the floor the body is standing on.

⚠ **`live` is never true for a remote body under lavapipe** — 20 of 20, 17 of
17, every run. The probe renders at about 1 fps while the shard ticks at 30,
so every sample is a clamp. It is recorded and printed, never obeyed:
`bodies::stream` reads the same interpolator with no `live` check, so it draws
the mannequin at the clamp, and a camera that skipped stale bodies would
refuse to photograph one that is on screen. An early cut of this filtered on
it and skipped every body on the island.

**Landed: the harness and the measurement. NOT landed: the assertions.**
`gates --capture <dir>` settles on observable state (all three rings full —
25 chunks, 25 scatter parents, 25 clutter tiles, reported at the frame it
happens), warms 30 frames, shoots six vantages and exits; `ci/native_bar.py`
reads those captures and the reference set through one estimator. What neither
does yet is FAIL. Nothing in `ci/gates.sh` runs either, and until it does the
render path's coverage is `cargo clippy -p client --features render` and a
human looking at a PNG. That is the top of §8's list, it is the pivot's stated
debt, and calling it anything other than open would be the "pass it didn't
earn" this repo names as its worst bug class.

⚠ **The probe is a player, and wolves kill it.** Recorded 2026-08-16, after it
had cost several runs and been diagnosed as everything else first. A capture
run connects as an ordinary client and its body stands in the world, unarmed
and unmoving, from the moment the shard places it until the sixth vantage is
written. That is long enough to be found.

**The tell is that the kill rate tracks the BUILD, not the seed: 3 of 4 runs
carrying a heavy change died, against 0 of 2 baseline runs on the same seed and
the same spawn.** Nothing about the wolves changed. A heavier build takes longer
in wall-clock to fill three rings and warm 30 frames — that is the whole
mechanism — so the probe stands there longer and the leash brings something to
it. Which makes this worse than a flake: **the measurement's failure rate is a
function of the size of the thing being measured.** The captures you most want
are the ones least likely to reach disk, and the bias points the wrong way for
every purpose the harness has. Anyone reading a run of missing vantages as
"expensive change, must have hung" would be reading the correlation exactly
backwards.

**Pin `dev_spawn` in `shard.toml`.** A pinned spawn is already the rule for a
different reason — the shard hashes a spawn per player id, so two unpinned runs
compare two places and not two builds (`render/ground_splat.rs` states where
that already cost a before/after). It also happens to fix this, by letting you
choose ground the roster has not homed on.

**Not to `1024,1024`.** The island centre is the obvious pin and it is the
worst one available: `mob::home_of` draws each slot's home from the seed and
rejects it against `HOME_MAX_SLOPE`, so homes concentrate on exactly the flat
walkable interior the centre is the middle of, and every one of them is leashed
to return there. `1500,600` is the spawn the ground material's own measurements
were taken at and it has been quiet across runs.

**The harness has a noise floor of roughly 0.3%, and it had never been
measured.** Re-running the *same build* twice through `ci/native_bar.py`'s
estimator moves near-band neighbour contrast by −0.3% and near saturation by
−0.6% — the probe is a live client against a live shard, so wind phase, clutter
animation and mob positions do not repeat between runs, and only `5-sky` (which
frames nothing that moves) comes back bit-stable. Anything a change buys under
about a percent is therefore **not a result**, and the way to know which side of
the line you are on is to run the unchanged build twice and subtract, rather
than to trust one A/B. This is the same lesson as the clock rule above wearing
different clothes: the number that looks like a measurement is the one to check
against a second source. Measured 2026-08-16 while landing the ground roughness
maps, whose whole reported effect (−0.4%) turned out to sit inside it.

**What would actually fix it** is making the probe not a target — a capture
client that the sim does not treat as huntable, or a shard flag that stands the
roster down for a capture run. Both are sim-core changes for a harness's
benefit, which is the wrong direction for a wall, so the pin is the answer
until the harness is a gate rather than a measurement.

Also landed, and it is the cheapest thing in the document: **the render
feature now compiles under a lint gate.** `cargo clippy -p client --features
render --all-targets -- -D warnings` is green and it caught three real
findings on its first run — before it, cargo skipped `gates.rs` entirely and
a bin containing `this is not rust at all !!!` would have passed.

**The tier.** `ci/gates.sh` grows a native renderer tier. It was written as
"beside the browser one"; the browser client is cut (`DECISIONS.md`
2026-08-06), so it is not beside anything — **it is the renderer tier**, and
the three browser gates it was going to sit next to are dead weight nobody
owes a fix. Scheduled the way `renderer_touched` schedules today's: a diff
touching `crates/client/**` or `assets/**` runs it. **The compile half
landed 2026-08-06** — `ci/gates.sh` now runs `clippy -p client --features
render -D warnings` plus the three renderer-tier suites. The half that
photographs anything did not. Bevy is several hundred
crates and minutes of build; that cost belongs in the tier that owns it, never
in the ~106 s code tier. Use `bevy/dynamic_linking` for local iteration.

**The capture protocol**, and every clause is a trap already paid for:

- Fresh process per shot. Fixed seed, fixed dev spawn, one shard per vantage,
  **one live renderer at a time** (two was the browser tier's whole problem).
  **The fixed seed is 20260731**, and this line names it because for four
  passes nothing did: the probe has no seed of its own — it photographs
  whatever the shard it dialled sends in the welcome (`bin/gates.rs`), so the
  shard configs *were* the instrument setting, silently, and
  `gates-loop/art/capture-native.sh` carries a second copy of the value as its
  own default. Both say 20260731. It nearly moved on 2026-08-14 on a
  measurement that turned out to sweep a quarter of the island
  (`sim-core/tests/relief.rs` header); if it ever does move, **frames are not
  comparable across the change**, exactly as re-aiming a vantage makes them
  incomparable — say so in the report rather than letting the next reader
  compare them.
- Settle on observable state — welcome received, `snapshots > n`, chunk queue
  drained — and never on elapsed time. Budget in **frames**, not milliseconds.
- **Fixed hour, and it is pinned in the client rather than the harness** (2026-08-15,
  `DECISIONS.md` §open "capture clock v0"). `rig::DayPin` puts a `--capture` run's
  tick at `CAPTURE_DAY_FRAC = DAY_PORTION * 0.5` — the arch's peak, the one
  fraction where `sun_elevation` returns `RIG_SUN_ELEVATION` exactly. **Since
  the bearing started sweeping (2026-08-15) it is also the one fraction where
  `sun_azimuth` returns `RIG_SUN_AZIMUTH` exactly**, which is why the sweep
  cost no scored frame its comparability: naming the hour now fixes both
  coordinates, where naming an elevation would fix only one. Until then
  the hour was *whatever tick the shard had reached* when the probe fired, i.e. a
  function of how long the build took: **24.47° at tick 0, 27.33° typical, 30.36°**
  on a slow box, rising monotonically, so a slower box scored a brighter frame and
  no scored frame was ever taken at the authored register. It pins the tick and not
  the sun, because `render/audio.rs` reads the same clock through `is_night`.
  Same consequence as the seed clause above: **frames either side of 2026-08-15 are
  not tonally comparable.**
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

**There is no gate in this repo that looks at a frame, and that is settled
rather than owed.** This section was written when `web/` kept its gates "until
the native client can do what it does"; the browser client was cut and then
deleted (`DECISIONS.md` 2026-08-06) and that day never came. The operator's
call is that it should not: `ci/vantages.mjs` passed all 36 of its checks on a
beige smear with no sky, no horizon and no object in it, so the automated
version demonstrably did not work, and booting the game and looking is cheaper
and cannot be fooled by a wash. **Do not write a replacement pixel gate.**

What may still be gated about a frame is the part that is arithmetic — the
mesh fits the volume the sim blocks, the pipeline count after the world is up —
and the shape of that is `crates/client/tests/tree.rs`, in Rust, headless.
There are no tiers any more either; `ci/gates.sh` has one behaviour and one
final line.

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
| frame | 60 fps on a mid laptop iGPU | `DESIGN.md` §9 — survives the move; **measured on a GPU, never on the gate box**. Since 2026-08-20 there is a mechanism under it rather than only a hope: `render/quality.rs`'s LOW/MEDIUM/HIGH ladder, whose top rung is this table's frame exactly (`DECISIONS.md` §open, graphics tiers v0) |
| texture payload | < 12 MB before compression | `ART.md` §7 — **retired**: it was a first-visit download, and a depot install is not one |
| clutter ring | 5×5 tiles of 16 m, 721 elements/tile peak | `sim-core::terrain`, and it is frame-budget-bound, not design-bound |
| eye height | 1.6 m | `DECISIONS.md` §open, client cosmetics |

**Every budget above is the GPU's, and the client's CPU frame had never been
measured at all** — a table of triangle and draw-call ceilings says nothing
about what the main thread spends before the first draw call is issued. It was
measured on 2026-08-11 and two of the three biggest items were waste rather
than work: `water::animate` resolved its wave field three times a vertex
(1.01 → 0.38 ms **every frame**), and one 65² ground chunk cost 28 ms to build
— a whole dropped frame, one per streaming frame — of which mikktspace was 12
and duplicated `terrain::height` taps most of the rest (now 5.4 ms).

**The ranked remainder was taken on 2026-08-19** and the shape of the answer
changed the paragraph above rather than extending it: the largest remaining
costs were not arithmetic to sharpen but work to refuse or to move.
`clutter_fill` is 2.87 → 1.02 ms a tile (a caller-owned lattice memo, plus a
stratum that now refuses on its own hash before resolving the ground it would
be tested against); the far mesh's ~190 ms `Loading` frame is off the main
thread entirely (`AsyncComputeTaskPool`); the sea carries its last sweep across
a snap instead of re-tapping it; and two per-frame systems that reconciled a
whole mirror now run on a `Feed::applied` bit. Every one is bit-identical or
non-numeric — `sim-core/tests/lattice.rs`, `client/tests/ground_async.rs` and
`client/tests/water_carry.rs` are the evidence, and the last two exist because
`tests/ground.rs` and `tests/water.rs` call the pure functions directly and can
see nothing about when or on which thread a system calls them.

`NOW.md` §0pf carries what is left and `findings/client-frame-20260819.md` the
method; the numbers are release, on the gate box, and **no GPU has ever run
this client**, which is why they sit here as a note rather than as rows in the
table.

The first thing to actually press on the triangle ceiling was the generated
conifer: a full 328-tree scatter ring at 5.9 k tris a tree is 1.9 M, and
`crates/client/tests/tree.rs` *printed* the ring rather than asserting it,
precisely because 1.5 M is the number this table is unsure of.

**It fits now — 1.94 M → 510 k, landed 2026-08-20** (`DECISIONS.md` §open,
tree LOD v0). Past `TREE_LOD_SWAP_M` a tree is one opaque hull lathed through
its own vertices (`tree::impostor_of`, 105 tris) instead of a 5.9 k bark mesh
plus an alpha-masked canopy, swapped by `VisibilityRange` with a 15 m dithered
crossfade. The gate asserts the fit and asserts it would NOT fit without the
swap, so an impostor that quietly became a copy of the tree goes red.

Two notes that outlive the number. **The triangle count is the smaller half of
the win**: SSAO carries `#[require(DepthPrepass, NormalPrepass)]`, so the same
geometry is rasterized in two prepasses, the main pass and each of §R5's four
shadow cascades — and `bevy_light`'s `check_dir_light_mesh_visibility` consults
`VisibleEntityRanges` exactly as the camera's own check does, so the swap is
paid back in every one of them. **And none of it is measured on a GPU** (the
paragraph above), so this is counts × passes like every other budget here.
The billboard (`TERRAIN.md` §4) is still unbuilt and is still the cheaper end;
the hull is the step that needed no new material, no bake and no render target.

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

R1 through R6 exist to satisfy that list. That sentence used to end "when a
capture passes it, the browser client can start being deleted, and not one
slice earlier" — **the order reversed**: the browser client was cut on
2026-08-06 by operator word, before any native capture passed this list. The
list is unchanged and still the bar; what is gone is the fallback that made
missing it survivable.

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
     where a wall belongs. Base height is resampled locally by calling
     `sim_core::build::column_floor_y(seed, cx, cz) + level*3` — the sim's
     one implementation (cell-center terrain snapped to `BUILD_BASE_Q_M`,
     2026-08-15; this line restated the raw formula until then, and the day
     the sim's rule changed is the day a restated copy would have drawn
     every floor off the surface the sim walks) —
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
4. ~~**A hemisphere fill.**~~ Landed 2026-08-15 (`render/fill.rs`). It bought
   the *direction*, not the p10 — see §0. What is left of this row is the
   transfer half, which is item 6.
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
- **Bevy's default feature set** pulls more than this client draws, but the
  list of what is dead has to be re-read rather than assumed — and it has now
  been wrong twice. The old version of this bullet named `ui`, which is the
  entire interaction surface (~5,400 lines and 78 `Node` sites across
  `render/{ui,menu,settings,pause,loading,hud,chat,map,death}.rs` and
  `render/panels/`: server select, loading bar, Esc menu, settings, HUD,
  inventory, crafting, build wheel, chat, the map and the death screen). It
  then named **`bevy_audio`** as genuinely unused, correctly identifying the
  blocker as *asset licensing, not the API* — and that is exactly the blocker
  the audio slice removed on 2026-08-06. `crates/client/src/sound/synth.rs`
  GENERATES the bank from arithmetic at boot, so there is no sample to
  license, and the client makes sound. `wav` had to be **added** to the
  feature set: Bevy's defaults enable `bevy_audio` and `vorbis` only, so a
  generated WAV would have panicked with `UnrecognizedFormat` at the moment
  it played. Audio's boundary rule is this document's rule one surface over —
  **Bevy plays, it does not decide** — with the model in
  `crates/client/src/sound/` (pure, code tier, 63 assertions) and
  `render/audio.rs` owning nothing but the bank, the listener and the voices.
  **The score (2026-08-11) is the same rule under load**: `sound::music` is a
  gap-and-intensity director (`reference/AUDIO.md` §8) that decides which
  piece plays and when, headless and testable; `render/audio.rs::music` spawns
  what it names and holds the level. It is also the one audio system with no
  run condition at all, because the menus have music and have no world.
  **`bevy_gltf`, `bevy_scene` and `bevy_animation` stopped being unused with
  the mannequin** (2026-08-07, `render/anim.rs`) — this paragraph named all
  three as trim candidates and only the reasoning survives. Trimming is a build-time and payload win, not a picture win — it
  happens when it is in the way.
- **`bevy_scene` is a decided no, not an unexamined gap.** Its three
  advertised wins each land on a wall here. *Entity-ID-preserving save
  games*: there is no client-side save game and there must not be — the
  authoritative state is the server's, persisted in the WAL with the content
  hash pinned (wall 7), and a scene that round-trips entities is a second
  copy of world state living in the ECS, which is §1. *Linked instancing*:
  the world's repetition is already `terrain::scatter`'s pure hash streamed
  by rings keyed on cell, and moving the spawn description into an asset the
  ring does not own is the same objection that keeps
  `bevy_procedural_tree`'s own plugin unused. *Hot reloading*: worth having,
  and it is **not** `bevy_scene` — it is `bevy_asset`'s file watcher, shipped
  as `--features hot`, and it reloads the textures already in `assets/`
  (and WGSL, when there is some) without one component becoming `Reflect`.
  The client currently has zero `Reflect` derives, and scene serialization
  would require them on everything that goes in — a maintenance surface
  bought for a feature already ruled out.
- **The unused Bevy capability that actually points at a ranked gap is
  custom materials.** There is not one `AsBindGroup` or line of WGSL in the
  tree, and per-instance tint (`ART.md` rule 7) is
  `ExtendedMaterial<StandardMaterial, _>` work. It is not scheduled here
  because it is inside the coupled lighting set §2 reserves for one owner and
  one iteration.
  ⚠ **This bullet listed the hemisphere fill as the second such gap and that
  was wrong** — it landed 2026-08-15 with no custom material at all, because
  `EnvironmentMapLight`'s diffuse cubemap is sampled by the world normal and
  therefore already *is* a per-normal term. The general lesson, since this doc
  is where a future pass sizes its slice: **"`StandardMaterial` cannot express
  this" is a claim about `StandardMaterial`, not about Bevy.** Check the
  built-in light and probe components for the quantity before concluding a
  shader is owed; the wrong answer here would have cost a slice.
