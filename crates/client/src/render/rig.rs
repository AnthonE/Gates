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
use bevy::core_pipeline::Skybox;
use bevy::light::{light_consts::lux, EnvironmentMapLight, SunDisk};
use bevy::pbr::{Atmosphere, AtmosphereSettings, ScatteringMedium, ScreenSpaceAmbientOcclusion};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::{ColorGrading, Msaa};

use super::{Eye, EYE_HEIGHT};

/// Compass bearing of the sun **at solar noon**, radians (0 = +Z, increasing
/// toward +X — the sim's yaw convention, so this reads the same as every
/// other bearing in the tree). Carried over from the browser rig, which is
/// the one constant of its set that was never in dispute.
///
/// **It is the sweep's ANCHOR since 2026-08-15, not the whole answer.** This
/// used to be the sun's bearing at every hour, and the operator scored that
/// from the player's side: *"the sun donest move"*. [`sun_azimuth`] now sweeps
/// [`RIG_SUN_ARC`] of bearing across the cycle and passes through this value
/// **exactly** at [`CAPTURE_DAY_FRAC`].
///
/// **Anchoring on noon rather than on dawn is the whole reason nothing else
/// in the rig moved.** The capture register is noon ([`DayPin::capture`]),
/// `ART.md` §1's shadow-length band is measured at noon, `sky.rs` bakes its
/// deck at noon, and every frame a visual judge has ever scored was shot
/// there. A sweep anchored anywhere else would have moved all of them for a
/// reason nobody asked for; anchored here it changes only the hours nobody
/// has measured. `crates/client/tests/sun.rs` pins that.
pub const RIG_SUN_AZIMUTH: f32 = 2.35;

/// How much compass bearing the sun sweeps between sunrise and sunset,
/// radians.
///
/// ⚠ **PROPOSED — this number awaits the operator's word.** It is a tunable
/// no doc has spoken, so per `CLAUDE.md` it ships carrying its derivation
/// rather than being quietly picked, and it belongs in `DECISIONS.md` §open
/// ("sun arc v0") the day it is spoken.
///
/// **Derived, not chosen.** [`sun_elevation`] puts the sun on the horizon at
/// dawn, at [`RIG_SUN_ELEVATION`] at noon and back on the horizon at dusk,
/// symmetric about noon. A real sun whose path starts and ends exactly on the
/// horizon and is symmetric about noon is the **equinoctial** one, and at the
/// equinox the sun rises due east and sets due west at *every* latitude — a
/// bearing sweep of exactly π. The same curve's 35° peak fixes the matching
/// latitude at 90° − 35° = 55°, which is the band `ART.md` §1's temperate
/// pine-and-granite island already implies. So the two halves of the sun's
/// path agree about where this island is, which is the property that makes
/// this a derivation and not a preference.
///
/// **What a smaller value would buy, because that is the operator's actual
/// choice.** The v0 argument for a fixed bearing was legibility — `to_sun`'s
/// own doc said a sweep "would drag every shadow through a half-turn a cycle
/// for no legibility gain". π *is* that half-turn, spread over the 31.5 min
/// daylit half: 5.7° of shadow rotation per minute. A smaller arc holds a
/// shadow's direction more nearly constant and crowds sunrise and sunset
/// toward the noon bearing; below ~1.58 rad (91°) they both fall on the same
/// side of the noon bearing's meridian and the sun reads as wobbling rather
/// than crossing the sky.
pub const RIG_SUN_ARC: f32 = std::f32::consts::PI;

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

/// The cascade split's shape — how the sun's shadow distance is divided, as
/// opposed to how far it reaches or how many pieces it is cut into.
///
/// **Named here because `render/quality.rs` builds the same config at three
/// tiers and these three do not move between them.** The count and the
/// maximum distance are a budget; where the resolution steps is a look, and a
/// tier that re-split it would be changing the picture rather than its cost.
pub const CASCADE_MIN_M: f32 = 0.2;
/// See [`CASCADE_MIN_M`].
pub const CASCADE_FIRST_M: f32 = 12.0;
/// See [`CASCADE_MIN_M`].
pub const CASCADE_OVERLAP: f32 = 0.2;

/// The sun, and the only directional light.
#[derive(Component)]
pub struct Sun;

