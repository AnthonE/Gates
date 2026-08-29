//! The sea: a graded volume with a swell on it, not a blue plate.
//!
//! **Bevy draws; it does not decide.** Nothing here is gameplay — the sim's
//! only fact about water is `terrain::SEA_LEVEL` and the swim rule in
//! `TERRAIN.md` §3, and the surface drawn here is deliberately not consulted
//! by either. A wave that pushed a player would be sim state in the ECS.
//!
//! ## What this is, and the order it was built in
//!
//! `reference/WATER.md` §1 is the reference game's own published build order
//! for the same problem, and this file follows it because their order is the
//! surprising one: **a surface, then its optics, then its motion, then its
//! reflections, then its foam** — waves third, not first. So:
//!
//! 1. **The surface** is one eye-centred mesh with a uniform core and a
//!    geometric skirt, reaching past the island in every direction. One mesh
//!    rather than a near grid plus a far plane, because two translucent
//!    surfaces that overlap blend twice and the sea would darken along a seam
//!    that moves with the player.
//! 2. **The optics are a volume** (§2): colour and alpha both come from
//!    `exp(-depth · extinction)` — the reference's "depth-based colour
//!    extinction and thickness-based visibility" are one arithmetic here
//!    because they are one physical fact. The old plane was one colour at one
//!    alpha, which is what §2 replaced there.
//! 3. **The motion** is a sum of four directional waves with analytic
//!    gradients, each retired where the mesh under it is too coarse to carry
//!    it (see [`band_weight`]) — the same "retire an octave on its own
//!    footprint" law the ground's material already runs on (`TERRAIN.md` §4).
//!    Below the shortest of them the detail is a tiling ripple normal map that
//!    scrolls (see [`ripple_map`]).
//! 4. **Reflections are not built**, and that is §5 and §6 read together:
//!    reflections are the expensive half of water in the reference's own
//!    settings screen, and the payoff they name is putting the *sky* into the
//!    reflection — which a `StandardMaterial` under Bevy's atmosphere already
//!    gets from its specular term.
//! 5. **Foam** is a band *standing off* the waterline, slope-weighted (their
//!    published sentence about their own ocean foam is that it is "mostly
//!    visible on terrain slopes with higher inclination", §4), with its
//!    contour displaced by noise and its level surged by the swell. Every one
//!    of those three is there to stop the shore reading as a drawn line — see
//!    [`FOAM_PEAK_M`], [`FOAM_JITTER_M`] and [`foam_surge`]. The land half of
//!    the same job is `terrain_mesh::wet_factor`.
//!
//! ## What still makes a hard edge, and what would fix it
//!
//! The alpha ramp here is a *vertex* quantity: it grades on the water column
//! read off `terrain::height` at 2 m spacing and interpolated across the
//! triangle. Against the terrain that is a good approximation of a per-pixel
//! depth fade, because the terrain is the thing it sampled. Against anything
//! else it knows nothing — a boulder, a foundation or a player standing in the
//! shallows gets a hard ring where the sea meets it, because no vertex of this
//! mesh has ever heard of them.
//!
//! The fix is the standard one and it is a shader: sample the depth prepass in
//! the fragment, take the difference between the scene depth and the water's
//! own, and fade alpha and add foam as it goes to zero. That needs an
//! `ExtendedMaterial` and the first WGSL in the tree (`RENDER.md` §8), which is
//! its own slice; it is written down in `NOW.md` rather than half-built here.
//!
//! ## The frame cost, and where it is paid
//!
//! Two budgets, deliberately split:
//!
//! - **On a snap** (the eye crossing a [`SNAP_M`] cell): every vertex's world
//!   position, UV, depth and static colour are rebuilt. That is ~7.9 k
//!   `terrain::height` taps — less than the ground streamer already pays every
//!   frame for one 65² chunk and its normals, so it is not a new class of
//!   cost.
//! - **Every frame**: four sines per vertex and a write of position, normal
//!   and colour. No `terrain` taps, no allocation — the attribute buffers are
//!   built once and mutated in place, which is `CLAUDE.md`'s no-per-frame-
//!   allocation rule on the one system that runs every single frame.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::math::Affine2;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sim_core::terrain::{self, SEA_LEVEL};

use super::{Eye, WorldEntity, WorldId};

// ---------------------------------------------------------------------------
// The mesh: a uniform core with a geometric skirt.
// ---------------------------------------------------------------------------

/// Vertex spacing in the uniform core, metres.
///
/// It bounds the shortest wave the *mesh* can carry: [`band_weight`] retires a
/// wave whose quarter-wavelength is under the local spacing, so 2 m of core
/// admits everything at or above 8 m and `tests/water.rs` asserts the shortest
/// wave in [`WAVES`] clears it. Everything below that wavelength is the ripple
/// map's, which is a texture and does not care.
pub const STEP_M: f32 = 2.0;

/// Half-extent of the uniform core, metres. Past this the spacing grows
/// geometrically.
pub const CORE_M: f32 = 64.0;

/// How much each skirt step grows over the last. 1.7 puts the first skirt gap
/// at 3.4 m — barely coarser than the core, so the wave weights fade rather
/// than step at the join.
pub const SKIRT_RATIO: f32 = 1.7;

/// How far the mesh must reach, metres. The island is
/// `terrain::ISLAND_SIZE` across, so a player standing on one shore has to
/// have sea drawn past the far one; this is that diagonal with margin.
pub const SEA_REACH_M: f32 = 2600.0;

/// The grid re-centres on a lattice this coarse, metres.
///
/// **Snapping is what makes the depth cache legal.** An eye-centred grid that
/// followed the camera continuously would move every vertex every frame, so
/// every `terrain::height` tap would be stale every frame. Snapped, the world
/// positions are constant between crossings and the expensive half is paid
/// ~0.6 times a second at a walk.
pub const SNAP_M: f32 = 8.0;

/// How far out vertices carry a *sampled* depth. Past this the water is deep
/// by fiat.
///
/// Legal because the sea is drawn after the opaque pass and depth-tested: any
/// far vertex that is actually over land is behind the ground mesh and never
/// reaches the blend. What it must not do is read *shallow* over land it can
/// see past, which this radius keeps well outside.
pub const DEPTH_R_M: f32 = 220.0;

/// The depth a vertex past [`DEPTH_R_M`] is given. Any value that saturates
/// [`transmittance`] will do; this one is finite and five times the sea
/// floor's own depth, which keeps every curve in this file on ordinary
/// arithmetic.
pub const DEEP_SENTINEL_M: f32 = 64.0;

// ---------------------------------------------------------------------------
// The waves.
// ---------------------------------------------------------------------------

/// One directional wave. `dir` is a unit vector in world XZ — asserted, not
/// assumed (`tests/water.rs`).
#[derive(Clone, Copy)]
pub struct Wave {
    pub dir: [f32; 2],
    /// Wavelength, metres.
    pub len_m: f32,
    /// Amplitude, metres. Peak displacement is [`SHAPE_PEAK`] times this.
    pub amp_m: f32,
}

