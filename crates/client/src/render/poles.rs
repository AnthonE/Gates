//! The power poles along the ring (`sim_core::poles`), drawn: weathered
//! timber poles with a crossarm and three insulators, a transformer can on
//! every sixth, and three conductors slung between neighbours that both
//! stand. Built once per world in chunks round the ring, so the frustum
//! culls what is behind the camera.
//!
//! The wires are a few centimetres across, so far off they are less than a
//! pixel and only shimmer: they fade out over [`WIRE_FADE_M`], and the poles,
//! which still read as a line along the coast, over [`POLE_FADE_M`].

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use sim_core::poles::{self, POLE_H_M};
use sim_core::terrain::{self, RING_BEARINGS};

use super::depot::Surface;
use super::town::dressed_material;
use super::{WorldEntity, WorldId};

#[derive(Component)]
pub struct PoleVisual;

/// Bearings per drawn chunk of poles, and of wires: a visibility range is
/// measured from a mesh's origin, set at its middle, so the wires, which
/// fade near, go in short runs.
const CHUNK: usize = 16;
const WIRE_CHUNK: usize = 2;
/// The crossarm's height on the pole, and the conductors' spread along it.
const ARM_Y_M: f32 = POLE_H_M - 0.6;
const ARM_HALF_M: f32 = 1.05;
/// How far a span's middle hangs below its ends, as a share of its length.
const SAG: f32 = 0.022;
const WIRE_R_M: f32 = 0.016;
/// Every this many poles carries a transformer.
const TRANSFORMER_EVERY: usize = 6;
const WIRE_FADE_M: std::ops::Range<f32> = 260.0..340.0;
const POLE_FADE_M: std::ops::Range<f32> = 900.0..1100.0;

const WOOD: [f32; 4] = [0.6, 0.53, 0.45, 1.0];
const HARDWARE: [f32; 4] = [0.36, 0.37, 0.38, 1.0];
const CERAMIC: [f32; 4] = [0.82, 0.8, 0.74, 1.0];
const CONDUCTOR: [f32; 4] = [0.14, 0.14, 0.15, 1.0];

/// A plain triangle list: positions, normals, metre UVs, vertex colours.
#[derive(Default)]
struct Soup {
    p: Vec<[f32; 3]>,
    n: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    c: Vec<[f32; 4]>,
    i: Vec<u32>,
}

impl Soup {
    fn is_empty(&self) -> bool {
        self.i.is_empty()
    }

    /// A tube from `a` (radius `ra`) to `b` (radius `rb`), `sides` round,
    /// open-ended, its UVs in metres round and along.
    fn tube(&mut self, a: Vec3, b: Vec3, ra: f32, rb: f32, sides: u32, col: [f32; 4]) {
        let d = (b - a).normalize_or_zero();
        if d == Vec3::ZERO {
            return;
        }
        let up = if d.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
        let u = d.cross(up).normalize();
        let v = u.cross(d);
        let len = (b - a).length();
        let base = self.p.len() as u32;
        for k in 0..=sides {
            let t = k as f32 / sides as f32 * std::f32::consts::TAU;
            let r = u * t.cos() + v * t.sin();
            for (o, rad, along) in [(a, ra, 0.0), (b, rb, len)] {
                self.p.push((o + r * rad).to_array());
                self.n.push(r.to_array());
                self.uv.push([t * ra, along]);
                self.c.push(col);
            }
        }
        for k in 0..sides {
            let (a0, b0, a1, b1) = (
                base + 2 * k,
                base + 2 * k + 1,
                base + 2 * k + 2,
                base + 2 * k + 3,
            );
            self.i.extend_from_slice(&[a0, a1, b0, b0, a1, b1]);
        }
    }

    /// A box centred on `c`, `half` along the unit `along`, `w` across it
    /// horizontally and `h` tall.
    fn bar(&mut self, c: Vec3, along: Vec3, half: f32, w: f32, h: f32, col: [f32; 4]) {
        let x = along.normalize();
        let z = x.cross(Vec3::Y).normalize();
        let ex = [x * half, Vec3::Y * (h * 0.5), z * (w * 0.5)];
        for (axis, sign) in [
            (0, 1.0),
            (0, -1.0),
            (1, 1.0),
            (1, -1.0),
            (2, 1.0),
            (2, -1.0),
        ] {
            let n = ex[axis].normalize() * sign;
            let (o1, o2) = (ex[(axis + 1) % 3], ex[(axis + 2) % 3]);
            let fc = c + ex[axis] * sign;
            let base = self.p.len() as u32;
            for (s1, s2) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let p = fc + o1 * s1 + o2 * s2;
                self.p.push(p.to_array());
                self.n.push(n.to_array());
                self.uv.push([p.x + p.z, p.y]);
                self.c.push(col);
            }
            // Wound to face `n`.
            let tri = if (o1.cross(o2)).dot(n) > 0.0 {
                [base, base + 1, base + 2, base, base + 2, base + 3]
            } else {
                [base, base + 2, base + 1, base, base + 3, base + 2]
            };
            self.i.extend_from_slice(&tri);
        }
    }

    fn mesh(self) -> Mesh {
        let mut m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.p)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.n)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uv)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.c)
        .with_inserted_indices(Indices::U32(self.i));
        // The depot's surfaces carry normal maps, which want tangents.
        if let Err(e) = m.generate_tangents() {
            warn!("poles: no tangents ({e})");
        }
        m
    }
}

/// Where a pole's three conductors hang from, and which way its arm runs.
struct Pole {
    base: Vec3,
    radial: Vec3,
    tips: [Vec3; 3],
}

