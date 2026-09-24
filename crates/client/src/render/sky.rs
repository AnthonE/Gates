//! Clouds — the frame's top stop.
//!
//! **The measurement that forced this, and the two things it ruled out.** Our
//! captures read whole-frame p90 143.6 against the reference photographs'
//! 170.2, and `ART.md` §4 says where the difference comes from: *"Cloudless
//! cannot reach a p90 of 189 with a sky mean of 143; the references get there
//! with lit cumulus."* Two cheaper answers were tried against the arithmetic
//! and both fail:
//!
//!   · **The sun disk cannot do it.** `SunDisk::EARTH` is 9.31 mrad, and at
//!     75° horizontal over 1280 px that is ~9 px across — about 65 px, or
//!     0.007% of the frame. A p90 needs ~92,000 pixels to move. Raising its
//!     intensity is measuring the wrong thing.
//!   · **Bloom cannot do it either.** `Bloom::NATURAL` is
//!     `BloomCompositeMode::EnergyConserving`, whose final composite is
//!     `{src: Constant, dst: OneMinusConstant}` — a lerp between the scene and
//!     its blur. It redistributes energy and never adds any.
//!
//! **A top stop must come from AREA.** So: clouds, and they have to be their
//! own draw.
//!
//! **Why a `Skybox` cubemap rather than a dome or a fullscreen pass**, which
//! is the load-bearing decision in this file:
//!
//!   · `Skybox` draws at the end of `MainOpaquePass`, which is *before*
//!     `AtmosphereNode::RenderSky`, so the atmosphere composites the clouds
//!     itself as `dst = inscattering + transmittance·dst`. The clouds get the
//!     same aerial perspective every other distant thing gets, from the same
//!     owner. **`ART.md` rule 5 is satisfied by construction rather than by
//!     tuning** — which is exactly what the browser client could not manage,
//!     and it spent 200 lines of comment hand-fitting one horizon seam.
//!   · An `AlphaMode::Blend` sky dome draws in `MainTransparentPass`, strictly
//!     *after* the sky is resolved, so it would composite over finished sky
//!     with no attenuation at all — and the fix for that is a `DistanceFog`,
//!     which is a second owner of haze. That is the coupled-lighting failure
//!     this repo has already paid for once.
//!   · `FullscreenMaterial` (new in 0.18) binds only `(texture, sampler,
//!     uniform)`: no depth, no view uniform. It cannot tell sky from geometry
//!     and cannot even build a ray direction. It is a post-process API.
//!   · Ordering a custom render node against the atmosphere is not possible:
//!     `AtmosphereNode` lives in a private `mod node;` in `bevy_pbr` and is
//!     never re-exported, so there is no edge to add.
//!
//! The cubemap is generated from the world seed. No asset, no shader, no
//! download. **Since weather v0 it is composed, not baked once**: a tileable
//! noise field is built at boot, and [`compose`] re-projects the deck from it
//! every half second or so, a few rows a frame — the wind drifting it, the
//! weather thickening and darkening it, its lit side facing the sun it is
//! actually lit by, and the night's stars and moon behind it.

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::Skybox;
use bevy::image::Image;
use bevy::math::curve::Curve;
use bevy::pbr::{DistanceFog, Falloff, FogFalloff, ScatteringMedium};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};

use super::fill::linear_to_srgb;
use super::rig::{EyeCam, CAPTURE_DAY_FRAC};
use super::WorldId;

/// Cube face size in texels. 6 × 256² = 393k texels, 1.5 MB of RGBA8 on the
/// desktop; a quarter of that in a browser, where every re-upload is a
/// WebGL texture rebuilt.
pub const SKY_FACE: u32 = if cfg!(target_arch = "wasm32") {
    128
} else {
    256
};

/// Cloud-deck altitude, metres. Cumulus bases sit near a kilometre; the exact
/// value only sets how fast the deck compresses toward the horizon.
pub const CLOUD_ALT_M: f32 = 1400.0;
/// Horizontal scale of the deck's noise, metres per unit.
pub const CLOUD_SCALE_M: f32 = 900.0;
/// How much of the sky carries cloud, 0..1. `ART.md` §1 asks for cumulus with
/// gaps — a solid overcast has no top stop either, because the bright tops are
/// what carry the range.
pub const CLOUD_COVER: f32 = 0.52;

/// Sunlit cumulus, cd/m². `Skybox::brightness` is documented in cd/m² and is
/// multiplied by `Exposure::exposure()` when the component is extracted, and
/// `exposure() = exp2(-ev100)/1.2` — so at the rig's ev100 14.2 this lands a
/// 1.0 texel at 1.0 linear, pre-tonemap. The top stop in physical units
/// rather than as a number somebody liked.
///
/// **It also must be set explicitly: `Skybox::brightness` defaults to 0.0**,
/// and a skybox at zero brightness is an invisible one.
pub const CLOUD_NITS: f32 = 26_000.0;

