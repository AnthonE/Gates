//! The rock formations (`sim_core::boulder`), drawn.
//!
//! Each block the sim collides against is drawn as one of the **rock
//! pieces** ([`PIECE_GLB`]): a sculpted outcrop from Meshy, the way the
//! reference game draws a handful of authored rock meshes everywhere rather
//! than shaping each one by hand. Only the piece's shape is read: it is
//! stretched into the block's box ([`pick_piece`] takes the piece the box
//! stretches least) and drawn in the ground's own splat material, so a
//! boulder is the same granite as the cliff it stands under
//! (`ground_splat.wgsl`'s rock face), darkened through the macro channel
//! (`UV_1.x`) in its cracks and at its foot, with moss in patches on what
//! faces up. Loose stones lie around every seated block's foot.
//!
//! Until the pieces load, or if they cannot, a block is drawn as a
//! **fractured rock** ([`rock_block`]): a box with rounded edges, its faces
//! pushed in and out in steps so they break into planes and its sides
//! grooved along strata. The cliffs' crags and the rock landmarks still are.
//!
//! The mesh stands inside the collision box, never out of it: its rounding,
//! taper, chisel and fracture all cut in (until 2026-10-17 a facet stood up
//! to half a fracture proud of the box, rock a body walked into). Under a
//! metre in, so a body held `occupy::ROCK_SKIN_M` off the box is not
//! stopped in the air.
//!
//! One entity per formation, built in world space, streamed by formation cell
//! out to [`ROCK_RING`] cells — rocks are landmarks, so they are drawn far
//! beyond the prop rings. [`rock_block`] is public because the landmark kits
//! draw their rock parts with it (`landmarks.rs`).
//!
//! The Verdigris' spires (`boulder::BLOCK_SPIRE`) are the one block kind not
//! built into the formation's mesh: each is a generated model of black glass
//! with its own lit veins ([`SPIRE_GLB`]), stood in its block's box and hung
//! off the formation's entity, so it streams and despawns with the rest.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use sim_core::boulder::{self, BLOCK_SPIRE, BOULDER_CELL_M, ON_GROUND};
use sim_core::terrain;

use super::ground_splat::{ATTRIBUTE_MARKINGS, ATTRIBUTE_ROAD, ATTRIBUTE_ZONE};
use super::terrain_mesh::{Ring, UV_PER_M};
use super::weather::WeatherNow;
use super::{Eye, WorldEntity, WorldId};

/// Formation cells drawn either side of the eye's: ~900 m.
pub const ROCK_RING: i32 = 14;
/// Formations built per frame. An empty cell is a few taps and costs nothing
/// against this; a present one is a few more and a mesh.
pub const BUILDS_PER_FRAME: usize = 2;
/// The ground material's value multiplier (`UV_1.x`) on a boulder, at its
/// top. Granite at full value is the ground's brightest identity, and a
/// lone rock of it in the sun read as a white lump — ore, not a boulder.
/// Weathered rock is darker than a fresh cliff face, and darker again at its
/// foot, where it meets the soil.
pub const ROCK_VALUE: f32 = 0.6;
/// The share of [`ROCK_VALUE`] a rock keeps at its foot.
pub const ROCK_FOOT_VALUE: f32 = 0.7;
/// Edge rounding, as a share of a rock's smallest half-extent — one radius in
/// metres on every edge, so a tall piece is not rounded a metre and a half
/// down its top and a stack of them does not open gaps between its pieces.
pub const ROCK_ROUND: f32 = 0.32;
/// The most any edge is rounded, metres.
pub const ROCK_ROUND_MAX_M: f32 = 0.9;
/// How much narrower a rock's top is than its foot, as a share of its
/// half-extent, at most — weathering takes the top first.
pub const ROCK_TAPER: f32 = 0.12;
/// The most a taper may take from a side, metres: the collision box does not
/// taper, and a body should not stand a metre past the drawn edge.
pub const ROCK_TAPER_MAX_M: f32 = 0.6;
/// Most a face is pushed in, metres. Under a metre, for the reason in the
/// module note.
pub const ROCK_FRACTURE_MAX_M: f32 = 0.9;
/// How far a rock resting on another is drawn down into it, metres.
pub const STACK_SINK_M: f32 = 0.6;
/// Horizontal size of one face segment, metres.
const SEG_M: f32 = 1.6;
/// Vertical size of one side segment, metres — finer, so the strata show.
const SEG_V_M: f32 = 0.9;
/// Segments across each rounded edge, per face: one a face draws an edge as
/// a bevel of two facets. The round was cut by the face grid alone until
/// 2026-10-17, and a segment is wider than the round, so every edge was
/// drawn nearly sharp (operator, of a cliff ledge: "looks bad with its sharp
/// edges"). Two a face drew it rounder for 70% more triangles, on rocks
/// drawn out to [`ROCK_RING`].
const ROUND_SEGS: usize = 1;

/// The rock pieces, each standing on its base at the origin, centred in X/Z,
/// at its own proportions (`assets/models/MANIFEST.md`). Geometry only.
pub const PIECE_GLB: [&str; PIECES] = [
    "models/prop/rock/block_a.glb",
    "models/prop/rock/block_b.glb",
    "models/prop/rock/block_c.glb",
    "models/prop/rock/block_d.glb",
    "models/prop/rock/block_e.glb",
    "models/prop/rock/block_f.glb",
];
/// The same pieces remeshed low, drawn past [`PIECE_NEAR_CELLS`]: ~950
/// blocks stand in the ring, and the full pieces everywhere would be most of
/// the frame's triangles.
pub const PIECE_FAR_GLB: [&str; PIECES] = [
    "models/prop/rock/block_a_far.glb",
    "models/prop/rock/block_b_far.glb",
    "models/prop/rock/block_c_far.glb",
    "models/prop/rock/block_d_far.glb",
    "models/prop/rock/block_e_far.glb",
    "models/prop/rock/block_f_far.glb",
];
pub const PIECES: usize = 6;
/// Formation cells either side of the eye's drawn with the full pieces
/// (~220 m); a cell crossing it is rebuilt with the other set.
pub const PIECE_NEAR_CELLS: i32 = 3;
/// How much a piece's hash may outweigh a better fit, in log-stretch: two
/// neighbours of the same shape do not always take the same piece.
const PIECE_SHUFFLE: f32 = 0.35;
/// The value multiplier's swing between a crack and a ridge, either way.
const PIECE_HOLLOW_DARK: f32 = 0.3;
/// Two faces of a piece meeting at a vertex are shaded as one surface while
/// their normals are within this cosine (~32°); past it the fracture shows
/// as an edge. Smoothed across every angle the pieces read as dough.
const PIECE_CREASE: f32 = 0.85;

/// The Verdigris' spires: black glass with green veins
/// (`assets/models/MANIFEST.md`), each fitted to a unit box standing on its
/// base at the origin, so a block's box is its scale. Two tall shards and two
/// stout ones; [`spire_model`] picks.
pub const SPIRE_GLB: [&str; 4] = [
    "models/prop/fel/spire_a.glb",
    "models/prop/fel/spire_b.glb",
    "models/prop/fel/spire_c.glb",
    "models/prop/fel/spire_d.glb",
];
/// How bright a spire's veins burn, linear multiplier on its emissive map
/// (the veins' own albedo, `ci/fel_veins.py`): the fel beasts' cracks by
/// night, a third of that under the sun. **(knob)**
pub const SPIRE_GLOW_NIGHT: f32 = super::beasts::FEL_GLOW;
pub const SPIRE_GLOW_DAY: f32 = 1.0;
/// How far a spire's base is drawn under the lowest ground at its corners,
/// metres, so no slope shows it hanging.
pub const SPIRE_SINK_M: f32 = 0.3;

