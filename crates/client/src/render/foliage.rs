//! Foliage that moves: wind, trails and a distance fade for the grass, the
//! bushes and the near trees — Rust's (Facepunch's) foliage shader, rebuilt.
//!
//! The reference game's natural world is alive because of one shared foliage
//! shader, not because of its art: everything green sways on one wind with a
//! fast flutter on top (devblogs 149–150), grass fades out smoothly with 3D
//! distance (164, 167), players leave trails of flattened grass for 60 s
//! (128), cards seen edge-on are hidden (193), and leaves pass light from
//! behind (147, 198). This file is that shader's material.
//!
//! - **One material type, [`FoliageMaterial`]:** `StandardMaterial` with
//!   `foliage.wgsl`'s vertex stage (and `foliage_prepass.wgsl`'s twin, so the
//!   depth prepass and the shadow pass move the same vertex the same way).
//! - **Per-material shape, global state.** A material's uniform is its
//!   stiffness and fade ([`Kind::params`]) and never changes: Bevy re-extracts
//!   and re-specializes every mesh wearing a material that changed, which for
//!   the tree ring would be every tree every frame. The wind, the clock and
//!   the trails live in two textures written straight into GPU memory each
//!   frame ([`write_textures`]), which no material sees change.
//! - **Grass is baked, plants are not.** A grass tile is world-space, so its
//!   cards carry their height above the root and a per-card random in
//!   `UV_1` (`clutter::card`). A tree or a bush is drawn in its own frame, so
//!   the shader reads the height off the local y and the phase off the
//!   instance's position.
//! - **Trees and bushes are swapped, not rebuilt.** `props.rs` keeps building
//!   `StandardMaterial` handles (every renderer-tier gate reads those), and
//!   registers the ones that sway here; [`swap`] gives a freshly spawned mesh
//!   the foliage twin of its material in the frame it appears.

use std::sync::Arc;

use bevy::asset::{Asset, RenderAssetUsages};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, StandardMaterial};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TexelCopyBufferLayout, TextureDimension, TextureFormat,
};
use bevy::render::renderer::RenderQueue;
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::shader::ShaderRef;

use super::{weather::WeatherNow, Eye};

/// Grass, bushes and the near trees: `StandardMaterial` plus the sway.
pub type FoliageMaterial = ExtendedMaterial<StandardMaterial, Foliage>;

/// The main-pass shader, resolved against the asset root.
pub const SHADER: &str = "shaders/foliage.wgsl";
/// Its prepass twin (depth, normals, shadows).
pub const PREPASS_SHADER: &str = "shaders/foliage_prepass.wgsl";

/// Where grass starts to thin out, metres from the eye. **(knob)**
///
/// Each card sinks into the ground at its own distance between this and
/// [`GRASS_FADE_END_M`], so the turf thins rather than ending on a line. The
/// end sits inside the clutter ring's nearest edge (`3 × CLUTTER_TILE_M` =
/// 48 m), so a tile streaming in or out is never seen doing it.
pub const GRASS_FADE_START_M: f32 = 30.0;
/// Where the last blade has sunk, metres. See [`GRASS_FADE_START_M`].
pub const GRASS_FADE_END_M: f32 = 46.0;

/// Texels per side of the trail map.
pub const TRAMPLE_TEXELS: usize = 128;
/// Metres one trail texel covers — a footprint is a few of them.
pub const TRAMPLE_TEXEL_M: f32 = 0.25;
/// The trail window's edge, metres, centred on the eye.
pub const TRAMPLE_M: f32 = TRAMPLE_TEXELS as f32 * TRAMPLE_TEXEL_M;
/// How long a trail takes to spring back, seconds — the reference game's 60.
pub const TRAIL_S: f32 = 60.0;
/// A walker's footprint radius on the trail map, metres.
pub const STAMP_R_M: f32 = 0.5;
/// How fast a walker flattens what they stand on, full per second. Walking
/// at 5 m/s over a 1 m footprint presses ~0.5, so a second pass presses
/// further — the reference game's "walking over the same piece of grass twice
/// will push it down even more".
pub const STAMP_RATE: f32 = 2.5;

