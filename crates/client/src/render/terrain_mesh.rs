//! The ground: a heightfield meshed straight out of `sim_core::terrain`.
//!
//! This is meshing, not design. `terrain::height` is a pure function of the
//! seed, the server sims on it, the browser drew it, and the three agree
//! bit-for-bit — so the only questions here are resolution, streaming budget
//! and what the vertices carry.
//!
//! The scheme is the browser's, because it was measured rather than guessed
//! (`web/src/terrain.js`): one far mesh of the whole island at 8 m built
//! once, plus a ring of 64 m near chunks at 1 m streamed around the player,
//! the far mesh dropped 0.15 m so the near↔far boundary cannot z-fight.
//! Adjacent near chunks share exact edge heights — they sample the same
//! function at the same coordinates — so same-LOD seams cannot crack.
//!
//! **Normals are analytic, never from the triangulation.** A heightfield that
//! takes its normal from its own triangles renders its own tessellation:
//! every quad's diagonal shows up as a shading crease that moves when the LOD
//! does. The property that matters — the world-XZ gradient is continuous
//! across a triangle edge — is a property of central differences, which is
//! what this takes. `ci/bump_basis.mjs` held it as a gate and went with the
//! browser client, so the arithmetic is still right and nothing checks it.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::ExtendedMaterial;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};
use sim_core::terrain::{self, SEA_LEVEL};

use super::ground_splat::{GroundMaterial, GroundSplat};
use super::textures::GroundMaps;
use super::{Eye, WorldEntity, WorldId};

/// Near-chunk edge, metres.
pub const CHUNK_M: f32 = 64.0;
/// Vertices per near-chunk side: 64 m at 1 m plus the shared edge.
pub const NEAR_N: usize = 65;
/// Near ring radius in chunks — a 5×5 ring, 160 m to the corner.
pub const NEAR_RADIUS: i32 = 2;
/// Vertices per far-mesh side: 2048 m at 8 m plus the edge.
pub const FAR_N: usize = 257;
/// Far-mesh sample step, metres.
pub const FAR_STEP: f32 = 8.0;
/// How far the far mesh sits below the near ring so the boundary cannot
/// z-fight, metres.
pub const FAR_DROP: f32 = 0.15;

/// The y the FAR mesh actually draws at `(x, z)` — not `terrain::ground`.
///
/// **A prop outside the near ring must be planted on this and not on the
/// heightfield, and the gap is 0.63 m on the shipped seed.** The far mesh samples
/// `terrain::ground` on an [`FAR_STEP`]-metre lattice and interpolates
/// linearly across each 8 m quad, then sits the whole sheet [`FAR_DROP`] lower
/// so the near↔far boundary cannot z-fight. So the surface a player SEES out
/// there is a chord across the real terrain, and on anything but flat ground
/// the chord is below the curve — a tree placed at `slot.y` stands with its
/// base in the air over a valley and buried on a ridge, at exactly the ranges
/// where a floating trunk is the most obvious thing in the frame. Measured
/// worst separation over a 2,000-sample line across the island on seed
/// 20260731: **0.630 m**, against a 6.6 m conifer — a tenth of the tree
/// hanging in the air, which is what `tests/outer_ring.rs` searches for and
/// then plants at.
///
/// This is `heightfield`'s own sampling restated for one point instead of for
/// a grid: the four surrounding lattice corners, minus the drop.
///
/// **Bilinear, where the mesh is two triangles, and the gap is stated rather
/// than hidden.** A quad rasterises as a pair of triangles and is therefore
/// planar either side of one diagonal; bilinear is the average of the two ways
/// that diagonal can run. The two agree EXACTLY at the four corners — which is
/// what `tests/outer_ring.rs` pins, because it is the only place the two can be
/// compared without rebuilding the mesh — and differ inside the quad by at most
/// the quad's twist, `|h00 − h10 − h01 + h11| / 4`. That is small against the
/// 0.63 m the naive `slot.y` carries, so it buys the fix without a second
/// triangulation to keep in step. Choosing the
/// diagonal correctly would mean knowing `heightfield`'s index winding here,
/// which is a coupling worth more than the centimetres.
///
/// Not used by the near ring: inside [`NEAR_RADIUS`] the drawn ground IS
/// `terrain::ground` at 1 m, so `scatter`'s own `slot.y` is already exact.
pub fn far_ground_y(seed: u64, haven: &terrain::Haven, x: f32, z: f32) -> f32 {
    let gx = (x / FAR_STEP).floor() * FAR_STEP;
    let gz = (z / FAR_STEP).floor() * FAR_STEP;
    let tx = (x - gx) / FAR_STEP;
    let tz = (z - gz) / FAR_STEP;

    // One memo for the four corners: they are one lattice quad apart, so this
    // is the memo's best case and the same reason `heightfield` holds one.
    let mut lat = terrain::Lattice::new();
    let h00 = terrain::ground_memo(&mut lat, seed, haven, gx, gz);
    let h10 = terrain::ground_memo(&mut lat, seed, haven, gx + FAR_STEP, gz);
    let h01 = terrain::ground_memo(&mut lat, seed, haven, gx, gz + FAR_STEP);
    let h11 = terrain::ground_memo(&mut lat, seed, haven, gx + FAR_STEP, gz + FAR_STEP);

    let a = h00 + (h10 - h00) * tx;
    let b = h01 + (h11 - h01) * tx;
    a + (b - a) * tz - FAR_DROP
}

/// Chunks QUEUED per frame, and chunks torn down per frame. Stream-in AND
/// stream-out are budgeted: the teardown spike is the half everyone forgets
/// (`CLAUDE.md` traps).
///
/// ⚠ **It used to mean "built" and it does not any more.** The build is on
/// `AsyncComputeTaskPool` now, so this bounds how fast work is HANDED to the
/// pool, not how much of the frame it costs — the frame's share is
/// [`CHUNK_LANDS_PER_FRAME`] below. The two are kept at 1 apiece rather than
/// raised: a wider queue would fill the pool with chunks the player has
/// already walked past, and this budget is what the sibling ring streamers
/// (`props`, `clutter`) are also rationed against.
pub const CHUNK_BUILDS_PER_FRAME: usize = 1;
/// Finished chunks taken into the world per frame.
///
/// A landing is not free even though the meshing is off-thread: `meshes.add`
/// hands ~400 KB to the renderer and the upload is the frame's. Bounding it
/// keeps the trade honest — the point of the pool was to stop paying a 5.4 ms
/// build on the frame, not to pay twenty-five uploads on one instead.
pub const CHUNK_LANDS_PER_FRAME: usize = 1;