#[derive(Resource, Default)]
pub struct RockRing {
    seed: Option<u64>,
    /// Each cell's formation entity, and whether it was built near.
    built: HashMap<(i32, i32), (Option<Entity>, bool)>,
    spires: Option<SpireModels>,
    pieces: Option<PieceModels>,
}

/// The rock pieces, near and far, loaded on first use and kept across worlds.
struct PieceModels {
    near: [Handle<Mesh>; PIECES],
    far: [Handle<Mesh>; PIECES],
    /// Read off the loaded meshes once every one has landed.
    ready: Option<[Vec<Piece>; 2]>,
    /// A file failed: the procedural rock stands in for good.
    failed: bool,
}

impl PieceModels {
    fn load(server: &AssetServer) -> Self {
        let mesh = |path: &'static str| {
            server.load(
                GltfAssetLabel::Primitive {
                    mesh: 0,
                    primitive: 0,
                }
                .from_asset(path),
            )
        };
        PieceModels {
            near: PIECE_GLB.map(mesh),
            far: PIECE_FAR_GLB.map(mesh),
            ready: None,
            failed: false,
        }
    }

    /// The pieces, near then far: `Some(None)` to draw the procedural rock,
    /// `None` while a file is still loading.
    fn get(
        &mut self,
        server: &AssetServer,
        meshes: &Assets<Mesh>,
    ) -> Option<Option<&[Vec<Piece>; 2]>> {
        if self.ready.is_none() && !self.failed {
            let all = self.near.iter().chain(&self.far);
            if all.clone().any(|h| {
                matches!(
                    server.get_load_state(h),
                    Some(bevy::asset::LoadState::Failed(_))
                )
            }) {
                warn!("rock pieces failed to load: drawing the procedural rock");
                self.failed = true;
            } else if all.clone().all(|h| meshes.contains(h)) {
                let read = |hs: &[Handle<Mesh>; PIECES]| -> Option<Vec<Piece>> {
                    hs.iter()
                        .map(|h| meshes.get(h).and_then(Piece::read))
                        .collect()
                };
                match (read(&self.near), read(&self.far)) {
                    (Some(n), Some(mut f)) => {
                        // Picked by the near piece's proportions, so a rock
                        // keeps its piece as it crosses the near ring.
                        for (f, n) in f.iter_mut().zip(&n) {
                            f.size = n.size;
                        }
                        self.ready = Some([n, f]);
                    }
                    _ => {
                        warn!("a rock piece has no positions: drawing the procedural rock");
                        self.failed = true;
                    }
                }
            } else {
                return None;
            }
        }
        Some(self.ready.as_ref())
    }
}

/// The spire models' one primitive and one material each, loaded on first
/// use and kept across worlds.
struct SpireModels {
    mesh: [Handle<Mesh>; 4],
    mat: [Handle<StandardMaterial>; 4],
}

impl SpireModels {
    fn load(server: &AssetServer) -> Self {
        SpireModels {
            mesh: SPIRE_GLB.map(|path| {
                server.load(
                    GltfAssetLabel::Primitive {
                        mesh: 0,
                        primitive: 0,
                    }
                    .from_asset(path),
                )
            }),
            mat: SPIRE_GLB.map(|path| {
                server.load(
                    GltfAssetLabel::Material {
                        index: 0,
                        is_scale_inverted: false,
                    }
                    .from_asset(path),
                )
            }),
        }
    }
}

impl RockRing {
    pub fn len(&self) -> usize {
        self.built.len()
    }
    pub fn is_empty(&self) -> bool {
        self.built.is_empty()
    }
}

/// Keep the formations within [`ROCK_RING`] cells of the eye built, nearest
/// first, and drop the ones that fall out of it.
#[allow(clippy::too_many_arguments)]
pub fn stream(
    mut commands: Commands,
    mut rocks: ResMut<RockRing>,
    mut meshes: ResMut<Assets<Mesh>>,
    ground: Res<Ring>,
    server: Res<AssetServer>,
    world: Res<WorldId>,
    eye: Res<Eye>,
    blight: Option<Res<super::blight::Blight>>,
) {
    let live = blight.as_deref().and_then(|b| b.now);
    let Some(material) = ground.ground_material() else {
        return;
    };
    let rocks = &mut *rocks;
    let spires = rocks
        .spires
        .get_or_insert_with(|| SpireModels::load(&server));
    // Wait for the pieces rather than draw every rock twice.
    let Some(pieces) = rocks
        .pieces
        .get_or_insert_with(|| PieceModels::load(&server))
        .get(&server, &meshes)
    else {
        return;
    };
    if rocks.seed != Some(world.seed) {
        for (_, (e, _)) in rocks.built.drain() {
            if let Some(e) = e {
                commands.entity(e).despawn();
            }
        }
        rocks.seed = Some(world.seed);
    }
    let cx = (eye.pos.x / BOULDER_CELL_M).floor() as i32;
    let cz = (eye.pos.z / BOULDER_CELL_M).floor() as i32;
    rocks.built.retain(|&(x, z), (e, _)| {
        let keep = (x - cx).abs() <= ROCK_RING + 1 && (z - cz).abs() <= ROCK_RING + 1;
        if !keep {
            if let Some(e) = e {
                commands.entity(*e).despawn();
            }
        }
        keep
    });
    let mut budget = BUILDS_PER_FRAME * super::stream_boost();
    // Rings outward, so the nearest rocks land first.
    for r in 0..=ROCK_RING {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let key = (cx + dx, cz + dz);
                let near = pieces.is_some() && r <= PIECE_NEAR_CELLS;
                match rocks.built.get(&key).copied() {
                    Some((_, n)) if n == near => continue,
                    Some((None, _)) => {
                        rocks.built.insert(key, (None, near));
                        continue;
                    }
                    // Crossed the near ring: rebuilt with the other pieces,
                    // in the same frame as the old one goes.
                    Some((Some(e), _)) => commands.entity(e).despawn(),
                    None => {}
                }
                let f = boulder::formation(world.seed, &world.haven, key.0, key.1);
                if f.n == 0 {
                    rocks.built.insert(key, (None, near));
                    continue;
                }
                // Visible itself, mesh or not: a formation of nothing but
                // spires is a parent the spires inherit their visibility from.
                let mut cell =
                    commands.spawn((WorldEntity, Transform::IDENTITY, Visibility::default()));
                let set = pieces.map(|p| p[usize::from(!near)].as_slice());
                if let Some(mesh) = formation_mesh(world.seed, &world.haven, &f, live.as_ref(), set)
                {
                    cell.insert((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material.clone())));
                }
                for b in f.iter().filter(|b| b.kind == BLOCK_SPIRE) {
                    let k = spire_model(b.shape);
                    cell.with_child((
                        Mesh3d(spires.mesh[k].clone()),
                        MeshMaterial3d(spires.mat[k].clone()),
                        spire_transform(world.seed, &world.haven, b),
                    ));
                }
                rocks.built.insert(key, (Some(cell.id()), near));
                budget -= 1;
                if budget == 0 {
                    return;
                }
            }
        }
    }
}

