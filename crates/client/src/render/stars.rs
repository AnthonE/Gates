//! Stars (weather v0): points of light in a clear night sky, behind the deck.
//!
//! **Points, not texels.** They were first put in the composed sky cube,
//! where a star is one texel of a 256² face — about five pixels of soft
//! blob on a 1080p screen, and a sky of them read as snow. Here each star
//! is a quad the vertex stage sizes in pixels, far out along its direction,
//! so it stays a point at any resolution and the land hides it by depth.
//!
//! **Behind the deck, off the deck's own field.** The composer's noise
//! (`sky::CloudField`) is uploaded once as a texture, and the fragment
//! stage puts a star behind cloud exactly as `SkyComposer::cover_at` asks
//! what covers the sun: the same projection, threshold, edge and horizon
//! fade, at the drift of the deck on screen.
//!
//! They draw in the transparent pass, after the atmosphere has painted the
//! sky, so the air neither tints nor dims them; the vertex stage fades the
//! low ones itself.

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

use super::sky;
use super::weather::WeatherNow;

/// The shader, resolved against the asset root `bin/gates.rs` sets.
pub const SHADER: &str = "shaders/stars.wgsl";

/// Stars in the sky: enough that a clear night is full of them, and a few
/// bright enough to find first.
pub const STARS: u32 = if cfg!(target_arch = "wasm32") {
    1_200
} else {
    2_400
};
/// A star's quad at its brightest, pixels across.
pub const STAR_PX: f32 = 3.0;

/// The uniform, laid out to match `Stars` in the shader.
#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct StarParams {
    /// x: how much of the starfield shows, `0..=1`; y: the quad's size in
    /// pixels; zw unused.
    pub look: Vec4,
    /// The deck's shape: x its altitude over its noise scale (so a ray's
    /// `xz / y` times this is noise units), y the horizon cutoff, z the fade
    /// above it, w unused.
    pub deck: Vec4,
    /// The deck on screen: xy its drift (noise units), z the field threshold
    /// (`1 − cover`), w the edge's width.
    pub cloud: Vec4,
    /// The field texture: x one over its period (noise units), y half a
    /// texel (uv); zw unused.
    pub field: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct StarMaterial {
    #[uniform(0)]
    pub params: StarParams,
    #[texture(1)]
    #[sampler(2)]
    pub field: Handle<Image>,
}

impl Material for StarMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The starfield's entity.
#[derive(Component)]
pub struct Stars;

/// The handle `drive` writes.
#[derive(Resource)]
pub struct StarHandle(pub Handle<StarMaterial>);

/// Deterministic per-star randomness — a look, not a sim draw.
fn hash01(i: u32, k: u32) -> f32 {
    let mut h = ((i as u64) << 32 | k as u64) ^ 0x5354_4152_5321;
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^= h >> 33;
    (h >> 40) as f32 / 16_777_216.0
}

/// The starfield mesh: `n` quads, each its star's direction (POSITION, unit,
/// above the horizon), its corners (UV_0), and its brightness, twinkle phase
/// and warmth (COLOR).
pub fn star_mesh(n: u32) -> Mesh {
    let count = n as usize;
    let mut pos = Vec::with_capacity(count * 4);
    let mut uv = Vec::with_capacity(count * 4);
    let mut col = Vec::with_capacity(count * 4);
    let mut idx: Vec<u16> = Vec::with_capacity(count * 6);
    for i in 0..n {
        // Uniform over the cap above ~2°: y uniform is area uniform.
        let y = 0.03 + 0.97 * hash01(i, 0);
        let a = std::f32::consts::TAU * hash01(i, 1);
        let r = (1.0 - y * y).sqrt();
        let dir = [r * a.cos(), y, r * a.sin()];
        // Mostly faint, a few bright: the twelfth power leaves about one
        // star in fifteen over half brightness.
        let bright = 0.12 + 0.88 * hash01(i, 2).powi(12);
        let phase = hash01(i, 3);
        let warmth = hash01(i, 4);
        let base = (i * 4) as u16;
        for corner in [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
            pos.push(dir);
            uv.push(corner);
            col.push([bright, phase, warmth, 1.0]);
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

/// How much of the starfield shows: the night, less the fog, none under the
/// sea — in steps of 1/64, so a dusk is a few dozen material writes rather
/// than one a frame. The deck hides the rest star by star.
pub fn showing(w: &WeatherNow) -> f32 {
    if w.underwater {
        return 0.0;
    }
    ((w.night * (1.0 - w.fog)).clamp(0.0, 1.0) * 64.0).round() / 64.0
}

/// The uniform for this frame, the deck at `shown` (the composer's last
/// published deck — what is on screen).
pub fn params_for(w: &WeatherNow, shown: &sky::ComposeParams) -> StarParams {
    StarParams {
        look: Vec4::new(showing(w), STAR_PX, 0.0, 0.0),
        deck: Vec4::new(
            sky::CLOUD_ALT_M / sky::CLOUD_SCALE_M,
            sky::DECK_CUTOFF,
            sky::HORIZON_FADE,
            0.0,
        ),
        cloud: Vec4::new(shown.drift.x, shown.drift.y, 1.0 - shown.cover, sky::EDGE),
        field: Vec4::new(
            1.0 / sky::FIELD_PERIOD as f32,
            0.5 / sky::FIELD_N as f32,
            0.0,
            0.0,
        ),
    }
}

/// Build the starfield. After `sky::setup`, whose composer holds the field.
pub fn setup(
    mut commands: Commands,
    composer: Option<Res<sky::SkyComposer>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StarMaterial>>,
) {
    let Some(composer) = composer else {
        return;
    };
    let material = materials.add(StarMaterial {
        params: StarParams::default(),
        field: images.add(composer.field().image()),
    });
    commands.spawn((
        super::WorldEntity,
        Stars,
        Mesh3d(meshes.add(star_mesh(STARS))),
        MeshMaterial3d(material.clone()),
        Transform::default(),
        // Hidden until a night asks for it: a day draws no stars at all.
        Visibility::Hidden,
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.insert_resource(StarHandle(material));
}

/// Show the stars at night, behind the deck on screen. Writes the material
/// only when something moved, since a write rebuilds its bind group — a
/// clear night's deck drifts, and the composer republishes it about twice
/// a second natively, so that is the rate this writes at.
pub fn drive(
    weather: Res<WeatherNow>,
    composer: Option<Res<sky::SkyComposer>>,
    handle: Option<Res<StarHandle>>,
    mut materials: ResMut<Assets<StarMaterial>>,
    mut stars: Query<&mut Visibility, With<Stars>>,
) {
    let show = showing(&weather) > 0.0;
    for mut v in stars.iter_mut() {
        let want = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    let (Some(handle), Some(composer)) = (handle, composer) else {
        return;
    };
    let Some(shown) = composer.shown() else {
        return;
    };
    if !show {
        return;
    }
    let want = params_for(&weather, shown);
    let stale = materials.get(&handle.0).is_some_and(|m| m.params != want);
    if stale {
        if let Some(m) = materials.get_mut(&handle.0) {
            m.params = want;
        }
    }
}
