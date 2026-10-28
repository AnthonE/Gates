//! Rain (weather v0): streaks that follow the eye, and the ground they wet.
//!
//! **One mesh, one draw, and no particles.** The existing effect pools are
//! entity pools of a hundred-odd sparks; rain wants thousands of drops a
//! frame and a WebGL2 target, so it is a fixed mesh of streak quads whose
//! vertex stage (`assets/shaders/rain.wgsl`) wraps each drop into a box
//! around the camera by how far the rain has fallen ([`Travel`]). How hard
//! it rains is how many drops are alive — drops ranked over the intensity
//! collapse — so a drizzle and a storm are the same mesh with one number
//! moved.
//!
//! Under a roof the drops within a few metres of the eye are put out and the
//! depth test hides the rest behind the walls; underwater there is no rain.
//!
//! **Snow is the same mesh** (snow v0). Where the air freezes
//! (`WeatherNow::snow`) a drop is drawn as a soft round flake that drifts
//! down at a walking pace, wanders on its own wobble and rides the wind,
//! instead of a streak. Each drop has its own draw against the snow share, so
//! the sleet between rain and snow is a mix of the two and the change comes
//! a drop at a time.

use bevy::asset::Asset;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

use super::weather::WeatherNow;

/// The shader, resolved against the asset root `bin/gates.rs` sets.
pub const SHADER: &str = "shaders/rain.wgsl";

/// Drops in the mesh — the most a storm draws.
pub const DROPS: u32 = if cfg!(webgl2) { 2_500 } else { 6_000 };
/// Half the box the drops fall in around the eye, metres.
pub const BOX_HALF: Vec3 = Vec3::new(14.0, 9.0, 14.0);
/// How fast a drop falls, m/s — a raindrop's terminal speed.
pub const FALL_MS: f32 = 9.0;
/// The ground kept clear around an eye with a roof over it, metres.
pub const SHELTER_CLEAR_M: f32 = 5.0;
/// How fast a snowflake falls, m/s — the reference band is 1 to 1.5.
pub const SNOW_FALL_MS: f32 = 1.2;
/// The rain's glint by night: the light a streak gives back with no sun.
pub const GLINT: f32 = 0.12;
/// How much brighter a flake is drawn than a streak in the sun's share of
/// the light — a flake is white and opaque where a drop is a glint of the
/// sky behind it — and its share of the night's glint: a flake peaks at
/// twice a streak's alpha, so at half the colour it is no brighter than
/// one in the dark, where the ground draws near black (`rig.rs`).
pub const FLAKE_GAIN: f32 = 2.0;
pub const FLAKE_GLINT: f32 = 0.5;
/// Each drop falls at its own whole number of `1 / SPEED_STEPS` of the
/// travel (17 to 23 twentieths), so the travel wrapped at `SPEED_STEPS` box
/// lengths moves every drop by whole boxes, which the wrap around the eye
/// cannot see. `rain.wgsl` draws the speeds the same way.
pub const SPEED_STEPS: f32 = 20.0;
/// A flake's wobble swings at 1.3 and 0.9 rad/s (`rain.wgsl`), and both
/// come round together every 20π s: its clock wraps there.
pub const WOBBLE_PERIOD_S: f32 = 20.0 * std::f32::consts::PI;

/// The uniform, laid out to match `Rain` in the shader. 96 bytes: WebGL2
/// wants uniforms of at least 16.
#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct RainParams {
    /// xyz the fall velocity (m/s), w the streak length (m).
    pub fall: Vec4,
    /// xyz the box's half extents (m), w the intensity `0..1`.
    pub box_: Vec4,
    /// rgb the streak's lit colour, w the sheltered clear radius (m).
    pub look: Vec4,
    /// xyz how far the flakes have travelled ([`Travel::flakes`], m), w
    /// the share of drops that fall as snow `0..1` (snow v0).
    pub snow: Vec4,
    /// xyz how far the streaks have travelled ([`Travel::drops`], m), w the
    /// flakes' wobble clock ([`Travel::wobble`], s).
    pub drops: Vec4,
    /// rgb a flake's lit colour; w reserved and zero.
    pub flake: Vec4,
}