/// A new world: forget the formations (their entities are `WorldEntity`).
/// The spire models stay loaded.
pub fn teardown(mut rocks: ResMut<RockRing>) {
    rocks.seed = None;
    rocks.built.clear();
}

/// Which of [`SPIRE_GLB`] a spire wears. Its `shape` byte set its height
/// (`boulder::strays`), so the upper half of the byte stands the tall shards
/// (a, c) and the lower half the stout ones (b, d); the low bit picks
/// between the two.
pub fn spire_model(shape: u8) -> usize {
    match (shape >= 128, shape & 1) {
        (true, 0) => 0,
        (true, _) => 2,
        (false, 0) => 1,
        (false, _) => 3,
    }
}

/// A spire's model stood in its block's box: its base just under the lowest
/// ground at its corners, its top at the block's.
fn spire_transform(seed: u64, haven: &terrain::Haven, b: &boulder::Block) -> Transform {
    let (s, c) = sim_core::yaw_dir(b.yaw);
    let mut yaw = s.atan2(c);
    let (mut wx, mut wz) = (2.0 * b.hx, 2.0 * b.hz);
    // Every model is wider across its own x than its z, so a block longer
    // in z turns it a quarter so the long sides agree.
    if b.hz > b.hx {
        yaw += std::f32::consts::FRAC_PI_2;
        std::mem::swap(&mut wx, &mut wz);
    }
    let mut lo = terrain::ground(seed, haven, b.x, b.z);
    for (lx, lz) in [(b.hx, b.hz), (b.hx, -b.hz), (-b.hx, b.hz), (-b.hx, -b.hz)] {
        let (x, z) = b.to_world(lx, lz);
        lo = lo.min(terrain::ground(seed, haven, x, z));
    }
    let base = lo - SPIRE_SINK_M;
    Transform {
        translation: Vec3::new(b.x, base, b.z),
        rotation: Quat::from_rotation_y(yaw),
        scale: Vec3::new(wx, (b.y1 - base).max(1.0), wz),
    }
}

/// Burn the spires' veins by the hour: brighter in the dark, in steps of
/// 1/32 of the day, since a write re-uploads the material.
pub fn spire_glow(
    weather: Res<WeatherNow>,
    rocks: Res<RockRing>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(spires) = rocks.spires.as_ref() else {
        return;
    };
    let day = (weather.sun_lux.clamp(0.0, 1.0) * 32.0).round() / 32.0;
    let k = SPIRE_GLOW_NIGHT + (SPIRE_GLOW_DAY - SPIRE_GLOW_NIGHT) * day;
    let want = LinearRgba::rgb(k, k, k);
    for h in &spires.mat {
        if materials.get(h).is_some_and(|m| m.emissive != want) {
            if let Some(m) = materials.get_mut(h) {
                m.emissive = want;
            }
        }
    }
}

/// Integer hash to [0, 1).
fn h01(a: u32, b: u32) -> f32 {
    let mut x = a.wrapping_mul(0x9E37_79B9) ^ b.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    (x >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// Smooth value noise in [0, 1].
fn noise3(p: Vec3, seed: u32) -> f32 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (Vec3::splat(3.0) - 2.0 * f);
    let c = |dx: i32, dy: i32, dz: i32| {
        let k = ((i.x as i32 + dx) as u32).wrapping_mul(73_856_093)
            ^ ((i.y as i32 + dy) as u32).wrapping_mul(19_349_663)
            ^ ((i.z as i32 + dz) as u32).wrapping_mul(83_492_791);
        h01(k, seed)
    };
    let x00 = c(0, 0, 0) + (c(1, 0, 0) - c(0, 0, 0)) * u.x;
    let x10 = c(0, 1, 0) + (c(1, 1, 0) - c(0, 1, 0)) * u.x;
    let x01 = c(0, 0, 1) + (c(1, 0, 1) - c(0, 0, 1)) * u.x;
    let x11 = c(0, 1, 1) + (c(1, 1, 1) - c(0, 1, 1)) * u.x;
    let y0 = x00 + (x10 - x00) * u.y;
    let y1 = x01 + (x11 - x01) * u.y;
    y0 + (y1 - y0) * u.z
}

/// A rock mesh being built in world space, with the attributes the ground
/// material reads: flat-shaded triangles ([`RockSoup::tri`]) or shared,
/// smooth-shaded vertices ([`RockSoup::vertex`], [`RockSoup::face`]).
#[derive(Default)]
pub struct RockSoup {
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    /// Splat weights: rock, with moss (grass and litter) where it grows.
    col: Vec<[f32; 4]>,
    /// `UV_1`: the value multiplier, and dry.
    val: Vec<[f32; 2]>,
    idx: Vec<u32>,
}

impl RockSoup {
    pub fn is_empty(&self) -> bool {
        self.idx.is_empty()
    }

    /// One flat-shaded triangle, its front the side its winding faces.
    fn tri(&mut self, p: [Vec3; 3], moss: [f32; 3], value: [f32; 3]) {
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        let v = [0, 1, 2].map(|k| self.vertex(p[k], n, moss[k], value[k]));
        self.face(v[0], v[1], v[2]);
    }

    /// A vertex, for [`RockSoup::face`] to share.
    pub fn vertex(&mut self, p: Vec3, n: Vec3, moss: f32, value: f32) -> u32 {
        self.pos.push(p.to_array());
        self.nrm.push(n.to_array());
        self.col.push([0.0, moss * 0.55, moss * 0.45, 1.0 - moss]);
        self.val.push([value, 0.0]);
        (self.pos.len() - 1) as u32
    }

    /// A triangle over three vertices, its front the side its winding faces.
    pub fn face(&mut self, a: u32, b: u32, c: u32) {
        self.idx.extend_from_slice(&[a, b, c]);
    }

    /// The soup as a ground-material mesh. `seed` places it in the island's
    /// zones: a rock in the Verdigris goes to obsidian with lit cracks the way
    /// the ground under it does (`ATTRIBUTE_ZONE`; the desert does not tint
    /// rock, so its share stays 0).
    pub fn mesh(self, seed: u64, haven: &terrain::Haven) -> Mesh {
        let n = self.pos.len();
        // The heart's edge noise, as the ground carries it: the shader
        // rebuilds the live spread from it, so the rock in the green and the
        // ground under it are the same green. And how far the vertex stands
        // above the ground, as a NEGATIVE desert share: the spread's coast
        // and ridge read the ground's height, never the rock's, or a beach
        // boulder would go green over clean sand (the shader clamps the
        // share at 0, so the desert never sees it).
        let mut lat = terrain::Lattice::new();
        let zone: Vec<[f32; 2]> = self
            .pos
            .iter()
            .map(|p| {
                let ground = terrain::ground_memo(&mut lat, seed, haven, p[0], p[2]);
                let rise = (p[1] - ground).max(0.0);
                [-rise, terrain::fel_wander_memo(&mut lat, seed, p[0], p[2])]
            })
            .collect();
        let uv: Vec<[f32; 2]> = self
            .pos
            .iter()
            .map(|p| [p[0] * UV_PER_M, p[2] * UV_PER_M])
            .collect();
        // The planar tangent the ground's UV implies: world +X, projected onto
        // each face.
        let tan: Vec<[f32; 4]> = self
            .nrm
            .iter()
            .map(|n| {
                let n = Vec3::from_array(*n);
                let mut t = Vec3::X - n * n.x;
                if t.length_squared() < 1e-4 {
                    t = Vec3::Z - n * n.z;
                }
                let t = t.normalize();
                [t.x, t.y, t.z, 1.0]
            })
            .collect();
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, self.val)
        .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, tan)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.col)
        .with_inserted_attribute(ATTRIBUTE_ROAD, vec![[0.0f32; 2]; n])
        .with_inserted_attribute(ATTRIBUTE_MARKINGS, vec![[0.0f32; 4]; n])
        .with_inserted_attribute(ATTRIBUTE_ZONE, zone)
        .with_inserted_indices(Indices::U32(self.idx))
    }
}

