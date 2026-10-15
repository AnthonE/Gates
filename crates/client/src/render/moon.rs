//! The moon: a disk sharp at any resolution, with its seas on it.
//!
//! **Points, not texels — the stars' lesson again** (`stars.rs`). The moon
//! was texels of the composed sky cube, about five across on a browser's
//! 128² face, and arrived as a smudge with a stepped rim. Here it is one
//! quad far out along its direction, its rim cut per pixel.
//!
//! **Behind the deck, off the deck's own field**, at the deck on screen:
//! this frame's weather, since every target draws its sky per pixel
//! (`deck.rs`), with the fine detail the flat deck shares (`deck.wgsl`).
//!
//! **Over the cube's own moon natively.** The cube keeps a small soft one
//! for the sea to reflect (`sky::MOON_R`), and the atmosphere draws it, so
//! there the disk covers what is behind it rather than adding to it. A
//! browser's sky has no disk, and there it adds.
//!
//! **It keeps a calendar** (`sim_core::world::moon_phase`, a month of
//! `MOON_DAYS`). The disk is shaded for its phase, a terminator facing the
//! sun and the dark limb faintly lit by the earth, and the light it gives
//! the night ([`shine`]) is its lit fraction while it is up: the ambient's
//! share past the floor (`rig::night_light`), the deck's underside, its
//! halo, the sea's reflection of it, and how many stars it washes out.

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

/// How bright the earth lights the moon's dark side, as a share of the
/// sunlit disk, at new moon — the earth is full from there, and wanes as
/// the moon waxes. Far over the real ~1/1000, so a crescent holds the rest
/// of its disk ashen, the way a person remembers it.
pub const EARTHSHINE: f32 = 0.03;

/// The share of the disk the sun lights at `phase` of the month
/// (`sim_core::world::moon_phase`): 0 new, 1 full.
pub fn lit_fraction(phase: f32) -> f32 {
    0.5 - 0.5 * (std::f32::consts::TAU * phase).cos()
}

/// Toward the moon at `frac` of the day, `phase` through its month.
///
/// **On the sun's own path, `phase` of a day behind it**, not opposite the
/// sun, where it rode before it had phases: a moon opposite the sun is full
/// by geometry, and a crescent there would face away from the light it is
/// lit by. Behind the sun on its path, a full moon is where the noon sun
/// was, up all night at the noon sun's height; a crescent hangs low over
/// the sunset it follows or the sunrise it leads; a new moon is under the
/// horizon with the sun, which is what makes its night moonless. The night
/// is an eighth of the day, so every moon but the few round new is up for
/// all of it.
pub fn toward(frac: f32, phase: f32) -> Vec3 {
    super::rig::to_sun((frac - phase).rem_euclid(1.0))
}

/// How much of the moon is over the horizon, `0..=1`: a setting moon takes
/// its light with it over a few degrees rather than at once.
pub fn above(moon: Vec3) -> f32 {
    let t = ((moon.y + 0.02) / 0.08).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What the moon gives the night at `frac` of the day, `phase` through its
/// month, `0..=1`: its lit fraction, while it is up.
pub fn shine(frac: f32, phase: f32) -> f32 {
    lit_fraction(phase) * above(toward(frac, phase))
}

/// The sun's light on the disk, in the frame the vertex stage builds (x
/// right and y up as the eye sees it, z toward the eye), for a disk `lit`
/// of the way full. The side it comes from is the geometry's, so the
/// terminator faces the sun; its angle is the calendar's, since on this
/// sky's stylised paths the two only roughly agree.
pub fn sunlight_on_disk(moon: Vec3, sun: Vec3, lit: f32) -> Vec3 {
    let d = moon.normalize_or(Vec3::Y);
    let c = d.cross(Vec3::Y);
    let right = if c.length_squared() > 1e-6 {
        c.normalize()
    } else {
        Vec3::X
    };
    let up = right.cross(d);
    let side = Vec2::new(sun.dot(right), sun.dot(up)).normalize_or(Vec2::X);
    // The phase angle's cosine: 1 full, -1 new.
    let cos_a = 2.0 * lit.clamp(0.0, 1.0) - 1.0;
    let sin_a = (1.0 - cos_a * cos_a).max(0.0).sqrt();
    (side * sin_a).extend(cos_a)
}

/// The uniform, laid out to match `Moon` in the shader.
#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct MoonParams {
    pub dir: Vec4,
    pub light: Vec4,
    pub deck: Vec4,
    pub cloud: Vec4,
    pub field: Vec4,
    pub lit: Vec4,
}

impl MoonParams {
    /// The moon toward `moon`, `night` of the way into the night, behind
    /// the deck `shown`, lit as `shown` says (its sun and its phase).
    pub fn new(moon: Vec3, night: f32, shown: &ComposeParams) -> Self {
        let nits = sky::CLOUD_NITS;
        let rgb = Vec3::from_array(sky::MOON_RGB) * nits;
        Self {
            dir: moon
                .normalize_or(Vec3::Y)
                .extend((sky::MOON_R * QUAD_OVER).tan()),
            light: rgb.extend(night.clamp(0.0, 1.0) * above(moon)),
            deck: Vec4::new(
                sky::CLOUD_ALT_M / sky::CLOUD_SCALE_M,
                sky::DECK_CUTOFF,
                sky::HORIZON_FADE,
                deck::DETAIL,
            ),
            cloud: Vec4::new(shown.drift.x, shown.drift.y, 1.0 - shown.cover, sky::EDGE),
            field: Vec4::new(
                1.0 / sky::FIELD_PERIOD as f32,
                0.5 / sky::FIELD_N as f32,
                if sky::BAKE_BACKDROP { 0.0 } else { 1.0 },
                QUAD_OVER,
            ),
            lit: sunlight_on_disk(moon, shown.sun, shown.moon_lit)
                .extend(EARTHSHINE * (1.0 - shown.moon_lit)),
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
    let (Some(handle), Some(_)) = (handle, composer) else {
        return;
    };
    let live = ComposeParams::from_weather(&weather);
    let want = MoonParams::new(weather.moon, weather.night, &live);
    let stale = materials.get(&handle.0).is_some_and(|m| m.params != want);
    if stale {
        if let Some(m) = materials.get_mut(&handle.0) {
            m.params = want;
        }
    }
}
