//! The sky's clouds, drawn per pixel, on every target.
//!
//! **Why.** The deck was the composed cube (`sky.rs`): 256² texels a face
//! on the desktop, 128² in a browser. At 75° of view a texel is three to six
//! pixels across, so every cloud edge arrived as a bilinear smear. Two
//! materials draw it per pixel instead, one per kind of target:
//!
//!   · **[`CloudMaterial`], wherever the atmosphere draws** (the desktop and
//!     the browser's WebGPU module). One fullscreen triangle at depth 0 in
//!     the OPAQUE pass — where the `Skybox` drew the cube — so
//!     `AtmosphereNode::RenderSky` reads it as sky and composites it as
//!     `inscattering + T·cloud`, exactly as it did the cube, and the header
//!     of `sky.rs` still holds by construction. At LOW it is the flat deck,
//!     `sky::compose_range`'s arithmetic per pixel; at MEDIUM and HIGH it is
//!     a slab of cloud a kilometre thick, raymarched (`cloud.wgsl`).
//!     **The `Skybox` goes on this target**: it draws after every opaque
//!     item and replaces what is at depth 0, so it would paint the cube
//!     back over the deck. Its prepass wrote the sky's motion vectors for
//!     TAA, and `cloud_prepass.wgsl` writes them now.
//!   · **[`DeckMaterial`], under WebGL2**, which has no atmosphere: the
//!     clear sky, clouds and moon composed together, transparent, over the
//!     `Skybox` (`sky_deck.wgsl`).
//!
//! **The cube stays.** The sea reflects it (`water.rs`), and a reflection
//! through waves wants nothing sharper.

use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::asset::{Asset, RenderAssetUsages};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::core_pipeline::Skybox;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;

use super::rig::EyeCam;
use super::sky::{self, ComposeParams, DeckLight};
use super::weather::WeatherNow;
use crate::config::Clouds;

/// The browser's sky shader, resolved against the asset root `bin/gates.rs` sets.
pub const SHADER: &str = "shaders/sky_deck.wgsl";
/// The cloud deck's, where the atmosphere draws.
pub const CLOUD_SHADER: &str = "shaders/cloud.wgsl";
/// Its prepass: the motion vectors the `Skybox` used to write.
pub const CLOUD_PREPASS: &str = "shaders/cloud_prepass.wgsl";
/// The weather's fog over the sky, where the atmosphere draws.
pub const SKY_FOG_SHADER: &str = "shaders/sky_fog.wgsl";

/// How far finer octaves move the field ([`deck.wgsl`]'s `deck_detail`):
/// about a quarter of a cloud edge's width, enough to give the rim lumps.
pub const DETAIL: f32 = 0.06;

/// Draw order in the transparent pass, as `Material::depth_bias` (more
/// negative draws first): the sky, then the stars over it, then the moon,
/// added over them (the stars behind its disk are left out where they are
/// drawn, `stars.wgsl`). Each far past any real depth.
pub const DECK_ORDER: f32 = -3.0e8;
pub const STARS_ORDER: f32 = -2.0e8;
pub const MOON_ORDER: f32 = -1.0e8;
/// The sky's fog, over the stars and the moon it hides.
pub const SKY_FOG_ORDER: f32 = -0.5e8;

/// How thick the raymarched slab is, metres, from `sky::CLOUD_ALT_M` up: a
/// fair-weather cumulus is a few hundred metres to a kilometre tall, and the
/// field's densest columns build to the ceiling.
pub const SLAB_M: f32 = 1000.0;
/// Extinction per metre at density 1. A real cumulus is ~0.05; a thinner
/// one leaves the edges translucent enough to read as soft, while three
/// hundred metres of core still stops the sun.
pub const CLOUD_SIGMA: f32 = 0.018;
/// One tile of the detail noise, metres. It divides the field's own period
/// (`FIELD_PERIOD × CLOUD_SCALE_M`, 28.8 km), so when the drift wraps the
/// detail riding on it lands where it was (`tests/sky.rs`).
pub const DETAIL_TILE_M: f32 = 480.0;
/// The detail volume's texels a side: 7.5 m each in a release. A debug
/// build pays for every texel at boot unoptimized and takes an eighth.
pub const DETAIL_N: usize = if cfg!(debug_assertions) { 32 } else { 64 };
/// How fast a cumulus boils, m/s: the detail rising through the slab.
pub const BOIL_MS: f32 = 0.8;

