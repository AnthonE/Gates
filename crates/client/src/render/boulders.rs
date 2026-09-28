//! The rock formations (`sim_core::boulder`), drawn.
//!
//! Each block the sim collides against is drawn as a **fractured rock**: a
//! box with rounded edges, subdivided, its faces pushed in and out in steps
//! so they break into planes, its sides grooved along horizontal strata, and
//! moss in patches on its top — the reference game's granite outcrops, not
//! domes. It wears the ground's own splat material, so a boulder is the same
//! granite as the cliff it stands under (`ground_splat.wgsl`'s rock face),
//! darkened through the macro channel (`UV_1.x`) the way weathered rock is.
//! Loose stones lie around every seated block's foot.
//!
//! The mesh departs from the collision box by under a metre either way: the
//! sim holds a body `occupy::ROCK_SKIN_M` off the box, so a rock drawn much
//! wider would swallow a player and one drawn much narrower would stop them
//! in the air.
//!
//! One entity per formation, built in world space, streamed by formation cell
//! out to [`ROCK_RING`] cells — rocks are landmarks, so they are drawn far
//! beyond the prop rings. [`rock_block`] is public because the landmark kits
//! draw their rock parts with it (`landmarks.rs`).

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use sim_core::boulder::{self, BOULDER_CELL_M, ON_GROUND};
use sim_core::terrain;

use super::ground_splat::{ATTRIBUTE_MARKINGS, ATTRIBUTE_ROAD};
use super::terrain_mesh::{Ring, UV_PER_M};
use super::{Eye, WorldEntity, WorldId};

/// Formation cells drawn either side of the eye's: ~900 m.
pub const ROCK_RING: i32 = 14;
/// Formations built per frame. An empty cell is a hash and costs nothing
/// against this; a present one is a few height taps and a mesh.
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
/// Most a face is pushed in or out, metres. Under a metre, for the reason in
/// the module note.
pub const ROCK_FRACTURE_MAX_M: f32 = 0.9;
/// How far a rock resting on another is drawn down into it, metres.
pub const STACK_SINK_M: f32 = 0.6;
/// Horizontal size of one face segment, metres.
const SEG_M: f32 = 1.6;
/// Vertical size of one side segment, metres — finer, so the strata show.
const SEG_V_M: f32 = 0.9;

#[derive(Resource, Default)]
pub struct RockRing {
    seed: Option<u64>,
    built: HashMap<(i32, i32), Option<Entity>>,
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
pub fn stream(
    mut commands: Commands,
    mut rocks: ResMut<RockRing>,
    mut meshes: ResMut<Assets<Mesh>>,
    ground: Res<Ring>,
    world: Res<WorldId>,
    eye: Res<Eye>,
) {
    let Some(material) = ground.ground_material() else {
        return;
    };
    if rocks.seed != Some(world.seed) {
        for (_, e) in rocks.built.drain() {
            if let Some(e) = e {
                commands.entity(e).despawn();
            }
        }
        rocks.seed = Some(world.seed);
    }
    let cx = (eye.pos.x / BOULDER_CELL_M).floor() as i32;
    let cz = (eye.pos.z / BOULDER_CELL_M).floor() as i32;
    rocks.built.retain(|&(x, z), e| {
        let keep = (x - cx).abs() <= ROCK_RING + 1 && (z - cz).abs() <= ROCK_RING + 1;
        if !keep {
            if let Some(e) = e {
                commands.entity(*e).despawn();
            }
        }
        keep
    });
    let mut budget = BUILDS_PER_FRAME;
    // Rings outward, so the nearest rocks land first.
    for r in 0..=ROCK_RING {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let key = (cx + dx, cz + dz);
                if rocks.built.contains_key(&key) {
                    continue;
                }
                let f = boulder::formation(world.seed, &world.haven, key.0, key.1);
                if f.n == 0 {
                    rocks.built.insert(key, None);
                    continue;
                }
                let mesh = formation_mesh(world.seed, &f);
                let e = commands
                    .spawn((
                        WorldEntity,
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(material.clone()),
                        Transform::IDENTITY,
                    ))
                    .id();
                rocks.built.insert(key, Some(e));
                budget -= 1;
                if budget == 0 {
                    return;
                }
            }
        }
    }
}

