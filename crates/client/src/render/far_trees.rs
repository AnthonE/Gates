//! The treeline past the prop rings: every tree on the island as one
//! camera-facing card.
//!
//! **Why this exists.** The near ring draws real trees to 128–192 m and the
//! outer ring to ~290 m (`props::OUTER_RADIUS`), and past that the far ground
//! mesh ran to the horizon with nothing standing on it — a bald island under
//! a forest. Both rings' far trees wear these same cards ([`TreeCards`]). Rust's answer (`reference/FORESTS.md`
//! §5.5) is the one taken here: rank by distance, draw the nearest as meshes,
//! and bill everything else to a cheap card.
//!
//! - **The card is the real tree, photographed once.** At boot each pool
//!   variant (`tree::conifer`) is rasterised side-on on the CPU — bark and
//!   needle cards, the needle/leaf alpha included — into one atlas cell.
//!   Nothing here draws a tree the near ring does not draw.
//! - **Two triangles a tree, one mesh per 256 m tile.** A tile bakes every
//!   tree in its 1,024 scatter cells into one buffer, so the whole island is a
//!   few hundred draws and no per-tree entity. The vertex shader turns each
//!   card to the camera about the vertical.
//! - **It steps aside where the rings draw.** One bit per 64 m prop chunk of
//!   the window the rings stream in marks every chunk the near or outer ring
//!   has built, and a card standing in one collapses in the vertex shader. So
//!   the hand-off follows the rings' own streaming and never leaves a gap or
//!   draws a tree twice. The bits ride in the material's uniform, not in a
//!   texture: a mask image written after the material was prepared never
//!   reached the bind group, so every island card stood on top of the real
//!   tree in front of it, and on the stump of every tree felled.
//! - **Lit, fogged and hazed like everything else.** The fragment stage is
//!   `StandardMaterial`'s own; the vertex stage hands it a rounded normal so a
//!   card is lit like a crown rather than a sheet.
//!
//! Felled trees keep their island card until a rebuild; at this range they
//! are a few pixels, and the rings own everything close enough to chop.

use bevy::asset::{Asset, RenderAssetUsages};
use bevy::image::{ImageSampler, ImageSamplerDescriptor};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, StandardMaterial};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use sim_core::gather::cell_key;
use sim_core::terrain::{self, Occupant};

use super::mipmap;
use super::props::{self, PropRing};
use super::terrain_mesh::{self, CHUNK_M};
use super::tree;
use super::{Eye, WorldEntity, WorldId};

/// The card material: `StandardMaterial` with the turning vertex stage.
pub type TreeCardMaterial = ExtendedMaterial<StandardMaterial, TreeCard>;

/// The vertex shader, resolved against the asset root.
pub const SHADER: &str = "shaders/tree_card.wgsl";
/// Its depth-prepass twin.
pub const PREPASS_SHADER: &str = "shaders/tree_card_prepass.wgsl";

/// Edge of one tile of cards, metres — four prop chunks a side.
pub const TILE_M: f32 = 256.0;
/// Scatter cells walked per frame while the island is being carded.
///
/// A cell is one `scatter_memo` (a handful of height taps); 768 of them is a
/// few milliseconds, and the island is carded in a few hundred frames — the
/// nearest tiles first, so the horizon fills in from the player outward.
pub const CELLS_PER_FRAME: usize = 768;
/// One atlas cell's width and height, texels. Tall, because a tree is: the
/// conifer's card is ~5.5 m by 14 m, and a square cell spent half its texels
/// across a silhouette that is narrow.
pub const CARD_W: u32 = 128;
/// See [`CARD_W`].
pub const CARD_H: u32 = 256;
/// Supersampling of the bake, per axis. The card's alpha is the share of
/// subsamples a needle covered, which is what lets the mask cut anti-alias.
const BAKE_SS: u32 = 3;
/// The bark photograph's mean colour, sRGB — the near trunk is that photograph
/// times a mean-1 vertex field, so the bake multiplies the field by this.
const BARK_MEAN: u32 = 0x5c4636;
/// [`BARK_MEAN`] for the broadleaf's birch photograph (`birch_albedo.jpg`,
/// linear mean 0.316 0.315 0.320).
const BIRCH_MEAN: u32 = 0x989899;
/// How far a card is sunk, metres: the far ground sits under the true one.
const CARD_SINK_M: f32 = 0.3;