/// Coarse view steps and light steps for a [`Clouds`] setting. `(0, 0)` is
/// the flat deck. A coarse step reads the coverage alone; where it finds
/// cloud the march walks it at a quarter of the step with the detail and
/// the light march's reads (`cloud.wgsl`), so most of the sky costs one read
/// a step and a cloud costs about four times its share.
pub fn steps(c: Clouds) -> (u32, u32) {
    match c {
        Clouds::Low => (0, 0),
        Clouds::Medium => (16, 3),
        Clouds::High => (32, 5),
    }
}

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
    pub air: Vec4,
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
            halo: (v3(sky::HALO_RGB) * night * p.moonlight).extend(sky::MOON_SCATTER_COS),
            scatter: v3(l.scatter).extend(0.0),
            top: v3(l.top).extend(l.thresh),
            base: v3(l.base).extend(sky::EDGE),
            fog: (v3(l.fog_rgb) * l.gain).extend(0.0),
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
            air: p.fog.lanes(),
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

/// The cloud deck's uniform, laid out to match `Cloud` in `cloud.wgsl`,
/// whose comments say what each lane holds.
#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct CloudParams {
    pub top: Vec4,
    pub base: Vec4,
    pub light: Vec4,
    pub sun: Vec4,
    pub moon: Vec4,
    pub halo: Vec4,
    pub scatter: Vec4,
    pub shape: Vec4,
    pub drift: Vec4,
    pub shade: Vec4,
    pub hue: Vec4,
    pub slab: Vec4,
    pub march: Vec4,
    pub texel: Vec4,
    pub wind: Vec4,
}