/// Lit top and grey base — the two values `ART.md` §1 names. A cumulus that
/// is one value reads as fog, and it is the lit top that carries the p90.
const CLOUD_TOP: [f32; 3] = [1.0, 0.99, 0.97];
const CLOUD_BASE: [f32; 3] = [0.42, 0.45, 0.52];

// ── The browser's sky (browser sky v0 — DECISIONS.md §open) ─────────────────
//
// **Natively the atmosphere paints the sky and every clear texel of this
// cubemap must stay ZERO** — the skybox pipeline has `blend: None`, so a
// non-zero clear texel is a second sky added to `AtmosphereNode`'s own. In a
// browser there is no atmosphere at all (`rig.rs`: `mesh_view_layout_
// atmosphere` wants a storage buffer and WebGL2 allows none), so the same
// zero texel is the CLEAR COLOUR, and the first island a browser drew had
// white cumulus floating on black (`findings/web-build-20260909.md` §15.7).
// A page therefore bakes its sky INTO the deck: clouds composited over a
// clear-sky radiance rather than over nothing. Same texels, same composer.
//
// What the browser still does not get, said out loud: the atmosphere's own
// aerial perspective (a browser gets [`browser_haze`] instead — the same air
// as a `DistanceFog`, and the only haze on that target) and a sun disk. Its
// dusk now colours rather than only dims: the composer greys the backdrop
// under a heavy sky and warms it toward a low sun (weather v0).

/// Whether the deck carries a clear sky behind the clouds. Zero texels
/// natively, for the reason above; a sky in a browser.
pub const BAKE_BACKDROP: bool = cfg!(target_arch = "wasm32");

/// The horizon's luminance over the zenith's. A clear sky is brightest at
/// the horizon, where the eye looks through the most air: measured skies
/// put it at two to three times the zenith (Preetham et al.'s clear-sky
/// model at moderate turbidity), and the reference frames read the same way
/// — `ART.md` §1's "distant hills lighten" is this gradient landing on the
/// ground rather than the sky.
pub const HORIZON_GAIN: f32 = 2.4;

/// How far the horizon's colour is pulled toward neutral, `0..1`. Haze
/// scatters every wavelength, so the horizon is greyer than the zenith's
/// blue as well as brighter; at 0 the sky is one hue top to bottom.
pub const HORIZON_DESAT: f32 = 0.65;

/// The floor on `1 / (elevation + AIR_FLOOR)`, the air-mass curve the
/// gradient follows. A plane-parallel atmosphere's path length is `1/sin(e)`
/// and diverges at the horizon; the floor is what a curved one does to it —
/// Kasten & Young's air mass tops out near 38 at the horizon, i.e. a floor
/// near `1/38`. Larger reads as a broader, softer horizon band.
pub const AIR_FLOOR: f32 = 0.12;

/// Clear-sky radiance in world direction `d`, linear, **in the deck's texel
/// units** — `1.0` is one `CLOUD_NITS`, the same scale the clouds are stored
/// in, so the two composite in one space.
///
/// Zenith first, and it is DERIVED: `fill::sky_lux()` is the irradiance the
/// sky delivers to an up-facing surface, and a uniform sky of radiance `L`
/// delivers `π·L`, so the zenith's LUMINANCE is `sky_lux / π`, and its colour
/// is the air's own ([`air_chroma`]) rather than the fill's near-white tint,
/// which read as a grey sky on the first GPU that drew it. The horizon
/// is that, `HORIZON_GAIN` brighter and `HORIZON_DESAT` greyer, reached
/// along the air-mass curve; below the horizon the backdrop holds the
/// horizon colour flat (the sea and the land cover nearly all of it).
pub fn backdrop_at(d: Vec3) -> [f32; 3] {
    // Brightness from the fill, colour from the air. The fill's tint is an
    // ambient term's and nearly white on purpose; a sky that borrowed it
    // measured saturation 0.02 and read as overcast (2026-09-13).
    let lum = super::fill::luminance(super::fill::sky_lux()) / core::f32::consts::PI;
    let chroma = air_chroma();
    let zenith = [chroma[0] * lum, chroma[1] * lum, chroma[2] * lum];
    // The horizon: the zenith's luminance times the gain, with its chroma
    // pulled toward grey by the desaturation.
    let mut horizon = [0.0f32; 3];
    for c in 0..3 {
        let grey = zenith[c] + (lum - zenith[c]) * HORIZON_DESAT;
        horizon[c] = grey * HORIZON_GAIN;
    }
    // Air mass, normalised so `w` is 0 at the zenith and 1 at the horizon.
    let e = d.y.clamp(0.0, 1.0);
    let m = 1.0 / (e + AIR_FLOOR);
    let w = (m - 1.0 / (1.0 + AIR_FLOOR)) / (1.0 / AIR_FLOOR - 1.0 / (1.0 + AIR_FLOOR));
    let w = w.clamp(0.0, 1.0);
    [
        (zenith[0] + (horizon[0] - zenith[0]) * w) / CLOUD_NITS,
        (zenith[1] + (horizon[1] - zenith[1]) * w) / CLOUD_NITS,
        (zenith[2] + (horizon[2] - zenith[2]) * w) / CLOUD_NITS,
    ]
}