/// The wind's floor: grass never stands perfectly still.
const WIND_FLOOR: f32 = 0.25;
/// Seconds the shader's wind strength takes to follow the weather.
const WIND_EASE_S: f32 = 4.0;
/// The shader clock wraps here, seconds — one small jump every ~68 minutes
/// instead of sines losing precision as `f32` time grows.
const CLOCK_WRAP_S: f64 = 4096.0;

/// The per-material uniform. See `foliage_common.wgsl` for each lane.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct FoliageParams {
    pub sway: Vec4,
    pub fade: Vec4,
    pub misc: Vec4,
}

/// The extension: the uniform, the state texels and the trail map.
#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct Foliage {
    #[uniform(100)]
    pub params: FoliageParams,
    #[texture(101, sample_type = "float", filterable = false)]
    pub state: Handle<Image>,
    #[texture(102)]
    #[sampler(103)]
    pub trample: Handle<Image>,
}

impl MaterialExtension for Foliage {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
    // The same displacement in the prepass, or a card's depth lands where the
    // main pass does not draw it — `far_trees::TreeCard` learned that one.
    fn prepass_vertex_shader() -> ShaderRef {
        PREPASS_SHADER.into()
    }
    fn prepass_fragment_shader() -> ShaderRef {
        PREPASS_SHADER.into()
    }
}

/// What kind of plant a material dresses — its stiffness, its fade and how
/// much light it lets through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A grass tile's cards (`clutter.rs`): baked, faded, trampled.
    Grass,
    /// A bush's leaf cards, drawn in the blob's frame.
    BushLeaf,
    /// A near tree's trunk: sways with its canopy, never flutters.
    Bark,
    /// A conifer's needle cards.
    Needle,
    /// A broadleaf's leaf cards.
    Leaf,
}

/// A tree's sway: amplitude at the reference height, the reference height
/// (`props::PINE_H`), and the angular frequency. **Shared by bark and canopy
/// on purpose** — the two halves of one tree are two meshes, and any
/// difference here tears the canopy off its branches.
const TREE_SWAY: [f32; 3] = [0.22, 6.6, 0.9];

impl Kind {
    /// The uniform for this kind. Static: see the module doc for why it
    /// must never change at runtime.
    pub fn params(self) -> FoliageParams {
        let [ta, th, tw] = TREE_SWAY;
        let (sway, fade, misc) = match self {
            Kind::Grass => (
                Vec4::new(0.06, super::clutter::TUFT_H, 2.6, 0.012),
                // Half the leaves' edge-on cut: you look DOWN at grass, and
                // every vertical card is near edge-on from above.
                Vec4::new(GRASS_FADE_START_M, GRASS_FADE_END_M, 0.5, 0.0),
                Vec4::new(1.0, 0.0, 0.0, 0.0),
            ),
            Kind::BushLeaf => (
                Vec4::new(0.09, 1.5, 1.7, 0.02),
                // The bush's frame is the blob's: its root is `lift` below.
                Vec4::new(
                    0.0,
                    0.0,
                    1.0,
                    -super::props::archetype_lift(sim_core::terrain::Occupant::Bush),
                ),
                Vec4::ZERO,
            ),
            Kind::Bark => (Vec4::new(ta, th, tw, 0.0), Vec4::ZERO, Vec4::ZERO),
            Kind::Needle => (
                Vec4::new(ta, th, tw, 0.025),
                Vec4::new(0.0, 0.0, 1.0, 0.0),
                Vec4::ZERO,
            ),
            Kind::Leaf => (
                Vec4::new(ta, th, tw, 0.035),
                Vec4::new(0.0, 0.0, 1.0, 0.0),
                Vec4::ZERO,
            ),
        };
        FoliageParams { sway, fade, misc }
    }

