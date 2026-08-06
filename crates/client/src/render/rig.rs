//! The light rig — **one owner** (`ART.md` rule 5, `CLAUDE.md`'s
//! coupled-lighting law). Sun, sky, ambient, exposure, tone map and aerial
//! perspective are one set: three parallel passes over them worsened defects
//! 60→66 where one sequential owner cut them to 26. Nothing else in this
//! crate creates a light, sets an exposure, or touches a tone mapper.
//!
//! **What is different from the browser rig, and it is the whole reason the
//! sun could move.** `web/src/scene.js` documents, with a measured table, why
//! `SUN_ELEVATION` was stuck at 0.36 rad (20.6°) against `ART.md` §1's
//! 30–40° band: the ground's entire relief was a bump field, a normal
//! perturbed by δ changes `N·L` on flat ground by `cot(elevation)·δ`, and
//! raising the sun to 45° left 0.47% of the frame moving where 12.81% moved
//! at 0.36. Here the relief is the mesh itself and the population standing on
//! it, neither of which is a perturbed normal, so the register is midday.
//!
//! The sky is Bevy's Bruneton atmosphere rather than the hand-fitted gradient
//! dome plus fog seam the browser carried. That retires four coupled knobs
//! (`SKY_HAZE_TOP`, `SKY_CURVE`, `SKY_GAIN`, `FOG_NEAR`/`FOG_FAR`) in favour
//! of one physical model that also supplies `ART.md` §1's "air has depth" for
//! free: the aerial-perspective LUT lightens and desaturates distance toward
//! the sky's own colour, which is exactly what the fog seam was being tuned
//! by hand to fake.

use bevy::anti_alias::smaa::Smaa;
use bevy::camera::Exposure;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::{light_consts::lux, CascadeShadowConfigBuilder, SunDisk};
use bevy::pbr::{Atmosphere, AtmosphereSettings, ScatteringMedium, ScreenSpaceAmbientOcclusion};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::Msaa;

use super::{Eye, EYE_HEIGHT};

/// Compass bearing of the sun, radians (0 = +Z, increasing toward +X — the
/// sim's yaw convention, so this reads the same as every other bearing in the
/// tree). Carried over from the browser rig, which is the one constant of its
/// set that was never in dispute.
pub const RIG_SUN_AZIMUTH: f32 = 2.35;

/// Sun elevation above the horizon, radians. 0.61 rad = 35°, the middle of
/// `ART.md` §1's measured band: shadows in `generichighview2.jpg` run 1.5–2×
/// the height of what casts them, which is a sun in the 30–40° band. The
/// browser's 0.36 is NOT carried across — see the header.
///
/// **Named for the rig rather than `SUN_ELEVATION`, and the gate is the
/// reason.** `SUN_ELEVATION` is a registered knob (`DECISIONS.md` §open,
/// "lighting v1") pinned to `web/src/scene.js` at 0.36, and
/// `ci/knob_registry.mjs` refuses one name meaning two things — correctly,
/// because for as long as both renderers ship, "the sun's elevation" really
/// is two numbers in this repo. This is not a rename to dodge a gate: the
/// browser's 0.36 stays true of the browser, and when `web/` is retired this
/// takes the plain name and the registry row moves with it.
pub const RIG_SUN_ELEVATION: f32 = 0.61;

/// Field of view, degrees — **VERTICAL**, and the axis matters enough to say
/// twice. `PerspectiveProjection::fov` is documented in `bevy_camera` as "the
/// vertical field of view", and `THREE.PerspectiveCamera`'s first argument is
/// vertical too, so 75 here is the same 75 the browser client shipped and the
/// same one `DECISIONS.md` §open registers under client cosmetics. The two
/// clients frame identically.
///
/// This comment previously called it horizontal. That was wrong about the
/// name and right about the number, and "fix" it the obvious way — converting
/// 75 horizontal to ~46.7 vertical at 16:9 — would have silently narrowed the
/// frame and broken agreement with a registered knob.
pub const FOV_DEG: f32 = 75.0;
/// Far plane, metres. The island is 2048 m across and the far mesh draws all
/// of it.
pub const FAR_M: f32 = 2000.0;

/// How much thicker than earthlike this island's air is. PROPOSED — it is a
/// number this path invented, so it registers in `DECISIONS.md` §open the day
/// it is spoken; until then it carries its derivation and its measurement.
///
/// Derivation: 2 km of sightline against an atmosphere modelled for tens of
/// kilometres, so the far third of the frame arrives with almost no haze.
///
/// **Measurement, and it is `ART.md` rule 5 catching a hand in the till.**
/// The first value tried was 6.0 and it was chosen for what it does to the
/// SKY. It works there — sky mean 120 → 135 against the reference's 128 — but
/// the medium extinguishes the sun on its way down as well as scattering it
/// sideways, and the same frame's ground fell from near 84 to 63 with
/// saturation climbing 31% → 40%: midday turned into a hazy sunset. Air
/// density is not a sky knob. It is a member of the coupled set, and 1.6 is
/// what the whole set tolerates.
pub const AIR_DENSITY: f32 = 1.6;