/// The air at ground level, per metre, as `(extinction, inscattering)`: each
/// term of `medium` at its density where the falloff parameter is `1`, the
/// dense end — each arm below is `bevy_pbr::medium::Falloff::sample` (private)
/// evaluated at `p = 1`, written out because that function is not `pub`: the
/// exponential's formula is exactly `1` there, a tent is wherever `1` sits on
/// its slope, and a curve is asked. Extinction is what a term removes,
/// absorption plus scattering; inscattering is what it sends on, scattering.
///
/// **Earthlike at the ground is Rayleigh and Mie, and not ozone**: its tent
/// peaks at `0.75` and is `0.3` wide, so it is zero at `1`. The browser's haze
/// and its sky colour both read this off `rig::island_medium`, the medium the
/// desktop's atmosphere renders — a browser frame is thinner than a desktop
/// one, but it is made of the same air.
pub fn ground_air(medium: &ScatteringMedium) -> (Vec3, Vec3) {
    let (mut extinction, mut inscattering) = (Vec3::ZERO, Vec3::ZERO);
    for term in &medium.terms {
        let k = match term.falloff {
            Falloff::Linear | Falloff::Exponential { .. } => 1.0,
            Falloff::Tent { center, width } => {
                (1.0 - (1.0 - center).abs() / (width * 0.5).max(f32::EPSILON)).max(0.0)
            }
            Falloff::Curve(ref curve) => curve.sample(1.0).unwrap_or(0.0),
        };
        extinction += (term.absorption + term.scattering) * k;
        inscattering += term.scattering * k;
    }
    (extinction, inscattering)
}

/// The colour the air scatters, linear, at luminance 1: [`ground_air`]'s
/// inscattering normalised. Density scales how much air there is and not what
/// colour it is, so this does not move with `rig::AIR_DENSITY`. Computed once,
/// because the browser's bake asks for it once per texel.
pub fn air_chroma() -> [f32; 3] {
    static CHROMA: std::sync::OnceLock<[f32; 3]> = std::sync::OnceLock::new();
    *CHROMA.get_or_init(|| {
        let (_, s) = ground_air(&super::rig::island_medium());
        let l = super::fill::luminance(s.to_array());
        [s.x / l, s.y / l, s.z / l]
    })
}

/// The browser haze's colour at `light` (0 night .. 1 day): the sky's own
/// horizon, so a distant hill fades INTO the sky behind it and not into a
/// second colour, and dimmed with the day exactly as the deck is.
pub fn haze_color(light: f32) -> Color {
    let h = backdrop_at(Vec3::X);
    Color::linear_rgb(h[0] * light, h[1] * light, h[2] * light)
}

/// The browser's aerial perspective: a `DistanceFog` made of the desktop's air.
///
/// **On the desktop this would be a second owner of haze, and this file's
/// header refuses one** — the atmosphere hazes the terrain and the deck
/// itself. A browser has no atmosphere (`rig.rs`), so it has no first owner,
/// and until 2026-09-13 its distant hills arrived with no air on them at all.
/// The falloff is `Atmospheric`, with [`ground_air`]'s two coefficients off
/// `rig::island_medium`, so no number here is one anybody chose: a few percent
/// of blue per kilometre, thin at island scale, as the desktop's air is.
pub fn browser_haze(light: f32) -> DistanceFog {
    let (extinction, inscattering) = ground_air(&super::rig::island_medium());
    DistanceFog {
        color: haze_color(light),
        falloff: FogFalloff::Atmospheric {
            extinction,
            inscattering,
        },
        ..default()
    }
}