    /// Light let through from behind (`StandardMaterial::diffuse_transmission`).
    /// Bevy spends it from DIRECT light only — the reference game's own fix
    /// ("disabled translucency coming from indirect lighting", DB198).
    pub fn transmission(self) -> f32 {
        match self {
            Kind::Grass | Kind::BushLeaf => 0.3,
            Kind::Needle => 0.25,
            Kind::Leaf => 0.35,
            Kind::Bark => 0.0,
        }
    }
}

/// The foliage textures and the materials registered for swapping.
#[derive(Resource)]
pub struct Foliages {
    pub state: Handle<Image>,
    pub trample: Handle<Image>,
    pending: Vec<(Handle<StandardMaterial>, Kind)>,
    map: HashMap<AssetId<StandardMaterial>, Handle<FoliageMaterial>>,
}

impl Foliages {
    /// The registry, around the two textures every foliage material binds.
    pub fn new(state: Handle<Image>, trample: Handle<Image>) -> Self {
        Self {
            state,
            trample,
            pending: Vec::new(),
            map: HashMap::default(),
        }
    }

    /// Ask for every mesh drawn with `h` to sway as `kind`.
    pub fn register(&mut self, h: &Handle<StandardMaterial>, kind: Kind) {
        self.pending.push((h.clone(), kind));
    }

    /// The foliage twin of a `StandardMaterial`.
    ///
    /// **Transmission is paid for, not subtracted.** Bevy's diffuse
    /// transmission is energy-conserving — it takes `t` out of the lit side
    /// to give it to the back — so a card at `t` = 0.3 would draw 30% darker
    /// face-on than the frame `ART.md` was measured against. Dividing the base
    /// colour by `1 − t` gives the lit side back exactly and leaves the back
    /// lit at `t / (1 − t)` of it.
    pub fn make(&self, mut base: StandardMaterial, kind: Kind) -> FoliageMaterial {
        let t = kind.transmission();
        if t > 0.0 {
            let c = base.base_color.to_linear();
            let k = 1.0 / (1.0 - t);
            base.base_color =
                Color::LinearRgba(LinearRgba::new(c.red * k, c.green * k, c.blue * k, c.alpha));
            base.diffuse_transmission = t;
        }
        FoliageMaterial {
            base,
            extension: Foliage {
                params: kind.params(),
                state: self.state.clone(),
                trample: self.trample.clone(),
            },
        }
    }
}

/// The CPU half of the trail map: one float per texel, world-anchored and
/// wrapping, so the window follows the eye without copying a texel.
#[derive(Resource)]
pub struct Trample {
    px: Vec<f32>,
    /// The world column (row) each texel column (row) currently holds — a
    /// slot whose world cell moved out of the window is cleared before reuse.
    cols: Vec<i64>,
    rows: Vec<i64>,
    /// The window's minimum world cell.
    origin: IVec2,
    /// The shader's wind strength, eased toward the weather.
    wind: f32,
}

impl Default for Trample {
    fn default() -> Self {
        Self {
            px: vec![0.0; TRAMPLE_TEXELS * TRAMPLE_TEXELS],
            cols: vec![i64::MIN; TRAMPLE_TEXELS],
            rows: vec![i64::MIN; TRAMPLE_TEXELS],
            origin: IVec2::ZERO,
            wind: WIND_FLOOR,
        }
    }
}