impl CloudParams {
    /// The deck for a compose's params at `nits` cd/m² per deck unit,
    /// marched `steps` ([`steps`]), its grain averaged by TAA when
    /// `temporal`, the cumulus `boil_m` metres into its rise.
    pub fn new(
        p: &ComposeParams,
        nits: f32,
        steps: (u32, u32),
        temporal: bool,
        boil_m: f32,
    ) -> Self {
        let l = DeckLight::new(p);
        let v3 = Vec3::from_array;
        // The slab is shaped by the sun, and by the moon once the sun is
        // down; held a little above the horizon so a light march leaves
        // the slab inside its steps instead of running along it.
        let toward = if p.sun.y > -0.02 { p.sun } else { p.moon };
        let toward = Vec3::new(toward.x, toward.y.max(0.15), toward.z).normalize_or(Vec3::Y);
        let field_m = sky::CLOUD_SCALE_M * sky::FIELD_PERIOD as f32;
        // The detail rides the deck: the field is read at `xz / scale +
        // drift`, so the same offset in metres keeps a lump on its cloud.
        let ride = p.drift * sky::CLOUD_SCALE_M;
        Self {
            top: v3(l.top).extend(l.thresh),
            base: v3(l.base).extend(sky::EDGE),
            light: toward.extend(0.0),
            sun: Vec4::new(l.sun_h.x, l.sun_h.y, l.toward.x, l.toward.y),
            moon: p.moon.extend(sky::HALO_COS),
            halo: (v3(sky::HALO_RGB) * p.night * p.moonlight).extend(sky::MOON_SCATTER_COS),
            scatter: v3(l.scatter).extend(0.0),
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
            shade: Vec4::new(sky::CORE_SHADE, sky::RIM_GLOW, sky::SIDE_VIEW_Y, nits),
            hue: Vec3::from_array(sky::column_depth()).extend(
                if super::quality::gpu_caps().dual_source_blending {
                    1.0
                } else {
                    0.0
                },
            ),
            slab: Vec4::new(
                sky::CLOUD_ALT_M,
                sky::CLOUD_ALT_M + SLAB_M,
                sky::CLOUD_SCALE_M,
                DETAIL_TILE_M,
            ),
            march: Vec4::new(
                steps.0 as f32,
                steps.1 as f32,
                CLOUD_SIGMA,
                if temporal { 1.0 } else { 0.0 },
            ),
            texel: Vec4::new(
                field_m / sky::FIELD_N as f32,
                DETAIL_TILE_M / DETAIL_N as f32,
                0.0,
                0.0,
            ),
            // Sampled at `p + wind`, so a lump rises as `wind.y` falls.
            wind: Vec4::new(
                ride.x.rem_euclid(DETAIL_TILE_M),
                (-boil_m).rem_euclid(DETAIL_TILE_M),
                ride.y.rem_euclid(DETAIL_TILE_M),
                0.0,
            ),
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct CloudMaterial {
    #[uniform(0)]
    pub params: CloudParams,
    #[texture(1)]
    #[sampler(2)]
    pub field: Handle<Image>,
    #[texture(3, dimension = "3d")]
    #[sampler(4)]
    pub detail: Handle<Image>,
}

impl Material for CloudMaterial {
    fn vertex_shader() -> ShaderRef {
        CLOUD_SHADER.into()
    }

    fn fragment_shader() -> ShaderRef {
        CLOUD_SHADER.into()
    }

    fn prepass_vertex_shader() -> ShaderRef {
        CLOUD_PREPASS.into()
    }

    fn prepass_fragment_shader() -> ShaderRef {
        CLOUD_PREPASS.into()
    }

    /// **Masked, for the ORDER and not for any mask.** The opaque pass
    /// draws its opaque phase, then its alpha-mask phase, then the
    /// `Skybox`; in the second phase the land is already in the depth
    /// buffer, so the march's fragments behind a hill fail the depth test
    /// before they run — with or without a depth prepass, which LOW does
    /// not have. The shader never discards.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Mask(0.5)
    }

    fn enable_shadows() -> bool {
        false
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

fn hash3(seed: u64, x: i32, y: i32, z: i32) -> u64 {
    let mut h = seed
        ^ (x as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as i64 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (z as i64 as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    h
}

/// One octave of tiling cellular noise at `p` (cell units, `0..cells`):
/// one minus the distance to the nearest of one point per cell, so each
/// cell is a rounded lump — the cauliflower a cumulus is made of.
fn worley(seed: u64, p: Vec3, cells: i32) -> f32 {
    let c = p.floor();
    let mut best = f32::MAX;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let cx = c.x as i32 + dx;
                let cy = c.y as i32 + dy;
                let cz = c.z as i32 + dz;
                let h = hash3(
                    seed,
                    cx.rem_euclid(cells),
                    cy.rem_euclid(cells),
                    cz.rem_euclid(cells),
                );
                let jitter = Vec3::new(
                    (h & 0xFFFF) as f32 / 65_536.0,
                    ((h >> 16) & 0xFFFF) as f32 / 65_536.0,
                    ((h >> 32) & 0xFFFF) as f32 / 65_536.0,
                );
                let f = Vec3::new(cx as f32, cy as f32, cz as f32) + jitter;
                best = best.min(f.distance_squared(p));
            }
        }
    }
    1.0 - best.sqrt().min(1.0)
}

/// The detail volume's values, `n³` of them x-fastest, spread over `0..=1`:
/// three octaves of [`worley`], each twice as fine, tiling on the cube.
pub fn detail_noise(seed: u64, n: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(n * n * n);
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let u = Vec3::new(x as f32, y as f32, z as f32) / n as f32;
                let v = 0.625 * worley(seed ^ 0x0c1d, u * 4.0, 4)
                    + 0.25 * worley(seed ^ 0x0c2d, u * 8.0, 8)
                    + 0.125 * worley(seed ^ 0x0c3d, u * 16.0, 16);
                out.push(v);
            }
        }
    }
    let lo = out.iter().copied().fold(f32::MAX, f32::min);
    let hi = out.iter().copied().fold(f32::MIN, f32::max);
    let span = (hi - lo).max(1e-6);
    for v in out.iter_mut() {
        *v = (*v - lo) / span;
    }
    out
}

