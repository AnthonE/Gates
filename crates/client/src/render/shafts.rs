//! Light shafts through the weather's fog — god rays, the way the reference
//! game's fog has drawn since its volumetric fog landed: a fog lit by the sun
//! and shaded by what stands in it, so the trees between the eye and a low
//! sun cut the air into beams.
//!
//! **Bevy's volumetric fog, and only a share of the fog.** `bevy_pbr`'s
//! `VolumetricFogNode` raymarches a box (`FogVolume`) against the sun's shadow
//! map after the main pass (`EndMainPass → VolumetricFog →
//! StartMainPassPostProcessing`), so it lands over the atmosphere's sky, the
//! sky fog layer and everything transparent, and composites as
//! `src + (1 − α)·dst`. Its march ends at the OPAQUE depth, so a flame, a
//! spark or a rain streak is fogged as though it stood on what is behind it.
//! It cannot be the whole fog, for two reasons it does not advertise:
//!
//!   * **It is dark past the shadow map.** In-scatter is the shadow lookup,
//!     and outside the last cascade that lookup answers zero, so a volume
//!     wider than the sun's shadow distance is black at its far edge. The box
//!     is sized to the cascades ([`reach_m`]) and follows the eye.
//!   * **It has no sky light.** Its ambient term is `exp(−σ·L)` times a
//!     constant — brightest on the shortest ray and blind to density — so it
//!     is off, and the volume is lit by the sun alone.
//!
//! So the weather's ground fog is SPLIT: the volume carries the sun's share
//! of the light a droplet receives ([`sun_share`]) and the flat fogs
//! (`DistanceFog`, `sky_fog.wgsl`) keep the rest. Where the volume is lit it
//! is normalised to the flat fog's own colour seen side-on ([`volume`]), so a
//! lit view reads as the fog it was; in a shadow it adds nothing and absorbs
//! its share, and that difference is the shaft. Because the volume lies IN
//! FRONT of the flat fog rather than mixed through it, a shadowed ray reads
//! darker than the same fog without its sunlit share would — the shafts'
//! shadows are deeper than a mixed fog's, and that stays. The extinction is
//! conserved where it is seen ([`split`]): the box is shorter than the fog's
//! sight line, and the flat part is thickened by what the box does not
//! reach, so a hill at the visibility distance straight ahead is exactly as
//! fogged as before and the horizon seam the sky fog closes (`sky::SkyFog`)
//! stays closed.
//!
//! **It fades, it never pops.** Every switch that takes the pass away — the
//! sun dropping to `rig::SHADOW_MIN_ELEV`, cloud over it reaching
//! `rig::SHADOW_MAX_VEIL`, the fog thinning to [`SHAFT_MIN_DEPTH`] — is met
//! by [`fade`] at zero first, so the volume is already empty and the flat
//! fog whole when the pass goes. A lightning flash takes it at once (the
//! bolt's light takes the volume's one light slot) behind a flash that hides
//! the change, and it comes back over [`SHAFT_FLASH_RECOVER_S`].
//!
//! **Where it runs:** wherever the atmosphere draws (desktop, the browser's
//! WebGPU module) and the CLOUDS row is at its top rung — the same family of
//! raymarched air (`quality::Tier::shafts`). Never under WebGL2: no compute,
//! no storage buffers, no atmosphere, and `effective` already holds that
//! target's clouds flat.
//!
//! **Cost, read off the shader, not measured:** one fullscreen pass of up to
//! [`SHAFT_STEPS`] shadow-map and transmittance samples per pixel
//! ([`steps`]), run only while the sun casts through a fog thick enough to
//! show ([`SHAFT_MIN_DEPTH`]); otherwise the sun carries no
//! `VolumetricLight`, Bevy's extract drops the view's fog and the pass is not
//! recorded at all.
//!
//! **Flaws that are Bevy's, or the box's, and stay:** the march length is the
//! hit's view DEPTH, not its distance (`volumetric_fog.wgsl`:
//! `ray_length_view` is a difference of `z`s, stepped along a unit ray), so
//! off the view axis the volume is short by `cos θ` — 41% at the frame's side
//! edges at this FOV — and its extinction with it: a hill at the sight line
//! there keeps 2.7 to 3 of its 3 optical depths, and the shortfall turns
//! with the camera. Looking down from more than [`FOG_LAYER_M`] above the
//! ground, the box's floor ends the volume before the ground does, and the
//! valley floor is short by what the box would have carried below it.
//! [`split`] is exact on the view axis, level, and only there
//! (`tests/shafts.rs` bounds the frame's edge).