/// One rock to draw: an oriented box with a planar top, in world space.
#[derive(Clone, Copy, Debug)]
pub struct RockShape {
    pub x: f32,
    pub z: f32,
    pub hx: f32,
    pub hz: f32,
    /// The block's local +Z in world space, `(sin, cos)` of its bearing.
    pub dir: (f32, f32),
    pub y0: f32,
    pub y1: f32,
    pub tx: f32,
    pub tz: f32,
    /// Variation key.
    pub key: u32,
    /// How much moss grows on what faces up: `0` none, `1` patches, more
    /// for a top grassed nearly over.
    pub moss: f32,
    /// How deep the planes that knock off its edges and corners cut,
    /// metres: `0` leaves the rounded box.
    pub chisel: f32,
    /// Coarsest the faces may be: 1 draws a chunk, not a block.
    pub min_seg: usize,
    /// The ground material's value multiplier at the rock's top
    /// ([`ROCK_VALUE`] for a weathered boulder).
    pub value: f32,
    /// Close the bottom too. A seated rock's is buried and skipped, but a
    /// cap or slab standing on another overhangs it, and from under it a
    /// missing bottom is sky through the rock.
    pub underside: bool,
}

impl RockShape {
    /// A sim block.
    pub fn of(seed: u64, b: &boulder::Block, moss: f32) -> Self {
        RockShape {
            x: b.x,
            z: b.z,
            hx: b.hx,
            hz: b.hz,
            dir: sim_core::yaw_dir(b.yaw),
            y0: b.y0,
            y1: b.y1,
            tx: b.tx,
            tz: b.tz,
            key: (seed as u32) ^ ((b.shape as u32) << 8) ^ (b.yaw as u32).wrapping_mul(0x2545_F491),
            moss,
            chisel: 0.0,
            min_seg: 2,
            value: ROCK_VALUE,
            underside: b.on != ON_GROUND,
        }
    }

    fn world(&self, lx: f32, lz: f32) -> (f32, f32) {
        let (s, c) = self.dir;
        (self.x + lx * c + lz * s, self.z - lx * s + lz * c)
    }

    fn world_dir(&self, n: Vec3) -> Vec3 {
        let (s, c) = self.dir;
        Vec3::new(n.x * c + n.z * s, n.y, -n.x * s + n.z * c)
    }
}