/// The four ground identities' albedo, LINEAR, in the order `terrain::splat`
/// returns them: sand · grass · forest litter · rock.
///
/// Derived from `ART.md` §3's measured surfaces — the hue and saturation are
/// the reference's, the value is the *reflectance* that produces the
/// reference's lit luma under a midday sun, not the lit luma itself. All four
/// sit inside `ALBEDO_LUMA_BAND = [0.05, 0.55]`, and the two facts §3 states
/// plainly are visible in the table: granite is warm grey and roughly 2×
/// turf's value, and grass is the darkest thing on the island.
///
/// **Every sentence above was already here and none of it was true, because
/// nothing checked it** (2026-08-14). `crates/client/tests/ground_identity.rs`
/// checks it now, and it opened red on five counts: turf sat at 84.0°/22.9%
/// against §3's 63–74°/29–33%, litter at 31.1°/29.6% against 34–42°/10.5–19.5%,
/// granite at 7.6% against 10–19%, the granite:turf separation at 1.44× against
/// the "roughly 2×" this comment claims, and litter — not grass — was the
/// darkest identity. The visual judge measured the consequence on the frames
/// without being able to see the cause: **hue 29–35° across the whole island
/// and zero pixels in §3's grass band.** The mechanism is that litter was the
/// most saturated identity on the island *and* it is warm, so it hijacked the
/// hue of every mix it appeared in — and it appears in 37.6% of the land.
///
/// Re-placed against §3 under four constraints, three of them from documents:
/// §3's hue and saturation exactly (both scale-invariant under a white light,
/// which is what lets an albedo be compared to a lit measurement at all);
/// §5's `[0.05, 0.55]` linear floor; and — the fourth, which is discipline
/// rather than a document — **the area-weighted mean linear luma is held.**
/// Brightness is the coupled-lighting owner's (`CLAUDE.md` traps: tonemap,
/// sky, exposure and fog are one owner, and splitting them across passes has
/// measurably made things worse). This pass changes what the ground IS, not
/// how bright it is.
///
/// ⚠ **Two of those three sentences were re-stated on 2026-08-15 and one was
/// wrong.** The mean was held at 0.09390 "to within 0.01%" — a quadrant's
/// number — and grass was placed on §5's floor rather than in §3's band, which
/// is what capped the granite:turf separation at 1.91×. Both are corrected
/// below.
///
/// ⚠ **Re-placed 2026-08-15 under the corrected weights, because two of the
/// four identities were the same paint.** The visual judge
/// (`pass-20260815-042118-10-visual.md` gap 1) measured the delivered ground at
/// hue 33–37°, saturation 23–24% and luma 96–113 at *every* sample of six
/// frames — one tan island — with ~0.4% of pixels reading as granite where
/// `ART.md` §0 records 8.9% of the land within 300 m of the capture spawn
/// carrying it. A probe over 34,806 land samples at that spawn found the cause
/// is **not** the classifier: `terrain::splat_from` delivers near-pure
/// identities (max weight p50 = 1.000, 92.2% of samples above 0.8) and
/// reproduces §0's granite share to the digit (8.89%). The cause was this
/// table. **Forest litter and granite were 1.0° apart in hue, 0.5 points in
/// saturation and 1.059× in value — 6.7 luma out of 255 — and those two
/// identities own 89.4% of the land inside that 300 m.** Granite was not
/// missing from the frame; it was painted as litter.
///
/// The re-place moves three of the four onto §3's own **luma** column, which
/// this table had previously read only for chroma:
///
/// | identity | was | now | §3 |
/// |---|---|---|---|
/// | beach sand | 94.2 | **117.0** | 117 |
/// | grass | 62.8 | **64.5** | 59–70 |
/// | forest litter | 113.3 | **102.8** | *no row* — absorbs the mean |
/// | granite | 120.0 | **147.0** | 127–167 |
///
/// Anchoring turf at its band's centre rather than at §5's floor is what makes
/// the other two land on §3's numbers verbatim: sand is §3's 117 and granite
/// §3's 147 *because* 117/64.5 and 147/64.5 are the document's own ratios.
/// Granite:turf therefore reaches **2.28×** — §3's own separation, which the
/// paragraph below correctly said was unreachable while the mean was pinned to
/// a quadrant's — and granite:litter goes 1.059× → **1.429×**, a gap of 6.7 →
/// 44.2 luma. Gated by `granite_stands_clear_of_the_ground_it_shares`.
///
/// **It is brightness-neutral by construction and that is the point.** The
/// area-weighted mean linear luma is held at the island's own **0.10746** —
/// the value these constants actually deliver, not the 0.09390 they were
/// pinned to, which was that same quadrant — so it moves by −0.024% and
/// `fill::bounce_albedo` by under 0.3%. Brightness is still the coupled
/// owner's (`CLAUDE.md` traps) and nothing here takes it: what changed is the
/// distribution across the four identities, not the total. What this pass does
/// NOT buy is *structure* — all four identities still share one greyscale
/// detail map and one `perceptual_roughness`, so granite has stone's value now
/// and not stone's surface. `NOW.md` carries that as the splat material.
///
/// The retracted paragraph, kept because the mistake is the lesson. This read:
/// "over
/// 39,521 land samples at seed 20260731 the mean splat is sand 0.008, grass
/// 0.619, litter 0.373, rock 0.0000. **Granite's value is therefore free** —
/// it is pinned to §3's ratio against litter and costs the mean nothing,
/// because granite never reaches the ground at all." The 39,521 is what
/// `-1024..1024` returns on a world that runs `0..2048`; see
/// [`super::fill::GROUND_MIX`] for the full retraction and
/// `crates/client/tests/ground_mix.rs` for the gate.
///
/// Whole-island the mix is sand 0.0113, grass 0.5186, litter 0.3801, **rock
/// 0.0900** (0.0916 until worldgen shape v1 re-measured it, 2026-08-26).
/// Granite is the third identity by area and the brightest of the
/// four, so its value was never free and the mean it was pinned to was 14.1%
/// low. **That is the constraint the table above is placed under**, and the
/// reason litter is the one identity whose value is derived rather than read:
/// granite's share is what it has to absorb.
pub const GROUND_ALBEDO: [[f32; 3]; 4] = [
    // beach sand — hue 42.0°, sat 10.0%, **luma 117.0** (§3 "beach sand — 117
    // luma, 42°, 10%", now read whole rather than for its chroma alone).
    [0.1895, 0.1775, 0.1513],
    // grass — hue 68.5°, sat 31.0% and **luma 64.5**: the centres of §3's
    // 63–74°, 29–33% and 59–70. Still the darkest identity, but it no longer
    // sits on `ALBEDO_LUMA_BAND`'s floor — it sits at its own band's centre,
    // 0.0526 linear, and clears §5's 0.05 with room. The old value read 62.8,
    // which is 0.04997 linear: marginally UNDER the floor the comment here
    // claimed it sat exactly on, and the reason it sat there was the old
    // mean rather than anything §3 says.
    [0.0526, 0.0574, 0.0281],
    // forest litter — hue 38.0°, sat 15.0% (§3 "dirt path — 139 luma, 38°,
    // 15%"). §3 sampled a bare compacted path, not needles under canopy, so
    // it pins this identity's hue and saturation and NOT its value; the value
    // is what absorbs the held-mean constraint, and it is the only one of the
    // four that is not §3's own number.
    [0.1505, 0.1335, 0.1069],
    // granite — hue 39.0°, sat 14.5%, **luma 147.0**, the centres of §3's
    // 35–43°, 10–19% and 127–167.
    [0.3238, 0.2888, 0.2299],
];