/// A new world: forget the formations (their entities are `WorldEntity`).
pub fn teardown(mut rocks: ResMut<RockRing>) {
    *rocks = RockRing::default();
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

/// A rock mesh being built: flat-shaded triangles in world space, with the
/// attributes the ground material reads.
#[derive(Default)]
pub struct RockSoup {
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    /// Splat weights: rock, with moss (grass and litter) where it grows.
    col: Vec<[f32; 4]>,
    /// `UV_1`: the value multiplier, and dry.
    val: Vec<[f32; 2]>,
}

impl RockSoup {
    pub fn is_empty(&self) -> bool {
        self.pos.is_empty()
    }

    fn tri(&mut self, p: [Vec3; 3], moss: [f32; 3], value: [f32; 3]) {
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
        for k in 0..3 {
            self.pos.push(p[k].to_array());
            self.nrm.push(n.to_array());
            let m = moss[k];
            self.col.push([0.0, m * 0.55, m * 0.45, 1.0 - m]);
            self.val.push([value[k], 0.0]);
        }
    }

    pub fn mesh(self) -> Mesh {
        let n = self.pos.len();
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
        .with_inserted_indices(Indices::U32((0..n as u32).collect()))
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
    /// Whether moss may grow on its top.
    pub moss: bool,
    /// Coarsest the faces may be: 1 draws a chunk, not a block.
    pub min_seg: usize,
}

impl RockShape {
    /// A sim block.
    pub fn of(seed: u64, b: &boulder::Block, moss: bool) -> Self {
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
            min_seg: 2,
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
    let seg = |m: f32, per: f32| ((m / per).ceil() as usize).clamp(r.min_seg, 8);
    let (nx, ny, nz) = if r.min_seg <= 1 {
        (1, 1, 1)
    } else {
        (
            seg(2.0 * r.hx, SEG_M),
            seg(hgt, SEG_V_M),
            seg(2.0 * r.hz, SEG_M),
        )
    };
    let amp = (0.16 * r.hx.min(r.hz).min(hgt * 0.5)).min(ROCK_FRACTURE_MAX_M);
    // A few metres a stratum, per rock.
    let layer = 0.9 + 1.4 * h01(r.key, 3);
    let chunk = r.min_seg <= 1;
    // One rounding radius in metres on every axis, as a share of each axis.
    let half_h = hgt * 0.5;
    let rw = (ROCK_ROUND * r.hx.min(r.hz).min(half_h)).min(ROCK_ROUND_MAX_M);
    let rho = Vec3::new(rw / r.hx, rw / half_h, rw / r.hz).min(Vec3::splat(0.95));
    let taper =
        (ROCK_TAPER * (0.4 + 0.6 * h01(r.key, 9))).min(ROCK_TAPER_MAX_M / r.hx.min(r.hz).max(0.1));
    // Where a point of the unit box lands, how far it was pushed (for the
    // shading), and its outward direction.
    let place = |u: Vec3| -> (Vec3, f32, Vec3) {
        let inner = u.clamp(-(Vec3::ONE - rho), Vec3::ONE - rho);
        // The offset from the core in rounding radii: a unit sphere's
        // direction, which is also the outward normal, the radius being the
        // same in metres on every axis.
        let n = ((u - inner) / rho).normalize_or_zero();
        let p = inner + n * rho;
        let t = (p.y + 1.0) * 0.5;
        let shrink = 1.0 - taper * t;
        let (lx, lz) = (p.x * r.hx * shrink, p.z * r.hz * shrink);
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
        let mut push = (stepped - 0.5) * amp;
        // A top is ground: pushed about less.
        push *= 1.0 - 0.7 * n.y.max(0.0);
        // Strata: a groove every `layer` metres up the sides, wandering.
        let side = 1.0 - n.y.abs();
        let band = (w.y - r.y0) / layer + noise3(q * 0.7, r.key ^ 0x77) * 0.6;
        if band - band.floor() < 0.2 {
            push -= amp * 0.9 * side;
        }
        (w + wn * push, push / amp.max(1e-3), wn)
    };
    // The six faces as (normal axis, sign), each gridded along the other two
    // axes with the same counts as its neighbours, so the edges they share
    // are the same points and the rock is closed. The bottom is buried and
    // skipped.
    let counts = [nx, ny, nz];
    let faces: [(usize, f32); 5] = [(1, 1.0), (0, 1.0), (0, -1.0), (2, 1.0), (2, -1.0)];
    for (axis, sg) in faces {
        let (a, b) = match axis {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        let (na, nb) = (counts[a], counts[b]);
        let at = |i: usize, j: usize| {
            let mut u = Vec3::ZERO;
            u[axis] = sg;
            u[a] = -1.0 + 2.0 * i as f32 / na as f32;
            u[b] = -1.0 + 2.0 * j as f32 / nb as f32;
            u
        };
        let mut grid = Vec::with_capacity((na + 1) * (nb + 1));
        for i in 0..=na {
            for j in 0..=nb {
                grid.push(place(at(i, j)));
            }
        }
        let g = |i: usize, j: usize| grid[i * (nb + 1) + j];
        let mut out = Vec3::ZERO;
        out[axis] = sg;
        let outward = r.world_dir(out);
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
                    let n = (p[1] - p[0]).cross(p[2] - p[0]);
                    if n.dot(outward) < 0.0 {
                        p.swap(1, 2);
                        d.swap(1, 2);
                    }
                    let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
                    let moss = if r.moss {
                        let m = ((n.y - 0.6) / 0.3).clamp(0.0, 1.0);
                        let c = (p[0] + p[1] + p[2]) / 3.0;
                        let patch = (noise3(c * 0.22, r.key ^ 0x3c) * 1.8 - 0.5).clamp(0.0, 1.0);
                        m * m * (3.0 - 2.0 * m) * patch * 0.85
                    } else {
                        0.0
                    };
                    let face = 0.8 + 0.4 * h01(r.key ^ 0x9e, (i * 131 + j * 17 + axis) as u32);
                    let value = |k: usize| {
                        let up = ((p[k].y - r.y0) / hgt).clamp(0.0, 1.0);
                        // Recesses darker: the cheap half of occlusion.
                        let recess = 1.0 + d[k] * 0.3;
                        ROCK_VALUE
                            * (ROCK_FOOT_VALUE + (1.0 - ROCK_FOOT_VALUE) * up)
                            * face
                            * recess
                    };
                    soup.tri(p, [moss; 3], [value(0), value(1), value(2)]);
                }
            }
        }
    }
}

/// Loose stones around a seated block's foot: a handful to a dozen, a hand
/// to a knee high, lying on the ground just off its sides.
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
        let size = 0.12 + 0.36 * h(3) * h(3);
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
                y1: gy + size * (0.6 + 0.6 * h(6)),
                tx: (h(7) - 0.5) * 0.6,
                tz: (h(8) - 0.5) * 0.6,
                key: r.key ^ (k as u32 + 1).wrapping_mul(0x9E37_79B9),
                moss: false,
                min_seg: 1,
            },
        );
    }
}

/// Every block of a formation, and the loose stones round it, as one mesh.
pub fn formation_mesh(seed: u64, f: &boulder::Formation) -> Mesh {
    let mut soup = RockSoup::default();
    for b in f.iter() {
        // Moss where the ground around is green: not on a beach, not above
        // the treeline.
        let h0 = terrain::height(seed, b.x, b.z);
        let moss = h0 > terrain::BEACH_MAX_H + 1.5 && h0 < terrain::TREELINE_H;
        let mut r = RockShape::of(seed, b, moss);
        if b.on == ON_GROUND {
            rock_block(&mut soup, &r);
            talus(&mut soup, seed, &r);
        } else {
            // Drawn reaching into the block under it, so the seam between
            // the two is a crease and never a gap.
            r.y0 -= STACK_SINK_M;
            rock_block(&mut soup, &r);
        }
    }
    soup.mesh()
}
