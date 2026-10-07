//! The moon: a disk sharp at any resolution, with its seas on it.
//!
//! **Points, not texels — the stars' lesson again** (`stars.rs`). The moon
//! was texels of the composed sky cube, about five across on a browser's
//! 128² face, and arrived as a smudge with a stepped rim. Here it is one
//! quad far out along its direction, its rim cut per pixel.
//!
//! **Behind the deck, off the deck's own field**, at the deck on screen:
//! the composer's last cube natively, this frame's weather in a browser,
//! whose sky is drawn per pixel (`deck.rs`) with the fine detail this
//! shares (`deck.wgsl`).
//!
//! **Over the cube's own moon natively.** The cube keeps a small soft one
//! for the sea to reflect (`sky::MOON_R`), and the atmosphere draws it, so
//! there the disk covers what is behind it rather than adding to it. A
//! browser's sky has no disk, and there it adds.

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

use super::deck;
use super::sky::{self, ComposeParams};
use super::weather::WeatherNow;

/// The shader, resolved against the asset root `bin/gates.rs` sets.
pub const SHADER: &str = "shaders/moon.wgsl";

/// The quad over the disk, so the antialiased rim has room.
const QUAD_OVER: f32 = 1.1;

/// The uniform, laid out to match `Moon` in the shader.
#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct MoonParams {
    pub dir: Vec4,
    pub light: Vec4,
    pub deck: Vec4,
    pub cloud: Vec4,
    pub field: Vec4,
}

impl MoonParams {
    /// The moon toward `moon`, `night` of the way into the night, behind
    /// the deck `shown`.
    pub fn new(moon: Vec3, night: f32, shown: &ComposeParams) -> Self {
        let nits = sky::CLOUD_NITS;
        let rgb = Vec3::from_array(sky::MOON_RGB) * nits;
        Self {
            dir: moon
                .normalize_or(Vec3::Y)
                .extend((sky::MOON_R * QUAD_OVER).tan()),
            light: rgb.extend(night.clamp(0.0, 1.0)),
            deck: Vec4::new(
                sky::CLOUD_ALT_M / sky::CLOUD_SCALE_M,
                sky::DECK_CUTOFF,
                sky::HORIZON_FADE,
                if deck::PER_PIXEL { deck::DETAIL } else { 0.0 },
            ),
            cloud: Vec4::new(shown.drift.x, shown.drift.y, 1.0 - shown.cover, sky::EDGE),
            field: Vec4::new(
                1.0 / sky::FIELD_PERIOD as f32,
                0.5 / sky::FIELD_N as f32,
                if deck::PER_PIXEL { 0.0 } else { 1.0 },
                QUAD_OVER,
            ),
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct MoonMaterial {
    #[uniform(0)]
    pub params: MoonParams,
    #[texture(1)]
    #[sampler(2)]
    pub field: Handle<Image>,
}

impl Material for MoonMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    /// Premultiplied: the shader's alpha says whether the disk covers or adds.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn depth_bias(&self) -> f32 {
        deck::MOON_ORDER
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

/// The moon's entity.
#[derive(Component)]
pub struct Moon;

/// The handle `drive` writes.
#[derive(Resource)]
pub struct MoonHandle(pub Handle<MoonMaterial>);

/// One quad, corners -1..1; the shader puts it on the sky.
pub fn quad_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-1.0f32, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
        ],
    );
    mesh.insert_indices(Indices::U16(vec![0, 1, 2, 0, 2, 3]));
    mesh
}

/// Build the moon. After `sky::setup`, whose composer holds the field.
///
/// Drawn by day too, at nothing: the quad costs nothing and its pipeline is
/// built at load rather than at dusk.
pub fn setup(
    mut commands: Commands,
    composer: Option<Res<sky::SkyComposer>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<MoonMaterial>>,
) {
    let Some(composer) = composer else {
        return;
    };
    let material = materials.add(MoonMaterial {
        params: MoonParams::new(Vec3::NEG_Y, 0.0, &ComposeParams::noon(sky::BAKE_BACKDROP)),
        field: composer.field_image().clone(),
    });
    commands.spawn((
        super::WorldEntity,
        Moon,
        Mesh3d(meshes.add(quad_mesh())),
        MeshMaterial3d(material.clone()),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.insert_resource(MoonHandle(material));
}

/// Put the moon where the weather says, behind the deck on screen.
pub fn drive(
    weather: Res<WeatherNow>,
    composer: Option<Res<sky::SkyComposer>>,
    handle: Option<Res<MoonHandle>>,
    mut materials: ResMut<Assets<MoonMaterial>>,
) {
    let (Some(handle), Some(composer)) = (handle, composer) else {
        return;
    };
    let live = ComposeParams::from_weather(&weather);
    let shown = if deck::PER_PIXEL {
        live
    } else {
        composer.shown().copied().unwrap_or(live)
    };
    let want = MoonParams::new(weather.moon, weather.night, &shown);
    let stale = materials.get(&handle.0).is_some_and(|m| m.params != want);
    if stale {
        if let Some(m) = materials.get_mut(&handle.0) {
            m.params = want;
        }
    }
}