/// Draw one rock into `soup`.
pub fn rock_block(soup: &mut RockSoup, r: &RockShape) {
    let hgt = (r.y1 - r.y0).max(0.3);
    // One rounding radius in metres on every axis, as a share of each axis.
    let half_h = hgt * 0.5;
    let rw = (ROCK_ROUND * r.hx.min(r.hz).min(half_h)).min(ROCK_ROUND_MAX_M);
    // Face segments over the flat between the rounds (`lines` below).
    let seg = |m: f32, per: f32| ((m / per).ceil() as usize).clamp(r.min_seg, 8);
    let (nx, ny, nz) = if r.min_seg <= 1 {
        (1, 1, 1)
    } else {
        let under = if r.underside { 2.0 } else { 1.0 };
        (
            seg(2.0 * (r.hx - rw), SEG_M),
            seg(hgt - under * rw, SEG_V_M),
            seg(2.0 * (r.hz - rw), SEG_M),
        )
    };
    let amp = (0.16 * r.hx.min(r.hz).min(hgt * 0.5)).min(ROCK_FRACTURE_MAX_M);
    // A few metres a stratum, per rock.
    let layer = 0.9 + 1.4 * h01(r.key, 3);
    let chunk = r.min_seg <= 1;
    let rho = Vec3::new(rw / r.hx, rw / half_h, rw / r.hz).min(Vec3::splat(0.95));
    let taper =
        (ROCK_TAPER * (0.4 + 0.6 * h01(r.key, 9))).min(ROCK_TAPER_MAX_M / r.hx.min(r.hz).max(0.1));
    let cuts = chisel_planes(r, half_h, chunk);
    // Where a point of the unit box lands, how far it was pushed (for the
    // shading), and its outward direction.
    let place = |u: Vec3| -> (Vec3, f32, Vec3) {
        let inner = u.clamp(-(Vec3::ONE - rho), Vec3::ONE - rho);
        // The offset from the core in rounding radii: a unit sphere's
        // direction, which is also the outward normal, the radius being the
        // same in metres on every axis.
        let n = ((u - inner) / rho).normalize_or_zero();
        let p = inner + n * rho;
        let shrink = 1.0 - taper * (p.y + 1.0) * 0.5;
        // Local metres about the box's middle, then knocked back onto every
        // chisel plane it stands past: a convex rock cut by planes, its
        // edges and corners broken off as flat facets.
        let mut l = Vec3::new(p.x * r.hx * shrink, p.y * half_h, p.z * r.hz * shrink);
        for _ in 0..CHISEL_PASSES {
            for (cn, cd) in cuts {
                let over = cn.dot(l) - cd;
                if over > 0.0 {
                    l -= cn * over;
                }
            }
        }
        let t = (l.y / half_h + 1.0) * 0.5;
        let (lx, lz) = (l.x, l.z);
        let top = r.y1 + r.tx * lx + r.tz * lz;
        let (wx, wz) = r.world(lx, lz);
        let w = Vec3::new(wx, r.y0 + t * (top - r.y0), wz);
        let wn = r.world_dir(n);
        if chunk {
            // A loose stone: every corner jittered, nothing else.
            let j = Vec3::new(
                h01(r.key, (u.x * 3.0 + u.y * 7.0 + u.z * 13.0 + 40.0) as u32) - 0.5,
                h01(
                    r.key ^ 0x51,
                    (u.x * 5.0 + u.y * 11.0 + u.z * 3.0 + 40.0) as u32,
                ) - 0.5,
                h01(
                    r.key ^ 0xA3,
                    (u.x * 7.0 + u.y * 2.0 + u.z * 17.0 + 40.0) as u32,
                ) - 0.5,
            );
            return (w + j * r.hx.min(r.hz) * 0.5, 0.0, wn);
        }
        // Fracture planes: noise in steps, so a face breaks into flats with
        // sharp edges between them rather than rolling.
        let q = w * (1.0 / 2.6);
        let f = noise3(q, r.key) * 0.6 + noise3(q * 2.3, r.key ^ 0x5bd1) * 0.4;
        let stepped = ((f * 4.0).floor() / 3.0).clamp(0.0, 1.0);
        // A top is ground: pushed about less.
        let flat = 1.0 - 0.85 * n.y.max(0.0);
        // How proud the facet is, for the shading, about 0.
        let mut proud = (stepped - 0.5) * flat;
        // Strata: a groove every `layer` metres up the sides, wandering.
        let side = 1.0 - n.y.abs();
        let band = (w.y - r.y0) / layer + noise3(q * 0.7, r.key ^ 0x77) * 0.6;
        if band - band.floor() < 0.2 {
            proud -= 0.9 * side;
        }
        // Every facet pushed in, never out: the box is what the sim
        // collides, so a facet proud of it is rock a body walks into.
        let push = (proud - 0.5 * flat) * amp;
        (w + wn * push, proud, wn)
    };
    // Each axis' grid lines across the unit box: its `n` face segments over
    // the flat middle and `ROUND_SEGS` more over the round at either end, so
    // the round is drawn, and how many segments the low round took. A
    // buried bottom's round is never seen and keeps one segment.
    let lines = |n: usize, rho: f32, low: bool| -> (Vec<f32>, usize) {
        if chunk {
            return (
                (0..=n).map(|i| -1.0 + 2.0 * i as f32 / n as f32).collect(),
                0,
            );
        }
        let flat = 1.0 - rho;
        let round = |k: usize| rho * k as f32 / ROUND_SEGS as f32;
        let mut v = Vec::with_capacity(n + 1 + 2 * ROUND_SEGS);
        let lo = if low { ROUND_SEGS } else { 0 };
        v.extend((0..lo).map(|k| -1.0 + round(k)));
        let from = if low { -flat } else { -1.0 };
        v.extend((0..=n).map(|i| from + (flat - from) * i as f32 / n as f32));
        v.extend((1..ROUND_SEGS).map(|k| flat + round(k)));
        // The far end exactly 1, where the face that meets this one along it
        // stands (`u[axis] = sg`): `flat + rho` is not 1.0 in floats, and a
        // shared edge placed from two different `u` is a crack.
        v.push(1.0);
        (v, lo)
    };
    let axes = [
        lines(nx, rho.x, true),
        lines(ny, rho.y, r.underside),
        lines(nz, rho.z, true),
    ];
    // A segment's face segment: a round shades as the face it rounds off.
    let flat_n = [nx, ny, nz];
    let coarse = |a: usize, i: usize| i.saturating_sub(axes[a].1).min(flat_n[a] - 1);
    // The six faces as (normal axis, sign), each gridded along the other two
    // axes on the same lines as its neighbours, so the edges they share
    // are the same points and the rock is closed. The bottom is buried and
    // skipped unless the rock stands on another (`underside`).
    let counts = [
        axes[0].0.len() - 1,
        axes[1].0.len() - 1,
        axes[2].0.len() - 1,
    ];
    let faces: [(usize, f32); 6] = [
        (1, 1.0),
        (0, 1.0),
        (0, -1.0),
        (2, 1.0),
        (2, -1.0),
        (1, -1.0),
    ];
    for (axis, sg) in faces {
        if axis == 1 && sg < 0.0 && !r.underside {
            continue;
        }
        let (a, b) = match axis {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        let (na, nb) = (counts[a], counts[b]);
        let at = |i: usize, j: usize| {
            let mut u = Vec3::ZERO;
            u[axis] = sg;
            u[a] = axes[a].0[i];
            u[b] = axes[b].0[j];
            u
        };
        let mut grid = Vec::with_capacity((na + 1) * (nb + 1));
        for i in 0..=na {
            for j in 0..=nb {
                grid.push(place(at(i, j)));
            }
        }
        let g = |i: usize, j: usize| grid[i * (nb + 1) + j];
        // Which way the grid winds a quad, `a` then `b`, against the face's
        // outward `sg`: `a × b` is +X, −Y and +Z for the three axes. Every
        // triangle winds the way its quad does, never by its own normal: a
        // facet on a rounded corner pushed in past square to its face's
        // outward was flipped to face it, into the rock, and culled — a
        // see-through sliver where the rock wraps round a corner (operator,
        // 2026-10-20). Placing is orientation-preserving, so this is out.
        let inward = (if axis == 1 { -sg } else { sg }) < 0.0;
        for i in 0..na {
            for j in 0..nb {
                let q = [g(i, j), g(i + 1, j), g(i + 1, j + 1), g(i, j + 1)];
                // Alternate the diagonal by hash, so no grid shows.
                let flip = h01(r.key ^ (axis as u32 * 977), (i * 31 + j) as u32) < 0.5;
                let tris = if flip {
                    [[0, 1, 2], [0, 2, 3]]
                } else {
                    [[0, 1, 3], [1, 2, 3]]
                };
                for t in tris {
                    let mut p = [q[t[0]].0, q[t[1]].0, q[t[2]].0];
                    let mut d = [q[t[0]].1, q[t[1]].1, q[t[2]].1];
                    if inward {
                        p.swap(1, 2);
                        d.swap(1, 2);
                    }
                    let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
                    let moss = if r.moss > 0.0 {
                        // A lusher top holds turf on steeper facets too.
                        let lush = (r.moss - 1.0).max(0.0);
                        let lo = 0.6 - 0.15 * lush.min(1.0);
                        let m = ((n.y - lo) / 0.3).clamp(0.0, 1.0);
                        let c = (p[0] + p[1] + p[2]) / 3.0;
                        let patch = (noise3(c * 0.22, r.key ^ 0x3c) * 1.8 - 0.5 + 0.9 * lush)
                            .clamp(0.0, 1.0);
                        m * m * (3.0 - 2.0 * m) * patch * 0.85 * r.moss.min(1.0)
                    } else {
                        0.0
                    };
                    let (ci, cj) = (coarse(a, i), coarse(b, j));
                    let face = 0.8 + 0.4 * h01(r.key ^ 0x9e, (ci * 131 + cj * 17 + axis) as u32);
                    let value = |k: usize| {
                        let up = ((p[k].y - r.y0) / hgt).clamp(0.0, 1.0);
                        // Recesses darker: the cheap half of occlusion.
                        let recess = 1.0 + d[k] * 0.3;
                        r.value * (ROCK_FOOT_VALUE + (1.0 - ROCK_FOOT_VALUE) * up) * face * recess
                    };
                    soup.tri(p, [moss; 3], [value(0), value(1), value(2)]);
                }
            }
        }
    }
}

/// Chisel planes on one rock: at most this many.
const CHISEL_CUTS: usize = 6;
/// Rounds of knocking a point back onto the planes; two planes meeting at an
/// edge settle in a couple.
const CHISEL_PASSES: usize = 3;

/// The planes that break a rock's edges, as `(normal, offset)` in its local
/// metres about the box's middle (`+Y` up, `+Z` its `dir`): each faces up or
/// out toward an edge or corner and stands `chisel` metres or less inside
/// the box's own support there. An unused plane never cuts.
fn chisel_planes(r: &RockShape, half_h: f32, chunk: bool) -> [(Vec3, f32); CHISEL_CUTS] {
    let mut cuts = [(Vec3::Y, f32::INFINITY); CHISEL_CUTS];
    if chunk || r.chisel <= 0.0 {
        return cuts;
    }
    let n = 3 + (h01(r.key, 0xc0) * (CHISEL_CUTS - 2) as f32) as usize;
    for (k, cut) in cuts.iter_mut().take(n.min(CHISEL_CUTS)).enumerate() {
        let h = |c: u32| h01(r.key ^ 0xc415_e100, k as u32 * 8 + c);
        let sx = if h(1) < 0.5 { -1.0 } else { 1.0 };
        let sz = if h(2) < 0.5 { -1.0 } else { 1.0 };
        // An edge (one horizontal axis near zero) or a corner, leaning up
        // anywhere from level to steep.
        let (ax, az) = match (h(3) * 3.0) as u32 {
            0 => (0.15 * h(4), 1.0),
            1 => (1.0, 0.15 * h(4)),
            _ => (0.4 + 0.6 * h(4), 0.4 + 0.6 * h(5)),
        };
        let cn = Vec3::new(sx * ax, 0.1 + 1.3 * h(6), sz * az).normalize();
        let support = cn.x.abs() * r.hx + cn.y.abs() * half_h + cn.z.abs() * r.hz;
        let depth = r.chisel * (0.35 + 0.65 * h(7));
        *cut = (cn, support - depth);
    }
    cuts
}

/// Loose stones around a seated block's foot: a handful to a dozen, no
/// higher than an ankle, lying on the ground just off its sides. Clutter, as
/// grass is: nothing collides with them, so none stands high enough to read
/// as rock a body walks through (they ran to knee height until 2026-10-17).
fn talus(soup: &mut RockSoup, seed: u64, r: &RockShape) {
    let n = ((r.hx + r.hz) * 1.1) as usize;
    for k in 0..n.clamp(3, 14) {
        let h = |c: u32| h01(r.key ^ 0x7a1u32.wrapping_mul(k as u32 + 1), c);
        // A point on the footprint's edge, then out from it.
        let t = h(1) * 4.0;
        let (lx, lz) = match t as u32 {
            0 => (r.hx, -r.hz + 2.0 * r.hz * t.fract()),
            1 => (-r.hx, -r.hz + 2.0 * r.hz * t.fract()),
            2 => (-r.hx + 2.0 * r.hx * t.fract(), r.hz),
            _ => (-r.hx + 2.0 * r.hx * t.fract(), -r.hz),
        };
        let out = 0.4 + 2.2 * h(2) * h(2);
        let (ox, oz) = (lx + lx.signum() * out * 0.7, lz + lz.signum() * out * 0.7);
        let (x, z) = r.world(ox, oz);
        let gy = terrain::height(seed, x, z);
        let size = 0.1 + 0.12 * h(3) * h(3);
        let (s, c) = (h(4) * std::f32::consts::TAU).sin_cos();
        rock_block(
            soup,
            &RockShape {
                x,
                z,
                hx: size,
                hz: size * (0.6 + 0.4 * h(5)),
                dir: (s, c),
                y0: gy - size * 0.7,
                y1: gy + size * (0.4 + 0.5 * h(6)),
                tx: (h(7) - 0.5) * 0.6,
                tz: (h(8) - 0.5) * 0.6,
                key: r.key ^ (k as u32 + 1).wrapping_mul(0x9E37_79B9),
                moss: 0.0,
                chisel: 0.0,
                min_seg: 1,
                value: ROCK_VALUE,
                underside: false,
            },
        );
    }
}

/// Every block of a formation, and the loose stones round it, as one mesh:
/// `None` when the formation is all spires, which are models of their own.
/// Each block is one of `pieces`, or the procedural rock without them.
pub fn formation_mesh(
    seed: u64,
    haven: &terrain::Haven,
    f: &boulder::Formation,
    now: Option<&sim_core::blight::Now>,
    pieces: Option<&[Piece]>,
) -> Option<Mesh> {
    let mut soup = RockSoup::default();
    let mut lat = terrain::Lattice::new();
    for b in f.iter().filter(|b| b.kind != BLOCK_SPIRE) {
        // Moss where the ground around is green: not on a beach, not above
        // the treeline, not where the spread stands now.
        let h0 = terrain::height(seed, b.x, b.z);
        let moss = h0 > terrain::BEACH_MAX_H + 1.5
            && h0 < terrain::TREELINE_H
            && super::blight::live_fel_memo(now, &mut lat, seed, b.x, b.z, h0) < 0.3;
        let mut r = RockShape::of(seed, b, if moss { 1.0 } else { 0.0 });
        if b.on != ON_GROUND {
            // Drawn reaching into the block under it, so the seam between
            // the two is a crease and never a gap.
            r.y0 -= STACK_SINK_M;
        }
        match pieces {
            Some(p) => piece_block(&mut soup, &r, p),
            None => rock_block(&mut soup, &r),
        }
        if b.on == ON_GROUND {
            talus(&mut soup, seed, &r);
        }
    }
    (!soup.is_empty()).then(|| soup.mesh(seed, haven))
}

/// One rock piece's geometry, read once off its loaded mesh.
#[derive(Clone, Debug)]
pub struct Piece {
    /// Positions in its own box: x and z −1..1 across the footprint, pulled
    /// in off the corners the sim's footprint cuts ([`boulder::CHAMFER`]);
    /// y 0..1 from its base to its top.
    pos: Vec<Vec3>,
    idx: Vec<u32>,
    /// Its own proportions, width, height and depth, at any scale.
    size: Vec3,
    /// How hollow the surface is at each vertex: + in a crack, − on a ridge,
    /// about ±1 at the most.
    hollow: Vec<f32>,
    /// The triangles round each vertex: vertex `v`'s are
    /// `ring[ring_at[v]..ring_at[v + 1]]`.
    ring: Vec<u32>,
    ring_at: Vec<u32>,
}

impl Piece {
    /// `None` for a mesh with no positions or no triangles.
    pub fn read(mesh: &Mesh) -> Option<Piece> {
        use bevy::mesh::VertexAttributeValues as V;
        let Some(V::Float32x3(raw)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
            return None;
        };
        let raw: Vec<Vec3> = raw.iter().map(|p| Vec3::from_array(*p)).collect();
        let idx: Vec<u32> = match mesh.indices() {
            Some(i) => i.iter().map(|k| k as u32).collect(),
            None => (0..raw.len() as u32).collect(),
        };
        if idx.len() < 3 {
            return None;
        }
        let (lo, hi) = raw.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| {
            (lo.min(*p), hi.max(*p))
        });
        let size = (hi - lo).max(Vec3::splat(1e-4));
        // A crack is a vertex its neighbours stand out in front of, measured
        // in the piece's own proportions: the mean of its neighbours off its
        // tangent plane, over the length of its edges.
        let mut nrm = vec![Vec3::ZERO; raw.len()];
        let mut mean = vec![Vec3::ZERO; raw.len()];
        let mut edge = vec![0.0f32; raw.len()];
        let mut deg = vec![0.0f32; raw.len()];
        for t in idx.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|k| k as usize);
            let n = (raw[b] - raw[a]).cross(raw[c] - raw[a]);
            for (v, o1, o2) in [(a, b, c), (b, c, a), (c, a, b)] {
                nrm[v] += n;
                mean[v] += raw[o1] + raw[o2];
                edge[v] += raw[o1].distance(raw[v]) + raw[o2].distance(raw[v]);
                deg[v] += 2.0;
            }
        }
        let hollow = (0..raw.len())
            .map(|v| {
                if deg[v] == 0.0 {
                    return 0.0;
                }
                let off = mean[v] / deg[v] - raw[v];
                let len = (edge[v] / deg[v]).max(1e-6);
                (off.dot(nrm[v].normalize_or_zero()) / len * 4.0).clamp(-1.0, 1.0)
            })
            .collect();
        let mid = (lo + hi) * 0.5;
        let mut pos: Vec<Vec3> = raw
            .iter()
            .map(|p| {
                Vec3::new(
                    (p.x - mid.x) / (0.5 * size.x),
                    (p.y - lo.y) / size.y,
                    (p.z - mid.z) / (0.5 * size.z),
                )
            })
            .collect();
        // Inside the sim's footprint, whose corners are cut.
        let corner = pos
            .iter()
            .map(|p| p.x.abs() + p.z.abs())
            .fold(0.0, f32::max);
        let k = ((2.0 - boulder::CHAMFER) / corner).min(1.0);
        for p in &mut pos {
            p.x *= k;
            p.z *= k;
        }
        let mut ring_at = vec![0u32; pos.len() + 1];
        for &v in &idx {
            ring_at[v as usize + 1] += 1;
        }
        for v in 0..pos.len() {
            ring_at[v + 1] += ring_at[v];
        }
        let mut ring = vec![0u32; idx.len()];
        let mut fill = ring_at.clone();
        for (k, &v) in idx.iter().enumerate() {
            ring[fill[v as usize] as usize] = (k / 3) as u32;
            fill[v as usize] += 1;
        }
        Some(Piece {
            pos,
            idx,
            size,
            hollow,
            ring,
            ring_at,
        })
    }
}

