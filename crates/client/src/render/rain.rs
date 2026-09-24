//! Rain (weather v0): streaks that follow the eye, and the ground they wet.
//!
//! **One mesh, one draw, and no particles.** The existing effect pools are
//! entity pools of a hundred-odd sparks; rain wants thousands of drops a
//! frame and a WebGL2 target, so it is a fixed mesh of streak quads whose
//! vertex stage (`assets/shaders/rain.wgsl`) wraps each drop into a box
//! around the camera by the clock. How hard it rains is how many drops are
//! alive — drops ranked over the intensity collapse — so a drizzle and a
//! storm are the same mesh with one number moved.
//!
//! Under a roof the drops within a few metres of the eye are put out and the
//! depth test hides the rest behind the walls; underwater there is no rain.

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
pub const DROPS: u32 = if cfg!(target_arch = "wasm32") {
    2_500
} else {
    6_000
};
/// Half the box the drops fall in around the eye, metres.
pub const BOX_HALF: Vec3 = Vec3::new(14.0, 9.0, 14.0);
/// How fast a drop falls, m/s — a raindrop's terminal speed.
pub const FALL_MS: f32 = 9.0;
/// The ground kept clear around an eye with a roof over it, metres.
pub const SHELTER_CLEAR_M: f32 = 5.0;

/// The uniform, laid out to match `Rain` in the shader. 48 bytes: WebGL2
/// wants uniforms of at least 16.
#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct RainParams {
    /// xyz the fall velocity (m/s), w the streak length (m).
    pub fall: Vec4,
    /// xyz the box's half extents (m), w the intensity `0..1`.
    pub box_: Vec4,
    /// rgb the streak's lit colour, w the sheltered clear radius (m).
    pub look: Vec4,
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
/// corners (UV_0) and its rank and phase (COLOR).
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
        let base = (i * 4) as u16;
        for (x, y) in [(-1.0, 0.0), (1.0, 0.0), (1.0, 1.0), (-1.0, 1.0)] {
            pos.push(home);
            uv.push([x, y]);
            col.push([rank, phase, 0.0, 1.0]);
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

/// What the rain looks like in this weather: dry and still is all zeros.
pub fn params_for(w: &WeatherNow) -> RainParams {
    let intensity = if w.underwater { 0.0 } else { w.rain };
    let wind = w.wind_dir * (1.0 + 7.0 * w.wind);
    let fall = Vec3::new(wind.x, -(FALL_MS + 2.0 * w.rain), wind.y);
    // Lit by the sky it falls from: bright grey by day, a glint by night,
    // a white sheet in a flash.
    let lit = (0.12 + 0.6 * w.sun_lux * (1.0 - 0.45 * w.dark)).min(0.8) + 1.5 * w.flash;
    let clear = if w.sheltered { SHELTER_CLEAR_M } else { 0.0 };
    RainParams {
        fall: fall.extend(0.45 + 0.3 * w.rain),
        box_: BOX_HALF.extend(intensity),
        look: Vec4::new(0.55 * lit, 0.58 * lit, 0.62 * lit, clear),
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

/// Move the rain and the ground's wetness with the weather. Writes only
/// when something moved, since a write re-uploads a material.
pub fn drive(
    weather: Res<WeatherNow>,
    handle: Option<Res<RainHandle>>,
    mut rain: ResMut<Assets<RainMaterial>>,
    ring: Res<super::terrain_mesh::Ring>,
    mut ground: ResMut<Assets<super::ground_splat::GroundMaterial>>,
) {
    if let Some(handle) = handle {
        let want = params_for(&weather);
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
}
