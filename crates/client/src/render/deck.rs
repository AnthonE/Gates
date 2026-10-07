//! A browser's sky, drawn per pixel.
//!
//! **Why.** A browser has no atmosphere (`rig.rs`), so its whole sky was
//! the composed cube (`sky.rs`): clear sky, clouds and moon baked into 128²
//! texels a face. At 75° of view a texel is six pixels across, so every
//! cloud edge arrived as a bilinear smear and the moon as a stepped blob.
//! This draws the same sky, by the same arithmetic (`sky::DeckLight`), per
//! pixel: one fullscreen triangle at the far plane, the deck's noise read
//! from the field texture with two finer octaves on top that only a pixel
//! can show (`deck.wgsl`).
//!
//! **The cube stays.** The sea reflects it (`water.rs`), and a reflection
//! through waves wants nothing sharper. This covers it wherever the sky is
//! seen. The desktop keeps drawing its clouds from the cube, because there
//! the atmosphere has to composite them (`sky.rs`'s header).

use bevy::asset::Asset;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::core_pipeline::Skybox;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

use super::rig::EyeCam;
use super::sky::{self, ComposeParams, DeckLight};
use super::weather::WeatherNow;

/// The shader, resolved against the asset root `bin/gates.rs` sets.
pub const SHADER: &str = "shaders/sky_deck.wgsl";

/// Whether this target draws its sky per pixel: wherever the cube is the
/// sky rather than clouds for an atmosphere to composite.
pub const PER_PIXEL: bool = sky::BAKE_BACKDROP;

/// How far finer octaves move the field ([`deck.wgsl`]'s `deck_detail`):
/// about a quarter of a cloud edge's width, enough to give the rim lumps.
pub const DETAIL: f32 = 0.06;

/// Draw order in the transparent pass, as `Material::depth_bias` (more
/// negative draws first): the sky, then the stars over it, then the moon,
/// whose disk covers any star behind it. Each far past any real depth.
pub const DECK_ORDER: f32 = -3.0e8;
pub const STARS_ORDER: f32 = -2.0e8;
pub const MOON_ORDER: f32 = -1.0e8;

/// The uniform, laid out to match `Deck` in the shader.
#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct DeckParams {
    pub zenith: Vec4,
    pub horizon: Vec4,
    pub night: Vec4,
    pub glow: Vec4,
    pub sun: Vec4,
    pub moon: Vec4,
    pub halo: Vec4,
    pub scatter: Vec4,
    pub top: Vec4,
    pub base: Vec4,
    pub fog: Vec4,
    pub shape: Vec4,
    pub drift: Vec4,
    pub shade: Vec4,
}

impl DeckParams {
    /// The sky for a compose's params, drawn at `nits` cd/m² per deck unit.
    /// What `sky::compose_range` does per texel with `p.backdrop` set, with
    /// everything that does not vary across the sky worked out here.
    pub fn new(p: &ComposeParams, nits: f32) -> Self {
        let l = DeckLight::new(p);
        // The backdrop's weather grade is linear in its colour, so grading
        // the two ends grades every blend of them.
        let grade = |b: [f32; 3]| -> Vec3 {
            let lum = super::fill::luminance(b);
            Vec3::from_array(core::array::from_fn(|c| {
                (b[c] + (lum - b[c]) * 0.7 * p.dark) * (1.0 - 0.4 * p.dark) * p.light
            }))
        };
        let v3 = Vec3::from_array;
        let night = p.night;
        Self {
            zenith: grade(sky::backdrop_at(Vec3::Y)).extend(sky::AIR_FLOOR),
            horizon: grade(sky::backdrop_at(Vec3::X)).extend(nits),
            night: (v3(sky::NIGHT_SKY) * night).extend(night),
            glow: (v3(sky::DUSK_GLOW) * l.glow * (1.0 - 0.7 * p.dark)).extend(0.0),
            sun: Vec4::new(l.sun_h.x, l.sun_h.y, l.toward.x, l.toward.y),
            moon: p.moon.extend(sky::HALO_COS),
            halo: (v3(sky::HALO_RGB) * night).extend(sky::MOON_SCATTER_COS),
            scatter: v3(l.scatter).extend(0.0),
            top: v3(l.top).extend(l.thresh),
            base: v3(l.base).extend(sky::EDGE),
            fog: (v3(l.fog_rgb) * l.gain).extend(p.fog),
            shape: Vec4::new(
                sky::CLOUD_ALT_M / sky::CLOUD_SCALE_M,
                sky::DECK_CUTOFF,
                sky::HORIZON_FADE,
                DETAIL,
            ),
            drift: Vec4::new(
                p.drift.x,
                p.drift.y,
                1.0 / sky::FIELD_PERIOD as f32,
                0.5 / sky::FIELD_N as f32,
            ),
            shade: Vec4::new(sky::CORE_SHADE, sky::RIM_GLOW, sky::SIDE_VIEW_Y, 0.0),
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct DeckMaterial {
    #[uniform(0)]
    pub params: DeckParams,
    #[texture(1)]
    #[sampler(2)]
    pub field: Handle<Image>,
}

impl Material for DeckMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    /// Alpha 1 out of the shader: it replaces what the `Skybox` drew.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn depth_bias(&self) -> f32 {
        DECK_ORDER
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

/// The sky's entity.
#[derive(Component)]
pub struct DeckSky;

/// The handle `drive` writes.
#[derive(Resource)]
pub struct DeckHandle(pub Handle<DeckMaterial>);

/// One triangle over the whole screen, in clip space.
pub fn fullscreen_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[-1.0f32, -1.0, 0.0], [3.0, -1.0, 0.0], [-1.0, 3.0, 0.0]],
    );
    mesh.insert_indices(Indices::U16(vec![0, 1, 2]));
    mesh
}

/// Build the sky, on a target that draws it per pixel. After `sky::setup`.
pub fn setup(
    mut commands: Commands,
    composer: Option<Res<sky::SkyComposer>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DeckMaterial>>,
) {
    if !PER_PIXEL {
        return;
    }
    let Some(composer) = composer else {
        return;
    };
    let material = materials.add(DeckMaterial {
        params: DeckParams::new(&ComposeParams::noon(true), sky::CLOUD_NITS),
        field: composer.field_image().clone(),
    });
    commands.spawn((
        super::WorldEntity,
        DeckSky,
        Mesh3d(meshes.add(fullscreen_mesh())),
        MeshMaterial3d(material.clone()),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.insert_resource(DeckHandle(material));
}

/// This frame's sky, off this frame's weather — the drift moves every
/// frame here rather than every compose — at the `Skybox`'s brightness,
/// which carries the lightning.
pub fn drive(
    weather: Res<WeatherNow>,
    handle: Option<Res<DeckHandle>>,
    sky_box: Query<&Skybox, With<EyeCam>>,
    mut materials: ResMut<Assets<DeckMaterial>>,
) {
    let Some(handle) = handle else {
        return;
    };
    let nits = sky_box
        .single()
        .map(|s| s.brightness / sky::DECK_GAIN)
        .unwrap_or(sky::CLOUD_NITS);
    let want = DeckParams::new(&ComposeParams::from_weather(&weather), nits);
    let stale = materials.get(&handle.0).is_some_and(|m| m.params != want);
    if stale {
        if let Some(m) = materials.get_mut(&handle.0) {
            m.params = want;
        }
    }
}