/// A card's extent at slot scale 1: half-width and height, metres.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CardDims {
    pub half_w: f32,
    pub h: f32,
}

/// Chunks per side of the window the ring mask covers: the outer ring's.
pub const MASK_SIDE: i32 = 2 * props::OUTER_RADIUS + 1;
const _: () = assert!(MASK_SIDE * MASK_SIDE <= 128, "the mask is one UVec4");

/// The card's uniform.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct CardParams {
    /// x = chunk edge (m), y = 1 on a ring card (it ignores the mask: its
    /// entity IS the ring's tree), z and w reserved.
    pub params: Vec4,
    /// The mask's window: x, y = its first chunk, z = its side in chunks.
    pub window: IVec4,
    /// One bit per window chunk, row-major from the first: set where a ring
    /// draws that chunk's trees, so an island card there collapses.
    pub bits: UVec4,
}

#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct TreeCard {
    #[uniform(100)]
    pub card: CardParams,
}

impl MaterialExtension for TreeCard {
    fn vertex_shader() -> ShaderRef {
        SHADER.into()
    }
    // A card is two triangles standing on a point until the vertex stage
    // turns them, so the prepass needs the same turn (`PREPASS_SHADER`).
    // **It needs the prepass itself**: with a depth prepass on the camera the
    // main pass writes no depth for masked meshes, and a card out of the
    // prepass wrote none anywhere — the sky pass fogged it at the depth of
    // the terrain behind it, a see-through ghost of a tree (2026-09-28).
    fn prepass_vertex_shader() -> ShaderRef {
        PREPASS_SHADER.into()
    }
    // A card turned to the camera casts a card's shadow toward the sun, which
    // is wrong from every angle but one; the shadow cascades end short of
    // where cards begin anyway.
    fn enable_shadows() -> bool {
        false
    }
}

/// The baked cards, built once at startup: the atlas, each variant's extent,
/// one per-variant card mesh for the rings, and the materials.
#[derive(Resource)]
pub struct TreeCards {
    pub dims: Vec<CardDims>,
    /// One card per pool variant in the tree's own frame (base at the origin),
    /// for a ring tree's far part — `props::spawn_slot` and
    /// `props::spawn_outer_tree` draw these where they drew the lathed hull.
    pub ring_meshes: Vec<Handle<Mesh>>,
    /// Per tint, for the ring cards. They ignore the mask.
    pub ring_materials: [Handle<TreeCardMaterial>; props::TINT_POOL],
    /// The island tiles' material, which steps aside for the rings.
    pub island_material: Handle<TreeCardMaterial>,
}

/// Bake the cards. At startup, once: six trees rasterised on the CPU.
pub fn init(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<TreeCardMaterial>>,
) {
    let (atlas, dims) = bake_atlas();
    let atlas = images.add(atlas);
    let mut material = |tint: f32, ring: bool| {
        materials.add(TreeCardMaterial {
            base: StandardMaterial {
                base_color: Color::linear_rgb(tint, tint, tint),
                base_color_texture: Some(atlas.clone()),
                alpha_mode: AlphaMode::Mask(0.5),
                perceptual_roughness: 0.9,
                reflectance: super::fresnel::DIELECTRIC,
                // The near canopy's own specular, or the card sheens where the
                // tree it stands for does not.
                specular_tint: Color::linear_rgb(
                    tree::CANOPY_SPECULAR_TINT,
                    tree::CANOPY_SPECULAR_TINT,
                    tree::CANOPY_SPECULAR_TINT,
                ),
                // The card is wound to face the camera by construction, and a
                // double-sided flip would turn its rounded normal inside out.
                cull_mode: None,
                ..default()
            },
            extension: TreeCard {
                card: CardParams {
                    params: Vec4::new(CHUNK_M, if ring { 1.0 } else { 0.0 }, 0.0, 0.0),
                    window: IVec4::new(0, 0, MASK_SIDE, 0),
                    bits: UVec4::ZERO,
                },
            },
        })
    };
    let ring_materials = props::tint_pool().map(|t| material(t, true));
    let island_material = material(1.0, false);
    let pool = dims.len();
    let ring_meshes = (0..pool)
        .map(|v| {
            let mut b = TileBuild::new((0, 0));
            let (u0, u1) = (v as f32 / pool as f32, (v + 1) as f32 / pool as f32);
            push_quad(&mut b, Vec3::ZERO, dims[v].half_w, dims[v].h, u0, u1, 1.0);
            meshes.add(tile_mesh(b))
        })
        .collect();
    commands.insert_resource(TreeCards {
        dims,
        ring_meshes,
        ring_materials,
        island_material,
    });
}