fn pole(seed: u64, haven: &terrain::Haven, k: usize) -> Option<Pole> {
    let (x, z) = poles::at(haven, k)?;
    let y = terrain::ground(seed, haven, x, z);
    let (dx, dz) = sim_core::yaw_dir((k as u16) << 8);
    let radial = Vec3::new(dx, 0.0, dz);
    let base = Vec3::new(x, y, z);
    let arm = base + Vec3::Y * ARM_Y_M;
    Some(Pole {
        base,
        radial,
        tips: [
            arm + radial * ARM_HALF_M + Vec3::Y * 0.2,
            base + Vec3::Y * (POLE_H_M + 0.22),
            arm - radial * ARM_HALF_M + Vec3::Y * 0.2,
        ],
    })
}

fn draw_pole(wood: &mut Soup, iron: &mut Soup, p: &Pole, k: usize) {
    let top = p.base + Vec3::Y * POLE_H_M;
    wood.tube(p.base - Vec3::Y * 0.6, top, 0.16, 0.12, 8, WOOD);
    let arm = p.base + Vec3::Y * ARM_Y_M;
    wood.bar(arm, p.radial, ARM_HALF_M + 0.15, 0.1, 0.12, WOOD);
    for s in [1.0, -1.0] {
        iron.tube(
            p.base + Vec3::Y * (ARM_Y_M - 0.7),
            arm + p.radial * (0.6 * s) - Vec3::Y * 0.05,
            0.02,
            0.02,
            4,
            HARDWARE,
        );
    }
    for t in &p.tips {
        iron.tube(*t - Vec3::Y * 0.22, *t, 0.05, 0.035, 6, CERAMIC);
    }
    if k.is_multiple_of(TRANSFORMER_EVERY) {
        // On the road side of the pole.
        let c = p.base + Vec3::Y * (ARM_Y_M - 2.0) - p.radial * 0.42;
        iron.tube(
            c - Vec3::Y * 0.45,
            c + Vec3::Y * 0.45,
            0.28,
            0.28,
            10,
            [0.5, 0.52, 0.5, 1.0],
        );
        iron.bar(c + Vec3::Y * 0.47, p.radial, 0.3, 0.6, 0.04, HARDWARE);
        iron.tube(
            c + Vec3::Y * 0.45,
            p.tips[1] - Vec3::Y * 0.3,
            0.012,
            0.012,
            4,
            CONDUCTOR,
        );
    }
}

fn draw_span(wire: &mut Soup, a: &Pole, b: &Pole) {
    for i in 0..3 {
        // Conductors keep their side: the arm's +radial tip on one pole goes
        // to the +radial tip on the next.
        let (p, q) = (a.tips[i], b.tips[i]);
        let len = (q - p).length();
        let sag = len * SAG;
        const SEGS: usize = 10;
        let mut prev = p;
        for s in 1..=SEGS {
            let t = s as f32 / SEGS as f32;
            let pt = p.lerp(q, t) - Vec3::Y * (sag * 4.0 * t * (1.0 - t));
            wire.tube(prev, pt, WIRE_R_M, WIRE_R_M, 4, CONDUCTOR);
            prev = pt;
        }
    }
}

/// Build the poles and their wires once per world.
pub fn spawn(
    mut commands: Commands,
    world: Res<WorldId>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    drawn: Query<(), With<PoleVisual>>,
) {
    if !drawn.is_empty() {
        return;
    }
    let root = commands
        .spawn((
            PoleVisual,
            WorldEntity,
            Transform::IDENTITY,
            Visibility::default(),
        ))
        .id();
    let timber = materials.add(dressed_material(Surface::Timber, &server));
    let steel = materials.add(dressed_material(Surface::Steel, &server));
    let all: Vec<Option<Pole>> = (0..RING_BEARINGS)
        .map(|k| pole(world.seed, &world.haven, k))
        .collect();
    let mut count = 0;
    let mut put = |commands: &mut Commands, soup: Soup, mat: &Handle<StandardMaterial>, fade| {
        if soup.is_empty() {
            return;
        }
        // Each run's mesh about its own middle, and the fade measured from
        // there: a range read off the AABB never drew at all here.
        let mut soup = soup;
        let mid = soup.p.iter().fold(Vec3::ZERO, |a, p| a + Vec3::from(*p)) / soup.p.len() as f32;
        for p in soup.p.iter_mut() {
            *p = (Vec3::from(*p) - mid).to_array();
        }
        commands.spawn((
            ChildOf(root),
            Mesh3d(meshes.add(soup.mesh())),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(mid),
            VisibilityRange {
                start_margin: 0.0..0.0,
                end_margin: fade,
                use_aabb: false,
            },
        ));
    };
    for chunk in 0..RING_BEARINGS / CHUNK {
        let (mut wood, mut iron) = (Soup::default(), Soup::default());
        for (k, p) in all.iter().enumerate().skip(chunk * CHUNK).take(CHUNK) {
            let Some(p) = p else { continue };
            draw_pole(&mut wood, &mut iron, p, k);
            count += 1;
        }
        put(&mut commands, wood, &timber, POLE_FADE_M);
        put(&mut commands, iron, &steel, POLE_FADE_M);
    }
    for chunk in 0..RING_BEARINGS / WIRE_CHUNK {
        let mut wire = Soup::default();
        for k in chunk * WIRE_CHUNK..(chunk + 1) * WIRE_CHUNK {
            if let (Some(p), Some(q)) = (&all[k], &all[(k + 1) % RING_BEARINGS]) {
                draw_span(&mut wire, p, q);
            }
        }
        put(&mut commands, wire, &steel, WIRE_FADE_M);
    }
    info!("poles: {count} along the ring");
}