fn hash3(seed: u64, x: i32, y: i32, z: i32) -> f32 {
    let mut h = seed
        ^ (x as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (z as i64 as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h >> 40) as f32 / 16_777_216.0
}

/// Value noise on a 2D lattice that wraps every `period` cells,
/// smoothstep-interpolated.
fn value_tiled(seed: u64, x: f32, y: f32, period: i32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (x - xi, y - yi);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (i, j) = (xi as i32, yi as i32);
    let w = |k: i32| k.rem_euclid(period);
    let a = hash3(seed, w(i), w(j), 0);
    let b = hash3(seed, w(i + 1), w(j), 0);
    let c = hash3(seed, w(i), w(j + 1), 0);
    let d = hash3(seed, w(i + 1), w(j + 1), 0);
    let top = a + (b - a) * sx;
    let bot = c + (d - c) * sx;
    top + (bot - top) * sy
}

/// Four octaves, each half the amplitude and twice the frequency — and twice
/// the lattice period, so every octave tiles on the same [`FIELD_PERIOD`]
/// square and the deck can drift forever without a seam.
fn fbm_tiled(seed: u64, x: f32, y: f32) -> f32 {
    let mut f = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    let mut period = FIELD_PERIOD as i32;
    for o in 0..4 {
        f += amp
            * value_tiled(
                seed ^ (o as u64).wrapping_mul(0x9E37_79B9),
                x * freq,
                y * freq,
                period,
            );
        amp *= 0.5;
        freq *= 2.0;
        period *= 2;
    }
    f
}

/// World direction for a cube texel.
///
/// **Cube space is world space with z flipped**: `skybox.wgsl` samples with
/// `ray_direction * vec3(1.0, 1.0, -1.0)`. Author in world space and flip
/// here, or every cloud sits on the wrong side of the sky.
/// **The one cubemap face convention in this crate**, and it is `pub` for the
/// same reason [`super::rig::to_sun`] is: two bakers need it and they must not
/// each re-derive it. `sky.rs` composes the cloud deck through this and
/// `fill.rs` bakes the hemisphere sky fill through it, and a face order that
/// disagreed between them would put the sky on the underside of every prop
/// while both files read as correct alone.
///
/// The trailing `-d.z` is the left-handed cubemap convention, and it is
/// carried HERE rather than at each call site: `bevy_pbr`'s
/// `environment_map.wgsl` negates z on the sample direction
/// (`irradiance_sample_dir.z = -irradiance_sample_dir.z`), so a texel written
/// at `cube_dir(f, x, y, n)` is the one a shader sampling that WORLD direction
/// reads back. Callers work in world space and never see the flip.
pub fn cube_dir(face: usize, x: u32, y: u32, n: u32) -> Vec3 {
    let uc = 2.0 * (x as f32 + 0.5) / n as f32 - 1.0;
    let vc = 2.0 * (y as f32 + 0.5) / n as f32 - 1.0;
    let d = match face {
        0 => Vec3::new(1.0, -vc, -uc),
        1 => Vec3::new(-1.0, -vc, uc),
        2 => Vec3::new(uc, 1.0, vc),
        3 => Vec3::new(uc, -1.0, -vc),
        4 => Vec3::new(uc, -vc, 1.0),
        _ => Vec3::new(-uc, -vc, -1.0),
    };
    Vec3::new(d.x, d.y, -d.z).normalize()
}

/// The horizontal direction the deck's light march steps toward at hour
/// `frac`, i.e. toward the sun — the rig's own `to_sun`, never a re-derived
/// one (`crates/client/tests/sun.rs` holds the two together).
///
/// **The deck is lit by the sun it is actually lit by since weather v0.** It
/// used to be baked once with its lit faces at the noon sun and then turned
/// about Y as the bearing swept (`deck_rotation`), which kept the light
/// right but spun the cloud pattern once a cycle — the opposite signature
/// to wind (`NOW.md` §0sun). The composer re-marches toward the current
/// sun on every compose, and the pattern moves only with the wind.
pub fn deck_march_dir(frac: f32) -> Vec2 {
    let s = super::rig::to_sun(frac);
    Vec2::new(s.x, s.z).normalize_or_zero()
}

// ── The composer (weather v0) ────────────────────────────────────────────

/// Lattice period of the deck's noise, in noise units: the field tiles every
/// `FIELD_PERIOD × CLOUD_SCALE_M` ≈ 29 km, about the distance to the deck's
/// own cutoff at the horizon, so no repeat sits inside one view.
pub const FIELD_PERIOD: u32 = 32;
/// The field's texels per side: ~28 m each on a desktop release, where the
/// highest octave's ~110 m wavelength is four texels across. Half that in a
/// browser and in a debug build, where the boot pays for every texel twice.
pub const FIELD_N: usize = if cfg!(target_arch = "wasm32") || cfg!(debug_assertions) {
    512
} else {
    1024
};
/// Below this `d.y` (~2.6°) there is no deck: `1/d.y` explodes.
const DECK_CUTOFF: f32 = 0.045;
/// The last stretch above the cutoff the deck fades over.
const HORIZON_FADE: f32 = 0.10;
/// How far the light march steps toward the sun, noise units.
const LIT_STEP: f32 = 0.35;
/// The coverage threshold's softness: edges are wisps, not a cutout.
const EDGE: f32 = 0.22;
/// The moon's disk, as cosines of its radius (~1.7°) and its soft edge
/// (~2.3°) — drawn big, the way a person remembers it — and its halo (~8.6°).
const MOON_COS_IN: f32 = 0.999_55;
const MOON_COS_OUT: f32 = 0.999_2;
const HALO_COS: f32 = 0.988_77;
const MOON_RGB: [f32; 3] = [0.78, 0.8, 0.84];
const HALO_RGB: [f32; 3] = [0.02, 0.024, 0.032];
/// A star's peak, in deck units. Against the fixed exposure this is a point
/// you can find, not a sky full of them.
const STAR_PEAK: f32 = 0.35;
/// About one texel in 110 above the horizon carries a star.
const STAR_DENSITY: f32 = 0.009;
/// How brightly the moon lights the deck's underside at deep night, as a
/// share of the sun: dark shapes against the stars rather than nothing.
const NIGHT_CLOUD_LIGHT: f32 = 0.012;
/// The browser's night sky floor (the desktop's atmosphere has its own).
const NIGHT_SKY: [f32; 3] = [0.0015, 0.002, 0.0035];
/// The warm glow a low sun puts on the browser's horizon near it.
const DUSK_GLOW: [f32; 3] = [0.10, 0.045, 0.02];
/// What fog and haze are lit by after dark, so night fog is a dark grey
/// wall rather than a hole.
const NIGHT_FOG: [f32; 3] = [0.0025, 0.003, 0.0045];

/// Texels composed per frame. A full deck is six frames of this on a
/// desktop; a debug build takes an eighth, because its client crate is not
/// optimized and a frame there is already slow.
const COMPOSE_BUDGET: usize = {
    let b = if cfg!(target_arch = "wasm32") {
        16_384
    } else {
        65_536
    };
    if cfg!(debug_assertions) {
        b / 8
    } else {
        b
    }
};
/// The least time between two composes, seconds: often enough that the
/// drift reads as motion, rare enough that the re-upload is noise.
const COMPOSE_INTERVAL_S: f32 = if cfg!(target_arch = "wasm32") {
    1.5
} else {
    0.5
};

/// The deck's noise, built once per world: a tileable fBm field the composer
/// samples wherever each texel's ray meets the drifting deck.
pub struct CloudField {
    n: usize,
    data: Box<[f32]>,
}

impl CloudField {
    pub fn new(seed: u64, n: usize) -> Self {
        let mut data = vec![0.0f32; n * n];
        let step = FIELD_PERIOD as f32 / n as f32;
        for j in 0..n {
            for i in 0..n {
                data[j * n + i] = fbm_tiled(seed, i as f32 * step, j as f32 * step);
            }
        }
        Self {
            n,
            data: data.into_boxed_slice(),
        }
    }

    /// Bilinear sample at noise-unit coordinates, wrapping.
    #[inline]
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        let k = self.n as f32 / FIELD_PERIOD as f32;
        let (u, v) = (x * k, y * k);
        let (fu, fv) = (u.floor(), v.floor());
        let (tu, tv) = (u - fu, v - fv);
        let n = self.n as i64;
        let i0 = (fu as i64).rem_euclid(n) as usize;
        let j0 = (fv as i64).rem_euclid(n) as usize;
        let i1 = if i0 + 1 == self.n { 0 } else { i0 + 1 };
        let j1 = if j0 + 1 == self.n { 0 } else { j0 + 1 };
        let row0 = j0 * self.n;
        let row1 = j1 * self.n;
        let (a, b) = (self.data[row0 + i0], self.data[row0 + i1]);
        let (c, d) = (self.data[row1 + i0], self.data[row1 + i1]);
        let top = a + (b - a) * tu;
        let bot = c + (d - c) * tu;
        top + (bot - top) * tv
    }
}