/// The island's cards, and how far the carding has got.
#[derive(Resource, Default)]
pub struct FarForest {
    /// The seed the tiles below belong to.
    seed: Option<u64>,
    /// Tiles still to card, nearest LAST (popped off the end).
    queue: Vec<(i32, i32)>,
    /// The tile being carded and how far into its cells.
    current: Option<TileBuild>,
    tiles: HashMap<(i32, i32), Entity>,
    /// The mask's window and bits as last written to the island material.
    masked: Option<(IVec4, UVec4)>,
}

impl FarForest {
    /// Tiles carded so far.
    pub fn len(&self) -> usize {
        self.tiles.len()
    }
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }
}

struct TileBuild {
    key: (i32, i32),
    next: usize,
    lat: terrain::Lattice,
    pos: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    uv_b: Vec<[f32; 2]>,
    color: Vec<[f32; 4]>,
}

impl TileBuild {
    fn new(key: (i32, i32)) -> Self {
        Self {
            key,
            next: 0,
            lat: terrain::Lattice::new(),
            pos: Vec::new(),
            uv: Vec::new(),
            uv_b: Vec::new(),
            color: Vec::new(),
        }
    }
}

/// Cells a tile holds per side.
fn tile_cells() -> i32 {
    (TILE_M / terrain::CELL_SIZE) as i32
}

/// Tiles per island side.
fn tiles_per_side() -> i32 {
    (terrain::ISLAND_SIZE / TILE_M).ceil() as i32
}

/// Card and carding, every frame: build the material once, walk
/// [`CELLS_PER_FRAME`] cells of the current tile, and keep the ring mask in
/// step with what the prop rings have built.
#[allow(clippy::too_many_arguments)]
pub fn stream(
    mut commands: Commands,
    mut forest: ResMut<FarForest>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<TreeCardMaterial>>,
    world: Res<WorldId>,
    eye: Res<Eye>,
    ring: Res<PropRing>,
    cards: Option<Res<TreeCards>>,
) {
    let Some(cards) = cards else {
        return;
    };
    let forest = &mut *forest;
    if forest.seed != Some(world.seed) {
        for (_, e) in forest.tiles.drain() {
            commands.entity(e).despawn();
        }
        forest.seed = Some(world.seed);
        forest.current = None;
        forest.masked = None;
        // Nearest last, so `pop` takes the nearest.
        let n = tiles_per_side();
        let (ex, ez) = (eye.pos.x, eye.pos.z);
        let mut q: Vec<(i32, i32)> = (0..n).flat_map(|z| (0..n).map(move |x| (x, z))).collect();
        let d2 = |t: &(i32, i32)| {
            let cx = (t.0 as f32 + 0.5) * TILE_M - ex;
            let cz = (t.1 as f32 + 0.5) * TILE_M - ez;
            cx * cx + cz * cz
        };
        q.sort_by(|a, b| d2(b).total_cmp(&d2(a)));
        forest.queue = q;
    }
    // The mask: every chunk a ring has trees standing in, over the window the
    // rings stream in. Written into the material, so it is re-prepared with
    // the bits rather than sampling a texture it bound once.
    let now = ring_mask(&eye, ring.chunks());
    if forest.masked != Some(now) {
        if let Some(m) = materials.get_mut(&cards.island_material) {
            m.extension.card.window = now.0;
            m.extension.card.bits = now.1;
        }
        forest.masked = Some(now);
    }

    // The carding.
    if forest.current.is_none() {
        match forest.queue.pop() {
            Some(key) => forest.current = Some(TileBuild::new(key)),
            None => return,
        }
    }
    let Some(mut build) = forest.current.take() else {
        return;
    };
    let tc = tile_cells();
    let total = (tc * tc) as usize;
    let end = (build.next + CELLS_PER_FRAME).min(total);
    for i in build.next..end {
        let cell_x = build.key.0 * tc + (i as i32 % tc);
        let cell_z = build.key.1 * tc + (i as i32 / tc);
        let slot = terrain::scatter_memo(
            &mut build.lat,
            world.seed,
            &world.table,
            &world.haven,
            cell_x,
            cell_z,
        );
        if slot.occupant != Occupant::Tree {
            continue;
        }
        let key = cell_key(cell_x as u16, cell_z as u16);
        push_card(&mut build, &cards.dims, &slot, key, &world);
    }
    build.next = end;
    if build.next < total {
        forest.current = Some(build);
        return;
    }
    if build.pos.is_empty() {
        return;
    }
    let key = build.key;
    let mesh = tile_mesh(build);
    let material = cards.island_material.clone();
    let e = commands
        .spawn((
            WorldEntity,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            NotShadowCaster,
            NotShadowReceiver,
        ))
        .id();
    forest.tiles.insert(key, e);
}