/// How far the damp band reaches **along the ground**, metres.
///
/// **Two bounds, and each one is the answer to the case the other gets wrong.**
/// The band ends at whichever is reached first, and the gate walks four bank
/// steepnesses to check that neither ever collapses:
///
/// - **Height alone** (what this shipped as first) makes the band a function of
///   the slope rather than of the water. On a beach at a 4% grade a 2.5 m band
///   is *sixty metres* of damp sand; on a 60% bank it is four. One of those is
///   a stain across the whole frame and the other is fine, and nothing about
///   the tide changed between them.
/// - **Run alone** fails at the other end: seven metres of horizontal run up a
///   cliff face is fourteen metres of wet rock.
///
/// Together the damp band is a few metres of ground on anything a player would
/// call a shore, which is what makes the meeting of land and water a gradient
/// instead of an edge. The gradient is the analytic one the mesh already has
/// for its normals, so the second bound costs nothing.
pub const WET_REACH_M: f32 = 7.0;

/// How far above sea level the ground still reads as wet, metres.
///
/// The land half of the reference's **shoreline wetness**
/// (`reference/WATER.md` §4), which they shipped twice: first as experimental
/// terrain-water blending to make the transition "more seamless", then as a
/// flag any object using their standard shaders could set. Ours is neither a
/// flag nor a shader — it is a modifier on the vertex colour the ground is
/// already drawn with, which is the size this can honestly be here.
///
/// It is also `ART.md` §5's one outstanding named material: *"a darker, more
/// saturated band at the waterline"*, which the browser client had as
/// `WET_RANGE` and the native ground never got.
pub const WET_BAND_M: f32 = 2.5;

/// What a soaked surface keeps of its dry value.
///
/// Wet sand is not sand with a filter on it: water fills the voids between
/// grains, light that would have scattered straight back out is refracted into
/// the pile and absorbed, and the surface loses roughly half its reflectance
/// while its remaining colour gets *more* saturated for the same reason. Both
/// halves are here; the third half — a wet surface is also smoother, so its
/// specular tightens — is `perceptual_roughness`, which is per-material and
/// cannot vary per vertex without the shader `RENDER.md` §8 owns.
pub const WET_VALUE: f32 = 0.55;
/// How much the remaining colour's chroma is stretched about its own luma.
pub const WET_SATURATION: f32 = 0.35;

/// The dark end of `ART.md` §5's `ALBEDO_LUMA_BAND`, restated here because
/// [`wetted`] is the one modifier in the client that can drive a surface
/// through it.
///
/// **It binds, and the gate found the case.** The darkest identity is grass at
/// 0.0543 Rec.709 luma (it was forest litter at 0.072, before the identities
/// were re-placed against `ART.md` §3 — see [`GROUND_ALBEDO`]); [`WET_VALUE`]
/// would take it to 0.030, under a floor that exists because no real material
/// is a black hole. Clamping here rather than weakening [`WET_VALUE`] keeps the
/// soak honest on sand — the identity that is actually at a waterline, which
/// lands at 0.178 and soaks its full 0.55 — while refusing to author a black
/// surface anywhere.
///
/// Grass no longer sits *on* the floor: the 2026-08-15 re-place moved it to
/// §3's own band centre (0.0526 linear, Rec.601), clear of §5's 0.05 rather
/// than a thousandth under it as the previous placement was. The clamp still
/// binds on wet turf, because 0.55 of 0.054 is under the floor either way —
/// what changed is that it is the *soak* that reaches the floor and not the
/// authored albedo. Sand, litter and granite all still soak in full.
pub const ALBEDO_LUMA_FLOOR: f32 = 0.05;

/// How wet the ground is here: 1 at or below sea level, 0 outside the band,
/// smooth between.
///
/// `slope` is the local rise/run — the analytic gradient the mesh is already
/// computing for its normals, so this costs nothing new. The band ends at
/// whichever bound is reached first: [`WET_REACH_M`] metres of horizontal run,
/// or [`WET_BAND_M`] metres of height.
///
/// Below the waterline it is exactly 1 rather than extrapolating: the seabed is
/// not "more than wet", and a curve that kept going would drive the sand black
/// at the sentinel depths the water grid uses.
pub fn wet_factor(y: f32, slope: f32) -> f32 {
    if y <= SEA_LEVEL {
        return 1.0;
    }
    let rise = y - SEA_LEVEL;
    let by_height = rise / WET_BAND_M;
    // `rise / slope` is the horizontal run back to sea level along a plane of
    // this gradient. A floor on the slope keeps a dead-flat pan from dividing
    // by zero and wetting the horizon; past it, `by_height` is the bound that
    // binds anyway.
    let by_run = rise / (slope.max(1e-3) * WET_REACH_M);
    let t = by_height.max(by_run).clamp(0.0, 1.0);
    let t = 1.0 - t;
    t * t * (3.0 - 2.0 * t)
}