/// Which of `pieces` a rock wears, and whether turned a quarter: the one its
/// box stretches least, in plan first, with a hash so neighbours of the same
/// shape differ.
pub fn pick_piece(r: &RockShape, pieces: &[Piece]) -> (usize, bool) {
    let hgt = (r.y1 - r.y0).max(0.3);
    let mut best = (f32::INFINITY, 0, false);
    for (k, p) in pieces.iter().enumerate() {
        for quarter in [false, true] {
            let (pw, pd) = if quarter {
                (p.size.z, p.size.x)
            } else {
                (p.size.x, p.size.z)
            };
            let (sx, sz, sy) = (2.0 * r.hx / pw, 2.0 * r.hz / pd, hgt / p.size.y);
            let score = (sx / sz).ln().abs()
                + 0.5 * (sy / (sx * sz).sqrt()).ln().abs()
                + PIECE_SHUFFLE * h01(r.key ^ 0x9e3c, k as u32);
            if score < best.0 {
                best = (score, k, quarter);
            }
        }
    }
    (best.1, best.2)
}

/// Draw one rock into `soup` as a piece stretched into its box: its base at
/// `y0`, its top on the box's tilted top, turned as [`pick_piece`] says and
/// a half more by hash. Its normals are its own, smoothed over the shape it
/// was stretched into up to its creases.
pub fn piece_block(soup: &mut RockSoup, r: &RockShape, pieces: &[Piece]) {
    let (k, quarter) = pick_piece(r, pieces);
    let p = &pieces[k];
    let half = h01(r.key, 0x4a1f) < 0.5;
    let world: Vec<Vec3> = p
        .pos
        .iter()
        .map(|u| {
            // Turns only, never a mirror: a mirror would wind it inside out.
            let (mut x, mut z) = if quarter { (-u.z, u.x) } else { (u.x, u.z) };
            if half {
                (x, z) = (-x, -z);
            }
            let (lx, lz) = (x * r.hx, z * r.hz);
            let top = r.y1 + r.tx * lx + r.tz * lz;
            let (wx, wz) = r.world(lx, lz);
            Vec3::new(wx, r.y0 + u.y * (top - r.y0), wz)
        })
        .collect();
    // Each face's normal, area-weighted, in the shape it was stretched into.
    let face: Vec<Vec3> = p
        .idx
        .chunks_exact(3)
        .map(|t| {
            let [a, b, c] = [t[0], t[1], t[2]].map(|k| k as usize);
            (world[b] - world[a]).cross(world[c] - world[a])
        })
        .collect();
    // A vertex is drawn once per surface meeting at it ([`PIECE_CREASE`]):
    // the faces round it go to the first group they lie within the crease
    // of, and each corner of each face takes its group's vertex.
    let mut corner = vec![0u32; p.idx.len()];
    let mut groups: Vec<(Vec3, Vec3, u32)> = Vec::new();
    for (v, w) in world.iter().enumerate() {
        groups.clear();
        for &t in &p.ring[p.ring_at[v] as usize..p.ring_at[v + 1] as usize] {
            let f = face[t as usize];
            let fu = f.normalize_or_zero();
            match groups.iter_mut().find(|g| g.0.dot(fu) >= PIECE_CREASE) {
                Some(g) => g.1 += f,
                None => groups.push((fu, f, 0)),
            }
        }
        for g in groups.iter_mut() {
            g.2 = soup.pos.len() as u32;
            let n = g.1.normalize_or_zero();
            soup_vertex(soup, r, p, v, *w, n);
        }
        for &t in &p.ring[p.ring_at[v] as usize..p.ring_at[v + 1] as usize] {
            let fu = face[t as usize].normalize_or_zero();
            let g = groups
                .iter()
                .find(|g| g.0.dot(fu) >= PIECE_CREASE)
                .map_or(groups[0].2, |g| g.2);
            let t = t as usize;
            for k in 0..3 {
                if p.idx[3 * t + k] as usize == v {
                    corner[3 * t + k] = g;
                }
            }
        }
    }
    for t in corner.chunks_exact(3) {
        soup.face(t[0], t[1], t[2]);
    }
}