/// The ring mask for an eye: the window of [`MASK_SIDE`]² chunks centred on
/// the eye's chunk — the one the rings stream around — and a bit for every
/// chunk in it that a ring holds. A ring chunk outside the window (one still
/// waiting to be dropped) keeps its island cards for the frame or two it
/// takes, rather than the window growing.
pub fn ring_mask(eye: &Eye, chunks: impl Iterator<Item = (i32, i32)>) -> (IVec4, UVec4) {
    let cx = (eye.pos.x / CHUNK_M).floor() as i32 - props::OUTER_RADIUS;
    let cz = (eye.pos.z / CHUNK_M).floor() as i32 - props::OUTER_RADIUS;
    let mut bits = [0u32; 4];
    for (x, z) in chunks {
        let (lx, lz) = (x - cx, z - cz);
        if lx < 0 || lz < 0 || lx >= MASK_SIDE || lz >= MASK_SIDE {
            continue;
        }
        let i = (lz * MASK_SIDE + lx) as usize;
        bits[i / 32] |= 1 << (i % 32);
    }
    (IVec4::new(cx, cz, MASK_SIDE, 0), UVec4::from_array(bits))
}

/// A new world: drop the tiles and card the next island from scratch. The
/// tile entities are `WorldEntity` and already gone; the atlas is a function
/// of the client, not the island, and stays.
pub fn teardown(mut forest: ResMut<FarForest>) {
    *forest = FarForest::default();
}

fn push_card(
    b: &mut TileBuild,
    dims: &[CardDims],
    slot: &terrain::Slot,
    key: u32,
    world: &WorldId,
) {
    let pool = dims.len();
    if pool == 0 {
        return;
    }
    let variant = props::species_variant(slot, pool);
    let d = dims[variant];
    let y = terrain_mesh::far_ground_y(world.seed, &world.haven, slot.x, slot.z) - CARD_SINK_M;
    let u0 = variant as f32 / pool as f32;
    let u1 = (variant + 1) as f32 / pool as f32;
    // A mirror on half of them, so a stand is not one silhouette repeated.
    let (ua, ub) = if props::hash01(key, 0x0f11) < 0.5 {
        (u1, u0)
    } else {
        (u0, u1)
    };
    // The near ring's per-instance value spread, in the vertex colour here
    // because a tile is one material.
    let tint = props::tint_pool()[props::tint_of(key)];
    push_quad(
        b,
        Vec3::new(slot.x, y, slot.z),
        d.half_w * slot.scale,
        d.h * slot.scale,
        ua,
        ub,
        tint,
    );
}

/// One card: four corners on the base line, lifted by their share of the
/// height, with the sideways offset in `uv_b` for the vertex stage.
fn push_quad(b: &mut TileBuild, base: Vec3, hw: f32, h: f32, ua: f32, ub: f32, tint: f32) {
    let c = [tint, tint, tint, 1.0];
    for (s, t) in [(-1.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (-1.0, 1.0)] {
        b.pos.push([base.x, base.y + t * h, base.z]);
        b.uv.push([if s < 0.0 { ua } else { ub }, 1.0 - t]);
        b.uv_b.push([s * hw, s]);
        b.color.push(c);
    }
}

fn tile_mesh(b: TileBuild) -> Mesh {
    let n = b.pos.len() / 4;
    let mut idx = Vec::with_capacity(n * 6);
    for i in 0..n as u32 {
        let o = i * 4;
        idx.extend_from_slice(&[o, o + 1, o + 2, o, o + 2, o + 3]);
    }
    let normals = vec![[0.0, 1.0, 0.0]; b.pos.len()];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, b.pos)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, b.uv)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, b.uv_b)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, b.color)
    .with_inserted_indices(Indices::U32(idx))
}