pub fn setup(
    mut commands: Commands,
    mut media: ResMut<Assets<ScatteringMedium>>,
    mut images: ResMut<Assets<Image>>,
) {
    // The hemisphere sky fill (`fill.rs`). Built here rather than in its own
    // setup system because the fill is a member of the coupled set and rule 5
    // means one owner — the same reason the sun, the exposure and the tone
    // mapper are all in this function.
    let fill_map = images.add(super::fill::cubemap(super::fill::FILL_FACE));

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
        // **Grouped, and it has to be.** Bevy implements `Bundle` for tuples
        // up to 16 elements and this camera reached exactly 16; a 17th fails
        // as "`(..., ..., ...)` is not a `Bundle`" pointing at the `spawn`
        // rather than at the component that overflowed it. The tonal three —
        // what the scene is exposed at, the curve it is mapped through, and
        // the grade applied after — are one nested tuple, which is also the
        // grouping `ART.md` rule 5 describes: one owner for the transfer.
        (
            Exposure { ev100: 14.2 },
            // Chosen by measurement in a later slice, not by name (`RENDER.md`
            // §4 R2/R5). TonyMcMapface is Bevy's default and is neutral with a
            // gentle roll-off; what it is NOT is the browser's Khronos PBR
            // Neutral, whose `x - 6.25x²` toe under 0.08 squared the shadows and
            // delivered a face arriving at linear 0.02 as 8/255.
            Tonemapping::TonyMcMapface,
            // **The tonal instrument, and it is deliberately at identity.**
            //
            // Until now there was no grading stage of any kind in this client:
            // `rg 'ColorGrading|AutoExposure' crates/client/src` returned nothing,
            // so the frame's transfer was a fixed `ev100` and a tone curve and
            // there was no third thing to turn. That matters because `RENDER.md`
            // §0's own measured gap against the reference set is *exactly* a
            // grading gap — the darks read ~30 luma too bright, the highlights ~15
            // too dim, and chroma-per-luma is ~32% short — and a missing
            // instrument is why `NOW.md` §0fill has been re-deferred pass after
            // pass with the cause already solved.
            //
            // **Identity, not a grade**, and the distinction is the whole reason
            // this is safe to land unlooked-at. Every field below is Bevy's
            // default, so the frame does not move by one level from this line; it
            // exists so the next pass has a knob to turn instead of a slice to
            // build. Choosing the values is the coupled owner's job with the game
            // open — `CLAUDE.md` measured three parallel passes at that making
            // things worse 60→66 and one sequential owner cutting them to 26 —
            // and the numbers go to `DECISIONS.md` §open when they are chosen,
            // never invented here.
            ColorGrading::default(),
        ),
        // **The sky fill, and it is a HEMISPHERE now** (`fill.rs`) — `ART.md`
        // §4 asks for one in those words ("sky half cool blue, earth half
        // warm"), and the line below used to say why we could not have it:
        // Bevy's `AmbientLight` is a uniform term, so the sky-half/earth-half
        // split was "owed to a later slice". This is that slice, and the debt
        // is paid without a custom material — `environment_map.wgsl` samples
        // its diffuse cubemap BY THE WORLD NORMAL, so a cubemap holding
        // `fill_at(n)` is a hemisphere light exactly.
        //
        // The magnitude is unchanged where it can be seen: `fill_at(+Y)` is
        // the same irradiance the uniform term delivered, so open ground
        // facing the sky does not move. What moves is down-facing faces, which
        // now receive the ground's own warm bounce instead of a blue sky they
        // are not looking at — the mechanism behind the visual judge's "the
        // tree base measures flat within ±5% right up to the trunk", which is
        // `ART.md` rule 2's decal edge stated as a number.
        //
        // Indirect-only and occlusion-weighted for free: `pbr_functions.wgsl`
        // accumulates `environment_light.diffuse * diffuse_occlusion`, and the
        // SSAO below is folded into `diffuse_occlusion` by `min()`. That is
        // §4's occlusion law — both halves of it — without a line of WGSL.
        EnvironmentMapLight {
            diffuse_map: fill_map.clone(),
            // The same map. A hemisphere IS the low-frequency environment a
            // rough surface reflects, and every ground material here is
            // 0.92 perceptual roughness, so the specular lookup wants exactly
            // this and nothing sharper. One mip, so `smallest_specular_mip_
            // level = mip_level_count - 1 = 0` and the roughness-to-mip term
            // resolves to level 0 rather than reading past the end.
            specular_map: fill_map,
            intensity: super::fill::peak_lux(),
            rotation: Quat::IDENTITY,
            affects_lightmapped_mesh_diffuse: false,
        },
        // What is LEFT of the uniform term: the night floor, and nothing else.
        // `day_night` drives this to zero across dawn as the hemisphere above
        // takes over, so the two never both carry the day — which would double
        // the fill. At night it is the whole fill, unchanged from what this
        // shipped: `NIGHT_AMBIENT_LUX` at the sky's own chroma.
        //
        // Zero at noon is not a hole in `ART.md` rule 3's "no pure black":
        // the hemisphere's earth half is a much better floor than a uniform
        // term ever was, because it is nonzero in every direction and warm
        // where the ground is.
        AmbientLight {
            color: Color::srgb(0.80, 0.85, 0.95),
            brightness: 0.0,
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
        // …and Medium is now `Quality::High`'s row rather than a literal
        // here. `rig::setup` spawns the DEFAULT tier, so a fresh boot draws
        // exactly what it drew before tiers existed and `quality::apply`
        // writes nothing until a player moves the knob.
        ScreenSpaceAmbientOcclusion {
            // **The DEFAULT tier, not the player's** — a bundle is static
            // and the LOW rung carries no such component at all, so a rig
            // that tried to spawn the current tier could express two of the
            // three rungs and not the third. `quality::apply` reconciles on
            // the frame this camera appears (it watches `Added<EyeCam>` for
            // exactly this), so a persisted LOW is corrected before the first
            // frame is drawn rather than being a bundle shape.
            quality_level: super::quality::tier(crate::config::Quality::default())
                .ssao
                .expect("the default tier carries ambient occlusion"),
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
        // Four cascades to 200 m — the near ring is 5×5 chunks of 64 m, so
        // 160 m from the player to its corner and shadows past the ring have
        // nothing standing in them. **Both numbers are `Quality::High`'s row
        // now**, and the shape of the split ([`CASCADE_MIN_M`] and its two
        // neighbours) is what does not move between tiers.
        super::quality::tier(crate::config::Quality::default()).cascades(),
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
/// The spawn pose — noon; `day_night` rewrites it every frame from the
/// server's clock, so this is only what a frame with no snapshot yet shows.
///
/// [`CAPTURE_DAY_FRAC`] rather than a bare elevation since the sweep landed:
/// noon is now two numbers (a height *and* a bearing) and naming the hour is
/// the only spelling that cannot pair one with the other's.
fn sun_rotation() -> Quat {
    sun_rotation_at(CAPTURE_DAY_FRAC)
}

/// **Where the sun is** — the unit vector from the world origin toward it, at
/// a point in the day/night cycle.
///
/// **It takes the HOUR, not an elevation, and that is the single-owner
/// property rather than a convenience.** Until 2026-08-15 the sun climbed and
/// sank on one fixed bearing, so an elevation was a complete answer and this
/// function read [`RIG_SUN_AZIMUTH`] as a constant; its own doc argued the
/// case ("a sweeping azimuth would drag every shadow through a half-turn a
/// cycle for no legibility gain"). The operator scored that from the other
/// side — *"the sun donest move"* — so the sun now has two coordinates that
/// both move, and a caller holding only one of them can pair this morning's
/// height with this afternoon's bearing while every gate stays green. One
/// argument in, both derived here, no such pairing exists.
///
/// **The one owner, and the reason it is `pub`.** Two consumers need this
/// vector and they must not each re-derive it: the `Sun` light's rotation
/// below, and `sky.rs`'s cloud-deck light march, which re-wrote these same
/// three lines by hand until 2026-08-15. They agreed — but by coincidence
/// of two identical edits, not by construction, and the visual judge
/// (`pass-20260814-223652-01-visual.md`, ranked gap 2a) asked for exactly
/// this seam to be closed before the next lighting measurement is taken on
/// top of it.
///
/// **`to_sun`, not "the light direction", and the name is the trap.** Bevy
/// hands the GPU `direction_to_light = transform.back()` for BOTH the `N·L`
/// term and the atmosphere's sun disc (`bevy_pbr` `light.rs`, "direction is
/// negated to be ready for N.L"), while `Transform::forward()` is the
/// direction the light *travels*. So the law is `direction_to_light ==
/// -forward() == to_sun`, and [`sun_rotation_at`] is the only place that
/// negation is written. `crates/client/tests/sun.rs` gates it.
pub fn to_sun(frac: f32) -> Vec3 {
    to_sun_at(sun_elevation(frac), sun_azimuth(frac))
}

/// The spherical construction on its own: a unit vector at an arbitrary
/// height and bearing, in the sim's yaw convention.
///
/// `pub` for one reason — a gate that wants to check the *geometry* at an
/// elevation the cycle never reaches (`tests/sun.rs` hand-checks a shadow
/// length at 80° and at 5°) must be able to ask for one without inventing an
/// hour that would produce it. Nothing in the renderer calls this; every
/// shipped path goes through [`to_sun`], which is what keeps the height and
/// the bearing from being sourced separately.
pub fn to_sun_at(elevation: f32, azimuth: f32) -> Vec3 {
    let (se, ce) = (elevation.sin(), elevation.cos());
    let (sa, ca) = (azimuth.sin(), azimuth.cos());
    Vec3::new(sa * ce, se, ca * ce)
}

/// The sun's rotation at a point in the cycle: a light whose forward is
/// the direction sunlight travels, i.e. `-to_sun`.
pub fn sun_rotation_at(frac: f32) -> Quat {
    Transform::default()
        .looking_at(-to_sun(frac), Vec3::Y)
        .rotation
}

// ── Day and night (day/night v0 — DECISIONS.md §open) ────────────────────
//
// One owner, still: the cycle lives HERE because it moves the sun, the
// ambient, and (through `daylight`) the cloud deck — the coupled set §2
// reserves for this file. The clock is the server's tick, derived through
// `sim_core::world::day_frac` off the smoothed estimate `Feed` carries, so
// two clients at one campfire see one sky.

/// How far below the horizon the sun sinks at deep night, radians. Deep
/// enough that the atmosphere's twilight band clears the horizon glow.
pub const NIGHT_DIP: f32 = 0.35;

/// Ambient at deep night, lux. "Genuinely dark" (`ALPHA.md` §1) against
/// the fixed ev100 14.2 exposure: shapes read at arm's length, the ridge
/// line is gone, and a torch will one day be worth carrying. Moonlight is
/// ~0.25 lux; this is far brighter because the exposure never adapts —
/// the number is a look, not a photometric claim.
pub const NIGHT_AMBIENT_LUX: f32 = 60.0;

/// The sun's elevation at a point in the cycle: a sine arch over the day
/// portion peaking at `RIG_SUN_ELEVATION`, a shallower inverted arch
/// below the horizon across the night.
pub fn sun_elevation(frac: f32) -> f32 {
    use std::f32::consts::PI;
    let day = sim_core::limits::DAY_PORTION;
    if frac < day {
        (PI * frac / day).sin() * RIG_SUN_ELEVATION
    } else {
        -((PI * (frac - day) / (1.0 - day)).sin()) * NIGHT_DIP
    }
}

/// The sun's bearing at a point in the cycle, radians in the same convention
/// as [`RIG_SUN_AZIMUTH`] — and **unwrapped**, so it falls monotonically
/// across the whole cycle and lands one full turn below its dawn value at the
/// next dawn. Nothing that reads it has to think about a seam at zero;
/// [`to_sun`] takes a sine and a cosine of it and `sky.rs` takes a
/// difference, and both are blind to the turn.
///
/// **The bearing FALLS, and the sign is the one thing here that a picture
/// cannot be wrong about quietly.** East is `-X` (`DECISIONS.md` 2026-08-15,
/// `tests/compass.rs`) while this angle is the sim's yaw convention (0 = +Z,
/// increasing toward +X), so the two run opposite ways: `look::bearing_of` is
/// `360° − yaw`. A sun that travels east → west is therefore one whose yaw
/// angle decreases. Backwards, it rises in the west — which is invisible to
/// `test_protocol_golden`, to `test_replay`, to clippy and to every pixel
/// statistic this repo owns, exactly like the mirrored compass was. The gate
/// is `tests/sun.rs` §`the_sun_travels_the_way_the_compass_calls_east_to_west`,
/// and it asks the compass's own owner rather than restating the convention.
///
/// **Day:** [`RIG_SUN_ARC`] of sweep, linear in the fraction and symmetric
/// about noon. Linear rather than a true solar azimuth curve on purpose —
/// the elevation arch next door is already a stylised sine, and precision on
/// one axis of a two-axis path buys nothing but the illusion of rigour.
///
/// **Night:** the sun keeps going the *same way* around, under the world,
/// covering the remaining `2π − RIG_SUN_ARC` so the next dawn starts exactly
/// where the last one did. That is not bookkeeping — it is why the twilight
/// band sits where the sun *set* for the first hour of night and swings round
/// to where it will *rise* before dawn, which is what the atmosphere draws off
/// the light's transform with no help from us.
pub fn sun_azimuth(frac: f32) -> f32 {
    use std::f32::consts::TAU;
    let day = sim_core::limits::DAY_PORTION;
    if frac < day {
        // Exactly `RIG_SUN_AZIMUTH` at `frac == day * 0.5`: `(day*0.5)/day` is
        // 0.5 to the bit (IEEE division of a value by twice itself), so noon
        // is bit-identical to what this rig shipped before the sweep.
        RIG_SUN_AZIMUTH + RIG_SUN_ARC * (0.5 - frac / day)
    } else {
        RIG_SUN_AZIMUTH - RIG_SUN_ARC * 0.5 - (TAU - RIG_SUN_ARC) * ((frac - day) / (1.0 - day))
    }
}

/// How much of full daylight the frame gets, `0..=1`: the sun's height as
/// a fraction of its noon height, zero from the moment it dips. One
/// scalar feeds the sun's illuminance, the ambient lerp and the cloud
/// deck's brightness, so the three cannot disagree about what time it is.
pub fn daylight(frac: f32) -> f32 {
    (sun_elevation(frac).sin() / RIG_SUN_ELEVATION.sin()).clamp(0.0, 1.0)
}

// ── The capture probe's clock (capture clock v0 — DECISIONS.md §open) ────

/// The tick whose [`sim_core::world::day_frac`] is `want` — the inverse of
/// the sim's forward map, and the only one in the tree. Written here rather
/// than in the test that used to carry a private copy: an inverse nothing
/// checks against the forward function is a second clock model, and
/// `tests/daynight.rs` now round-trips this one through `day_frac` instead
/// of agreeing with itself.
pub fn tick_at_frac(want: f32) -> u64 {
    use sim_core::limits::{DAY_PHASE_TICKS, DAY_TICKS};
    ((want * DAY_TICKS as f32) as u64 + DAY_TICKS - DAY_PHASE_TICKS) % DAY_TICKS
}

/// The hour a `--capture` run draws: the sine arch's peak. This is the ONE
/// fraction at which [`sun_elevation`] returns [`RIG_SUN_ELEVATION`] and
/// [`daylight`] returns 1.0 exactly — so it is derived from the register the
/// rig is authored against, not a number chosen for a picture.
pub const CAPTURE_DAY_FRAC: f32 = sim_core::limits::DAY_PORTION * 0.5;

/// **Which tick the renderer reads the hour from.** `None` in play — the
/// server's smoothed estimate, so two clients at one campfire see one sky.
/// `Some` on a `--capture` run, where the hour is pinned.
///
/// **Why a probe pins its clock at all**, and it is the fullscreen rule one
/// field further (`DECISIONS.md` §Spoken 2026-08-13: a `--capture` run takes
/// the defaults wholesale *except* what would let the box decide the frame).
/// A capture shard boots at tick 0 and the probe fires whenever the client
/// finished building and settling — so the sun's height was a function of
/// how long `cargo` took. Measured on this tree: tick 0 is **24.5°**, the
/// ~2 200 ticks a build costs put it at **27.3°**, and a slower box reaches
/// **30.4°** — a ~6° swing, all of it below or at the edge of `ART.md` §1's
/// 30–40° band, and monotonically rising, so a slower box scored a brighter
/// frame. No frame any visual judge has scored was taken at the register
/// this rig is tuned for.
///
/// **It pins the TICK, not the sun**, because the sun is not the only reader:
/// `render/audio.rs` gates the bird layer on `world::is_night` off the same
/// estimate. Pinning a sun elevation would leave the birds at the box's hour
/// and the light at the pinned one — the seam `to_sun` was just closed for,
/// re-opened one level up. One tick in, every consumer derives from it
/// through the sim's own `day_frac`/`is_night`, so there is still exactly
/// one model of what a tick means.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DayPin(pub Option<u64>);

impl DayPin {
    /// The pin a `--capture` run gets: noon, the rig's authored register.
    pub fn capture() -> Self {
        Self(Some(tick_at_frac(CAPTURE_DAY_FRAC)))
    }

    /// The tick to read the hour from. Every clock consumer in the renderer
    /// goes through here.
    pub fn tick(&self, server_tick_est: f64) -> u64 {
        self.0.unwrap_or_else(|| server_tick_est.max(0.0) as u64)
    }
}

/// Drive the rig from the server's clock. Runs every frame; the queries
/// are empty until `setup` has spawned the rig, which makes the system a
/// no-op on every screen that is not the world.
pub fn day_night(
    feed: Res<super::feed::Feed>,
    pin: Res<DayPin>,
    mut sun: Query<(&mut Transform, &mut DirectionalLight), With<Sun>>,
    mut cam: Query<
        (
            &mut AmbientLight,
            &mut EnvironmentMapLight,
            Option<&mut Skybox>,
        ),
        With<EyeCam>,
    >,
) {
    let frac = sim_core::world::day_frac(pin.tick(feed.server_tick_est));
    let light = daylight(frac);
    if let Ok((mut t, mut d)) = sun.single_mut() {
        t.rotation = sun_rotation_at(frac);
        d.illuminance = lux::DIRECT_SUNLIGHT * light;
        // A light at zero illuminance still schedules its cascades;
        // shadows off at night saves the passes and no shadow is cast by
        // a sun that is under the ground anyway.
        d.shadows_enabled = light > 0.0;
    }
    if let Ok((mut amb, mut env, sky)) = cam.single_mut() {
        // **The handover, and the two terms are complements by construction.**
        // The hemisphere carries the day and the uniform term carries the
        // night, and each is scaled by the same `light` so their sum is
        // continuous across dawn and neither can double-count the other. At
        // `light == 1` the fill is exactly `fill_at(n)`; at `light == 0` it is
        // exactly `NIGHT_AMBIENT_LUX`, which is what this shipped before the
        // hemisphere existed. Both endpoints are gated (`tests/fill.rs`).
        //
        // Driving the env map by `intensity` rather than rebaking the cubemap
        // is the whole reason the map is stored normalized: the shape of the
        // hemisphere does not change with the hour, only its scale, so a
        // per-frame multiply replaces a per-frame 24 KB bake.
        amb.brightness = NIGHT_AMBIENT_LUX * (1.0 - light);
        env.intensity = super::fill::peak_lux() * light;
        if let Some(mut sky) = sky {
            // At night clouds are dark from below anyway.
            sky.brightness = super::sky::CLOUD_NITS * light;
            // **The half of the sweep that is not the sun.** The deck is baked
            // ONCE, with its lit faces pointing at the noon sun (`sky.rs`).
            // Under a fixed bearing that was correct at every hour for free —
            // only the sun's *height* moved and the deck's march ignores
            // height. A swept bearing breaks it: by dusk the lit side of every
            // cloud would face 90° away from the sun that is lighting the
            // ground, which is `ART.md` rule 5's coupled set disagreeing with
            // itself, and the visual judge has ranked exactly that seam before
            // (`pass-20260814-223652-01-visual.md`, gap 2a).
            //
            // Rotating the deck about `Y` by the bearing's own offset from
            // noon fixes it *exactly* rather than approximately: the deck is a
            // flat plane projected through `1/d.y`, which is invariant under a
            // yaw rotation, so the rotated cubemap is bit-for-bit the deck a
            // rebake at that bearing would produce — and a rebake is 393 k
            // texels of four-octave fBm, which is a boot cost, not a frame
            // one. The shapes drift with the light, one turn per cycle, most
            // of it under a `brightness` of zero.
            sky.rotation = super::sky::deck_rotation(frac);
        }
    }
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