/// One drawn vertex of a piece: moss on what faces up, darker in its cracks
/// and at its foot.
fn soup_vertex(soup: &mut RockSoup, r: &RockShape, p: &Piece, v: usize, w: Vec3, n: Vec3) {
    let moss = if r.moss > 0.0 {
        // As `rock_block`'s: patches on what faces up.
        let m = ((n.y - 0.6) / 0.3).clamp(0.0, 1.0);
        let patch = (noise3(w * 0.22, r.key ^ 0x3c) * 1.8 - 0.5).clamp(0.0, 1.0);
        m * m * (3.0 - 2.0 * m) * patch * 0.85 * r.moss.min(1.0)
    } else {
        0.0
    };
    let up = ((w.y - r.y0) / (r.y1 - r.y0).max(0.3)).clamp(0.0, 1.0);
    // Darker in the cracks and lighter on the ridges, and a broad mottle so
    // no two faces of one rock are one flat grey.
    let mottle = 0.85 + 0.3 * noise3(w * (1.0 / 3.5), r.key ^ 0x9e);
    let value = r.value
        * (ROCK_FOOT_VALUE + (1.0 - ROCK_FOOT_VALUE) * up)
        * (1.0 - PIECE_HOLLOW_DARK * p.hollow[v])
        * mottle;
    soup.vertex(w, n, moss, value);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two triangles that share an edge walk it in opposite directions, or
    /// one of them faces into the rock and is culled: a see-through sliver
    /// where a face wraps round a corner (operator, 2026-10-20). Returns the
    /// edges walked twice the same way, and the edges with no reverse.
    fn winding(soup: &RockSoup) -> (usize, usize) {
        let key = |i: u32| soup.pos[i as usize].map(f32::to_bits);
        let mut edges = HashMap::<([u32; 3], [u32; 3]), u32>::default();
        for t in soup.idx.chunks(3) {
            for k in 0..3 {
                let (a, b) = (key(t[k]), key(t[(k + 1) % 3]));
                if a != b {
                    *edges.entry((a, b)).or_default() += 1;
                }
            }
        }
        let twice = edges.values().filter(|&&n| n > 1).count();
        let open = edges
            .keys()
            .filter(|(a, b)| !edges.contains_key(&(*b, *a)))
            .count();
        (twice, open)
    }

    #[test]
    fn every_rock_faces_out_and_a_closed_one_is_sealed() {
        let mut bad = 0;
        for k in 0..400u32 {
            let h = |c: u32| h01(k, c);
            let hgt = 0.6 + 9.0 * h(3);
            let (s, c) = (h(4) * std::f32::consts::TAU).sin_cos();
            let hx = 0.7 + 13.0 * h(1) * h(1);
            let hz = 0.7 + 13.0 * h(2) * h(2);
            let underside = k % 2 == 0;
            let r = RockShape {
                x: 40.0 * h(10),
                z: 40.0 * h(11),
                hx,
                hz,
                dir: (s, c),
                y0: 5.0,
                y1: 5.0 + hgt,
                tx: (h(5) - 0.5),
                tz: (h(6) - 0.5),
                key: k.wrapping_mul(0x9E37_79B9),
                moss: 1.0,
                chisel: if k % 3 == 0 {
                    (0.5 * hx.min(hz).min(hgt)).min(0.4)
                } else {
                    0.0
                },
                min_seg: if k % 7 == 0 { 1 } else { 2 },
                value: ROCK_VALUE,
                underside,
            };
            let mut soup = RockSoup::default();
            rock_block(&mut soup, &r);
            let (twice, open) = winding(&soup);
            if twice > 0 || (underside && open > 0) {
                bad += 1;
                eprintln!("rock {k}: {twice} edges walked twice the same way, {open} open; {r:?}");
            }
        }
        assert_eq!(
            bad, 0,
            "{bad} of 400 rocks have triangles facing in, or holes"
        );
    }

    /// A shipped piece as `ci/rock_piece.py` writes it: positions and u32
    /// indices, tightly packed.
    fn shipped(path: &str) -> Mesh {
        let raw = std::fs::read(format!("../../assets/{path}")).expect(path);
        let jl = u32::from_le_bytes(raw[12..16].try_into().unwrap()) as usize;
        let js: serde_json::Value = serde_json::from_slice(&raw[20..20 + jl]).unwrap();
        let bin = &raw[20 + jl + 8..];
        let view = |acc: usize| {
            let a = &js["accessors"][acc];
            let v = &js["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
            let at = v["byteOffset"].as_u64().unwrap_or(0) as usize;
            let n = v["byteLength"].as_u64().unwrap() as usize;
            bin[at..at + n]
                .chunks_exact(4)
                .map(|b| [b[0], b[1], b[2], b[3]])
                .collect::<Vec<_>>()
        };
        let prim = &js["meshes"][0]["primitives"][0];
        let pos: Vec<[f32; 3]> = view(prim["attributes"]["POSITION"].as_u64().unwrap() as usize)
            .chunks_exact(3)
            .map(|c| [0, 1, 2].map(|k| f32::from_le_bytes(c[k])))
            .collect();
        let idx = view(prim["indices"].as_u64().unwrap() as usize)
            .into_iter()
            .map(u32::from_le_bytes)
            .collect();
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_indices(Indices::U32(idx))
    }

    /// Every shipped piece, near and far, is closed and faces out once it is
    /// stretched into a block: a hole or a flipped triangle in a delivery is
    /// sky through the rock, which is what `ci/rock_piece.py` caps.
    #[test]
    fn every_shipped_piece_is_sealed() {
        for path in PIECE_GLB.iter().chain(&PIECE_FAR_GLB) {
            let piece = Piece::read(&shipped(path)).expect(path);
            let r = RockShape {
                x: 3.0,
                z: -4.0,
                hx: 4.0,
                hz: 3.0,
                dir: (0.6, 0.8),
                y0: 0.0,
                y1: 4.0,
                tx: 0.2,
                tz: -0.1,
                key: 11,
                moss: 1.0,
                chisel: 0.0,
                min_seg: 2,
                value: ROCK_VALUE,
                underside: true,
            };
            let mut soup = RockSoup::default();
            piece_block(&mut soup, &r, std::slice::from_ref(&piece));
            // A flipped triangle leaves its edges with no reverse too, so
            // this is both. (A pinch, where two sheets of a crack touch
            // along an edge, is walked twice each way, and closed.)
            let (_, open) = winding(&soup);
            assert_eq!(open, 0, "{path}: {open} edges with no reverse");
        }
    }

    /// A piece stretched into a block stands inside the box the sim
    /// collides — its corners cut, its top tilted — on every turn, so no
    /// drawn rock is rock a body walks into.
    #[test]
    fn a_piece_stands_inside_its_block() {
        // A piece filling its own box, past the cut corners and off-centre,
        // as a delivery would be.
        let mut cube = Mesh::from(Cuboid::new(3.0, 2.0, 1.6));
        cube.translate_by(Vec3::new(5.0, 1.0, -2.0));
        let piece = Piece::read(&cube).expect("piece");
        for k in 0..200u32 {
            let h = |c: u32| h01(k, c);
            let b = boulder::Block {
                x: 30.0 * h(1),
                z: 30.0 * h(2),
                hx: 0.8 + 10.0 * h(3),
                hz: 0.8 + 10.0 * h(4),
                // Under its top's lowest corner, as `boulder::seat` buries it.
                y0: -12.0,
                y1: 4.0 + 6.0 * h(5),
                tx: (h(6) - 0.5) * boulder::TILT_MAX * 2.0,
                tz: (h(7) - 0.5) * boulder::TILT_MAX * 2.0,
                yaw: (h(8) * 65535.0) as u16,
                shape: (h(9) * 255.0) as u8,
                on: ON_GROUND,
                kind: 0,
            };
            let r = RockShape::of(7, &b, 1.0);
            let mut soup = RockSoup::default();
            piece_block(&mut soup, &r, std::slice::from_ref(&piece));
            for p in &soup.pos {
                let (x, y, z) = (p[0], p[1], p[2]);
                let s = boulder::surface(&b, x, z, 0.01);
                let Some((top, _, bottom)) = s else {
                    panic!("block {k}: ({x}, {z}) is off the footprint of {b:?}");
                };
                assert!(
                    y <= top + 0.01 && y >= bottom - 0.01,
                    "block {k}: y {y} outside {bottom}..{top}"
                );
            }
            let (twice, open) = winding(&soup);
            assert_eq!((twice, open), (0, 0), "block {k}: the box drawn inside out");
        }
    }
}