use bevy::camera::Exposure;
use bevy::ecs::system::SystemParam;
use bevy::light::{FogVolume, VolumetricFog, VolumetricLight};
use bevy::prelude::*;

use super::rig::{BoltLight, EyeCam, Sun};
use super::sky::FOG_LAYER_M;
use super::weather::WeatherNow;
use super::Eye;

/// Henyey–Greenstein asymmetry of the volume's droplets. Real fog is ~0.85,
/// a lobe ~670× its side-on radiance looking straight into the sun, and the
/// flat share it sits beside has no lobe at all. 0.3 is ×3.3 into the light
/// and ×0.5 with it behind ([`volume`] normalises side-on): the beams are
/// strongest where they are looked for, and a fog bank is not a lamp.
pub const SHAFT_G: f32 = 0.3;

/// Raymarch steps per pixel at most: Bevy's default. The ray is at most
/// `rig`'s shadow distance long, so ~3 m a step at HIGH.
pub const SHAFT_STEPS: u32 = 64;

/// The step the march aims for, metres: a shorter shadow distance marches
/// a smaller box in fewer steps ([`steps`]).
pub const SHAFT_STEP_M: f32 = 3.0;

/// How far each pixel's march start is jittered, metres, under TAA — about
/// a step, so the history resolves the stepping into a smooth beam. Zero
/// without TAA, where per-frame noise would shimmer instead.
pub const SHAFT_JITTER_M: f32 = 2.0;

/// The thinnest volume worth a pass: its optical depth from the eye to the
/// box's wall. Five percent of contrast between a beam and its shadow is at
/// the edge of what reads; [`fade`] brings the shafts in from here.
pub const SHAFT_MIN_DEPTH: f32 = 0.05;
/// The optical depth at which the shafts carry their whole share.
pub const SHAFT_FULL_DEPTH: f32 = 0.15;

/// The sun's elevation, radians, at which the shafts carry their whole
/// share (~6°): they fade in from `rig::SHADOW_MIN_ELEV`, where its
/// shadows start.
pub const SHAFT_FULL_ELEV: f32 = 0.10;

/// The cloud over the sun (`WeatherNow::cloud_occlusion`) at which the
/// shafts start to fade; they are gone at `rig::SHADOW_MAX_VEIL`, where its
/// shadows go.
pub const SHAFT_VEIL_FADE: f32 = 0.6;

/// How long the shafts take to come back after a lightning flash, seconds.
pub const SHAFT_FLASH_RECOVER_S: f32 = 1.0;

/// The least of the air's transmittance toward the sun the volume's colour
/// is lifted against ([`volume`]): blue's, under ~4° of sun, where the air
/// mass is too uncertain to lift against and the sun's share of the fog is a
/// few percent of a faded volume.
pub const SHAFT_MIN_T: f32 = 1e-3;

/// The narrowest box worth a pass, metres. The shadow distances under ~80 m
/// (`quality::SHADOW_M_LADDER`) leave no room beside the fog layer's height.
pub const SHAFT_MIN_REACH_M: f32 = 30.0;

/// This frame's split, written by `rig::day_night` (the one owner of the
/// fog) and read by the sky fog (`deck::drive`). `volume == 0` is off: the
/// flat fogs carry the whole weather, exactly as before this file existed.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct Shafts {
    /// The ground fog's extinction the flat fogs keep, per metre.
    pub flat: f32,
    /// The volume's, per metre.
    pub volume: f32,
}

/// Half the box's width, metres, for a shadow distance of `shadow_m`: the
/// widest box whose every point — the top corners included, [`FOG_LAYER_M`]
/// above the eye — is inside the last cascade, which Bevy bounds by view
/// depth and so by distance at most.
pub fn reach_m(shadow_m: f32) -> f32 {
    ((shadow_m * shadow_m - FOG_LAYER_M * FOG_LAYER_M).max(0.0) * 0.5).sqrt()
}