/// What every texel of the cube looks at, worked out once: its direction,
/// where its ray meets the deck (noise units), how far into the horizon
/// fade it sits, its star, and — in a browser — its clear sky.
pub struct TexelGeo {
    dir: Box<[Vec3]>,
    plane: Box<[Vec2]>,
    fade: Box<[f32]>,
    star: Box<[u8]>,
    backdrop: Box<[[f32; 3]]>,
}

impl TexelGeo {
    pub fn new(n: u32, with_backdrop: bool) -> Self {
        let count = (6 * n * n) as usize;
        let mut dir = Vec::with_capacity(count);
        let mut plane = Vec::with_capacity(count);
        let mut fade = Vec::with_capacity(count);
        let mut star = Vec::with_capacity(count);
        let mut backdrop = Vec::with_capacity(if with_backdrop { count } else { 0 });
        for face in 0..6usize {
            for y in 0..n {
                for x in 0..n {
                    let d = cube_dir(face, x, y, n);
                    dir.push(d);
                    if d.y > DECK_CUTOFF {
                        // Project onto a flat deck at `CLOUD_ALT_M`. The `1/d.y`
                        // is what compresses the deck toward the horizon — the
                        // reason a real cloudscape has big shapes overhead and a
                        // crowded band at the skyline.
                        let t = CLOUD_ALT_M / d.y;
                        plane.push(Vec2::new(d.x * t, d.z * t) / CLOUD_SCALE_M);
                        fade.push(((d.y - DECK_CUTOFF) / HORIZON_FADE).clamp(0.0, 1.0));
                    } else {
                        plane.push(Vec2::ZERO);
                        fade.push(0.0);
                    }
                    let h = hash3(0x5354_4152, face as i32, x as i32, y as i32);
                    star.push(if d.y > 0.03 && h < STAR_DENSITY {
                        // Mostly faint, a few bright: the square pushes the
                        // share toward the dim end.
                        let b = hash3(0x4252_4954, face as i32, x as i32, y as i32);
                        (40.0 + 215.0 * b * b) as u8
                    } else {
                        0
                    });
                    if with_backdrop {
                        backdrop.push(backdrop_at(d));
                    }
                }
            }
        }
        Self {
            dir: dir.into_boxed_slice(),
            plane: plane.into_boxed_slice(),
            fade: fade.into_boxed_slice(),
            star: star.into_boxed_slice(),
            backdrop: backdrop.into_boxed_slice(),
        }
    }

    /// Texels in the cube.
    pub fn len(&self) -> usize {
        self.dir.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dir.is_empty()
    }
}