/// Apply [`wet_factor`] to a LINEAR albedo: darker, and more saturated about
/// its own luma.
pub fn wetted(c: [f32; 3], wet: f32) -> [f32; 3] {
    if wet <= 0.0 {
        return c;
    }
    let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    // The soak, floored: a surface already at or under the band's dark end
    // does not get darker, and one above it may not be taken through.
    let value = if luma > ALBEDO_LUMA_FLOOR {
        (1.0 - wet * (1.0 - WET_VALUE)).max(ALBEDO_LUMA_FLOOR / luma)
    } else {
        1.0
    };
    let chroma = 1.0 + wet * WET_SATURATION;
    let mut out = [0.0f32; 3];
    for k in 0..3 {
        out[k] = ((luma + (c[k] - luma) * chroma) * value).max(0.0);
    }
    out
}

/// `1 / linear mean` of `rock_albedo.jpg`, per channel — the mean-placing
/// correction of `ART.md` §7, measured off the shipped file rather than
/// guessed. Its span (max/min) is 1.054, i.e. the correction stretches the
/// source's colour deviation by 5%, which is what the ×1 rule permits.
pub const ROCK_GAIN: [f32; 3] = [3.659, 3.713, 3.855];

/// One built ground chunk, so teardown can find it.
#[derive(Component)]
pub struct Chunk(pub i32, pub i32);

/// The static far mesh, which never streams. The sea used to carry this too
/// and no longer does: it is one eye-centred mesh now and re-centres like a
/// ring would (`render/water.rs`).
#[derive(Component)]
pub struct Static;

/// What the ring has built. A `HashMap` here is fine and would not be in
/// `sim-core`: the no-`HashMap`-iteration wall is about the deterministic
/// tick, and nothing in this file feeds one.
///
/// **Three of these fields exist because the build is off the main thread.**
/// `heightfield` is pure — it reads `sim_core::terrain`, touches no ECS and
/// allocates only what it returns — so it runs on `AsyncComputeTaskPool` and
/// this resource holds what is in flight. Two things had to be split when it
/// moved, and both were flags that were honest only while the build finished
/// inside the statement that started it:
///
///  - `far_started` guards the spawn; `far_done` is set when the mesh has
///    actually reached the world. `far_done` is what `far_ready` reports to
///    the loading bar, and a bar that read the OLD flag would end the loading
///    screen on the frame the work was queued — a player dropped into a world
///    with no island in it.
///  - `near_tasks` is the same guard for the ring. `built` was the only test
///    for "is this chunk handled", and it is written when the mesh exists, so
///    an async build leaves a window in which a key is neither built nor
///    skipped and the loop re-queues it every frame.
///
/// Dropping a `Task` cancels it, so `retain` sweeping `near_tasks` by the same
/// predicate as `built` is the whole teardown for a chunk that left the ring
/// before it finished.
#[derive(Resource, Default)]
pub struct Ring {
    built: HashMap<(i32, i32), Entity>,
    near_tasks: HashMap<(i32, i32), Task<Mesh>>,
    ground: Option<Handle<GroundMaterial>>,
    far_task: Option<Task<Mesh>>,
    far_started: bool,
    far_done: bool,
}

/// Chunks in a full near ring — what `is_full` is asserting against, and the
/// number a capture settles on rather than on a clock.
pub const RING_CHUNKS: usize = ((2 * NEAR_RADIUS + 1) * (2 * NEAR_RADIUS + 1)) as usize;

impl Ring {
    pub fn len(&self) -> usize {
        self.built.len()
    }
    pub fn is_empty(&self) -> bool {
        self.built.is_empty()
    }
    /// The far mesh is up and every near chunk is resident.
    ///
    /// Counts `built`, never `near_tasks`: a queued chunk is not a resident
    /// one, and the capture probe settles on this.
    pub fn is_full(&self) -> bool {
        self.far_done && self.built.len() >= RING_CHUNKS
    }
    /// Builds in flight — for a test that has to say "one task per chunk, ever".
    pub fn in_flight(&self) -> usize {
        self.near_tasks.len() + usize::from(self.far_task.is_some())
    }
    /// The far mesh alone. Read by the loading screen, which reports the near
    /// ring as a fraction and this as the bit it is: the whole island at 8 m
    /// is one build, not a stream, and a bar that folded it into the near
    /// ring's count would read full while the horizon was still missing.
    pub fn far_ready(&self) -> bool {
        self.far_done
    }
}

/// The ground's one material. Shared by every chunk so Bevy batches them into
/// one draw per pipeline; the identity variation rides the vertices and the
/// near-field grain rides the photograph.
///
/// **Four map sets now, one per identity** — landed 2026-08-15, and the
/// paragraph that stood here explaining why there could only be one is kept
/// below because it is still the reason the maps contribute LUMINANCE.
///
/// The old text: "A `StandardMaterial` has one base-colour slot, so the four
/// identities `terrain::splat` resolves cannot each carry their own photograph
/// here. The first cut picked `grass` because `MANIFEST.md` records it owning
/// ~99% of the near ring — and the capture measured near-band saturation
/// falling 32.5% → 15.0% against a reference of 33.2%, because
/// `base_color_texture` MULTIPLIES the authored colour by the photograph's own
/// colour, which is `ART.md` §7's named failure: a modifier that must set a
/// colour multiplies the surface's mean-1 LUMINANCE field, not its chroma."
///
/// **The slot limit is gone and the §7 rule is not.** `super::ground_splat`
/// binds four albedo and four normal maps through an `ExtendedMaterial`, so the
/// one-slot constraint is retired; but the deviation rule still binds, and the
/// spans measured over the four ground sources are why every map still arrives
/// as a luminance field rather than as colour:
///
/// | source | linear mean rgb | gain span | albedo sd |
/// |---|---|---|---|
/// | grass | 0.291 0.249 0.119 | **2.454** | 0.0743 |
/// | sand | 0.228 0.174 0.110 | **2.073** | 0.0480 |
/// | litter | 0.139 0.099 0.039 | **3.586** | 0.0527 |
/// | rock | 0.273 0.269 0.259 | **1.054** | 0.0924 |
///
/// Only `rock` clears the rule. Reducing each source to its own mean-1
/// luminance field gives every one of them a span of 1.000 by construction, so
/// all four may now ship their relief where before only granite's could — which
/// is the whole of what this slice buys. The colour stays entirely the authored
/// splat's, exactly as §7 asks.
///
/// **What the old note said was blocking it, and what actually was.** It read:
/// "Four maps blended by the splat weights needs a custom material, and in 0.18
/// `StandardMaterial` is `#[bindless(index_table(range(0..31)))]`". True, and
/// the way through is an extension that does *not* declare `#[bindless]`, which
/// forces the whole `ExtendedMaterial` non-bindless — see `ground_splat.rs`.
fn ground_material(
    materials: &mut Assets<GroundMaterial>,
    maps: &GroundMaps,
) -> Handle<GroundMaterial> {
    materials.add(ExtendedMaterial {
        base: StandardMaterial {
            // **Every texture slot here is deliberately empty and the base
            // colour is white.** The extension's fragment shader assigns
            // `base_color`, `perceptual_roughness` and `N` outright, so anything
            // set here would be computed and then thrown away — and a reader
            // would reasonably believe it was in the frame.
            base_color: Color::WHITE,
            // **The roughness maps are read now** (2026-08-16), and the note
            // that stood here is worth keeping as the shape of the mistake: the
            // reason recorded for four days was that `metallic_roughness_texture`
            // is a glTF-packed ORM slot whose B channel is metallic, so binding
            // a greyscale roughness jpg would make the ground a half-metal. That
            // is a constraint of **that slot**, not of the files, and it stopped
            // applying the moment a custom shader sampled them directly. It cost
            // four texture bindings and no ORM packing step at all
            // (`render/ground_splat.rs` 110–113).
            //
            // ⚠ **The same false reason is still recorded in `render/props.rs`,
            // where the five PROP roughness maps are still unread** — and it is
            // false there for a second, independent mechanism: Bevy computes
            // `metallic *= metallic_roughness.b`, a MULTIPLY against this very
            // field, which `StandardMaterial::default()` leaves at 0.0. A
            // greyscale map in that slot cannot make anything metal while
            // `metallic` is zero. What the prop half actually needs is a
            // decision about LEVEL (`perceptual_roughness` is a multiplier
            // there, not a replacement), which is why it is a separate slice
            // and not a slot assignment either. `NOW.md` carries it.
            metallic: 0.0,
            // `fresnel::DIELECTRIC`, not the 0.18 this shipped with. That
            // number put F0 at 0.52% against a dielectric's 4%, which is why
            // `ground_splat`'s four per-texel roughness maps measured as a
            // null result: they were shaping a lobe with an eighth of its
            // energy in it. `DECISIONS.md` §open, ground specular v0.
            reflectance: super::fresnel::DIELECTRIC,
            ..default()
        },
        extension: GroundSplat::new(maps),
    })
}