/// How many waves the swell is made of.
pub const WAVE_COUNT: usize = 4;

/// The swell (`DECISIONS.md` §open, "water v0").
///
/// Four lengths roughly an octave apart, on four bearings inside a ~50° fan —
/// a real sea state is a narrow-ish directional spectrum, and four wave trains
/// crossing at right angles read as a bathtub. The directions are authored as
/// unit vectors rather than as angles because nothing in this repo takes a
/// trig call it can avoid taking offline.
pub const WAVES: [Wave; WAVE_COUNT] = [
    Wave {
        dir: [0.9397, 0.3420],
        len_m: 52.0,
        amp_m: 0.42,
    },
    Wave {
        dir: [0.7660, 0.6428],
        len_m: 31.0,
        amp_m: 0.26,
    },
    Wave {
        dir: [0.9848, -0.1736],
        len_m: 17.0,
        amp_m: 0.13,
    },
    Wave {
        dir: [0.6428, -0.7660],
        len_m: 9.0,
        amp_m: 0.06,
    },
];

/// Gravity, m/s². The wave speeds are **derived, not chosen**: a deep-water
/// gravity wave runs at `ω = sqrt(g·k)`, so a 52 m swell has a 5.8 s period
/// and a 9 m chop has a 2.4 s one, and the long waves outrun the short ones by
/// construction. Picking four speeds by eye is how a sea ends up looking like
/// a scrolling texture.
pub const GRAVITY: f32 = 9.81;

/// Peak of [`shape`], in units of amplitude. `2u² − 0.75` at `u = 1`.
pub const SHAPE_PEAK: f32 = 1.25;
/// Peak of `d(shape)/dθ`, in units of amplitude·k. Maximised at `sin θ = 0.5`,
/// where `(1 + sin θ)·cos θ = 1.5·cos 30°`.
pub const SHAPE_MAX_SLOPE: f32 = 1.299_038;

/// A crest-sharpened sine and its derivative, both in θ.
///
/// `u = ½ + ½·sin θ`, `s = 2u² − ¾`. Squaring is the whole trick and it is
/// chosen for what it costs as much as for what it does: real water has sharp
/// crests and long flat troughs, `u²` produces exactly that asymmetry, and the
/// alternative — a fractional power — is a `powf` per wave per vertex, which
/// at 7.9 k vertices is the frame budget. The `¾` is the DC term that keeps
/// the mean at zero (`E[u²] = 3/8`), without which the sea would sit a couple
/// of decimetres below `SEA_LEVEL` and the shoreline would be drawn in the
/// wrong place.
pub fn shape(theta: f32) -> (f32, f32) {
    let (sin, cos) = theta.sin_cos();
    let u = 0.5 + 0.5 * sin;
    (2.0 * u * u - 0.75, 2.0 * u * cos)
}

/// Angular frequency of a deep-water wave of this length, rad/s.
pub fn omega(len_m: f32) -> f32 {
    let k = std::f32::consts::TAU / len_m;
    (GRAVITY * k).sqrt()
}

/// How much of a wave survives on a mesh with this vertex spacing.
///
/// Full at a quarter wavelength or finer, gone at a half, smooth between. **A
/// wave sampled at half its own wavelength is not a coarse wave, it is a
/// different wave**: it aliases into a long, slow, wrong one that crawls
/// across the horizon, and no amount of amplitude tuning removes it. This is
/// the ground material's own law — retire an octave on the footprint that
/// samples it, never on a distance (`TERRAIN.md` §4) — applied to geometry
/// instead of to a texture, and it is why the far sea flattens without a
/// single "fade the waves out past N metres" constant.
pub fn band_weight(spacing_m: f32, len_m: f32) -> f32 {
    let quarter = len_m * 0.25;
    let half = len_m * 0.5;
    if spacing_m <= quarter {
        return 1.0;
    }
    if spacing_m >= half {
        return 0.0;
    }
    let t = (half - spacing_m) / (half - quarter);
    t * t * (3.0 - 2.0 * t)
}

/// Depth at which the swell is at full height, metres.
pub const SHOAL_FULL_M: f32 = 3.5;