/// How far the streaks and the flakes have travelled, metres, and the
/// flakes' wobble clock, seconds (snow v0): integrated off the frame clock
/// and the live wind rather than `velocity × clock` in the shader, so a
/// wind shift turns the fall instead of throwing every drop across the box,
/// and wrapped where no drop can see it (`SPEED_STEPS`, `WOBBLE_PERIOD_S`),
/// so the clock's hourly wrap is not a jump either.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Travel {
    pub drops: Vec3,
    pub flakes: Vec3,
    pub wobble: f32,
}

impl Travel {
    /// `dt` seconds of the streaks falling at `fall` and the flakes
    /// drifting at `drift`, m/s.
    pub fn advance(&mut self, fall: Vec3, drift: Vec3, dt: f32) {
        self.advance_in(BOX_HALF, fall, drift, dt);
    }

    /// The same in a box of half extents `half` (the embers' own), so the
    /// wrap stays a whole number of its boxes.
    pub fn advance_in(&mut self, half: Vec3, fall: Vec3, drift: Vec3, dt: f32) {
        let wrap = 2.0 * half * SPEED_STEPS;
        self.drops = (self.drops + fall * dt).rem_euclid(wrap);
        self.flakes = (self.flakes + drift * dt).rem_euclid(wrap);
        self.wobble = (self.wobble + dt).rem_euclid(WOBBLE_PERIOD_S);
    }
}

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct RainMaterial {
    #[uniform(0)]
    pub params: RainParams,
}

impl Material for RainMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // A streak is a camera-facing quad either way round.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The rain's entity.
#[derive(Component)]
pub struct Rain;

/// The handle `drive` writes.
#[derive(Resource)]
pub struct RainHandle(pub Handle<RainMaterial>);

/// Deterministic per-drop randomness — a look, not a sim draw.
fn hash01(i: u32, k: u32) -> f32 {
    let mut h = (i as u64) << 32 | k as u64;
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^= h >> 33;
    (h >> 40) as f32 / 16_777_216.0
}