/// One ground vertex's colour: the splat identity mix, the macro break-up,
/// then the waterline — in that order, which is the whole of what the order
/// buys (see the comments inside).
///
/// Split out of [`heightfield`]'s loop so the tap-sharing gate can compare the
/// two builds without reaching for `props::hash01`: what that gate is about is
/// *which points were sampled*, and this is the part that is the same either
/// way.
pub fn vertex_color(y: f32, w: [u8; 4], x: f32, z: f32, grad: f32) -> [f32; 4] {
    let s = vertex_splat(w);
    let m = vertex_mods(y, x, z, grad);
    // The identity mix, from the same `splat` the browser's material was fed by.
    let mut c = [0.0f32; 3];
    for (k, f) in s.iter().enumerate() {
        for ch in 0..3 {
            c[ch] += GROUND_ALBEDO[k][ch] * f;
        }
    }
    // The break-up, then the waterline — the order is the whole of what it buys.
    let c = wetted([c[0] * m[0], c[1] * m[0], c[2] * m[0]], m[1]);
    [c[0], c[1], c[2], 1.0]
}

/// The four splat weights as floats, in `terrain::splat`'s order — sand ·
/// grass · litter · rock.
///
/// **`/ 255`, not normalised to sum 1.** `splat_from` returns four `u8`
/// summing to *approximately* 255, and dividing is what [`vertex_color`] has
/// always done; re-normalising here would change the delivered colour on every
/// vertex whose weights sum to 254 or 256, which is a silent balance edit
/// wearing a refactor's clothes.
pub fn vertex_splat(w: [u8; 4]) -> [f32; 4] {
    let inv = 1.0 / 255.0;
    [
        w[0] as f32 * inv,
        w[1] as f32 * inv,
        w[2] as f32 * inv,
        w[3] as f32 * inv,
    ]
}

/// The two per-vertex scalar modifiers the splat material needs at the
/// fragment: the macro break-up, and the waterline.
///
/// **Why these two travel as scalars and the colour does not.** Both are
/// modifiers on whatever identity the splat resolves rather than identities
/// themselves, so each is one number per vertex — which is exactly what fits in
/// `ATTRIBUTE_UV_1`'s two floats, and what lets the four weights have
/// `ATTRIBUTE_COLOR` to themselves.
///
/// Texture UV per metre of world — the ground's planar XZ projection.
///
/// One 1024² photograph therefore covers **4 m** and repeats 512 times per
/// island side. It was a bare `0.25` at the one call site until 2026-08-27;
/// it is named because `ground_splat.wgsl`'s biplanar wall tap has to build
/// its own UV at exactly this scale, and two copies of a projection constant
/// in two languages is the drift `CLAUDE.md` warns about. The shader reads it
/// from the uniform (`GroundSplatParams::wall.z`) rather than repeating it.
pub const UV_PER_M: f32 = 0.25;

/// Wavelength of the tile break-up, metres.
///
/// **This exists because the ground texture repeats every 4 m and nothing else
/// hides it.** `heightfield` writes a planar UV of `world.xz × 0.25`, so one
/// 1024² photograph covers 4 m and repeats **512 times per island side**, on a
/// rigid axis-aligned lattice; `ground_splat.wgsl` samples all sixteen maps at
/// that one UV with no rotation, no second scale and no stochastic tiling. Any
/// low-frequency content in a source — and every photograph has some — then
/// reads as a quilt at exactly 4 m. Measured over the four shipped albedos, the
/// sd of an 8×8 box downsample runs 1.3% of the mean (grass, which tiles
/// cleanly) to 4.1% (the old stratified `rock`, which did not).
///
/// 48 m is twelve tiles: far enough above the repeat that the two cannot beat
/// against each other, and small enough that a player standing still sees
/// several of them rather than one flat wash.
pub const MACRO_M: f32 = 48.0;

/// Peak deviation of that field, as a fraction of the surface's own colour.
pub const MACRO_AMP: f32 = 0.13;