/// How much of the swell survives in water this deep.
///
/// Zero at the waterline, and that is a hard requirement rather than a taste:
/// a wave with amplitude left at zero depth lifts the surface above the beach
/// it is meeting, and the sea visibly climbs the sand and sits there. Real
/// water solves this by breaking; we have no breaker, so the amplitude goes to
/// zero and [`shore_foam`] is what stands in for the energy that went
/// somewhere. `reference/WATER.md` §3 has the reference's answer (two
/// simulations faded by depth) and §9.7 says why it is not ours.
pub fn shoal(depth_m: f32) -> f32 {
    let t = (depth_m / SHOAL_FULL_M).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The wave field: surface height above [`SEA_LEVEL`] and its world-XZ
/// gradient, at one point.
///
/// `phase` is per-wave and advanced by the caller, never `k·x − ω·t` off a
/// clock: an `f32` elapsed-seconds multiplied by a wavenumber loses its low
/// bits within an hour of play, and a session is longer than that. Advancing
/// each wave's own phase by `ω·dt` and wrapping it to `[0, τ)` is exact for
/// as long as the process lives.
///
/// **The gradient treats `weight` as locally constant**, which it is not —
/// the band weights and the shoaling fade both vary with position. They vary
/// over tens of metres where the waves vary over metres, so the error is in
/// the third decimal of a normal; `tests/water.rs` asserts the gradient
/// against finite differences at fixed weights, which is the property this
/// function actually claims.
pub fn wave_field(
    x: f32,
    z: f32,
    phase: &[f32; WAVE_COUNT],
    spacing_m: f32,
    shoal: f32,
) -> (f32, f32, f32) {
    let (mut h, mut gx, mut gz) = (0.0f32, 0.0f32, 0.0f32);
    for (i, w) in WAVES.iter().enumerate() {
        let weight = band_weight(spacing_m, w.len_m) * shoal;
        if weight <= 0.0 {
            continue;
        }
        let k = std::f32::consts::TAU / w.len_m;
        let (s, ds) = shape(k * (w.dir[0] * x + w.dir[1] * z) + phase[i]);
        let a = w.amp_m * weight;
        h += a * s;
        gx += a * ds * k * w.dir[0];
        gz += a * ds * k * w.dir[1];
    }
    (h, gx, gz)
}

/// [`wave_field`] for a whole grid, once, into `out` as `[h, gx, gz]`.
///
/// **The one function that decides which axis a coordinate belongs to**, and
/// that is why it exists instead of three copies of the loop inside
/// [`animate`]. `coords[ix]` is an X offset and `coords[iz]` is a Z one; the
/// grid is square and the wave set is not, so a transposed pair produces a
/// sea whose swell runs across its own gradient — a picture nothing here
/// measures and a person would have to notice. `tests/water.rs` pins the
/// mapping against a direct call.
///
/// `out` is the caller's buffer and is written for the first `n·n` entries;
/// anything shorter is a caller bug and panics on the index rather than
/// quietly leaving the tail of the sea on the previous frame's surface.
pub fn resolve_field(
    centre: Vec2,
    coords: &[f32],
    spacing: &[f32],
    shoal: &[f32],
    phase: &[f32; WAVE_COUNT],
    out: &mut [[f32; 3]],
) {
    let n = coords.len();
    for iz in 0..n {
        for ix in 0..n {
            let i = iz * n + ix;
            let x = centre.x + coords[ix];
            let z = centre.y + coords[iz];
            let sp = spacing[ix].max(spacing[iz]);
            let (h, gx, gz) = wave_field(x, z, phase, sp, shoal[i]);
            out[i] = [h, gx, gz];
        }
    }
}

/// The surface normal for a gradient out of [`wave_field`].
pub fn wave_normal(gx: f32, gz: f32) -> Vec3 {
    Vec3::new(-gx, 1.0, -gz).normalize()
}

// ---------------------------------------------------------------------------
// The optics.
// ---------------------------------------------------------------------------

/// Extinction per metre of water, per channel — the sea's colour, expressed as
/// the thing that causes it.
///
/// Red goes first and green goes furthest, which is *coastal* water rather
/// than open ocean (in clear blue water it is blue that penetrates). That is
/// the correct family for this island and it is also the reference's own
/// stated target: they retuned their ocean to "a more Atlantic/Baltic sea
/// colour" so it would look less out of place (`reference/WATER.md` §2). The
/// Baltic is green because of what is dissolved in it.
pub const EXTINCT: [f32; 3] = [0.62, 0.16, 0.22];

/// **Single-scattering albedo**: of the light [`EXTINCT`] takes out of a beam,
/// the fraction that comes back to the eye rather than being absorbed. `b/σ`
/// in the usual notation, and it is *the sea's colour* in the most direct
/// sense there is.
///
/// **This replaced a two-point lerp between a shallow and a deep body colour,
/// and the difference was visible in the first capture.** A lerp is not wrong
/// so much as backwards-compatible with a misconception: it treats extinction
/// as the thing that darkens the water, so more depth meant a darker body, and
/// the sea rendered as a grey sheet with no colour in it at any distance.
/// Extinction removes light from what is *behind* the water; what the water
/// itself sends you is scatter, and scatter **accumulates** with depth. Green
/// is highest because in coastal water red is nearly all absorption and blue
/// is eaten by what is dissolved in it — the same fact `EXTINCT` states from
/// the other side.
pub const SCATTER_ALBEDO: [f32; 3] = [0.018, 0.125, 0.100];

/// The most opaque the sea ever gets. Short of 1 on purpose: a fully opaque
/// water surface has no depth cue left at all, and the last few per cent of
/// transmittance are what keep a headland's underwater slope readable.
pub const ALPHA_MAX: f32 = 0.94;

/// Transmittance through `depth_m` of water, per channel.
pub fn transmittance(depth_m: f32) -> [f32; 3] {
    let d = depth_m.max(0.0);
    [
        (-d * EXTINCT[0]).exp(),
        (-d * EXTINCT[1]).exp(),
        (-d * EXTINCT[2]).exp(),
    ]
}

/// Colour and alpha for water this deep, as **premultiplied** LINEAR rgba —
/// which is what the vertex buffer carries and what the material's
/// `AlphaMode::Premultiplied` expects.
///
/// The whole optical model, in three lines, and each line is one of the
/// reference's two named mechanisms or the thing that makes them one:
///
/// - `S_c·(1 − T_c)` is the water's own colour: **depth-based colour
///   extinction**, per channel, accumulating with depth.
/// - `α = 1 − mean(T)` is **thickness-based visibility**: how much of what is
///   behind the surface the water has eaten.
/// - Together they are `src + (1 − α)·dst`, which is exactly the blend the GPU
///   performs — so nothing here is "tuned to look right", it is the volume
///   rendering equation truncated at one scattering event.
///
/// **Why premultiplied, and it is the fix for the defect the first capture
/// showed.** Under `AlphaMode::Blend` the GPU scales the *whole* shaded result
/// by alpha, specular included — so in shallow water, where alpha is near zero
/// because you can see the sand, the sky reflection was scaled away with it,
/// and the shallows rendered as wet sand with a faint sheen. **Fresnel does not
/// know how deep the water is**: a surface at a grazing angle reflects nearly
/// all of the sky whether there is half a metre under it or fifty.
/// `AlphaMode::Premultiplied` blends `src + (1 − α)·dst` and Bevy's
/// `premultiply_alpha` passes the shaded colour through untouched, so the
/// diffuse body carries its own α (multiplied in here), the specular is added
/// at full strength, and the background is attenuated once.
///
/// **The one honest simplification**: a single alpha cannot carry three
/// transmittances, so the *background* is attenuated by the mean while the
/// water's own colour is exact per channel. A per-channel `dst` multiply needs
/// a custom material, which is a shader (`RENDER.md` §8).
pub fn depth_tint(depth_m: f32) -> [f32; 4] {
    let t = transmittance(depth_m);
    let alpha = ((1.0 - (t[0] + t[1] + t[2]) / 3.0) * ALPHA_MAX).clamp(0.0, ALPHA_MAX);
    [
        SCATTER_ALBEDO[0] * (1.0 - t[0]),
        SCATTER_ALBEDO[1] * (1.0 - t[1]),
        SCATTER_ALBEDO[2] * (1.0 - t[2]),
        alpha,
    ]
}

/// How deep the surf band reaches, metres — its OUTER edge.
pub const FOAM_DEPTH_M: f32 = 2.6;
/// Where the wash is thickest, metres of water.
///
/// **Offshore of the waterline, and that is the whole point of this constant.**
/// The first cut peaked foam at zero depth and fell away from there, which put
/// the brightest thing on the sea exactly along the polygon edge where the
/// water meets the sand — the line was not merely visible, it was *outlined*.
/// Look at where surf actually is in a photograph: the water at the very edge
/// is thin, clear and quiet, and the white is a band standing off it, where
/// waves are breaking. Foam that fades to nothing at the waterline is what
/// makes the waterline stop being a line.
pub const FOAM_PEAK_M: f32 = 0.60;
/// Shore slope (rise/run) at which foam is at full strength.
pub const FOAM_SLOPE_FULL: f32 = 0.30;
/// What a flat shore still gets, 0..1 — foam is *mostly* a slope effect, not
/// only one.
pub const FOAM_SLOPE_FLOOR: f32 = 0.35;
/// How far the noise moves the band's own contour, metres of depth.
///
/// **This is the other half of un-drawing the line.** A band keyed on depth
/// alone has edges that are exact iso-depth contours of the seabed, and an
/// iso-contour of a smooth heightfield is a smooth curve — the eye reads it as
/// drafting. Displacing the depth the band is evaluated at, by a noise field in
/// world space, turns both edges into lobes and fingers that run up the beach,
/// which is what surf does.
pub const FOAM_JITTER_M: f32 = 0.55;
/// Wavelength of the coarse octave of that noise, metres. Roughly the size of
/// one lobe of wash.
pub const FOAM_NOISE_M: f32 = 9.0;
/// The quietest the wash gets between surges, as a fraction of its own peak.
/// Not zero: a shoreline that goes completely dry between waves reads as the
/// foam being switched off, not as the water drawing back.
pub const FOAM_SURGE_FLOOR: f32 = 0.45;
/// The most foam a whitecap on the open swell contributes.
pub const CREST_FOAM_MAX: f32 = 0.30;
/// Foam colour, LINEAR. Aerated water is bright but it is not paper.
pub const FOAM_BODY: [f32; 3] = [0.55, 0.60, 0.60];
/// Alpha of a fully foamed vertex.
pub const FOAM_ALPHA: f32 = 0.96;

/// Two octaves of value noise in world metres, in `[0, 1]`. The field the wash
/// band's contour is displaced by.
///
/// Built on `props::hash01`, which is the client's one spatial hash — a second
/// noise implementation is a second thing to get seams wrong in.
pub fn foam_noise(x: f32, z: f32) -> f32 {
    fn lattice(x: f32, z: f32) -> f32 {
        let (xi, zi) = (x.floor(), z.floor());
        let (fx, fz) = (x - xi, z - zi);
        let (sx, sz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
        let (i, j) = (xi as i32 as u32, zi as i32 as u32);
        let h = |a: u32, b: u32| super::props::hash01(a, b);
        let top = h(i, j) + (h(i.wrapping_add(1), j) - h(i, j)) * sx;
        let bot = h(i, j.wrapping_add(1))
            + (h(i.wrapping_add(1), j.wrapping_add(1)) - h(i, j.wrapping_add(1))) * sx;
        top + (bot - top) * sz
    }
    let coarse = lattice(x / FOAM_NOISE_M, z / FOAM_NOISE_M);
    let fine = lattice(
        x / (FOAM_NOISE_M * 0.34) + 31.0,
        z / (FOAM_NOISE_M * 0.34) - 17.0,
    );
    (coarse * 0.68 + fine * 0.32).clamp(0.0, 1.0)
}

/// Foam at the shore: a band standing off the waterline, weighted by how
/// steeply the ground under it is rising, with its contour displaced by
/// [`foam_noise`].
///
/// The slope term is the reference's own published observation about their
/// ocean foam — "mostly visible on terrain slopes with higher inclination"
/// (`reference/WATER.md` §4) — and it is a statement about the *land*, which
/// is why this takes a terrain slope and not a water one.
///
/// `noise` is in `[0, 1]` and is passed in rather than sampled here so the
/// profile stays a pure function of three scalars and the gate can walk it.
pub fn shore_foam(depth_m: f32, slope: f32, noise: f32) -> f32 {
    // Displace the depth the band is read at. Both edges move together, so a
    // lobe that reaches further up the beach is also thicker.
    let d = depth_m + FOAM_JITTER_M * (noise.clamp(0.0, 1.0) - 0.5) * 2.0;
    if d <= 0.0 || d >= FOAM_DEPTH_M {
        return 0.0;
    }
    // Rise from the waterline to the peak, fall from the peak to the outer
    // edge. Smoothstep on both sides: a linear ramp has a visible kink at the
    // top, which on a wide band is another line.
    let t = if d < FOAM_PEAK_M {
        d / FOAM_PEAK_M
    } else {
        1.0 - (d - FOAM_PEAK_M) / (FOAM_DEPTH_M - FOAM_PEAK_M)
    };
    let band = t * t * (3.0 - 2.0 * t);
    let steep = (slope / FOAM_SLOPE_FULL).clamp(0.0, 1.0);
    // The same noise thins the band where it is already thin, so the wash has
    // texture rather than being one flat sheet of white.
    band * (FOAM_SLOPE_FLOOR + (1.0 - FOAM_SLOPE_FLOOR) * steep) * (0.55 + 0.45 * noise)
}

/// The wash running up and drawing back, from the longest wave's own phase.
///
/// **A moving edge cannot be a hard edge**, which is the cheapest thing in this
/// file and close to the most effective: an observer reads a boundary that
/// breathes as a process and a boundary that does not as geometry. Floored at
/// [`FOAM_SURGE_FLOOR`].
pub fn foam_surge(x: f32, z: f32, phase0: f32) -> f32 {
    let w = WAVES[0];
    let k = std::f32::consts::TAU / w.len_m;
    let s = 0.5 + 0.5 * (k * (w.dir[0] * x + w.dir[1] * z) + phase0).sin();
    FOAM_SURGE_FLOOR + (1.0 - FOAM_SURGE_FLOOR) * s
}

/// Whitecaps: foam on the top of the swell itself.
pub fn crest_foam(h: f32, reach_m: f32) -> f32 {
    if reach_m <= 0.0 {
        return 0.0;
    }
    let t = ((h / reach_m) - 0.62) / 0.33;
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t) * CREST_FOAM_MAX
}

/// Mix foam into a graded colour. Both sides are premultiplied, so the foam's
/// own colour is premultiplied here too — lerping a premultiplied colour
/// toward a straight one is how foam ends up brighter than the alpha that
/// carries it, which the GPU reads as a surface emitting light.
pub fn with_foam(mut rgba: [f32; 4], foam: f32) -> [f32; 4] {
    let f = foam.clamp(0.0, 1.0);
    for c in 0..3 {
        rgba[c] += (FOAM_BODY[c] * FOAM_ALPHA - rgba[c]) * f;
    }
    rgba[3] += (FOAM_ALPHA - rgba[3]) * f;
    rgba
}

// ---------------------------------------------------------------------------
// The ripple map.
// ---------------------------------------------------------------------------

/// Ripple map edge, texels.
pub const RIPPLE_TEX: u32 = 256;
/// World size of one ripple tile, metres.
pub const RIPPLE_TILE_M: f32 = 12.0;
/// How fast the ripple layer drifts over the world, tiles per second, and in
/// which direction. Slow: this is capillary detail riding the swell, not a
/// second swell.
pub const RIPPLE_DRIFT: [f32; 2] = [0.028, 0.011];

/// The ripple field, as integer harmonics of the tile so it tiles EXACTLY.
///
/// `(nx, nz, amplitude in metres)`. Non-integer harmonics are the classic way
/// to get a seam every 12 m across the whole ocean, and it is invisible in the
/// generator and obvious in the frame.
const RIPPLES: [(f32, f32, f32); 6] = [
    (3.0, 1.0, 0.030),
    (1.0, -2.0, 0.024),
    (5.0, 3.0, 0.016),
    (-2.0, 5.0, 0.014),
    (7.0, -4.0, 0.009),
    (4.0, 8.0, 0.007),
];

/// Height and UV-space gradient of the ripple field at `(u, v)` in tile units.
fn ripple_at(u: f32, v: f32) -> (f32, f32) {
    let (mut du, mut dv) = (0.0f32, 0.0f32);
    for (nx, nz, a) in RIPPLES {
        let w = std::f32::consts::TAU * (nx * u + nz * v);
        let c = w.cos();
        du += a * c * std::f32::consts::TAU * nx;
        dv += a * c * std::f32::consts::TAU * nz;
    }
    // Per metre, not per tile.
    (du / RIPPLE_TILE_M, dv / RIPPLE_TILE_M)
}

/// The tiling ripple normal map, with its own mip chain.
///
/// **The mips are the load-bearing half and Bevy will not build them.** An
/// `Image` constructed in code has `mip_level_count = 1`, and a one-level
/// normal map on a surface that reaches 2.6 km is a shimmering carpet of
/// aliased highlights — the failure is not subtle and it is worst exactly
/// where the sea is calmest. Each level is a box filter of the last with the
/// normals renormalized, which also gives the right *behaviour*: averaging
/// normals pulls them toward flat, so distant water goes smooth on its own.
pub fn ripple_map() -> Image {
    let n = RIPPLE_TEX;
    let levels = n.trailing_zeros() + 1;

    // Level 0, as unit normals in tangent space (+Z is the surface normal).
    let mut level: Vec<[f32; 3]> = Vec::with_capacity((n * n) as usize);
    for y in 0..n {
        for x in 0..n {
            let u = (x as f32 + 0.5) / n as f32;
            let v = (y as f32 + 0.5) / n as f32;
            let (du, dv) = ripple_at(u, v);
            let l = (du * du + dv * dv + 1.0).sqrt();
            level.push([-du / l, -dv / l, 1.0 / l]);
        }
    }

    let mut data: Vec<u8> = Vec::with_capacity((n * n * 4 * 2) as usize);
    let mut size = n;
    for lvl in 0..levels {
        if lvl > 0 {
            // Box filter, then renormalize. A normal map averaged and left
            // unnormalized shortens toward the origin and reads as a loss of
            // *lighting*, not of detail.
            let half = size / 2;
            let mut next = Vec::with_capacity((half * half) as usize);
            for y in 0..half {
                for x in 0..half {
                    let mut acc = [0.0f32; 3];
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let s = level[((y * 2 + dy) * size + (x * 2 + dx)) as usize];
                            for c in 0..3 {
                                acc[c] += s[c] * 0.25;
                            }
                        }
                    }
                    let l = (acc[0] * acc[0] + acc[1] * acc[1] + acc[2] * acc[2])
                        .sqrt()
                        .max(1e-6);
                    next.push([acc[0] / l, acc[1] / l, acc[2] / l]);
                }
            }
            level = next;
            size = half;
        }
        for texel in &level {
            for c in texel {
                data.push(((c * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            data.push(255);
        }
    }

    // **`Image::new` asserts `data.len() == width·height·format`, so the mip
    // chain cannot be handed to it** — it only knows about level 0. Build the
    // image from level 0, then declare the chain and swap the whole buffer in.
    // wgpu's `create_texture_with_data` reads the levels back out of it in the
    // order they were written, which is the order they are written above.
    let mut image = Image::new(
        Extent3d {
            width: n,
            height: n,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data[..(n * n * 4) as usize].to_vec(),
        // **Unorm, not UnormSrgb.** A normal map is a vector, not a colour;
        // sRGB-decoding one bends every normal toward the surface and the
        // whole sea goes glassy.
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = levels;
    image.data = Some(data);
    // **Repeat, and Bevy's default is ClampToEdge.** The UVs here are world
    // metres over `RIPPLE_TILE_M`, so they reach the hundreds; clamped, the
    // entire ocean would be one stretched texel of the tile's edge.
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// How many bytes [`ripple_map`] should hold for a given edge — the sum of the
/// whole mip chain. Split out so `tests/water.rs` can assert the chain is
/// complete without a GPU, which is the one thing a malformed mip count fails
/// at (loudly, inside wgpu, at first draw).
pub fn ripple_bytes(edge: u32) -> usize {
    let mut total = 0usize;
    let mut n = edge;
    loop {
        total += (n * n * 4) as usize;
        if n == 1 {
            break;
        }
        n /= 2;
    }
    total
}

// ---------------------------------------------------------------------------
// The material.
// ---------------------------------------------------------------------------

/// `StandardMaterial::reflectance` for water.
///
/// **Not a taste value and the old plane had it wrong.** Bevy maps the field
/// to `F0 = 0.16·reflectance²`, and water's Fresnel reflectance at normal
/// incidence is 2% — `((1.333 − 1)/(1.333 + 1))²` for an IOR of 1.333. So
/// `reflectance = sqrt(0.02/0.16) = 0.354`. The plane this replaced shipped
/// 0.55, which is `F0 = 4.8%`, nearly two and a half times too specular:
/// water lit like a polished dielectric, which is what a plastic sea is.
pub const WATER_REFLECTANCE: f32 = 0.353_553_4;

/// Water's index of refraction.
pub const WATER_IOR: f32 = 1.333;

/// Roughness. Not zero: a mirror sea has one specular pixel of sun and nothing
/// else, and the ripple map cannot widen a highlight it does not blur.
pub const WATER_ROUGHNESS: f32 = 0.08;

fn water_material(ripples: Handle<Image>) -> StandardMaterial {
    StandardMaterial {
        // White, because the colour IS the vertex colour: every vertex carries
        // its own graded body colour and alpha out of `depth_tint`, and a base
        // colour here would multiply all of them by one tint.
        base_color: Color::WHITE,
        normal_map_texture: Some(ripples),
        perceptual_roughness: WATER_ROUGHNESS,
        metallic: 0.0,
        reflectance: WATER_REFLECTANCE,
        ior: WATER_IOR,
        // See `premultiplied`: this is what keeps the sky in the shallows.
        alpha_mode: AlphaMode::Premultiplied,
        // Visible from underneath. A player wading past chest height is
        // looking at the surface from below, and a back-face-culled sea simply
        // is not there.
        cull_mode: None,
        double_sided: true,
        ..default()
    }
}

// ---------------------------------------------------------------------------
// The resource, and the two systems.
// ---------------------------------------------------------------------------

/// The sea's one mesh and everything cached about it.
#[derive(Resource, Default)]
pub struct Sea {
    /// Coordinates along one axis, relative to the grid centre, ascending.
    coords: Vec<f32>,
    /// Local spacing at each coordinate: the wider of its two neighbours'
    /// gaps, so a vertex at a join is judged by the coarser side.
    spacing: Vec<f32>,
    /// Depth of water under each vertex, metres. Negative over dry land.
    depth: Vec<f32>,
    /// Static colour and alpha per vertex, before any foam.
    base: Vec<[f32; 4]>,
    /// The shore wash at each vertex, before this frame's surge. Kept apart
    /// from [`Sea::base`] because it is the half that breathes: the colour is a
    /// function of where the vertex is, the wash is a function of where the
    /// vertex is AND when it is being asked.
    shore: Vec<f32>,
    /// How much swell each vertex may carry: the shoaling fade, cached.
    shoal: Vec<f32>,
    /// Peak displacement available at each vertex — what `crest_foam` measures
    /// a crest against, so a whitecap is relative to the local sea state and
    /// not to a constant.
    reach: Vec<f32>,
    /// This frame's [`wave_field`] answer per vertex: `[h, gx, gz]`.
    ///
    /// **The only cache here that is a function of *when*, and it is a
    /// performance fix rather than a design one.** The position, the normal
    /// and the colour are three separate `attribute_mut` borrows and each one
    /// used to recompute the field it needs, so every vertex paid `wave_field`
    /// three times a frame — up to twelve `sin_cos` where four would do.
    /// Measured on the gate box at the shipped 89² grid: 1.01 ms a frame for
    /// the three passes against 0.38 ms for one pass and three reads, which is
    /// the single largest steady-state CPU cost the client had.
    ///
    /// It is sized once in [`setup`] and mutated in place forever after, like
    /// every other buffer in this struct — the comment the old code carried
    /// ("a scratch `Vec` would be the per-frame allocation this whole design
    /// exists to avoid") was true of a `Vec` built inside the system and is
    /// not true of one that lives as long as the sea.
    field: Vec<[f32; 3]>,
    mesh: Option<Handle<Mesh>>,
    material: Option<Handle<StandardMaterial>>,
    /// The snapped grid centre, in [`SNAP_M`] cells. `None` until the first
    /// build.
    cell: Option<(i32, i32)>,
    /// Per-wave phase, advanced by `ω·dt` and wrapped — never `k·x − ω·t`.
    phase: [f32; WAVE_COUNT],
    /// The ripple layer's scroll, wrapped into one tile so it cannot drift
    /// into the range where an `f32` UV loses its low bits.
    drift: Vec2,
    /// Vertices the last sweep carried instead of re-tapping ([`Carry`]).
    ///
    /// **Recorded because the carry is otherwise invisible to a gate, and a
    /// mutant proved it.** `tests/water_carry.rs` compares a walked grid
    /// against a freshly built one — so an implementation that never carried
    /// anything would pass every assertion in it, and one that derived its
    /// index shift WRONG does exactly that: `carry_of` refuses the shift, the
    /// sweep rebuilds, the output is right and the saving is gone. Deriving
    /// the shift wrong is the most likely real bug in this whole seam, and it
    /// was undetectable until this field existed.
    carried: usize,
}

impl Sea {
    /// Vertices along one axis.
    /// The sea's mesh, for a probe that wants to weigh it.
    ///
    /// Read-only and used by nothing in the client: `examples/frame_cost.rs`
    /// measures what one `Assets::get_mut` re-clones, so the size in
    /// `NOW.md` §0pf is a number that can be re-run rather than one that was
    /// quoted once.
    pub fn mesh(&self) -> Option<&Handle<Mesh>> {
        self.mesh.as_ref()
    }

    pub fn n(&self) -> usize {
        self.coords.len()
    }
    /// Where the grid is centred, world metres.
    pub fn centre(&self) -> Option<Vec2> {
        self.cell
            .map(|(cx, cz)| Vec2::new(cx as f32 * SNAP_M, cz as f32 * SNAP_M))
    }

    // ── The five caches, read-only, for the gate ──────────────────────────
    //
    // `stream` carries most of them across a snap instead of re-tapping the
    // ground ([`Carry`]), and the only honest evidence for that is a walked
    // grid compared with a freshly built one, value by value. A digest would
    // say *that* they differ; these say *where*, which is what a failure has
    // to hand back. Read-only by construction — the caches are `stream`'s to
    // write and `animate`'s to read.
    pub fn depth(&self) -> &[f32] {
        &self.depth
    }
    pub fn base(&self) -> &[[f32; 4]] {
        &self.base
    }
    pub fn shore(&self) -> &[f32] {
        &self.shore
    }
    pub fn shoal(&self) -> &[f32] {
        &self.shoal
    }
    pub fn reach(&self) -> &[f32] {
        &self.reach
    }
    /// The snapped grid centre in cells — `None` until the first sweep.
    pub fn cell(&self) -> Option<(i32, i32)> {
        self.cell
    }
    /// How many vertices the last sweep carried rather than re-tapped. See
    /// the field's own note: this is the only thing that separates a working
    /// carry from a correct rebuild.
    pub fn carried(&self) -> usize {
        self.carried
    }
}

/// The axis coordinate table: a uniform core out to [`CORE_M`], then a
/// geometric skirt to [`SEA_REACH_M`], mirrored about zero.
pub fn axis_coords() -> Vec<f32> {
    let mut half = vec![0.0f32];
    while *half.last().unwrap() < CORE_M - 1e-3 {
        let next = half.last().unwrap() + STEP_M;
        half.push(next.min(CORE_M));
    }
    let mut gap = STEP_M;
    while *half.last().unwrap() < SEA_REACH_M {
        gap *= SKIRT_RATIO;
        let next = half.last().unwrap() + gap;
        half.push(next);
    }
    let mut out: Vec<f32> = half
        .iter()
        .rev()
        .filter(|v| **v > 0.0)
        .map(|v| -v)
        .collect();
    out.extend(half.iter().copied());
    out
}

/// Local spacing per coordinate — see [`Sea::spacing`].
pub fn axis_spacing(coords: &[f32]) -> Vec<f32> {
    (0..coords.len())
        .map(|i| {
            let before = if i > 0 {
                coords[i] - coords[i - 1]
            } else {
                0.0
            };
            let after = if i + 1 < coords.len() {
                coords[i + 1] - coords[i]
            } else {
                0.0
            };
            before.max(after)
        })
        .collect()
}

/// Build the sea's mesh, material and resource. Runs on entering
/// `Screen::Loading` like the rig and the sky.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut sea: ResMut<Sea>,
) {
    let coords = axis_coords();
    let spacing = axis_spacing(&coords);
    let n = coords.len();
    let count = n * n;

    // Everything is sized once, here, and mutated in place forever after.
    let positions = vec![[0.0f32, SEA_LEVEL, 0.0]; count];
    let normals = vec![[0.0f32, 1.0, 0.0]; count];
    let colors = vec![[0.0f32, 0.0, 0.0, 0.0]; count];
    let uvs = vec![[0.0f32, 0.0]; count];
    // The tangent is CONSTANT and is written exactly once. The UVs are world
    // XZ over the tile, so `∂P/∂u` is `+X` everywhere no matter where the grid
    // snaps to or how the surface tilts — which is what makes it legal to skip
    // regenerating tangents on a mesh whose normals change every frame.
    // `w = -1` so that `w·cross(N, T)` is `+Z`, the direction `v` runs in.
    let tangents = vec![[1.0f32, 0.0, 0.0, -1.0]; count];

    let mut indices = Vec::with_capacity((n - 1) * (n - 1) * 6);
    for iz in 0..n - 1 {
        for ix in 0..n - 1 {
            let a = (iz * n + ix) as u32;
            let b = a + 1;
            let c = a + n as u32;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
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
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
    mesh.insert_indices(Indices::U32(indices));

    let mesh = meshes.add(mesh);
    let material = materials.add(water_material(images.add(ripple_map())));

    sea.coords = coords;
    sea.spacing = spacing;
    sea.depth = vec![0.0; count];
    sea.base = vec![[0.0; 4]; count];
    sea.shore = vec![0.0; count];
    sea.shoal = vec![0.0; count];
    sea.reach = vec![0.0; count];
    sea.field = vec![[0.0; 3]; count];
    sea.mesh = Some(mesh.clone());
    sea.material = Some(material.clone());
    sea.cell = None;
    sea.phase = [0.0; WAVE_COUNT];
    sea.drift = Vec2::ZERO;

    commands.spawn((
        WorldEntity,
        // A translucent surface has no business in the shadow map: it would
        // paint the seabed with the silhouette of the sea.
        NotShadowCaster,
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::IDENTITY,
    ));
}

/// Drop the sea's caches when the world goes. The entity is a `WorldEntity`
/// and goes with the rest; this is the resource that would otherwise carry one
/// island's depths into the next one.
pub fn teardown(mut sea: ResMut<Sea>) {
    *sea = Sea::default();
}

/// A re-centre the coordinate table lets the last sweep answer.
///
/// **The core IS a shared lattice across a snap, and `NOW.md` §0pf said it was
/// not.** That item reads "the sea's axis is non-uniform, so there is no
/// half-lattice left to share — off-thread or coarser, not cleverer", and the
/// first half is true of the SKIRT and false of the core: [`SNAP_M`] is 8 m,
/// [`STEP_M`] is 2 m, and 8 is an exact multiple of 2, so a one-cell step
/// along one axis slides every core coordinate onto the coordinate four slots
/// away. The world position is then the same `f32` — not close, the same
/// bits — because `ox` is an exact multiple of 8 and a core coordinate is an
/// exact multiple of 2, both far under 2²⁴.
///
/// So a snap does not have to re-tap the ground it just tapped. What it has to
/// re-tap is the skirt (whose coordinates are geometric and slide onto
/// nothing), the four columns that entered the core, and everything if the
/// player moved diagonally or teleported.
///
/// **Every one of those claims is CHECKED rather than argued** — the shape
/// `terrain_mesh::heightfield`'s `share_x` guard already uses, for the same
/// reason: they are `f32` identities that hold for the table we ship and need
/// not hold for a table nobody has written yet. A table that fails any of them
/// falls back to the full sweep and the sea is unchanged.
#[derive(Clone, Copy)]
struct Carry {
    /// New index `i` in `lo..=hi` reads the last sweep's index `i + s`.
    lo: usize,
    hi: usize,
    s: isize,
    /// Which axis moved. The other one's coordinates did not change at all,
    /// so every line across it carries.
    on_x: bool,
}

/// What of the last sweep this one can carry, or `None` for a full rebuild.
///
/// Four conditions per index, and the last two are why this is a scan and not
/// an arithmetic range:
///
///  - the world coordinate is the same `f32`: `coords[i + s] == coords[i] + off`;
///  - the LOCAL SPACING is the same, because `reach` is a function of it and
///    of nothing else that moves — the outermost core coordinate's spacing is
///    the first skirt gap, not `STEP_M`, so this excludes it;
///  - both coordinates are inside [`DEPTH_R_M`], so the deep-by-fiat flag is
///    decided by the *other* axis on both sides and therefore agrees;
///  - the surviving set is contiguous, checked rather than assumed.
fn carry_of(coords: &[f32], spacing: &[f32], from: (i32, i32), to: (i32, i32)) -> Option<Carry> {
    let (dx, dz) = (to.0 - from.0, to.1 - from.1);
    let (d, on_x) = match (dx, dz) {
        (0, 0) => return None,
        (d, 0) => (d, true),
        (0, d) => (d, false),
        // A diagonal snap slides both axes, and a vertex would have to read a
        // slot that moved in x AND in z — which is a different index in a
        // different row, not a shift. Rebuild.
        _ => return None,
    };
    let n = coords.len();
    let off = d as f32 * SNAP_M;
    // The candidate shift, derived from the two constants and then CHECKED.
    let s = (d as isize) * (SNAP_M / STEP_M) as isize;
    let ok = |i: usize| {
        let j = i as isize + s;
        if j < 0 || j as usize >= n {
            return false;
        }
        let j = j as usize;
        coords[j] == coords[i] + off
            && spacing[j] == spacing[i]
            && coords[i].abs() <= DEPTH_R_M
            && coords[j].abs() <= DEPTH_R_M
    };
    let lo = (0..n).find(|i| ok(*i))?;
    let hi = (0..n).rev().find(|i| ok(*i))?;
    if !(lo..=hi).all(ok) {
        return None;
    }
    Some(Carry { lo, hi, s, on_x })
}

impl Carry {
    /// Whether this vertex was answered by the last sweep.
    #[inline]
    fn holds(&self, ix: usize, iz: usize) -> bool {
        let i = if self.on_x { ix } else { iz };
        i >= self.lo && i <= self.hi
    }

    /// Slide one cache into place. Rows for a z-snap, a run per row for an
    /// x-snap; `copy_within` handles the overlap either way.
    fn slide<T: Copy>(&self, v: &mut [T], n: usize) {
        let (lo, hi, s) = (self.lo, self.hi, self.s);
        if self.on_x {
            for iz in 0..n {
                let base = iz * n;
                let src = base + (lo as isize + s) as usize..base + (hi as isize + s) as usize + 1;
                v.copy_within(src, base + lo);
            }
        } else {
            let src = ((lo as isize + s) as usize) * n..((hi as isize + s) as usize + 1) * n;
            v.copy_within(src, lo * n);
        }
    }
}

/// Re-centre the grid on the eye and rebuild everything that is a function of
/// *where* rather than of *when*.
///
/// Runs in the `Stream` set with the other ring builders, for the same reason
/// they are there: it reads `Eye::pos`, which `input::place_eye` writes.
pub fn stream(
    mut sea: ResMut<Sea>,
    mut meshes: ResMut<Assets<Mesh>>,
    world: Res<WorldId>,
    eye: Res<Eye>,
) {
    let cell = (
        (eye.pos.x / SNAP_M).floor() as i32,
        (eye.pos.z / SNAP_M).floor() as i32,
    );
    if sea.cell == Some(cell) {
        return;
    }
    let Some(handle) = sea.mesh.clone() else {
        return;
    };
    let Some(mesh) = meshes.get_mut(&handle) else {
        return;
    };
    let prev = sea.cell;
    sea.cell = Some(cell);
    let (ox, oz) = (cell.0 as f32 * SNAP_M, cell.1 as f32 * SNAP_M);
    let seed = world.seed;
    let n = sea.coords.len();

    // One memo for the whole sweep. The grid is walked row-major at 2 m in the
    // core, so consecutive taps are neighbours and share every lattice corner
    // above the finest octave — the same share `heightfield` takes, on the
    // same function, for the same reason. It changes no depth by a bit
    // (`sim-core/tests/lattice.rs`), only how many hashes the sweep draws.
    let mut lat = terrain::Lattice::new();

    // Split the borrows: the caches are read and written while the mesh's UV
    // buffer is written, and both live behind `&mut`.
    let Sea {
        coords,
        spacing,
        depth,
        base,
        shore,
        shoal: shoal_cache,
        reach,
        ..
    } = &mut *sea;

    // Slide what the last sweep already answered, then sweep only the rest.
    let carry = prev.and_then(|p| carry_of(coords, spacing, p, cell));
    let carried = carry.map_or(0, |c| (c.hi - c.lo + 1) * n);
    if let Some(c) = carry {
        c.slide(depth, n);
        c.slide(base, n);
        c.slide(shore, n);
        c.slide(shoal_cache, n);
        // `reach` too: it is `f(spacing) * shoal(depth)`, and `carry_of`
        // refuses any index whose spacing moved, so both factors slid.
        c.slide(reach, n);
    }

    for iz in 0..n {
        for ix in 0..n {
            if carry.is_some_and(|c| c.holds(ix, iz)) {
                continue;
            }
            let i = iz * n + ix;
            let x = ox + coords[ix];
            let z = oz + coords[iz];
            let far = coords[ix].abs().max(coords[iz].abs()) > DEPTH_R_M;
            let d = if far {
                // Deep by fiat past the sampling radius — see `DEPTH_R_M`. A
                // finite sentinel rather than a huge one: it is five times the
                // sea floor's own `SEA_FLOOR_DEPTH`, so it saturates every
                // curve here while keeping the arithmetic ordinary.
                DEEP_SENTINEL_M
            } else {
                SEA_LEVEL - terrain::height_memo(&mut lat, seed, x, z)
            };
            depth[i] = d;

            let sp = spacing[ix].max(spacing[iz]);
            let sh = shoal(d);
            shoal_cache[i] = sh;
            // What a crest could reach here, for `crest_foam` to measure
            // against: every wave that survives this spacing, at this shoaling.
            let mut r = 0.0;
            for w in WAVES.iter() {
                r += w.amp_m * band_weight(sp, w.len_m);
            }
            reach[i] = r * sh * SHAPE_PEAK;

            base[i] = depth_tint(d);
            // The slope tap is four more `height` calls, so it is paid only
            // inside the band that can use it — a thin ring around the island,
            // not the ocean. The window is widened by the jitter, because a
            // vertex outside the plain band can still be inside the displaced
            // one.
            shore[i] = if d > -FOAM_JITTER_M && d < FOAM_DEPTH_M + FOAM_JITTER_M {
                shore_foam(
                    d,
                    terrain::slope_memo(&mut lat, seed, x, z),
                    foam_noise(x, z),
                )
            } else {
                0.0
            };
        }
    }

    sea.carried = carried;

    if let Some(VertexAttributeValues::Float32x2(uv)) = mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0) {
        for iz in 0..n {
            for ix in 0..n {
                uv[iz * n + ix] = [
                    (ox + sea.coords[ix]) / RIPPLE_TILE_M,
                    (oz + sea.coords[iz]) / RIPPLE_TILE_M,
                ];
            }
        }
    }
}

/// Advance the swell and write this frame's surface.
///
/// The only per-frame work: four sines a vertex, three attribute writes, and
/// one `Affine2` on the material. No `terrain` taps and no allocation.
///
/// **"Four sines a vertex" is what this sentence always claimed and what the
/// code only now does.** The three attribute writes cannot hold their borrows
/// at once, so the first cut recomputed [`wave_field`] inside each of them —
/// three times a vertex, twelve `sin_cos` in the core band — and threw two of
/// the three answers away. The field is resolved once into [`Sea::field`] and
/// the three passes read it. Same arithmetic, same output: `tests/water.rs`
/// gates the equality against a direct call rather than trusting the reading.
pub fn animate(
    mut sea: ResMut<Sea>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    time: Res<Time>,
) {
    if sea.cell.is_none() {
        return;
    }
    let dt = time.delta_secs();
    for (i, w) in WAVES.iter().enumerate() {
        sea.phase[i] = (sea.phase[i] - omega(w.len_m) * dt).rem_euclid(std::f32::consts::TAU);
    }
    sea.drift.x = (sea.drift.x + RIPPLE_DRIFT[0] * dt).fract();
    sea.drift.y = (sea.drift.y + RIPPLE_DRIFT[1] * dt).fract();

    if let Some(h) = sea.material.clone() {
        if let Some(m) = materials.get_mut(&h) {
            m.uv_transform = Affine2::from_translation(sea.drift);
        }
    }

    let Some(handle) = sea.mesh.clone() else {
        return;
    };
    let Some(mesh) = meshes.get_mut(&handle) else {
        return;
    };
    let Some((cx, cz)) = sea
        .cell
        .map(|(a, b)| (a as f32 * SNAP_M, b as f32 * SNAP_M))
    else {
        return;
    };
    let n = sea.coords.len();
    let phase = sea.phase;

    // The trig, once. Split the borrows the way `stream` does: the field is
    // written while the coordinate and shoaling caches are read, and all four
    // live behind the same `&mut`.
    {
        let Sea {
            coords,
            spacing,
            shoal: shoal_cache,
            field,
            ..
        } = &mut *sea;
        resolve_field(
            Vec2::new(cx, cz),
            coords,
            spacing,
            shoal_cache,
            &phase,
            field,
        );
    }

    // Three buffers, fetched one at a time because `attribute_mut` borrows the
    // mesh. Each is now a read out of `field` and an arithmetic write — no
    // `sin_cos` past this point.
    if let Some(VertexAttributeValues::Float32x3(pos)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    {
        for iz in 0..n {
            for ix in 0..n {
                let i = iz * n + ix;
                pos[i] = [
                    cx + sea.coords[ix],
                    SEA_LEVEL + sea.field[i][0],
                    cz + sea.coords[iz],
                ];
            }
        }
    }
    if let Some(VertexAttributeValues::Float32x3(nor)) = mesh.attribute_mut(Mesh::ATTRIBUTE_NORMAL)
    {
        for (i, slot) in nor.iter_mut().enumerate().take(n * n) {
            let v = wave_normal(sea.field[i][1], sea.field[i][2]);
            *slot = [v.x, v.y, v.z];
        }
    }
    if let Some(VertexAttributeValues::Float32x4(col)) = mesh.attribute_mut(Mesh::ATTRIBUTE_COLOR) {
        for iz in 0..n {
            for ix in 0..n {
                let i = iz * n + ix;
                // The wash breathes; the whitecaps ride the swell. Summed
                // rather than maxed: a crest breaking inside the surf band is
                // the whitest thing on the sea, and clamping happens in
                // `with_foam`.
                let wash = if sea.shore[i] > 0.0 {
                    let x = cx + sea.coords[ix];
                    let z = cz + sea.coords[iz];
                    sea.shore[i] * foam_surge(x, z, phase[0])
                } else {
                    0.0
                };
                col[i] = with_foam(
                    sea.base[i],
                    wash + crest_foam(sea.field[i][0], sea.reach[i]),
                );
            }
        }
    }
}