// ── The bake ────────────────────────────────────────────────────────────────

/// Every pool variant rasterised side-on into one atlas row, with its mip
/// chain, and each card's extent in metres.
pub fn bake_atlas() -> (Image, Vec<CardDims>) {
    let pool = tree::CONIFER_POOL;
    let needle = tree::needle_image();
    let leaf = tree::leaf_image();
    let w = CARD_W * pool as u32;
    let h = CARD_H;
    let mut level0 = vec![0u8; (w * h * 4) as usize];
    let mut dims = Vec::with_capacity(pool);
    for v in 0..pool {
        let (bark, needles) = tree::conifer(v);
        let (card, gain, trunk) = if tree::species_of(v) == 0 {
            (&needle, tree::NEEDLE_MAP_GAIN, BARK_MEAN)
        } else {
            (&leaf, tree::LEAF_MAP_GAIN, BIRCH_MEAN)
        };
        let (cell, d) = bake_card(&bark, &needles, card, gain, trunk);
        dims.push(d);
        let x0 = v as u32 * CARD_W;
        for y in 0..CARD_H {
            for x in 0..CARD_W {
                let src = ((y * CARD_W + x) * 4) as usize;
                let dst = ((y * w + x0 + x) * 4) as usize;
                level0[dst..dst + 4].copy_from_slice(&cell[src..src + 4]);
            }
        }
    }
    let data = mipmap::chain(&level0, w, h, mipmap::Filter::Mask);
    let mut img = Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        level0,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.texture_descriptor.mip_level_count = mipmap::levels(w, h);
    img.data = Some(data);
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        ..default()
    });
    (img, dims)
}

struct Tri {
    p: [Vec3; 3],
    uv: [Vec2; 3],
    c: [[f32; 3]; 3],
    /// The mean of the corners' normals' y: up-facing needles are lit.
    ny: f32,
    leaf: bool,
}

fn tris_of(m: &Mesh, leaf: bool, out: &mut Vec<Tri>) {
    let Some(VertexAttributeValues::Float32x3(pos)) = m.attribute(Mesh::ATTRIBUTE_POSITION) else {
        return;
    };
    let uvs = match m.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(v)) => Some(v),
        _ => None,
    };
    let cols = match m.attribute(Mesh::ATTRIBUTE_COLOR) {
        Some(VertexAttributeValues::Float32x4(v)) => Some(v),
        _ => None,
    };
    let normals = match m.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(v)) => Some(v),
        _ => None,
    };
    let idx: Vec<usize> = match m.indices() {
        Some(Indices::U32(i)) => i.iter().map(|&k| k as usize).collect(),
        Some(Indices::U16(i)) => i.iter().map(|&k| k as usize).collect(),
        None => (0..pos.len()).collect(),
    };
    for t in idx.chunks_exact(3) {
        if t.iter().any(|&k| k >= pos.len()) {
            continue;
        }
        let t = [t[0], t[1], t[2]];
        let p = t.map(|k| Vec3::from_array(pos[k]));
        let uv = t.map(|k| {
            uvs.and_then(|u| u.get(k))
                .map_or(Vec2::ZERO, |u| Vec2::from_array(*u))
        });
        let c = t.map(|k| {
            cols.and_then(|c| c.get(k))
                .map_or([1.0; 3], |c| [c[0], c[1], c[2]])
        });
        let ny = t
            .iter()
            .map(|&k| normals.and_then(|n| n.get(k)).map_or(0.0, |n| n[1]))
            .sum::<f32>()
            / 3.0;
        out.push(Tri { p, uv, c, ny, leaf });
    }
}

fn srgb_to_linear(v: u8) -> f32 {
    let s = v as f32 / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5) as u8
}

/// The share of its albedo a card's needles keep. A real crown shows mostly
/// its own shade between the lit needles, and a card baked at bare albedo
/// read as a pale ghost beside the mesh trees in front of it (2026-09-28).
pub const CARD_CANOPY_SHADE: f32 = 0.78;