/// Smooth value noise on a [`MACRO_M`] lattice, mean 0, in about [-1, 1].
///
/// Quintic-faded rather than linearly interpolated, for the reason
/// `sim-core/tests/contour.rs` exists: a C⁰ field has a slope step at every
/// lattice line, and a slope step in something the eye integrates across is a
/// visible seam. This one multiplies a colour rather than a height, so it
/// cannot reach the analytic normal — but a 48 m grid of faint creases in the
/// ground's brightness is the same defect wearing different clothes, and the
/// fade costs two multiplies.
fn macro_noise(x: f32, z: f32) -> f32 {
    let (fx, fz) = (x / MACRO_M, z / MACRO_M);
    let (x0, z0) = (fx.floor(), fz.floor());
    let (tx, tz) = (fx - x0, fz - z0);
    let (ix, iz) = (x0 as i32, z0 as i32);
    let c = |dx: i32, dz: i32| super::props::hash01((ix + dx) as u32, (iz + dz) as u32) * 2.0 - 1.0;
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v) = (fade(tx), fade(tz));
    let a = c(0, 0) + (c(1, 0) - c(0, 0)) * u;
    let b = c(0, 1) + (c(1, 1) - c(0, 1)) * u;
    a + (b - a) * v
}

/// Rule 1: no surface may be one flat value. `[0]` is the MACRO break-up
/// (0.5–1 m) — the near ring's vertices are 1 m apart, so one hash per vertex
/// is exactly that scale. The near-field grain under 5 cm is the photograph's
/// job and the splat material is where it finally lands.
///
/// **Two scales, and they do different jobs.** The per-vertex hash is white
/// noise at the vertex spacing — 1 m near, 8 m far — which satisfies rule 1 and
/// is useless against a 4 m texture repeat, because it is neither correlated
/// across a tile nor larger than one. [`macro_noise`] is the one that breaks
/// the lattice. Both are mean 1 by construction (the hash term is
/// `0.88 + 0.24u`, mean exactly 1.00; the macro term is `1 + A·n` with `n`
/// mean 0), so their product is too and no gate that averages the island's
/// albedo moves — `fill::GROUND_MIX` and the bounce are folded from the splat
/// weights and the authored identities, and neither reads this slot.
pub fn vertex_mods(y: f32, x: f32, z: f32, grad: f32) -> [f32; 2] {
    let dither = 0.88 + 0.24 * super::props::hash01(x.to_bits(), z.to_bits());
    [
        dither * (1.0 + MACRO_AMP * macro_noise(x, z)),
        wet_factor(y, grad),
    ]
}

