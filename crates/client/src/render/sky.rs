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
//! The cubemap is generated at boot from the world seed. No asset, no shader,
//! no download — 6 × 256² texels built once.

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::Skybox;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};

use super::rig::{EyeCam, CAPTURE_DAY_FRAC};
use super::WorldId;

/// Cube face size in texels. 6 × 256² = 393k texels, 1.5 MB of RGBA8, built
/// once at boot in a few hundred milliseconds.
pub const SKY_FACE: u32 = 256;

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

/// Value noise on a 2D lattice, smoothstep-interpolated.
fn value2(seed: u64, x: f32, y: f32) -> f32 {
    let (xi, yi) = (x.floor(), y.floor());
    let (fx, fy) = (x - xi, y - yi);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (i, j) = (xi as i32, yi as i32);
    let a = hash3(seed, i, j, 0);
    let b = hash3(seed, i + 1, j, 0);
    let c = hash3(seed, i, j + 1, 0);
    let d = hash3(seed, i + 1, j + 1, 0);
    let top = a + (b - a) * sx;
    let bot = c + (d - c) * sx;
    top + (bot - top) * sy
}

/// Four octaves, each half the amplitude and twice the frequency. Enough
/// structure that no two cloud edges repeat inside one frame; more octaves
/// buy nothing a 256-texel face can resolve.
fn fbm(seed: u64, x: f32, y: f32) -> f32 {
    let mut f = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    for o in 0..4 {
        f += amp
            * value2(
                seed ^ (o as u64).wrapping_mul(0x9E37_79B9),
                x * freq,
                y * freq,
            );
        amp *= 0.5;
        freq *= 2.07;
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
/// each re-derive it. `sky.rs` bakes the cloud deck through this and
/// `fill.rs` bakes the hemisphere sky fill through it, and a face order that
/// disagreed between them would put the sky on the underside of every prop
/// while both files read as correct alone — the exact shape of the seam the
/// visual judge asked to be closed before the next lighting measurement is
/// taken on top of it.
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

/// The horizontal direction the deck's light march steps toward, i.e. toward
/// the sun.
///
/// **A function rather than three lines inside the bake loop, because a gate
/// cannot see into a loop body.** The rig owns the sun vector; this file
/// re-derived it from `RIG_SUN_AZIMUTH`/`RIG_SUN_ELEVATION` by hand until
/// 2026-08-15, three lines identical to `rig.rs`'s, so the deck and the
/// shadows agreed only by coincidence of two matching edits. Naming the
/// derivation is what lets `crates/client/tests/sun.rs` assert on the value
/// this file actually marches along — the first cut of that gate re-derived
/// `to_sun` on its own side and stayed green under a deliberately flipped
/// deck, which is the "a test that calls `sun_dir()` twice proves nothing"
/// trap named in `threejs-visual-validation`.
///
/// The deck is baked at NOON — [`CAPTURE_DAY_FRAC`], the one hour at which
/// `rig::sun_elevation` returns `RIG_SUN_ELEVATION` and `rig::sun_azimuth`
/// returns `RIG_SUN_AZIMUTH` — which `rig::day_night` already says out loud.
/// It named the elevation constant directly until the bearing started
/// sweeping (2026-08-15); naming the *hour* is now the only spelling that
/// cannot pick a height from one time of day and a bearing from another.
pub fn deck_march_dir() -> Vec2 {
    let s = super::rig::to_sun(CAPTURE_DAY_FRAC);
    Vec2::new(s.x, s.z).normalize_or_zero()
}

/// The rotation the baked deck is DRAWN under, so its lit faces keep facing
/// the sun as the sun's bearing sweeps.
///
/// **Why the deck rotates instead of the lit term following the sun.** The
/// lit/base split is baked into the texels ([`cloud_cubemap`] compares the
/// coverage field against a sample one step along [`deck_march_dir`]), so
/// there is no runtime term to steer — the only ways to keep the deck
/// coherent with a swept sun are to rebake it or to move it. A rebake is
/// 6 × 256² texels of four-octave fBm: a boot cost, not a frame cost, and it
/// allocates, which the trap list forbids on a client frame.
///
/// **Moving it is exact, not an approximation.** The deck is a flat plane at
/// `CLOUD_ALT_M` sampled through `1/d.y`, and both that projection and the
/// horizon fade depend only on `d.y`, which a rotation about `Y` preserves.
/// So the rotated cubemap is precisely the deck a rebake at that bearing
/// would have produced, with the noise field carried round with it. The
/// visible cost is honest and worth naming: the cloud pattern turns with the
/// light, one revolution per cycle — `RIG_SUN_ARC` of it across the daylit
/// half (≈5.7°/min at the default arc) and the rest at a `brightness` of
/// zero.
///
/// **The direction of the rotation, derived rather than tried.** `Skybox`
/// uploads `transform = rotation.inverse()` and the shader computes
/// `sample_dir = transform * ray_dir`, so a texel authored at world direction
/// `w` is *displayed* at `rotation * w`. `Quat::from_rotation_y(θ)` carries
/// the yaw-convention horizontal `(sin a, cos a)` to `(sin(a+θ), cos(a+θ))`,
/// i.e. it ADDS `θ` to a bearing. The deck's lit side is authored at
/// `RIG_SUN_AZIMUTH`, so `θ` is the current bearing minus that. A sign error
/// here drags the clouds the wrong way through the sky while the sun is
/// perfectly correct, which is the kind of half-right that reads as a
/// rendering bug rather than a lighting one —
/// `tests/sun.rs` §`the_cloud_deck_follows_the_sun_all_day` gates it against
/// the shadows at 65 hours of the day rather than at noon alone.
pub fn deck_rotation(frac: f32) -> Quat {
    Quat::from_rotation_y(super::rig::sun_azimuth(frac) - super::rig::RIG_SUN_AZIMUTH)
}

/// Build the cloud cubemap for a seed.
pub fn cloud_cubemap(seed: u64) -> Image {
    let n = SKY_FACE;
    let mut data = vec![0u8; (n * n * 6 * 4) as usize];

    let toward = deck_march_dir();

    for face in 0..6usize {
        for y in 0..n {
            for x in 0..n {
                let d = cube_dir(face, x, y, n);
                let i = (((face as u32 * n + y) * n + x) * 4) as usize;

                // Below the horizon there is no deck. Everything stays ZERO —
                // and that is a hard requirement, not tidiness: the skybox
                // pipeline has `blend: None`, so it REPLACES the background,
                // and any non-zero clear-sky texel becomes a second sky added
                // to the atmosphere's own.
                if d.y <= 0.045 {
                    continue;
                }

                // Project onto a flat deck at `CLOUD_ALT_M`. The `1/d.y` is
                // what compresses the deck toward the horizon — the reason a
                // real cloudscape has big shapes overhead and a crowded band
                // at the skyline, which no amount of noise tuning fakes.
                let t = CLOUD_ALT_M / d.y;
                let px = d.x * t / CLOUD_SCALE_M;
                let pz = d.z * t / CLOUD_SCALE_M;

                let f = fbm(seed, px, pz);
                // Coverage, softened so edges are wisps rather than a cutout.
                let cov = ((f - (1.0 - CLOUD_COVER)) / 0.22).clamp(0.0, 1.0);
                if cov <= 0.0 {
                    continue;
                }
                // Fade the last few degrees into the horizon so the deck does
                // not end in a hard line where `1/d.y` explodes.
                let horizon = ((d.y - 0.045) / 0.10).clamp(0.0, 1.0);
                let cov = cov * horizon;

                // Lit top vs grey base. The gradient of the coverage field
                // toward the sun stands in for a light march: a texel whose
                // sunward neighbour is thinner is on a lit face.
                let e = 0.35;
                let f_sun = fbm(seed, px + toward.x * e, pz + toward.y * e);
                let lit = ((f - f_sun) * 6.0 + 0.5).clamp(0.0, 1.0);

                let mut rgb = [0.0f32; 3];
                for c in 0..3 {
                    rgb[c] = (CLOUD_BASE[c] + (CLOUD_TOP[c] - CLOUD_BASE[c]) * lit) * cov;
                }
                // sRGB encode: the cubemap is sampled as `Rgba8UnormSrgb`.
                for c in 0..3 {
                    let v = rgb[c].clamp(0.0, 1.0);
                    let s = if v <= 0.0031308 {
                        v * 12.92
                    } else {
                        1.055 * v.powf(1.0 / 2.4) - 0.055
                    };
                    data[i + c] = (s * 255.0).round() as u8;
                }
                data[i + 3] = 255;
            }
        }
    }

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

/// Hang the deck on the camera. Runs after the rig has spawned one.
pub fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    world: Res<WorldId>,
    cam: Query<Entity, With<EyeCam>>,
) {
    let Ok(cam) = cam.single() else {
        return;
    };
    let image = images.add(cloud_cubemap(world.seed));
    commands.entity(cam).insert(Skybox {
        image,
        brightness: CLOUD_NITS,
        ..default()
    });
}