impl Trample {
    /// Move the window to be centred on `centre` (world XZ), clearing every
    /// texel column and row whose world cell changed.
    pub fn recentre(&mut self, centre: Vec2) {
        let n = TRAMPLE_TEXELS as i64;
        let c = (centre / TRAMPLE_TEXEL_M).floor().as_ivec2();
        self.origin = c - IVec2::splat(TRAMPLE_TEXELS as i32 / 2);
        let (ox, oz) = (self.origin.x as i64, self.origin.y as i64);
        for i in 0..TRAMPLE_TEXELS {
            let want = ox + (i as i64 - ox).rem_euclid(n);
            if self.cols[i] != want {
                self.cols[i] = want;
                for r in 0..TRAMPLE_TEXELS {
                    self.px[r * TRAMPLE_TEXELS + i] = 0.0;
                }
            }
            let want = oz + (i as i64 - oz).rem_euclid(n);
            if self.rows[i] != want {
                self.rows[i] = want;
                self.px[i * TRAMPLE_TEXELS..(i + 1) * TRAMPLE_TEXELS].fill(0.0);
            }
        }
    }

    /// Spring every trail back by `dt` seconds' worth.
    pub fn decay(&mut self, dt: f32) {
        let d = dt / TRAIL_S;
        for v in &mut self.px {
            *v = (*v - d).max(0.0);
        }
    }

    /// Press the grass under a walker standing at `at` (world XZ) for `dt`.
    pub fn stamp(&mut self, at: Vec2, dt: f32) {
        let r = (STAMP_R_M / TRAMPLE_TEXEL_M).ceil() as i32;
        let c = (at / TRAMPLE_TEXEL_M).floor().as_ivec2();
        let n = TRAMPLE_TEXELS as i32;
        for dz in -r..=r {
            for dx in -r..=r {
                let cell = c + IVec2::new(dx, dz);
                let rel = cell - self.origin;
                if rel.x < 0 || rel.y < 0 || rel.x >= n || rel.y >= n {
                    continue;
                }
                let mid = (cell.as_vec2() + 0.5) * TRAMPLE_TEXEL_M;
                let fall = 1.0 - (mid - at).length() / STAMP_R_M;
                if fall <= 0.0 {
                    continue;
                }
                let i =
                    cell.y.rem_euclid(n) as usize * TRAMPLE_TEXELS + cell.x.rem_euclid(n) as usize;
                self.px[i] = (self.px[i] + fall.min(1.0) * STAMP_RATE * dt).min(1.0);
            }
        }
    }

    /// How flat the grass is at a world point, 0..1 — what the shader samples.
    pub fn at(&self, p: Vec2) -> f32 {
        let c = (p / TRAMPLE_TEXEL_M).floor().as_ivec2();
        let rel = c - self.origin;
        let n = TRAMPLE_TEXELS as i32;
        if rel.x < 0 || rel.y < 0 || rel.x >= n || rel.y >= n {
            return 0.0;
        }
        self.px[c.y.rem_euclid(n) as usize * TRAMPLE_TEXELS + c.x.rem_euclid(n) as usize]
    }

    /// The window's centre, world XZ — `state` texel 1, the shader's edge.
    pub fn centre(&self) -> Vec2 {
        (self.origin.as_vec2() + TRAMPLE_TEXELS as f32 * 0.5) * TRAMPLE_TEXEL_M
    }
}

/// This frame's texels, handed to the render world.
#[derive(Resource, Clone, ExtractResource)]
pub struct FoliageUpload {
    pub state: Handle<Image>,
    pub trample: Handle<Image>,
    pub state_px: [f32; 16],
    pub trample_px: Arc<Vec<u8>>,
}

/// Register the material, the textures and the GPU writer.
pub fn plugin(app: &mut App) {
    app.add_plugins(MaterialPlugin::<FoliageMaterial>::default());
    app.add_plugins(ExtractResourcePlugin::<FoliageUpload>::default());
    app.init_resource::<Trample>();
    app.add_systems(PreStartup, init);
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app.add_systems(
            Render,
            write_textures.in_set(RenderSystems::PrepareResources),
        );
    }
}