/// Build one heightfield patch. `n` vertices a side, `step` metres apart,
/// origin at its minimum corner.
///
/// **A 65² near chunk cost 28 ms to build and now costs 5.4** (medians,
/// release, on the gate box; the 257² far mesh went 485 ms → 186). [`stream`]
/// builds one chunk per frame, so 28 ms was a dropped frame every time the
/// near ring advanced — twenty-five of them on a join, five more every time
/// the player crossed a chunk edge. Two halves, both measured here against
/// what they replaced:
///
/// - **Nine `terrain::height` taps a vertex became three, bit-identically**
///   (15.9 ms → 5.4). Adjacent vertices were already sampling each other's
///   points and nobody was keeping the answers. (The root Cargo.toml calls a
///   chunk "4,225 `terrain::height` taps", which is the vertex count; it was
///   nine times that.)
/// - **The tangent is written rather than solved** (mikktspace, 12 ms on a
///   near chunk and 229 on the far mesh, gone). See the note at the bottom of
///   this function — that one is a near-equality, not an identity, and
///   `tests/ground.rs` bounds it.
///
/// Three shares, and **each one is checked at this origin rather than
/// assumed**, because every one of them is an f32 identity that holds for the
/// coordinates we ship and need not hold for coordinates we do not:
///
/// - the normal's `±d` arms land on a half-lattice, so vertex `k−1`'s `+d` is
///   vertex `k`'s `−d` — one row of `n+1` taps for `2n` reads (`share_x`), and
///   one row of `n` carried down into the next row (`share_z`);
/// - `terrain::slope`'s arm is a fixed 1 m, so at the near ring's 1 m pitch
///   its four taps ARE the four neighbouring vertices (`grid_slope`), which a
///   three-row rolling window with a border column already holds. The far mesh
///   is 8 m apart and pays the four (its share is the two above);
/// - `splat` re-derives the height it is standing on; `splat_from` takes the
///   one already in hand.
///
/// Any share whose identity fails falls back to the direct taps, so the
/// function stays correct for an origin, pitch or size no caller has asked
/// for yet. `tests/ground.rs` gates the equality against a naive rebuild.
#[allow(clippy::too_many_arguments)]
pub fn heightfield(
    seed: u64,
    haven: &terrain::Haven,
    ox: f32,
    oz: f32,
    n: usize,
    step: f32,
    drop: f32,
) -> Mesh {
    // One memo for the whole patch. Every tap this function takes sits inside
    // the patch it is building, and the coarsest lattice `terrain::height`
    // reads is 1,200 m across — so a 64 m chunk resolves under a hundred
    // distinct corner quads and draws them thirteen thousand times. It is a
    // stack local rather than a parameter because the entry point is what
    // `tests/ground.rs` calls with seven arguments and the win is inside one
    // call, not across calls (measured: sharing one table across a whole tile
    // ring bought nothing over one table per unit of work).
    let mut lat = terrain::Lattice::new();
    let count = n * n;
    let mut positions = Vec::with_capacity(count);
    let mut normals = Vec::with_capacity(count);
    let mut colors = Vec::with_capacity(count);
    let mut uvs = Vec::with_capacity(count);
    let mut mods = Vec::with_capacity(count);
    let mut tangents = Vec::with_capacity(count);

    // The central-difference arm. Half a step keeps the gradient local to the
    // quad it shades without sampling inside its own vertex.
    let d = (step * 0.5).max(0.5);

    // Every world coordinate below is one of these two, written the way the
    // naive loop wrote it — a share is only legal when two of these
    // expressions are equal to the bit, which is what the guards check.
    let vx = |ix: usize| ox + ix as f32 * step;
    let vz = |iz: usize| oz + iz as f32 * step;

    let share_x = (1..n).all(|k| vx(k - 1) + d == vx(k) - d);
    let share_z = (1..n).all(|k| vz(k - 1) + d == vz(k) - d);
    let grid_slope = (1..n).all(|k| {
        vx(k - 1) + 1.0 == vx(k)
            && vx(k) - 1.0 == vx(k - 1)
            && vz(k - 1) + 1.0 == vz(k)
            && vz(k) - 1.0 == vz(k - 1)
    });

    // A row of vertex heights with one border column each side, so a vertex on
    // the patch edge can still read its `slope` neighbour. Column `0` and
    // column `stride - 1` are written as the naive `x ± 1.0`; the interior is
    // the vertex lattice.
    let stride = n + 2;
    let col_x = |k: usize| {
        if k == 0 {
            vx(0) - 1.0
        } else if k == stride - 1 {
            vx(n - 1) + 1.0
        } else {
            vx(k - 1)
        }
    };
    let row_z = |j: usize| {
        if j == 0 {
            vz(0) - 1.0
        } else if j == stride - 1 {
            vz(n - 1) + 1.0
        } else {
            vz(j - 1)
        }
    };
    // `&mut Lattice` as a parameter rather than a capture: the closure is
    // called between other borrows of the same table.
    let fill_row = |lat: &mut terrain::Lattice, dst: &mut Vec<f32>, j: usize| {
        dst.clear();
        let z = row_z(j);
        for k in 0..stride {
            // The border columns are only ever read on the `grid_slope` path.
            dst.push(if grid_slope || (k > 0 && k < stride - 1) {
                terrain::ground_memo(lat, seed, haven, col_x(k), z)
            } else {
                0.0
            });
        }
    };

    let mut hprev: Vec<f32> = Vec::with_capacity(stride);
    let mut hcur: Vec<f32> = Vec::with_capacity(stride);
    let mut hnext: Vec<f32> = Vec::with_capacity(stride);
    if grid_slope {
        fill_row(&mut lat, &mut hprev, 0);
    }
    fill_row(&mut lat, &mut hcur, 1);
    // The normal's arms: `hxm[k]` is `x_k − d` (and `hxm[n]` the last `+ d`);
    // `hzp` is this row's `+ d`, which becomes the next row's `hzm`.
    let mut hxm = vec![0.0f32; n + 1];
    let mut hzm = vec![0.0f32; n];
    let mut hzp = vec![0.0f32; n];

    for iz in 0..n {
        let z = vz(iz);
        if grid_slope {
            fill_row(&mut lat, &mut hnext, iz + 2);
        }
        if share_x {
            for (k, slot) in hxm.iter_mut().enumerate() {
                let sx = if k == n { vx(n - 1) + d } else { vx(k) - d };
                *slot = terrain::ground_memo(&mut lat, seed, haven, sx, z);
            }
        }
        if share_z && iz > 0 {
            core::mem::swap(&mut hzm, &mut hzp);
        } else {
            for (ix, slot) in hzm.iter_mut().enumerate() {
                *slot = terrain::ground_memo(&mut lat, seed, haven, vx(ix), z - d);
            }
        }
        for (ix, slot) in hzp.iter_mut().enumerate() {
            *slot = terrain::ground_memo(&mut lat, seed, haven, vx(ix), z + d);
        }

        for ix in 0..n {
            let x = vx(ix);
            let y = hcur[ix + 1];
            positions.push([x, y - drop, z]);

            // Analytic normal: the surface gradient, not the triangulation.
            let hx = if share_x {
                hxm[ix + 1] - hxm[ix]
            } else {
                terrain::ground_memo(&mut lat, seed, haven, x + d, z)
                    - terrain::ground_memo(&mut lat, seed, haven, x - d, z)
            };
            let hz = hzp[ix] - hzm[ix];
            let n_v = Vec3::new(-hx, 2.0 * d, -hz).normalize();
            normals.push([n_v.x, n_v.y, n_v.z]);

            // The tangent, analytically, for the same reason the normal is
            // analytic — and it is the same gradient, so it is nearly free.
            // The UVs below are a planar XZ projection at a constant scale, so
            // `∂P/∂u` is the surface direction with no Z in it: `(2d, hx, 0)`.
            // That is exactly orthogonal to `n_v` by construction — their dot
            // is `−2d·hx + 2d·hx` — which is what the shader's mikktspace
            // frame wants and what a Gram-Schmidt step would otherwise cost.
            // `w = 1` is mikktspace's own answer for this parameterisation,
            // kept rather than re-derived: see the module note.
            let t_v = Vec3::new(2.0 * d, hx, 0.0).normalize();
            tangents.push([t_v.x, t_v.y, t_v.z, 1.0]);

            // `terrain::ground_slope`'s own body, over taps already in hand.
            //
            // ⚠ The fallback must be `ground_slope` and NOT `slope`: the taps
            // in `hcur`/`hnext`/`hprev` are `terrain::ground`'s, so the fast
            // branch computes a gradient of the CARVED surface, and a raw
            // `slope` here would make one vertex in a chunk shade against a
            // different island than its neighbour. The two branches are one
            // claim — "the gradient of the ground this mesh is drawing" — and
            // the only difference between them is whether the taps were
            // already in hand.
            let sl = if grid_slope {
                let sx = (hcur[ix + 2] - hcur[ix]) * 0.5;
                let sz = (hnext[ix + 1] - hprev[ix + 1]) * 0.5;
                (sx * sx + sz * sz).sqrt()
            } else {
                terrain::ground_slope_memo(&mut lat, seed, haven, x, z)
            };

            // `splat_from` rather than `splat` because the height and the
            // slope are the ones this vertex just resolved; `splat` would
            // sample both again.
            let w = terrain::splat_from(y, terrain::moisture_memo(&mut lat, seed, x, z), sl);
            // The gradient the normal was just built from, as a rise/run — the
            // waterline band is a horizontal distance and this is what converts
            // it. Free: `hx` and `hz` are already in hand.
            let grad = ((hx * hx + hz * hz).sqrt()) / (2.0 * d);
            // **`COLOR` carries the four weights, not the resolved colour.**
            // The colour is resolved per-PIXEL now (`ground_splat.wgsl`), which
            // is what lets each identity carry its own photograph;
            // `vertex_color` stays as the reference arithmetic the gate holds
            // the shader against.
            colors.push(vertex_splat(w));
            mods.push(vertex_mods(y, x, z, grad));
            uvs.push([x * UV_PER_M, z * UV_PER_M]);
        }

        if grid_slope {
            // Roll the window: this row's `+1 m` is the next row's centre.
            core::mem::swap(&mut hprev, &mut hcur);
            core::mem::swap(&mut hcur, &mut hnext);
        } else if iz + 1 < n {
            fill_row(&mut lat, &mut hcur, iz + 2);
        }
    }

    let mut indices = Vec::with_capacity((n - 1) * (n - 1) * 6);
    for iz in 0..n - 1 {
        for ix in 0..n - 1 {
            let a = (iz * n + ix) as u32;
            let b = a + 1;
            let c = a + n as u32;
            let dd = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, dd]);
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    // The two scalar modifiers. `UV_1` because it is the one remaining
    // interpolated slot Bevy's standard vertex stage already forwards to the
    // fragment (`forward_io::VertexOutput::uv_b`) — no custom vertex shader,
    // and `ATTRIBUTE_TANGENT` and the normal path stay exactly as they were.
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, mods);
    mesh.insert_indices(Indices::U32(indices));
    // Tangents, because a normal map without them is not a normal map: Bevy's
    // PBR shader needs `ATTRIBUTE_TANGENT` to build the tangent frame, and
    // without it the map is ignored and the surface silently stays flat — the
    // failure that looks like "the texture did not load".
    //
    // **Written, not solved.** `mesh.generate_tangents()` ran mikktspace over
    // the triangles and was, once the tap sharing above landed, the single
    // most expensive thing the client did — **12 ms of a 17 ms near chunk and
    // 229 ms of a 415 ms far mesh** (medians; the near figure varied 12–18 ms
    // run to run) — to re-derive a frame this parameterisation has in closed
    // form. `tests/ground.rs` holds the two builds side by side: the written
    // tangent is within **0.008° mean / 1.3° worst** of mikktspace's on the
    // near ring, 0.06° / 4.9° on the far mesh's 8 m triangles. That is the
    // same triangulation-vs-analytic difference this module's header already
    // resolved in the analytic direction for normals, and it is a NEAR
    // equality rather than the bit-identity the tap sharing gets — which is
    // why the gate states an angle instead of comparing bits.
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
    mesh
}