/// The camera, and the only camera.
#[derive(Component)]
pub struct EyeCam;

/// The sun, and the only directional light.
#[derive(Component)]
pub struct Sun;

pub fn setup(mut commands: Commands, mut media: ResMut<Assets<ScatteringMedium>>) {
    // The atmosphere needs a medium asset; the earthlike default is Rayleigh
    // + Mie + ozone, which is the model `ART.md` §1's "distant hills lighten,
    // desaturate and go blue" describes the output of.
    // …with the air thickened. The earthlike default is fitted to a planet
    // whose sightlines are tens of kilometres; this island is 2 km across, so
    // at the default density the far third of a frame arrives with almost no
    // haze on it and `ART.md` §1's "distant hills lighten, desaturate and go
    // blue" does not happen inside the world we actually draw. The multiplier
    // is the honest knob for it — it thickens the medium rather than painting
    // a fog colour over the result, so the sky and the haze stay the same
    // physical quantity and the horizon seam cannot open (which is the exact
    // seam the browser rig spent 200 lines of comment hand-fitting).
    let medium = media.add(ScatteringMedium::default().with_density_multiplier(AIR_DENSITY));

    commands.spawn((
        super::WorldEntity,
        EyeCam,
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: FOV_DEG.to_radians(),
            near: 0.1,
            far: FAR_M,
            ..default()
        }),
        Atmosphere::earthlike(medium),
        AtmosphereSettings {
            // The island is 2 km across, not 32 km. Fitting the aerial-view
            // LUT to the world we actually draw is what puts its slices where
            // the geometry is instead of spending all 32 of them on the first
            // 6% of the frustum.
            aerial_view_lut_max_distance: FAR_M,
            ..default()
        },
        // Sunlit exterior, pulled 0.8 stop off Bevy's `Exposure::SUNLIGHT`
        // (ev100 15). MEASURED, not chosen: the first native capture read
        // p50 61 / p90 111 / sky 97 against the reference median's 91 / 170 /
        // 128 (`ci/native_bar.py`, both sides through one estimator), which is
        // most of a stop of missing image. This is the rig's exposure and
        // nothing else sets one: if a surface is blown, its albedo is wrong
        // (`ART.md` §4).
        Exposure { ev100: 14.2 },
        // Chosen by measurement in a later slice, not by name (`RENDER.md`
        // §4 R2/R5). TonyMcMapface is Bevy's default and is neutral with a
        // gentle roll-off; what it is NOT is the browser's Khronos PBR
        // Neutral, whose `x - 6.25x²` toe under 0.08 squared the shadows and
        // delivered a face arriving at linear 0.02 as 8/255.
        Tonemapping::TonyMcMapface,
        // The sky fill, and `ART.md` rule 3's 0.30 floor lives here: no lit
        // surface's shaded face may fall below 0.30 of its lit face, and
        // outdoors that floor is the sky filling every upward face. Bevy's
        // ambient is a per-camera COMPONENT since 0.17 and a uniform term
        // rather than the browser's hemisphere, so it cannot be split
        // sky-half/ground-half — the down-facing-prop-face half of that split
        // is owed to a later slice and `RENDER.md` records it.
        AmbientLight {
            // Cool, not blue. The first capture's boulder had a NAVY shaded
            // face — 0.62/0.72/0.92 at a third of daylight is a saturated
            // blue doing all the work on every unlit surface, and the frame
            // measured 42% near-band saturation against the reference's 33%.
            color: Color::srgb(0.80, 0.85, 0.95),
            // **The floor is arithmetic, and the first cut missed it by 10×.**
            // `ART.md` rule 3: no lit surface's shaded face may fall below
            // 0.30 of its lit face. A 35° sun at 100,000 lux delivers
            // 100000·sin(35°) ≈ 57,000 lux to flat ground, so the fill that
            // reaches 0.30 of that is ~17,000 — not the 3,500 this shipped
            // with. Outdoors that is not a fudge: the sky really is a
            // 17,000-lux hemisphere and every upward face is looking at it.
            brightness: lux::AMBIENT_DAYLIGHT * 1.7,
            affects_lightmapped_meshes: false,
        },
        // ── The three post terms, and they belong to this owner too ──────
        //
        // MSAA OFF is not a preference: `bevy_pbr`'s SSAO node checks it and
        // warns that "SSAO is being used which requires Msaa::Off". Off costs
        // nothing here because SMAA below is a post-process resolve, not a
        // sample-count one.
        Msaa::Off,
        // **AO is how the fill's cost gets paid back.** Raising the ambient to
        // reach `ART.md` rule 3's 0.30 floor lifted the whole frame including
        // the darks — p10 went 41.9 → 64.9 against a reference of 41.0. That
        // is the documented failure mode of a global fill, and §4 of the art
        // bible states the fix in one line: raise the fill and put the
        // darkness back where it belongs. AO removes ambient only where
        // geometry occludes, which is under every boulder, inside every
        // canopy, and in the crease where a prop meets the ground.
        //
        // Medium, not the `High` default: this renders on a CPU rasterizer in
        // the gate and High is 18 samples per pixel.
        ScreenSpaceAmbientOcclusion {
            quality_level: bevy::pbr::ScreenSpaceAmbientOcclusionQualityLevel::Medium,
            ..default()
        },
        // SMAA rather than FXAA (blurrier) or TAA (needs motion vectors and a
        // temporal history a fresh-process capture does not have). Nothing in
        // this client had ANY anti-aliasing before: every pine edge, every
        // grass blade and every granite facet was a hard stair.
        Smaa::default(),
        // Bloom, and its first job is the sun. `SunDisk`'s own doc says it:
        // "In order to cause the sun to 'glow' and light up the surrounding
        // sky, enable bloom in your post-processing pipeline." The disk has
        // been rendering all along — a 9 mrad white dot with nothing around
        // it. NATURAL is the energy-conserving preset with no threshold, so
        // it scatters what is genuinely bright instead of hunting for pixels
        // over a cutoff.
        Bloom::NATURAL,
        Transform::from_xyz(0.0, EYE_HEIGHT, 0.0),
    ));

    commands.spawn((
        super::WorldEntity,
        Sun,
        DirectionalLight {
            // White. The atmosphere reddens it on its way down now, which is
            // what the browser's hand-picked 0xfff4e2 was standing in for.
            color: Color::WHITE,
            illuminance: lux::DIRECT_SUNLIGHT,
            shadows_enabled: true,
            ..default()
        },
        CascadeShadowConfigBuilder {
            num_cascades: 4,
            minimum_distance: 0.2,
            // The near ring is 5×5 chunks of 64 m — 160 m from the player to
            // its corner. Shadows past the ring have nothing standing in them.
            maximum_distance: 200.0,
            first_cascade_far_bound: 12.0,
            overlap_proportion: 0.2,
        }
        .build(),
        // The disk in the sky. It renders by default through the atmosphere
        // (`SunDisk::EARTH` is what an absent component resolves to), and it
        // is stated explicitly here for two reasons: it is a member of the
        // coupled set and so belongs in this file where the owner can see it,
        // and the intensity is a knob the reference frames have an opinion
        // about. 1.6 rather than the physical 1.0 — the references are
        // photographs with a real camera's flare and glare, and the disk is
        // what bloom has to work on.
        SunDisk {
            intensity: 1.6,
            ..SunDisk::EARTH
        },
        Transform::from_rotation(sun_rotation()),
    ));
}