/// March steps for a shadow distance of `shadow_m`: the longest march is
/// the eye to the box's far corner, which [`reach_m`] keeps inside it.
pub fn steps(shadow_m: f32) -> u32 {
    ((shadow_m / SHAFT_STEP_M).ceil() as u32).clamp(16, SHAFT_STEPS)
}

fn smooth(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of its share the volume carries, `0..=1`, for a sun `elev`
/// radians up under `veil` of cloud (`WeatherNow::cloud_occlusion`), and a
/// volume of optical depth `depth` from the eye to the box's wall at its full
/// share. Zero at and past every edge where the pass switches off — the sun
/// too low or too veiled to cast, the fog too thin to show — so the switch
/// happens with the volume already empty.
pub fn fade(elev: f32, veil: f32, depth: f32) -> f32 {
    let rise = smooth(super::rig::SHADOW_MIN_ELEV, SHAFT_FULL_ELEV, elev);
    let clear = 1.0 - smooth(SHAFT_VEIL_FADE, super::rig::SHADOW_MAX_VEIL, veil);
    let thick = smooth(SHAFT_MIN_DEPTH, SHAFT_FULL_DEPTH, depth);
    rise * clear * thick
}

/// The sun's share of the light a droplet receives, `0..=1`: its direct
/// illuminance at the ground (`sun_lux`, after the air's transmittance, as
/// the ground is lit) against the scalar irradiance of the hemisphere fill
/// around it (twice each half's irradiance on a facing surface, `fill.rs`),
/// at `fill` of the fill's noon strength. ~0.55 at a clear noon, ~0.2 in
/// the fog preset, whose sun is a fifth of itself.
pub fn sun_share(sun_lux: f32, fill: f32) -> f32 {
    let lum = super::fill::luminance;
    let around = 2.0 * (lum(super::fill::sky_lux()) + lum(super::fill::bounce_lux())) * fill;
    let sun = sun_lux.max(0.0);
    if sun + around <= 0.0 {
        return 0.0;
    }
    sun / (sun + around)
}

/// How the weather's ground fog `fog` (per metre, beside a rain haze of
/// `rain`) divides between the flat fogs and a volume of half-width `reach`
/// carrying `share` of it.
///
/// Inside the box the two add up to the fog when the box spans the fog's
/// sight line (`3/σ`, where 95% is gone). When it does not, the flat part is
/// thickened by the share the box cannot carry past its wall, so the total
/// is exact AT the sight line: denser than it was near the eye, thinner far
/// past it, and unchanged where a far hill fades out.
pub fn split(fog: f32, rain: f32, share: f32, reach: f32) -> Shafts {
    let volume = fog * share.clamp(0.0, 1.0);
    let sigma = fog + rain;
    if volume <= 0.0 || sigma <= 0.0 {
        return Shafts {
            flat: fog,
            volume: 0.0,
        };
    }
    let seen = 3.0 / sigma;
    let inside = (reach / seen).min(1.0);
    Shafts {
        flat: fog - volume * inside,
        volume,
    }
}

/// The box: [`FOG_LAYER_M`] above the eye, as the sky fog's layer is, and as
/// far below it, but never under the sea — the water writes no depth, and a
/// march through it would fog the seabed.
pub fn volume_transform(eye: Vec3, reach: f32) -> Transform {
    let top = eye.y + FOG_LAYER_M;
    let bottom = (eye.y - FOG_LAYER_M).max(sim_core::terrain::SEA_LEVEL);
    Transform::from_translation(Vec3::new(eye.x, (top + bottom) * 0.5, eye.z))
        .with_scale(Vec3::new(2.0 * reach, top - bottom, 2.0 * reach))
}

/// The volume for a box of `scale`, extinction `sigma` per metre, lit to the
/// flat fog's colour `rgb` (exposed linear, `sky::fog_rgb`) by a sun of
/// `sun_lux` seen at `exposure`, `t_sun` of which the air lets through
/// (`sky::sun_transmittance`).
///
/// The shader's in-scatter is `fog_color · light_tint · light_intensity ·
/// exposure · sun · HG(θ) · e^(−σR) · T` per unit of optical depth — `T` the
/// atmosphere's transmittance toward the sun (`#ifdef ATMOSPHERE`), which
/// reddens and dims it as it does the ground's sunlight. The flat fog's
/// colour has no `T` in it, so the tint is `1 / T` and the intensity
/// `1 / (exposure · sun · HG(90°))`: a lit volume seen SIDE-ON reads as the
/// flat fog at every hour, brighter looking into the light and darker with
/// it behind.
///
/// **Side-on, not the sphere's mean, and that was measured.** Normalised to
/// the mean at g 0.4, an A/B against the flat fog alone (fog preset, sun 35°
/// up) kept the frames into the sun within a level of their mean but read a
/// side-on sky 11% darker and one with the sun behind 12% darker (sRGB): the
/// mean is mostly the forward lobe, so every other view paid for it.
///
/// `e^(−σR)` is Bevy dimming the sun as though it crossed the whole box (`R`
/// its bounding radius) to reach every point; the sun this rig hands it is
/// already dimmed by the fog (`WeatherNow::occlusion`), so that is undone.
pub fn volume(
    rgb: [f32; 3],
    sigma: f32,
    scale: Vec3,
    sun_lux: f32,
    exposure: f32,
    t_sun: [f32; 3],
) -> FogVolume {
    let radius = 0.5 * scale.length();
    let lift = t_sun.map(|t| 1.0 / t.max(SHAFT_MIN_T));
    let g2 = SHAFT_G * SHAFT_G;
    let side = (1.0 - g2) / (4.0 * std::f32::consts::PI * (1.0 + g2) * (1.0 + g2).sqrt());
    let per_lux = 1.0 / (sun_lux * exposure * side).max(1e-9);
    FogVolume {
        fog_color: Color::linear_rgb(rgb[0], rgb[1], rgb[2]),
        density_factor: sigma,
        density_texture: None,
        density_texture_offset: Vec3::ZERO,
        // Droplets scatter and hardly absorb: extinction IS scattering.
        absorption: 0.0,
        scattering: 1.0,
        scattering_asymmetry: SHAFT_G,
        light_tint: Color::linear_rgb(lift[0], lift[1], lift[2]),
        light_intensity: per_lux * (sigma * radius).exp(),
    }
}

/// The volume at rest: spawned once by `rig::setup`, drawn by nothing until
/// the sun carries a `VolumetricLight`. A unit box, never a zero one — Bevy
/// inverts its transform every frame.
pub fn idle_volume() -> FogVolume {
    FogVolume {
        density_factor: 0.0,
        ..default()
    }
}

/// The camera's half, inserted by `quality::apply` on the tier that draws
/// shafts at a shadow distance of `shadow_m`. No ambient (see the header),
/// and jitter only where TAA resolves it.
pub fn camera_fog(temporal: bool, shadow_m: f32) -> VolumetricFog {
    VolumetricFog {
        ambient_color: Color::WHITE,
        ambient_intensity: 0.0,
        jitter: if temporal { SHAFT_JITTER_M } else { 0.0 },
        step_count: steps(shadow_m),
    }
}

/// What the sun is doing this frame, as `rig::day_night` set it.
#[derive(Clone, Copy, Debug)]
pub struct SunNow {
    /// The light's illuminance, lux, after cloud and fog.
    pub lux: f32,
    /// Whether it casts this frame.
    pub casts: bool,
    /// The sun's elevation, radians (`rig::sun_elevation`).
    pub elev: f32,
    /// The camera's exposure, ev100.
    pub ev100: f32,
    /// `rig::sun_lux` — what the flat fog's colour is lit by.
    pub lux_share: f32,
    /// The hemisphere fill's share of its noon strength.
    pub fill: f32,
    /// The player's shadow distance, metres.
    pub shadow_m: f32,
}

/// What `rig::day_night` drives the shafts through: the volume, the sun's
/// `VolumetricLight` and the lightning's light. A parameter of its own so
/// the rig's signature grows by one.
#[derive(SystemParam)]
pub struct ShaftRig<'w, 's> {
    commands: Commands<'w, 's>,
    eye: Option<Res<'w, Eye>>,
    state: Option<ResMut<'w, Shafts>>,
    cam: Query<'w, 's, Has<VolumetricFog>, With<EyeCam>>,
    sun: Query<'w, 's, (Entity, Has<VolumetricLight>), With<Sun>>,
    bolt: Query<'w, 's, &'static mut Visibility, With<BoltLight>>,
    volume: Query<'w, 's, Volume, (Without<Sun>, Without<BoltLight>)>,
    time: Option<Res<'w, Time>>,
    calm: Local<'s, SinceFlash>,
}

/// Seconds since the last lightning flash, for the shafts' way back.
pub struct SinceFlash(f32);

impl Default for SinceFlash {
    fn default() -> Self {
        Self(SHAFT_FLASH_RECOVER_S)
    }
}

/// The volume's two fields `drive` writes. Named for clippy's
/// `type_complexity` line; apart from the sun and the bolt, whose
/// `Transform`s `day_night` writes in the same system.
type Volume = (&'static mut Transform, &'static mut FogVolume);

impl ShaftRig<'_, '_> {
    /// Whether this tier draws shafts at all: the camera carries the fog.
    pub fn tier(&self) -> bool {
        self.cam.single().unwrap_or(false)
    }

    /// Light the volume for this frame and return the per-metre extinction
    /// it takes off the flat fog (zero while off).
    pub fn drive(&mut self, w: &WeatherNow, sun: SunNow) -> f32 {
        // **The lightning's light is hidden while it is dark, and the shafts
        // need it.** Bevy sorts directional lights by `(volumetric, shadows,
        // entity)` ascending and flags as volumetric only the first
        // `volumetric count` of them — so a dark, shadowless bolt light sorts
        // ahead of the sun, takes the one slot, and the march breaks on its
        // first light. Hidden, it is not extracted at all.
        let flash = w.flash > 0.0;
        for mut vis in self.bolt.iter_mut() {
            let want = if flash {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *vis != want {
                *vis = want;
            }
        }

        let calm = if flash {
            0.0
        } else {
            let dt = self.time.as_ref().map_or(0.0, |t| t.delta_secs());
            (self.calm.0 + dt).min(SHAFT_FLASH_RECOVER_S)
        };
        self.calm.0 = calm;

        let reach = reach_m(sun.shadow_m);
        let t_sun = super::sky::sun_transmittance(sun.elev.sin());
        let share = sun_share(sun.lux * super::fill::luminance(t_sun), sun.fill);
        let full = w.fog_part() * share * reach;
        let share = share
            * fade(sun.elev, w.cloud_occlusion(), full)
            * smooth(0.0, SHAFT_FLASH_RECOVER_S, calm);
        let cut = split(w.fog_part(), w.rain_part(), share, reach);
        let eye = self.eye.as_ref().map(|e| e.pos);
        // `fade` is zero wherever `casts` goes false, so this only flips
        // with the volume already empty — a flash, the tier and the sea
        // excepted, each of which changes the whole frame anyway.
        let on = self.tier()
            && sun.casts
            && !flash
            && !w.underwater
            && reach >= SHAFT_MIN_REACH_M
            && cut.volume > 0.0
            && eye.is_some()
            && !self.volume.is_empty();
        let cut = if on {
            cut
        } else {
            Shafts {
                flat: w.fog_part(),
                volume: 0.0,
            }
        };

        // Bevy's extract drops the view's fog when no light is volumetric,
        // so taking the component off the sun is what skips the pass.
        if let Ok((entity, lit)) = self.sun.single() {
            if on && !lit {
                self.commands.entity(entity).insert(VolumetricLight);
            } else if !on && lit {
                self.commands.entity(entity).remove::<VolumetricLight>();
            }
        }
        if let (true, Some(eye), Ok((mut t, mut v))) = (on, eye, self.volume.single_mut()) {
            let box_t = volume_transform(eye, reach);
            let grey = super::sky::fog_grey(w.fog_sigma());
            let rgb = super::sky::fog_rgb(sun.lux_share, w.dark, grey);
            let exposure = Exposure { ev100: sun.ev100 }.exposure();
            *t = box_t;
            *v = volume(rgb, cut.volume, box_t.scale, sun.lux, exposure, t_sun);
        }
        if let Some(state) = self.state.as_mut() {
            if **state != cut {
                **state = cut;
            }
        }
        w.fog_part() - cut.flat
    }
}