/// The drop mesh: `DROPS` quads, each a home in the unit box (POSITION), its
/// corners (UV_0) and its rank, phase and snow draw (COLOR).
pub fn drop_mesh(drops: u32) -> Mesh {
    let n = drops as usize;
    let mut pos = Vec::with_capacity(n * 4);
    let mut uv = Vec::with_capacity(n * 4);
    let mut col = Vec::with_capacity(n * 4);
    let mut idx: Vec<u16> = Vec::with_capacity(n * 6);
    for i in 0..drops {
        let home = [hash01(i, 0), hash01(i, 1), hash01(i, 2)];
        // Rank is the drop's index spread evenly, so the alive share tracks
        // the intensity exactly rather than by luck.
        let rank = (i as f32 + 0.5) / drops as f32;
        let phase = hash01(i, 3);
        // Under the snow share, a flake (snow v0).
        let flake = hash01(i, 4);
        let base = (i * 4) as u16;
        for (x, y) in [(-1.0, 0.0), (1.0, 0.0), (1.0, 1.0), (-1.0, 1.0)] {
            pos.push(home);
            uv.push([x, y]);
            col.push([rank, phase, flake, 1.0]);
        }
        idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    mesh.insert_indices(Indices::U16(idx));
    mesh
}

/// The streaks' fall and the flakes' drift in this weather, m/s. A flake
/// rides the same wind down at a walking pace, so a storm's snow drives
/// near level.
pub fn velocities(w: &WeatherNow) -> (Vec3, Vec3) {
    let wind = w.wind_dir * (1.0 + 7.0 * w.wind);
    let fall = Vec3::new(wind.x, -(FALL_MS + 2.0 * w.rain), wind.y);
    let drift = Vec3::new(wind.x, -SNOW_FALL_MS, wind.y);
    (fall, drift)
}

/// How hard it rains as drawn: not at all under the sea.
pub fn intensity(w: &WeatherNow) -> f32 {
    if w.underwater {
        0.0
    } else {
        w.rain
    }
}

/// What the rain looks like in this weather, `at` the travel so far.
pub fn params_for(w: &WeatherNow, at: &Travel) -> RainParams {
    let (fall, _) = velocities(w);
    // Lit by the sky it falls from: bright grey by day, a glint by night,
    // a white sheet in a flash. A flake gives back more of the sun and
    // less of the glint (`FLAKE_GAIN`, `FLAKE_GLINT`).
    let sky = (GLINT + 0.6 * w.sun_lux * (1.0 - 0.45 * w.dark)).min(0.8);
    let lit = sky + 1.5 * w.flash;
    let flake = GLINT * FLAKE_GLINT + (sky - GLINT) * FLAKE_GAIN + 1.5 * w.flash;
    let clear = if w.sheltered { SHELTER_CLEAR_M } else { 0.0 };
    let tint = |l: f32| Vec3::new(0.55 * l, 0.58 * l, 0.62 * l);
    RainParams {
        fall: fall.extend(0.45 + 0.3 * w.rain),
        box_: BOX_HALF.extend(intensity(w)),
        look: tint(lit).extend(clear),
        snow: at.flakes.extend(w.snow.clamp(0.0, 1.0)),
        drops: at.drops.extend(at.wobble),
        flake: tint(flake).extend(0.0),
    }
}

/// Build the rain. Runs with the rig, on entering the world.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<RainMaterial>>,
) {
    let material = materials.add(RainMaterial {
        params: RainParams::default(),
    });
    commands.spawn((
        super::WorldEntity,
        Rain,
        Mesh3d(meshes.add(drop_mesh(DROPS))),
        MeshMaterial3d(material.clone()),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.insert_resource(RainHandle(material));
}

/// Move the rain and the ground's wetness and snow with the weather. Writes
/// only when something moved, since a write re-uploads a material: the
/// travel moves only while there are drops to show it.
pub fn drive(
    weather: Res<WeatherNow>,
    time: Res<Time>,
    handle: Option<Res<RainHandle>>,
    mut rain: ResMut<Assets<RainMaterial>>,
    ring: Res<super::terrain_mesh::Ring>,
    mut ground: ResMut<Assets<super::ground_splat::GroundMaterial>>,
    mut travel: Local<Travel>,
) {
    if intensity(&weather) > 0.0 {
        let (fall, drift) = velocities(&weather);
        travel.advance(fall, drift, time.delta_secs());
    }
    if let Some(handle) = handle {
        let want = params_for(&weather, &travel);
        let stale = rain.get(&handle.0).is_some_and(|m| m.params != want);
        if stale {
            if let Some(m) = rain.get_mut(&handle.0) {
                m.params = want;
            }
        }
    }
    // The ground in steps of 1/64: a smooth soak without a re-upload a frame.
    let wet = (weather.ground_wet * 64.0).round() / 64.0;
    ring.set_rain_wet(&mut ground, wet);
    // The Verdigris' fissures by the hour: dimmer under the sun, brighter in
    // the dark, in steps of 1/32 of the day for the same reason.
    let day = (weather.sun_lux.clamp(0.0, 1.0) * 32.0).round() / 32.0;
    let (lo, hi) = (
        Vec3::from_array(super::ground_splat::FEL_GLOW_NIGHT),
        Vec3::from_array(super::ground_splat::FEL_GLOW_DAY),
    );
    ring.set_fel_glow(&mut ground, lo + (hi - lo) * day);
    // The snow the same way, its floor in whole metres and its band of
    // aridity and its least arctic widened to steps of 1/32.
    let snow = (weather.ground_snow * 64.0).round() / 64.0;
    let arid = weather.snow_arid;
    let band = Vec2::new((arid.x * 32.0).floor(), (arid.y * 32.0).ceil()) / 32.0;
    let arctic = (weather.snow_arctic * 32.0).floor() / 32.0;
    ring.set_snow(&mut ground, snow, weather.snow_floor.round(), band, arctic);
}