/// The sRGB encode, as a table: the composer writes ~65k texels a frame and
/// a `powf` per channel is most of that frame.
fn srgb_lut() -> &'static [u8; 4096] {
    static LUT: std::sync::OnceLock<[u8; 4096]> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        let mut t = [0u8; 4096];
        for (i, v) in t.iter_mut().enumerate() {
            *v = (linear_to_srgb(i as f32 / 4095.0) * 255.0).round() as u8;
        }
        t
    })
}

#[inline]
fn enc(lut: &[u8; 4096], x: f32) -> u8 {
    lut[(x.clamp(0.0, 1.0) * 4095.0 + 0.5) as usize]
}

/// Everything one compose depends on. Two equal params compose equal decks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComposeParams {
    /// How much of the sky carries cloud, `0..=1` ([`deck_cover`]).
    pub cover: f32,
    /// How heavy the cloud is: greyer bases, dimmer tops.
    pub dark: f32,
    /// How far the deck has drifted downwind, noise units.
    pub drift: Vec2,
    /// Toward the sun, unit.
    pub sun: Vec3,
    /// The light on the deck, a share of full sun (`rig::sun_lux`).
    pub light: f32,
    /// How far into the night, `0..=1`: stars and moon.
    pub night: f32,
    /// Toward the moon, unit.
    pub moon: Vec3,
    /// The fog band at the horizon, `0..=1`.
    pub fog: f32,
    /// Compose a clear sky behind the cloud (a browser).
    pub backdrop: bool,
}

impl ComposeParams {
    /// The deck every judged frame was shot under: the noon sun, clear
    /// weather, no drift, no night.
    pub fn noon(backdrop: bool) -> Self {
        Self {
            cover: CLOUD_COVER,
            dark: 0.0,
            drift: Vec2::ZERO,
            sun: super::rig::to_sun(CAPTURE_DAY_FRAC),
            light: 1.0,
            night: 0.0,
            moon: Vec3::Y,
            fog: 0.0,
            backdrop,
        }
    }

    /// This frame's deck, off the weather.
    pub fn from_weather(w: &super::weather::WeatherNow) -> Self {
        Self {
            cover: deck_cover(w.cloud),
            dark: w.dark,
            drift: w.drift / CLOUD_SCALE_M,
            sun: w.sun,
            light: w.sun_lux,
            night: w.night,
            moon: w.moon,
            fog: (w.fog_sigma() * 100.0).clamp(0.0, 1.0),
            backdrop: BAKE_BACKDROP,
        }
    }
}

/// The deck's coverage for the weather's cloud: clear weather (350‰) is the
/// cover every judged frame was shot at, a storm (1000‰) is all of it.
pub fn deck_cover(cloud: f32) -> f32 {
    if cloud <= 0.35 {
        CLOUD_COVER * cloud / 0.35
    } else {
        CLOUD_COVER + (1.0 - CLOUD_COVER) * ((cloud - 0.35) / 0.65).min(1.0)
    }
}