/// How much a baked texel's coverage is scaled up before the alpha test:
/// a texel half covered by needles is crown, not a gap. Modest, so the gaps
/// between limbs stay open and the outline stays ragged — at 2.0 with a
/// closing pass the crown baked to one solid blob and every far pine read as
/// a smooth green column.
pub const CARD_COVERAGE_BOOST: f32 = 1.35;

/// How dark a needle facing straight down bakes, as a share of one facing up.
///
/// The card is lit in the frame by one rounded normal for the whole crown, so
/// without this every needle in it is the same value and the crown reads as a
/// flat green column. The near tree's canopy normals are mostly its crown's
/// volume with a share of each card's own facing (`tree::blend_canopy_normals`),
/// which is what mottles it lit-and-dark; this bakes that mottling, using only
/// the vertical, because the card turns to every bearing and a side light baked
/// in would be wrong from all but one.
pub const CARD_LIGHT_FLOOR: f32 = 0.38;

/// One tree, side-on, orthographic, into a `CARD_W × CARD_H` RGBA8 sRGB cell.
fn bake_card(
    bark: &Mesh,
    needles: &Mesh,
    card: &Image,
    gain: f32,
    trunk: u32,
) -> (Vec<u8>, CardDims) {
    let (h, _) = tree::bounds(&[bark, needles]);
    let mut tris = Vec::new();
    tris_of(bark, false, &mut tris);
    tris_of(needles, true, &mut tris);
    // Half-width from the drawn x extent, not the radius: the card faces the
    // camera, so it is the silhouette's width that matters.
    let mut half_w = 0.0f32;
    for t in &tris {
        for p in &t.p {
            half_w = half_w.max(p.x.abs()).max(p.z.abs());
        }
    }
    let h = h.max(1.0);
    let half_w = half_w.max(0.5);
    let (nw, nh) = ((CARD_W * BAKE_SS) as usize, (CARD_H * BAKE_SS) as usize);
    let mut depth = vec![f32::NEG_INFINITY; nw * nh];
    let mut rgb = vec![[0.0f32; 3]; nw * nh];
    let bark_c = props::linear(trunk);
    let (tw, th, tex) = match card.data.as_ref() {
        Some(d) => (
            card.texture_descriptor.size.width as usize,
            card.texture_descriptor.size.height as usize,
            d.as_slice(),
        ),
        None => (0, 0, &[][..]),
    };
    let to_px = |p: Vec3| {
        Vec3::new(
            (p.x / half_w * 0.5 + 0.5) * nw as f32,
            (1.0 - p.y / h) * nh as f32,
            p.z,
        )
    };
    for t in &tris {
        let a = to_px(t.p[0]);
        let b = to_px(t.p[1]);
        let c = to_px(t.p[2]);
        let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        if area.abs() < 1e-6 {
            continue;
        }
        let x0 = a.x.min(b.x).min(c.x).floor().max(0.0) as usize;
        let x1 = (a.x.max(b.x).max(c.x).ceil() as usize).min(nw);
        let y0 = a.y.min(b.y).min(c.y).floor().max(0.0) as usize;
        let y1 = (a.y.max(b.y).max(c.y).ceil() as usize).min(nh);
        for py in y0..y1 {
            for px in x0..x1 {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let w0 = ((b.x - fx) * (c.y - fy) - (b.y - fy) * (c.x - fx)) / area;
                let w1 = ((c.x - fx) * (a.y - fy) - (c.y - fy) * (a.x - fx)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = a.z * w0 + b.z * w1 + c.z * w2;
                let i = py * nw + px;
                if z <= depth[i] {
                    continue;
                }
                let vc = [
                    t.c[0][0] * w0 + t.c[1][0] * w1 + t.c[2][0] * w2,
                    t.c[0][1] * w0 + t.c[1][1] * w1 + t.c[2][1] * w2,
                    t.c[0][2] * w0 + t.c[1][2] * w1 + t.c[2][2] * w2,
                ];
                let col = if t.leaf {
                    if tw == 0 {
                        vc
                    } else {
                        let uv = t.uv[0] * w0 + t.uv[1] * w1 + t.uv[2] * w2;
                        let tx = ((uv.x.rem_euclid(1.0) * tw as f32) as usize).min(tw - 1);
                        let ty = ((uv.y.rem_euclid(1.0) * th as f32) as usize).min(th - 1);
                        let k = (ty * tw + tx) * 4;
                        if k + 3 >= tex.len() || tex[k + 3] < mipmap::MASK_CUT {
                            continue;
                        }
                        let lit = (0.5 + 0.75 * t.ny).clamp(0.0, 1.0);
                        let shade = CARD_CANOPY_SHADE
                            * gain
                            * (CARD_LIGHT_FLOOR + (1.0 - CARD_LIGHT_FLOOR) * lit);
                        [
                            vc[0] * srgb_to_linear(tex[k]) * shade,
                            vc[1] * srgb_to_linear(tex[k + 1]) * shade,
                            vc[2] * srgb_to_linear(tex[k + 2]) * shade,
                        ]
                    }
                } else {
                    [vc[0] * bark_c[0], vc[1] * bark_c[1], vc[2] * bark_c[2]]
                };
                depth[i] = z;
                rgb[i] = col;
            }
        }
    }
    // Resolve the supersamples: colour averaged over what was covered, alpha
    // the share covered.
    let (rw, rh) = (CARD_W as usize, CARD_H as usize);
    let ss = BAKE_SS as usize;
    let mut acc = vec![[0.0f32; 4]; rw * rh];
    for y in 0..rh {
        for x in 0..rw {
            let mut s = [0.0f32; 4];
            for sy in 0..ss {
                for sx in 0..ss {
                    let i = (y * ss + sy) * nw + x * ss + sx;
                    if depth[i] > f32::NEG_INFINITY {
                        s[0] += rgb[i][0];
                        s[1] += rgb[i][1];
                        s[2] += rgb[i][2];
                        s[3] += 1.0;
                    }
                }
            }
            if s[3] > 0.0 {
                acc[y * rw + x] = [
                    s[0] / s[3],
                    s[1] / s[3],
                    s[2] / s[3],
                    s[3] / (ss * ss) as f32,
                ];
            }
        }
    }
    // Bleed colour into the empty texels, so a filtered edge never pulls in
    // black.
    for _ in 0..4 {
        let prev = acc.clone();
        for y in 0..rh {
            for x in 0..rw {
                if prev[y * rw + x][3] > 0.0 {
                    continue;
                }
                let mut s = [0.0f32; 4];
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= rw as i32 || ny >= rh as i32 {
                        continue;
                    }
                    let q = prev[ny as usize * rw + nx as usize];
                    if q[3] > 0.0 || (q[0] + q[1] + q[2]) > 0.0 {
                        s[0] += q[0];
                        s[1] += q[1];
                        s[2] += q[2];
                        s[3] += 1.0;
                    }
                }
                if s[3] > 0.0 {
                    acc[y * rw + x] = [s[0] / s[3], s[1] / s[3], s[2] / s[3], 0.0];
                }
            }
        }
    }
    // Close pinholes only. Needles rasterised through the needle photograph's
    // own holes leave a speckled mask, and a speckled mask at a distance reads
    // as a see-through ghost of a tree (2026-09-28) — but the gaps between
    // limbs are the silhouette, so only a hole nearly surrounded by crown is
    // filled.
    for t in acc.iter_mut() {
        t[3] = (t[3] * CARD_COVERAGE_BOOST).min(1.0);
    }
    let prev = acc.clone();
    for y in 1..rh - 1 {
        for x in 1..rw - 1 {
            if prev[y * rw + x][3] >= 0.5 {
                continue;
            }
            let mut solid = 0;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let q = prev[(y as i32 + dy) as usize * rw + (x as i32 + dx) as usize];
                    if (dx != 0 || dy != 0) && q[3] >= 0.5 {
                        solid += 1;
                    }
                }
            }
            if solid >= 7 {
                acc[y * rw + x][3] = 1.0;
            }
        }
    }
    let mut out = vec![0u8; rw * rh * 4];
    for (i, t) in acc.iter().enumerate() {
        out[i * 4] = linear_to_srgb(t[0]);
        out[i * 4 + 1] = linear_to_srgb(t[1]);
        out[i * 4 + 2] = linear_to_srgb(t[2]);
        out[i * 4 + 3] = (t[3] * 255.0 + 0.5) as u8;
    }
    (out, CardDims { half_w, h })
}