/// The sun's rotation: a light pointing along the direction sunlight travels.
fn sun_rotation() -> Quat {
    let (se, ce) = (RIG_SUN_ELEVATION.sin(), RIG_SUN_ELEVATION.cos());
    let (sa, ca) = (RIG_SUN_AZIMUTH.sin(), RIG_SUN_AZIMUTH.cos());
    // Where the sun sits, as a unit vector from the world origin.
    let to_sun = Vec3::new(sa * ce, se, ca * ce);
    Transform::default().looking_at(-to_sun, Vec3::Y).rotation
}

/// The camera rides the eye. Written here rather than in `input` because the
/// camera is the rig's, and one owner means one owner.
pub fn follow_eye(eye: Res<Eye>, mut cam: Query<&mut Transform, With<EyeCam>>) {
    let Ok(mut t) = cam.single_mut() else {
        return;
    };
    t.translation = eye.pos;
    // The sim's convention: yaw 0 faces +Z, increasing toward +X, pitch +up.
    // `web/src/scene.js` builds the same direction — the two clients must
    // agree about where "forward" points or an aim cone means two things.
    let cp = eye.pitch.cos();
    let dir = Vec3::new(eye.yaw.sin() * cp, eye.pitch.sin(), eye.yaw.cos() * cp);
    t.look_to(dir, Vec3::Y);
}