/// Compose texels `range` (flat, face-major indices) of the cube into `out`,
/// the whole cube's RGBA8.
pub fn compose_range(
    field: &CloudField,
    geo: &TexelGeo,
    p: &ComposeParams,
    range: std::ops::Range<usize>,
    out: &mut [u8],
) {
    let lut = srgb_lut();
    let sun_h = Vec2::new(p.sun.x, p.sun.z).normalize_or_zero();
    let toward = sun_h * LIT_STEP;
    // A low sun warms what it lights; one under the horizon lights nothing.
    let dusk = if p.sun.y > -0.1 {
        ((0.25 - p.sun.y) / 0.25).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let glow = dusk * ((p.sun.y + 0.1) / 0.1).clamp(0.0, 1.0);
    let warm = [1.0, 1.0 - 0.35 * dusk, 1.0 - 0.6 * dusk];
    let cloud_light = p.light.max(NIGHT_CLOUD_LIGHT * p.night);
    let top: [f32; 3] =
        core::array::from_fn(|c| CLOUD_TOP[c] * (1.0 - 0.5 * p.dark) * warm[c] * cloud_light);
    let base: [f32; 3] =
        core::array::from_fn(|c| CLOUD_BASE[c] * (1.0 - 0.6 * p.dark) * warm[c] * cloud_light);
    let fog_rgb = fog_rgb(p.light, p.dark, 1.0);
    let thresh = 1.0 - p.cover;
    for i in range {
        let d = geo.dir[i];
        // The sky behind the cloud: nothing natively (the atmosphere paints
        // it), a graded sky in a browser — and stars and a moon on both,
        // which the atmosphere does not draw.
        let mut sky = [0.0f32; 3];
        if p.backdrop {
            let b = geo.backdrop[i];
            let lum = super::fill::luminance(b);
            for c in 0..3 {
                sky[c] = (b[c] + (lum - b[c]) * 0.7 * p.dark) * (1.0 - 0.4 * p.dark) * p.light
                    + NIGHT_SKY[c] * p.night;
            }
            if glow > 0.0 && d.y > -0.1 {
                let facing = Vec2::new(d.x, d.z).normalize_or_zero().dot(sun_h).max(0.0);
                let low = (1.0 - d.y.max(0.0) / 0.4).max(0.0);
                let g = glow * facing * facing * facing * low * low * (1.0 - 0.7 * p.dark);
                for c in 0..3 {
                    sky[c] += DUSK_GLOW[c] * g;
                }
            }
        }
        if p.night > 0.0 && d.y > 0.0 {
            let s = geo.star[i];
            if s > 0 {
                let v = s as f32 / 255.0 * STAR_PEAK * p.night;
                sky[0] += v * 0.95;
                sky[1] += v * 0.97;
                sky[2] += v;
            }
            let cm = d.dot(p.moon);
            if cm > HALO_COS {
                let h = (cm - HALO_COS) / (1.0 - HALO_COS);
                let disk = ((cm - MOON_COS_OUT) / (MOON_COS_IN - MOON_COS_OUT)).clamp(0.0, 1.0);
                for c in 0..3 {
                    sky[c] += (MOON_RGB[c] * disk + HALO_RGB[c] * h * h) * p.night;
                }
            }
        }

        // The cloud over it.
        let fade = geo.fade[i];
        let mut cov = 0.0;
        let mut cloud = [0.0f32; 3];
        if fade > 0.0 {
            let q = geo.plane[i] + p.drift;
            let f = field.sample(q.x, q.y);
            let c0 = ((f - thresh) / EDGE).clamp(0.0, 1.0);
            if c0 > 0.0 {
                cov = c0 * fade;
                // Lit top vs grey base: a texel whose sunward neighbour is
                // thinner is on a lit face — a light march in one sample.
                let f_sun = field.sample(q.x + toward.x, q.y + toward.y);
                let lit = ((f - f_sun) * 6.0 + 0.5).clamp(0.0, 1.0);
                cloud = core::array::from_fn(|c| base[c] + (top[c] - base[c]) * lit);
            }
        }
        let mut rgb: [f32; 3] = core::array::from_fn(|c| cloud[c] * cov + sky[c] * (1.0 - cov));

        // Fog lies between the eye and everything: a band at the horizon
        // the colour the ground's fog fades to, so the seam is one colour.
        if p.fog > 0.0 {
            let band = p.fog * (1.0 - (d.y + 0.02) / 0.25).clamp(0.0, 1.0);
            for c in 0..3 {
                rgb[c] += (fog_rgb[c] - rgb[c]) * band;
            }
        }

        let o = i * 4;
        if p.backdrop || cov > 0.0 || rgb[0] + rgb[1] + rgb[2] > 0.0 {
            out[o] = enc(lut, rgb[0]);
            out[o + 1] = enc(lut, rgb[1]);
            out[o + 2] = enc(lut, rgb[2]);
            out[o + 3] = 255;
        } else {
            // **Zero, and that is a hard requirement natively**: the skybox
            // pipeline has `blend: None`, so it REPLACES the background, and
            // any non-zero clear texel becomes a second sky added to the
            // atmosphere's own.
            out[o..o + 4].fill(0);
        }
    }
}

/// The colour weather fog fades to, linear, in deck units (and, as the
/// browser's haze always was, as a `DistanceFog` colour): the sky's own
/// horizon, greyed as the fog thickens, dimmed by a heavy sky and the hour,
/// never quite black after dark.
pub fn fog_rgb(light: f32, dark: f32, grey: f32) -> [f32; 3] {
    let h = backdrop_at(Vec3::X);
    let lum = super::fill::luminance(h);
    let dim = light * (1.0 - 0.45 * dark);
    core::array::from_fn(|c| (h[c] + (lum - h[c]) * grey) * dim + NIGHT_FOG[c] * (1.0 - light))
}

/// [`fog_rgb`] for a fog of extinction `sigma`: thin weather keeps the
/// sky's blue, thick fog is neutral.
pub fn fog_color(light: f32, dark: f32, sigma: f32) -> Color {
    let c = fog_rgb(light, dark, (sigma * 400.0).clamp(0.0, 1.0));
    Color::linear_rgb(c[0], c[1], c[2])
}

/// The weather fog's falloff at extinction `sigma` per metre. On the
/// desktop that is all of it — the atmosphere owns clear-weather haze, and
/// `sigma` is zero then. A browser has no atmosphere, so its island air
/// ([`browser_haze`]'s) is the floor the weather adds to.
pub fn fog_falloff(sigma: f32) -> FogFalloff {
    if BAKE_BACKDROP {
        let (extinction, inscattering) = ground_air(&super::rig::island_medium());
        FogFalloff::Atmospheric {
            extinction: extinction + Vec3::splat(sigma),
            inscattering: inscattering + Vec3::splat(sigma),
        }
    } else {
        FogFalloff::Exponential { density: sigma }
    }
}

/// The weather's `DistanceFog` — what the rig inserts once on the desktop
/// (at zero density) and `day_night` rewrites every frame.
pub fn weather_fog(light: f32, dark: f32, sigma: f32) -> DistanceFog {
    DistanceFog {
        color: fog_color(light, dark, sigma),
        falloff: fog_falloff(sigma),
        ..default()
    }
}

fn cube_image(data: Vec<u8>) -> Image {
    let n = (data.len() / (6 * 4)) as f32;
    let n = n.sqrt() as u32;
    let mut image = Image::new(
        Extent3d {
            width: n,
            height: n,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image
}

/// Build the cloud cubemap for a seed, for this target, at noon in clear
/// weather: clear texels are zero on the desktop and a sky in a browser
/// ([`BAKE_BACKDROP`]).
pub fn cloud_cubemap(seed: u64) -> Image {
    cloud_cubemap_with(seed, BAKE_BACKDROP)
}

/// Build the noon, clear-weather deck for a seed, with or without a sky
/// behind the clouds. Both are reachable natively so `tests/sky.rs` can hold
/// the browser's to the desktop's: identical wherever a cloud is opaque,
/// the backdrop exactly wherever the desktop's texel is zero.
pub fn cloud_cubemap_with(seed: u64, backdrop: bool) -> Image {
    let field = CloudField::new(seed, FIELD_N);
    let geo = TexelGeo::new(SKY_FACE, backdrop);
    let mut data = vec![0u8; geo.len() * 4];
    compose_range(
        &field,
        &geo,
        &ComposeParams::noon(backdrop),
        0..geo.len(),
        &mut data,
    );
    cube_image(data)
}

/// The deck's live state: the field and geometry it composes from, the
/// half-built next cube, and the handle the `Skybox` shows.
#[derive(Resource)]
pub struct SkyComposer {
    field: CloudField,
    geo: TexelGeo,
    scratch: Vec<u8>,
    cursor: usize,
    params: ComposeParams,
    shown: Option<ComposeParams>,
    handle: Handle<Image>,
    next_at: f32,
    composing: bool,
}

impl SkyComposer {
    /// How much deck stands in front of direction `dir` right now, `0..=1`
    /// — the sun's disk, for the rig.
    pub fn cover_at(&self, dir: Vec3, cover: f32, drift_m: Vec2) -> f32 {
        if dir.y <= DECK_CUTOFF {
            return 0.0;
        }
        let t = CLOUD_ALT_M / dir.y;
        let q = (Vec2::new(dir.x * t, dir.z * t) + drift_m) / CLOUD_SCALE_M;
        let fade = ((dir.y - DECK_CUTOFF) / HORIZON_FADE).clamp(0.0, 1.0);
        let f = self.field.sample(q.x, q.y);
        ((f - (1.0 - cover)) / EDGE).clamp(0.0, 1.0) * fade
    }
}

/// Hang the deck on the camera. Runs after the rig has spawned one. The
/// first cube is composed here, whole, at noon in clear weather — the boot
/// pays what the old bake paid — and [`compose`] takes over once the world
/// runs.
pub fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    world: Res<WorldId>,
    cam: Query<Entity, With<EyeCam>>,
) {
    let Ok(cam) = cam.single() else {
        return;
    };
    let field = CloudField::new(world.seed, FIELD_N);
    let geo = TexelGeo::new(SKY_FACE, BAKE_BACKDROP);
    let params = ComposeParams::noon(BAKE_BACKDROP);
    let mut scratch = vec![0u8; geo.len() * 4];
    compose_range(&field, &geo, &params, 0..geo.len(), &mut scratch);
    let handle = images.add(cube_image(scratch.clone()));
    commands.entity(cam).insert(Skybox {
        image: handle.clone(),
        brightness: CLOUD_NITS,
        ..default()
    });
    commands.insert_resource(SkyComposer {
        field,
        geo,
        scratch,
        cursor: 0,
        params,
        shown: Some(params),
        handle,
        next_at: 0.0,
        composing: false,
    });
}

/// Re-compose the deck when the sky has moved on: a budget of texels a
/// frame into the scratch cube, then one re-upload when it is whole. The
/// params are latched for the whole pass, so a cube is never half one sky.
pub fn compose(
    time: Res<Time>,
    weather: Res<super::weather::WeatherNow>,
    composer: Option<ResMut<SkyComposer>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut composer) = composer else {
        return;
    };
    let c = &mut *composer;
    let now = time.elapsed_secs();
    if !c.composing {
        let want = ComposeParams::from_weather(&weather);
        if now < c.next_at || c.shown == Some(want) {
            return;
        }
        c.params = want;
        c.cursor = 0;
        c.composing = true;
    }
    let total = c.geo.len();
    let end = (c.cursor + COMPOSE_BUDGET).min(total);
    compose_range(&c.field, &c.geo, &c.params, c.cursor..end, &mut c.scratch);
    c.cursor = end;
    if end == total {
        // `RENDER_WORLD` drops the CPU copy after upload, so the asset is
        // replaced whole rather than edited in place.
        let _ = images.insert(&c.handle, cube_image(c.scratch.clone()));
        c.shown = Some(c.params);
        c.composing = false;
        c.next_at = now + COMPOSE_INTERVAL_S;
    }
}