/// Stream the near ring, and build the far mesh once.
pub fn stream(
    mut commands: Commands,
    mut ring: ResMut<Ring>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GroundMaterial>>,
    world: Res<WorldId>,
    maps: Res<GroundMaps>,
    eye: Res<Eye>,
) {
    let ground = ring
        .ground
        .get_or_insert_with(|| ground_material(&mut materials, &maps))
        .clone();
    let pool = AsyncComputeTaskPool::get();
    let (seed, haven) = (world.seed, world.haven);

    // ── Land whatever finished ────────────────────────────────────────────
    //
    // Polled at the TOP, so a mesh that completed while the last frame was
    // drawn reaches the world on this one rather than a frame later.
    if let Some(task) = ring.far_task.as_mut() {
        if let Some(mesh) = block_on(future::poll_once(task)) {
            ring.far_task = None;
            commands.spawn((
                WorldEntity,
                Static,
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(ground.clone()),
                Transform::IDENTITY,
            ));
            // Only NOW. `far_ready` is what ends the loading screen.
            ring.far_done = true;
        }
    }

    // The far mesh, once. It is 66 k vertices and it is the whole island —
    // one ~190 ms build, which used to be one ~190 ms FRAME with the session
    // pump inside it. It is queued on the first streaming frame and landed
    // whenever it finishes; the window stays up and the shard stays pumped
    // because neither is on the thread doing the work any more.
    if !ring.far_started {
        ring.far_started = true;
        ring.far_task =
            Some(pool.spawn(async move {
                heightfield(seed, &haven, 0.0, 0.0, FAR_N, FAR_STEP, FAR_DROP)
            }));
    }

    let cx = (eye.pos.x / CHUNK_M).floor() as i32;
    let cz = (eye.pos.z / CHUNK_M).floor() as i32;

    // Near chunks that finished. Bounded per frame for the same reason the
    // BUILDS are: `meshes.add` uploads a chunk and a frame that landed
    // twenty-five of them would trade the build spike for an upload one.
    // Found rather than collected: the budget is one, and a `Vec` here would
    // be a per-frame heap allocation — the exact thing the rest of this pass
    // took out of `decal::fade` and `ghost::track`.
    for _ in 0..CHUNK_LANDS_PER_FRAME {
        let Some(key) = ring
            .near_tasks
            .iter()
            .find(|(_, t)| t.is_finished())
            .map(|(k, _)| *k)
        else {
            break;
        };
        let Some(mut task) = ring.near_tasks.remove(&key) else {
            break;
        };
        let Some(mesh) = block_on(future::poll_once(&mut task)) else {
            // `is_finished` said yes and the poll said no — put it back rather
            // than dropping it, because dropping a `Task` CANCELS the work,
            // and this loop would then look for the same chunk again forever.
            ring.near_tasks.insert(key, task);
            break;
        };
        let e = commands
            .spawn((
                WorldEntity,
                Chunk(key.0, key.1),
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(ground.clone()),
                Transform::IDENTITY,
            ))
            .id();
        ring.built.insert(key, e);
    }

    // Stream out first: a ring that grows before it shrinks peaks at both
    // rings resident, which is the teardown spike in its other form.
    let mut dropped = 0usize;
    ring.built.retain(|(bx, bz), e| {
        if dropped >= CHUNK_BUILDS_PER_FRAME
            || ((*bx - cx).abs() <= NEAR_RADIUS && (*bz - cz).abs() <= NEAR_RADIUS)
        {
            return true;
        }
        dropped += 1;
        commands.entity(*e).despawn();
        false
    });
    // A chunk that left the ring before its build finished. Dropping the
    // `Task` cancels it, which is the whole teardown — and it is not optional:
    // without it a player walking a straight line accumulates one dead task
    // per chunk crossed, each still holding the pool.
    ring.near_tasks
        .retain(|(bx, bz), _| (*bx - cx).abs() <= NEAR_RADIUS && (*bz - cz).abs() <= NEAR_RADIUS);

    let mut queued = 0usize;
    for dz in -NEAR_RADIUS..=NEAR_RADIUS {
        for dx in -NEAR_RADIUS..=NEAR_RADIUS {
            if queued >= CHUNK_BUILDS_PER_FRAME {
                return;
            }
            let key = (cx + dx, cz + dz);
            // BOTH, and the second half is what stops the storm: `built` is
            // written when the mesh exists, so between the queue and the land
            // a key is in neither map and the loop would re-queue it on every
            // frame of that window.
            if ring.built.contains_key(&key) || ring.near_tasks.contains_key(&key) {
                continue;
            }
            let ox = key.0 as f32 * CHUNK_M;
            let oz = key.1 as f32 * CHUNK_M;
            let step = CHUNK_M / (NEAR_N - 1) as f32;
            ring.near_tasks.insert(
                key,
                pool.spawn(async move { heightfield(seed, &haven, ox, oz, NEAR_N, step, 0.0) }),
            );
            queued += 1;
        }
    }
}