/// The two textures, at boot.
pub fn init(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let state = images.add(Image::new_fill(
        Extent3d {
            width: 4,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0u8; 16],
        TextureFormat::Rgba32Float,
        RenderAssetUsages::RENDER_WORLD,
    ));
    let mut trample = Image::new_fill(
        Extent3d {
            width: TRAMPLE_TEXELS as u32,
            height: TRAMPLE_TEXELS as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0u8],
        TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    // Wrapping, so world cell `c` lives at texel `c mod N` (`Trample`).
    trample.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    let trample = images.add(trample);
    commands.insert_resource(FoliageUpload {
        state: state.clone(),
        trample: trample.clone(),
        state_px: [0.0; 16],
        trample_px: Arc::new(vec![0; TRAMPLE_TEXELS * TRAMPLE_TEXELS]),
    });
    commands.insert_resource(Foliages::new(state, trample));
}

/// Everything that walks through grass: other players and the animals.
type Walkers = Or<(With<super::bodies::Body>, With<super::mobs::Animal>)>;

/// How hard a blast's shock bends what it passes, in units of the plant's own
/// wind bend at full wind (`foliage_common.wgsl::displace`).
pub const BLAST_PUSH: f32 = 4.0;
/// How long after a blast its shock is still worth uploading, seconds.
pub const BLAST_SHOCK_S: f32 = 2.5;
/// How far a blast can be from the eye and still be uploaded, metres — past
/// the shock's own reach plus the grass ring.
pub const BLAST_UPLOAD_M: f32 = 140.0;
/// How hard the attack helicopter's downwash bends what is under it, in the
/// same units, at its lowest.
pub const HELI_WASH: f32 = 2.5;
/// Height above the ground at which the downwash is full, and where it is
/// gone, metres.
pub const HELI_WASH_FULL_M: f32 = 8.0;
pub const HELI_WASH_GONE_M: f32 = 40.0;

/// This frame's wind, clock and trails, into [`FoliageUpload`] — and the
/// local gusts Rust's foliage answers to as well as the global wind: a
/// blast's shock (texel 2) and the helicopter's downwash (texel 3).
#[allow(clippy::too_many_arguments)]
pub fn update(
    time: Res<Time>,
    eye: Res<Eye>,
    weather: Option<Res<WeatherNow>>,
    walkers: Query<&GlobalTransform, Walkers>,
    mut trample: ResMut<Trample>,
    mut up: ResMut<FoliageUpload>,
    contacts: Option<Res<super::impact::Contacts>>,
    heli: Query<&GlobalTransform, With<super::heli::HeliBody>>,
    world: Option<Res<super::WorldId>>,
    mut blast: Local<Option<(Vec2, f64)>>,
) {
    let dt = time.delta_secs().min(0.25);
    let w = weather.map(|w| *w).unwrap_or_default();
    let dir = if w.wind_dir.length_squared() > 0.5 {
        w.wind_dir
    } else {
        Vec2::new(0.6, 0.8)
    };
    let goal = WIND_FLOOR + (1.0 - WIND_FLOOR) * w.wind.clamp(0.0, 1.0);
    trample.wind += (goal - trample.wind) * (dt / WIND_EASE_S).min(1.0);

    let feet = Vec2::new(eye.pos.x, eye.pos.z);
    trample.recentre(feet);
    trample.decay(dt);
    if eye.placed {
        trample.stamp(feet, dt);
    }
    let reach = TRAMPLE_M * 0.5;
    for g in &walkers {
        let p = g.translation();
        let at = Vec2::new(p.x, p.z);
        if (at - feet).abs().max_element() < reach {
            trample.stamp(at, dt);
        }
    }

    let clock = (time.elapsed_secs_f64() % CLOCK_WRAP_S) as f32;
    let centre = trample.centre();

    // The most recent blast in reach, and how long ago.
    let now = time.elapsed_secs_f64();
    if let Some(c) = contacts.as_deref() {
        for c in c.iter() {
            if c.weapon == super::impact::Weapon::Blast && c.at.distance(eye.pos) < BLAST_UPLOAD_M {
                *blast = Some((Vec2::new(c.at.x, c.at.z), now));
            }
        }
    }
    let shock = match *blast {
        Some((at, t0)) if now - t0 < BLAST_SHOCK_S as f64 => {
            [at.x, at.y, BLAST_PUSH, (now - t0) as f32]
        }
        _ => [0.0; 4],
    };
    // The helicopter's downwash, by how low it is over the ground under it.
    let wash = heli
        .iter()
        .next()
        .zip(world.as_deref())
        .map(|(g, w)| {
            let p = g.translation();
            let ground =
                sim_core::terrain::height(w.seed, p.x, p.z).max(sim_core::terrain::SEA_LEVEL);
            let low = 1.0
                - ((p.y - ground - HELI_WASH_FULL_M) / (HELI_WASH_GONE_M - HELI_WASH_FULL_M))
                    .clamp(0.0, 1.0);
            [p.x, p.z, HELI_WASH * low, 0.0]
        })
        .unwrap_or([0.0; 4]);
    up.state_px = [
        dir.x,
        dir.y,
        trample.wind,
        clock,
        centre.x,
        centre.y,
        TRAMPLE_M,
        0.0,
        shock[0],
        shock[1],
        shock[2],
        shock[3],
        wash[0],
        wash[1],
        wash[2],
        wash[3],
    ];
    let px = Arc::make_mut(&mut up.trample_px);
    for (dst, v) in px.iter_mut().zip(&trample.px) {
        *dst = (v * 255.0 + 0.5) as u8;
    }
}

/// Give every mesh spawned this frame with a registered material its foliage
/// twin. After the streamers, so it sees their spawns the frame they land.
pub fn swap(
    mut commands: Commands,
    reg: Option<ResMut<Foliages>>,
    std_mats: Res<Assets<StandardMaterial>>,
    mut mats: ResMut<Assets<FoliageMaterial>>,
    added: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>),
        Added<MeshMaterial3d<StandardMaterial>>,
    >,
) {
    let Some(mut reg) = reg else { return };
    if !reg.pending.is_empty() {
        for (h, kind) in std::mem::take(&mut reg.pending) {
            let Some(base) = std_mats.get(&h) else {
                continue;
            };
            let m = reg.make(base.clone(), kind);
            let fh = mats.add(m);
            reg.map.insert(h.id(), fh);
        }
    }
    if reg.map.is_empty() {
        return;
    }
    for (e, m) in &added {
        if let Some(fh) = reg.map.get(&m.0.id()) {
            commands
                .entity(e)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert(MeshMaterial3d(fh.clone()));
        }
    }
}

/// Write this frame's texels into the textures the materials already bind.
///
/// **In place, on purpose.** Modifying the `Image` assets would make Bevy
/// build fresh GPU textures, and every foliage bind group would go on pointing
/// at the old ones; a write into the same texture is invisible to everything
/// but the shader.
pub fn write_textures(
    up: Option<Res<FoliageUpload>>,
    images: Res<RenderAssets<GpuImage>>,
    queue: Res<RenderQueue>,
) {
    let Some(up) = up else { return };
    if let Some(img) = images.get(&up.state) {
        let mut bytes = [0u8; 64];
        for (i, f) in up.state_px.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&f.to_le_bytes());
        }
        queue.write_texture(
            img.texture.as_image_copy(),
            &bytes,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(64),
                rows_per_image: None,
            },
            Extent3d {
                width: 4,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }
    if let Some(img) = images.get(&up.trample) {
        queue.write_texture(
            img.texture.as_image_copy(),
            &up.trample_px,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(TRAMPLE_TEXELS as u32),
                rows_per_image: None,
            },
            Extent3d {
                width: TRAMPLE_TEXELS as u32,
                height: TRAMPLE_TEXELS as u32,
                depth_or_array_layers: 1,
            },
        );
    }
}