/// [`detail_noise`] as a repeating, mipped 3D texture: a far step reads a
/// coarser level instead of aliasing.
pub fn detail_volume(seed: u64, n: usize) -> Image {
    let mut level: Vec<u8> = detail_noise(seed, n)
        .iter()
        .map(|v| (v * 255.0 + 0.5) as u8)
        .collect();
    let mut data = level.clone();
    let mut size = n;
    let mut levels = 1;
    while size > 1 {
        let h = size / 2;
        let mut next = vec![0u8; h * h * h];
        for z in 0..h {
            for y in 0..h {
                for x in 0..h {
                    let mut sum = 0u32;
                    for (dx, dy, dz) in [
                        (0, 0, 0),
                        (1, 0, 0),
                        (0, 1, 0),
                        (1, 1, 0),
                        (0, 0, 1),
                        (1, 0, 1),
                        (0, 1, 1),
                        (1, 1, 1),
                    ] {
                        let (sx, sy, sz) = (2 * x + dx, 2 * y + dy, 2 * z + dz);
                        sum += level[(sz * size + sy) * size + sx] as u32;
                    }
                    next[(z * h + y) * h + x] = ((sum + 4) / 8) as u8;
                }
            }
        }
        data.extend_from_slice(&next);
        level = next;
        size = h;
        levels += 1;
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width: n as u32,
            height: n as u32,
            depth_or_array_layers: n as u32,
        },
        TextureDimension::D3,
        TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = levels;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// The sky fog's uniform, laid out to match `SkyFog` in `sky_fog.wgsl`.
#[derive(Clone, Copy, Default, ShaderType, Debug, PartialEq)]
pub struct SkyFogParams {
    /// xyz the ground fog's colour, exposed linear, as `DistanceFog` takes it.
    pub color: Vec4,
    /// `sky::SkyFog::lanes`.
    pub air: Vec4,
    /// xyz the ground fog's sun glow (`sky::fog_glow`, times its alpha); w
    /// its exponent.
    pub glow: Vec4,
}

impl SkyFogParams {
    /// The fog over a compose's sky, the colour the rig gives the ground's
    /// (`sky::fog_color` of the same light, weight and extinction), or none
    /// under the sea, where the shell is the sky (`underwater.rs`).
    pub fn new(p: &ComposeParams, underwater: bool) -> Self {
        if underwater {
            return Self::default();
        }
        let c = sky::fog_rgb(p.light, p.dark, sky::fog_grey(p.fog.sigma()));
        let g = sky::fog_glow().to_linear();
        Self {
            color: Vec3::from_array(c).extend(0.0),
            air: p.fog.lanes(),
            glow: (Vec3::new(g.red, g.green, g.blue) * g.alpha).extend(sky::FOG_SUN_EXPONENT),
        }
    }
}

/// The weather's fog over the sky where the atmosphere draws (`sky_fog.wgsl`).
/// Under WebGL2 the browser's deck folds it in, having no atmosphere to sit
/// in front of.
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
pub struct SkyFogMaterial {
    #[uniform(0)]
    pub params: SkyFogParams,
}

impl Material for SkyFogMaterial {
    fn vertex_shader() -> ShaderRef {
        SKY_FOG_SHADER.into()
    }

    fn fragment_shader() -> ShaderRef {
        SKY_FOG_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn depth_bias(&self) -> f32 {
        SKY_FOG_ORDER
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

/// The browser's deck, which `drive` writes.
#[derive(Resource)]
pub struct DeckHandle(pub Handle<DeckMaterial>);

/// The cloud deck, which `drive` writes.
#[derive(Resource)]
pub struct CloudHandle(pub Handle<CloudMaterial>);

/// The sky's fog, which `drive` writes.
#[derive(Resource)]
pub struct SkyFogHandle(pub Handle<SkyFogMaterial>);

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

/// Build this target's sky. After `sky::setup`.
#[allow(clippy::too_many_arguments)]
pub fn setup(
    mut commands: Commands,
    composer: Option<Res<sky::SkyComposer>>,
    world: Res<super::WorldId>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut decks: ResMut<Assets<DeckMaterial>>,
    mut clouds: ResMut<Assets<CloudMaterial>>,
    mut fogs: ResMut<Assets<SkyFogMaterial>>,
) {
    let Some(composer) = composer else {
        return;
    };
    let triangle = meshes.add(fullscreen_mesh());
    let mesh = Mesh3d(triangle.clone());
    let common = (
        super::WorldEntity,
        DeckSky,
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    );
    if sky::BAKE_BACKDROP {
        let material = decks.add(DeckMaterial {
            params: DeckParams::new(&ComposeParams::noon(true), sky::CLOUD_NITS),
            field: composer.field_image().clone(),
        });
        commands.spawn((common, mesh, MeshMaterial3d(material.clone())));
        commands.insert_resource(DeckHandle(material));
    } else {
        let material = clouds.add(CloudMaterial {
            params: CloudParams::new(
                &ComposeParams::noon(false),
                sky::CLOUD_NITS,
                steps(Clouds::Low),
                false,
                0.0,
            ),
            field: composer.field_image().clone(),
            detail: images.add(detail_volume(world.seed, DETAIL_N)),
        });
        commands.spawn((common, mesh, MeshMaterial3d(material.clone())));
        commands.insert_resource(CloudHandle(material));
        let fog = fogs.add(SkyFogMaterial {
            params: SkyFogParams::default(),
        });
        commands.spawn((
            super::WorldEntity,
            Transform::default(),
            NoFrustumCulling,
            NotShadowCaster,
            NotShadowReceiver,
            Mesh3d(triangle),
            MeshMaterial3d(fog.clone()),
        ));
        commands.insert_resource(SkyFogHandle(fog));
    }
}

/// This frame's sky, off this frame's weather — the drift moves every
/// frame here rather than every compose — at the brightness the rig gives
/// the deck, which carries the lightning.
#[allow(clippy::too_many_arguments)]
pub fn drive(
    weather: Res<WeatherNow>,
    settings: Res<super::Settings>,
    time: Res<Time>,
    deck: Option<Res<DeckHandle>>,
    cloud: Option<Res<CloudHandle>>,
    fog: Option<Res<SkyFogHandle>>,
    cam: Query<(Option<&Skybox>, Has<TemporalAntiAliasing>), With<EyeCam>>,
    mut decks: ResMut<Assets<DeckMaterial>>,
    mut clouds: ResMut<Assets<CloudMaterial>>,
    mut fogs: ResMut<Assets<SkyFogMaterial>>,
) {
    let live = ComposeParams::from_weather(&weather);
    let nits = sky::deck_nits(weather.flash);
    if let Some(handle) = deck {
        // The browser's deck is drawn at the `Skybox`'s brightness, the
        // same number, so the cube under it and the deck agree.
        let nits = cam
            .single()
            .ok()
            .and_then(|(s, _)| s.map(|s| s.brightness / sky::DECK_GAIN))
            .unwrap_or(nits);
        let want = DeckParams::new(&live, nits);
        let stale = decks.get(&handle.0).is_some_and(|m| m.params != want);
        if stale {
            if let Some(m) = decks.get_mut(&handle.0) {
                m.params = want;
            }
        }
    }
    if let Some(handle) = cloud {
        let temporal = cam.single().is_ok_and(|(_, taa)| taa);
        let tier = super::quality::effective(settings.gfx).clouds;
        let want = CloudParams::new(
            &live,
            nits,
            steps(tier),
            temporal,
            time.elapsed_secs_wrapped() * BOIL_MS,
        );
        let stale = clouds.get(&handle.0).is_some_and(|m| m.params != want);
        if stale {
            if let Some(m) = clouds.get_mut(&handle.0) {
                m.params = want;
            }
        }
    }
    if let Some(handle) = fog {
        let want = SkyFogParams::new(&live, weather.underwater);
        let stale = fogs.get(&handle.0).is_some_and(|m| m.params != want);
        if stale {
            if let Some(m) = fogs.get_mut(&handle.0) {
                m.params = want;
            }
        }
    }
}
