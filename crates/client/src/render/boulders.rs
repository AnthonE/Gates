//! The rock formations (`sim_core::boulder`), drawn.
//!
//! Each dome the sim collides against is drawn as a jagged, flat-shaded rock
//! mesh built around it, **wearing the ground's own splat material** — so a
//! boulder is the same granite as the cliff it stands under, with the same
//! blocks, cracks and streaks (`ground_splat.wgsl`'s rock face), and moss on
//! its crown where the ground around it is green. `reference/ROCKS.md` §1:
//! "a cliff and a clutter stone are the same substance at a 30× scale".
//!
//! The mesh is the dome displaced by a few percent either way, never more:
//! the sim holds a body `boulder::ROCK_SKIN_M` off the dome, so a rock drawn
//! much wider would swallow a player and one drawn much narrower would stop
//! them in the air.
//!
//! One entity per formation, built in world space, streamed by formation cell
//! out to [`ROCK_RING`] cells — rocks are landmarks, so they are drawn far
//! beyond the prop rings.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use sim_core::boulder::{self, Dome, BOULDER_CELL_M};
use sim_core::terrain;

use super::ground_splat::{ATTRIBUTE_MARKINGS, ATTRIBUTE_ROAD};
use super::terrain_mesh::{Ring, UV_PER_M};
use super::{Eye, WorldEntity, WorldId};

/// Formation cells drawn either side of the eye's: ~900 m.
pub const ROCK_RING: i32 = 14;
/// Formations built per frame. An empty cell is a hash and costs nothing
/// against this; a present one is a few height taps and a mesh.
pub const BUILDS_PER_FRAME: usize = 2;
/// How far the drawn rock departs from the collision dome, as a share of its
/// radius, either way.
pub const ROCK_JAG: f32 = 0.07;

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

/// Smooth value noise over the unit sphere's directions, in [0, 1].
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

/// A unit icosphere, `sub` times subdivided: (vertices, triangles).
fn icosphere(sub: u32) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let t = (1.0 + 5.0f32.sqrt()) * 0.5;
    let mut v: Vec<Vec3> = [
        (-1.0, t, 0.0),
        (1.0, t, 0.0),
        (-1.0, -t, 0.0),
        (1.0, -t, 0.0),
        (0.0, -1.0, t),
        (0.0, 1.0, t),
        (0.0, -1.0, -t),
        (0.0, 1.0, -t),
        (t, 0.0, -1.0),
        (t, 0.0, 1.0),
        (-t, 0.0, -1.0),
        (-t, 0.0, 1.0),
    ]
    .iter()
    .map(|&(x, y, z)| Vec3::new(x, y, z).normalize())
    .collect();
    let mut f: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..sub {
        let mut mid: HashMap<(u32, u32), u32> = HashMap::default();
        let mut next = Vec::with_capacity(f.len() * 4);
        let mut m = |a: u32, b: u32, v: &mut Vec<Vec3>| {
            let k = (a.min(b), a.max(b));
            *mid.entry(k).or_insert_with(|| {
                v.push(((v[a as usize] + v[b as usize]) * 0.5).normalize());
                (v.len() - 1) as u32
            })
        };
        for [a, b, c] in f {
            let ab = m(a, b, &mut v);
            let bc = m(b, c, &mut v);
            let ca = m(c, a, &mut v);
            next.extend_from_slice(&[[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        f = next;
    }
    (v, f)
}

/// Every dome of a formation as one flat-shaded mesh in world space, carrying
/// the attributes the ground material reads.
pub fn formation_mesh(seed: u64, f: &boulder::Formation) -> Mesh {
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    let mut col = Vec::new();
    for d in f.iter() {
        dome_tris(seed, d, &mut pos, &mut nrm, &mut col);
    }
    let n = pos.len();
    let uv: Vec<[f32; 2]> = pos
        .iter()
        .map(|p: &[f32; 3]| [p[0] * UV_PER_M, p[2] * UV_PER_M])
        .collect();
    // The planar tangent the ground's UV implies: world +X, projected onto
    // each face.
    let tan: Vec<[f32; 4]> = nrm
        .iter()
        .map(|n: &[f32; 3]| {
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
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
    // The macro break-up at its mean, and dry.
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, vec![[1.0f32, 0.0]; n])
    .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, tan)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, col)
    .with_inserted_attribute(ATTRIBUTE_ROAD, vec![[0.0f32; 2]; n])
    .with_inserted_attribute(ATTRIBUTE_MARKINGS, vec![[0.0f32; 4]; n])
    .with_inserted_indices(Indices::U32((0..n as u32).collect()))
}

fn dome_tris(
    seed: u64,
    d: &Dome,
    pos: &mut Vec<[f32; 3]>,
    nrm: &mut Vec<[f32; 3]>,
    col: &mut Vec<[f32; 4]>,
) {
    let sub = if d.r > 7.0 {
        3
    } else if d.r > 2.5 {
        2
    } else {
        1
    };
    let (v, f) = icosphere(sub);
    let s = (seed as u32) ^ ((d.shape as u32) << 8) ^ (d.yaw as u32).wrapping_mul(0x2545_F491);
    let yaw = Quat::from_rotation_y(d.yaw as f32 / 256.0 * std::f32::consts::TAU);
    // Squash in plan a little so no rock is a circle from above, while the
    // widest axis stays inside the collision disc.
    let squash = 0.82 + 0.16 * h01(s, 7);
    // The rock's own displacement: broad lobes, then a sharper chip octave,
    // quantised a touch so faces read as fractured planes.
    let world: Vec<Vec3> = v
        .iter()
        .map(|&u| {
            let lobes = noise3(u * 1.6 + Vec3::splat(3.1), s);
            let chips = noise3(u * 4.3 + Vec3::splat(7.7), s ^ 0x5bd1);
            let m = 1.0 + ROCK_JAG * (2.0 * (0.65 * lobes + 0.35 * chips) - 1.0) * 1.4;
            let local = Vec3::new(u.x * d.r * m, u.y * d.h * m, u.z * d.r * m * squash);
            Vec3::new(d.x, d.y, d.z) + yaw * local
        })
        .collect();
    // The ground this formation stands in, for the moss.
    let h0 = terrain::height(seed, d.x, d.z);
    let moss_ok = h0 > terrain::BEACH_MAX_H + 1.5 && h0 < terrain::TREELINE_H;
    for [a, b, c] in f {
        let (pa, pb, pc) = (world[a as usize], world[b as usize], world[c as usize]);
        // Everything well under the ground is never seen.
        if pa.y.max(pb.y).max(pc.y) < d.y - d.h * 0.05 {
            continue;
        }
        let n = (pb - pa).cross(pc - pa).normalize_or_zero();
        // The icosphere is wound outward; keep it so after the yaw.
        let n = if n.dot((pa + pb + pc) / 3.0 - Vec3::new(d.x, d.y, d.z)) < 0.0 {
            -n
        } else {
            n
        };
        let moss = if moss_ok {
            let t = ((n.y - 0.72) / 0.2).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t) * 0.55
        } else {
            0.0
        };
        let w = [0.0, moss * 0.4, moss * 0.6, 1.0 - moss];
        for p in [pa, pb, pc] {
            pos.push(p.to_array());
            nrm.push(n.to_array());
            col.push(w);
        }
        // Keep the winding outward for the pipeline's back-face cull.
        if (pb - pa).cross(pc - pa).dot(n) < 0.0 {
            let k = pos.len();
            pos.swap(k - 2, k - 1);
        }
    }
}
