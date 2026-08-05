//! Scatter: what stands on the ground. Placement is `sim_core::terrain::
//! scatter` — the same cell draw the server resolves and the browser drew —
//! so this file owns only meshes and materials.
//!
//! **Silhouette before surface** (`ART.md` rule 6). A pine in the references
//! is tall, thin and ragged-edged, and a smooth cone is wrong at any texture
//! budget. The shape here is `web/src/props.js`'s, constant for constant,
//! because that shape was measured against the reference frames and is held
//! by a gate (`ci/pine_shape.mjs`) whose numbers are these numbers: five
//! whorls on a 5.7 m trunk, nine segments to a whorl, every rim pulled in and
//! drooped by its own hash so the outline is a fringe rather than a stack of
//! discs.
//!
//! The one thing not carried across yet is the per-instance colour tint. The
//! browser had it per instance because it drew through an `InstancedMesh`
//! with a colour attribute; here a shared material is what makes a forest one
//! draw call, so variation comes from `Slot`'s own yaw and scale plus a small
//! pool of mesh variants. `ART.md` rule 7 asks for more than that and
//! `RENDER.md` records the debt.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use sim_core::terrain::{self, Occupant};

use super::terrain_mesh::{CHUNK_M, NEAR_RADIUS};
use super::{Eye, WorldId};

/// Trunk height, metres. NOT the tree's height: the top whorl tapers to a
/// point and the trunk does not, so a trunk run to full height leaves a bare
/// spike above the canopy on every tree in the forest.
pub const PINE_TRUNK_H: f32 = 5.7;
/// Overall height — the top whorl's apex.
pub const PINE_H: f32 = 6.6;
/// How far a whorl vertex may be pulled IN toward its axis, as a fraction of
/// that whorl's radius. It only ever pulls in, never out, which is what keeps
/// the canopy inside `PINE_MAX_R`.
pub const PINE_RAGGED: f32 = 0.34;
/// How far a whorl's rim may hang BELOW its own base plane, metres. Without
/// it every whorl ends in a level disc and five level discs stacked up a
/// trunk are five horizontal lines.
pub const PINE_DROOP: f32 = 0.18;
/// The radius no part of a pine may exceed, metres — a CEILING, not what it
/// draws. `world.rs` derives `SPAWN_CLEAR_M = 4.0` from this, so a canopy
/// that grew past it would invalidate a spoken sim number from the renderer.
pub const PINE_MAX_R: f32 = 1.7;
/// Odd on purpose: an even count puts a vertex diametrically opposite every
/// other one, so the ragged pull reads as a squashed circle, not a whorl.
const PINE_SEGMENTS: usize = 9;
/// How far a canopy vertex's normal is blended from its own facet toward the
/// canopy volume. Nine segments means nine enormous flat plates each catching
/// the sun as one value, which is the whole visual signature of an asset-pack
/// tree — and it is a shading problem, not an outline one.
pub const PINE_NORMAL_BLEND: f32 = 0.7;

/// `(base y, base radius, height, hash seed)` — where a conifer's silhouette
/// actually comes from. Seeds are distinct from the mixer's own constants:
/// seeding a hash with one of the multipliers inside it correlates a whorl
/// with the one below it, and a correlated whorl is a vertical flute.
const PINE_WHORLS: [(f32, f32, f32, u32); 5] = [
    (1.55, 1.38, 2.15, 0x51ed_270b),
    (2.40, 1.19, 2.00, 0x2545_f491),
    (3.20, 0.99, 1.90, 0x1b87_3593),
    (4.00, 0.77, 1.75, 0x7feb_352d),
    (4.80, 0.55, 1.80, 0x846c_a68b),
];

/// `ART.md` §5's "two greens minimum": a dark lower green the bottom two
/// whorls wear and a lit upper green the top three do, plus the trunk. Each
/// band ramps over the span of the whorls that wear it rather than over each
/// whorl's own height, so the gradient is continuous across a boundary
/// instead of resetting five times up the tree.
const BAND_TRUNK: (u32, u32, f32, f32) = (0x503b2b, 0x70583f, 0.0, 1.9);
const BAND_SKIRT: (u32, u32, f32, f32) = (0x204825, 0x3c783d, 1.55, 4.4);
const BAND_CROWN: (u32, u32, f32, f32) = (0x2c5c2c, 0x4d8845, 3.2, PINE_H);
/// Which band each whorl wears — the low two dark, the top three lit.
const PINE_WHORL_BANDS: [u8; 5] = [1, 1, 2, 2, 2];

/// How many pine meshes the pool holds. Variation without breaking the forest
/// into one draw call per tree: the rag hash is seeded per entry, so no two
/// adjacent trees share an outline as well as a yaw and a scale.
///
/// **Deliberately NOT named `PINE_VARIANTS`**, which is a registered knob
/// (`DECISIONS.md` §open) pinned to `web/src/props.js` at 1. `ci/
/// knob_registry.mjs` went red on the collision the first time this compiled,
/// and it was right to: the browser's number is how many distinct pine
/// GEOMETRIES that renderer authors, this is how many baked meshes a shared-
/// material pool holds, and one registry name cannot mean two things.
const PINE_MESH_POOL: usize = 4;

/// Prop meshes and materials, built once and shared so a forest is instances
/// rather than draw calls (`DESIGN.md` §9).
#[derive(Resource)]
pub struct PropAssets {
    pines: Vec<Handle<Mesh>>,
    blob: Handle<Mesh>,
    boulder: Handle<Mesh>,
    bush: Handle<Mesh>,
    barrel: Handle<Mesh>,
    crate_box: Handle<Mesh>,
    cache_box: Handle<Mesh>,
    shelter: Handle<Mesh>,
    canopy: Handle<Mesh>,
    foliage: Handle<StandardMaterial>,
    rock: Handle<StandardMaterial>,
    ore_stone: Handle<StandardMaterial>,
    ore_metal: Handle<StandardMaterial>,
    ore_sulfur: Handle<StandardMaterial>,
    wood: Handle<StandardMaterial>,
    metal: Handle<StandardMaterial>,
}

/// What the scatter ring has spawned, one parent entity per chunk.
#[derive(Resource, Default)]
pub struct PropRing {
    built: HashMap<(i32, i32), Entity>,
}

impl PropRing {
    pub fn len(&self) -> usize {
        self.built.len()
    }
    pub fn is_empty(&self) -> bool {
        self.built.is_empty()
    }
    pub fn is_full(&self) -> bool {
        self.built.len() >= super::terrain_mesh::RING_CHUNKS
    }
}

/// sRGB hex to linear, which is what a vertex colour is in.
pub(super) fn linear(hex: u32) -> [f32; 3] {
    let f = |c: u32| {
        let s = c as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    [f((hex >> 16) & 0xff), f((hex >> 8) & 0xff), f(hex & 0xff)]
}

/// The one hash in this file: a 32-bit mix, used for the ragged pull and for
/// nothing that reaches the sim.
pub(super) fn hash2(a: u32, b: u32) -> u32 {
    let mut h = a.wrapping_mul(0x85eb_ca6b) ^ b.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_f491);
    h ^= h >> 13;
    h
}

pub(super) fn hash01(a: u32, b: u32) -> f32 {
    (hash2(a, b) >> 8) as f32 / 16_777_216.0
}

/// A band's colour at height `y`, ramped lo→hi across its own span.
fn band_color(band: (u32, u32, f32, f32), y: f32) -> [f32; 4] {
    let (lo, hi, y0, y1) = band;
    let t = ((y - y0) / (y1 - y0)).clamp(0.0, 1.0);
    let (l, h) = (linear(lo), linear(hi));
    [
        l[0] + (h[0] - l[0]) * t,
        l[1] + (h[1] - l[1]) * t,
        l[2] + (h[2] - l[2]) * t,
        1.0,
    ]
}

/// Raw triangle soup, so every builder here can flat-write and let the
/// normals be decided per triangle rather than per shared vertex.
#[derive(Default)]
pub(super) struct Soup {
    pos: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    col: Vec<[f32; 4]>,
}

impl Soup {
    /// One triangle with a facet normal, blended toward `volume_center` by
    /// `blend` — 0 is a flat plate, 1 is a soft volume. A needle mass does not
    /// have facets; it scatters light as a volume, and this is what stops
    /// nine segments reading as nine plates.
    pub(super) fn tri(
        &mut self,
        a: Vec3,
        b: Vec3,
        c: Vec3,
        color: impl Fn(Vec3) -> [f32; 4],
        volume_center: Option<Vec3>,
        blend: f32,
    ) {
        let facet = (b - a).cross(c - a).normalize_or_zero();
        for v in [a, b, c] {
            let n = match volume_center {
                Some(ctr) => {
                    let vol = (v - ctr).normalize_or_zero();
                    (facet * (1.0 - blend) + vol * blend).normalize_or_zero()
                }
                None => facet,
            };
            self.pos.push([v.x, v.y, v.z]);
            self.nrm.push([n.x, n.y, n.z]);
            self.col.push(color(v));
        }
    }

    pub(super) fn mesh(self) -> Mesh {
        let n = self.pos.len() as u32;
        let mut m = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        );
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.pos);
        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.nrm);
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.col);
        m.insert_indices(Indices::U32((0..n).collect()));
        m
    }
}

/// One pine. `variant` seeds the rag so the pool's trees differ in outline,
/// not only in yaw and scale.
fn pine_mesh(variant: u32) -> Mesh {
    let mut s = Soup::default();

    // The trunk: a tapered prism, 7 sides. Its band stops at 1.9 m because
    // everything above that is inside the canopy and invisible.
    let sides = 7usize;
    let (r0, r1) = (0.22f32, 0.13f32);
    for i in 0..sides {
        let a0 = i as f32 / sides as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / sides as f32 * std::f32::consts::TAU;
        let (b0, b1) = (
            Vec3::new(a0.cos() * r0, 0.0, a0.sin() * r0),
            Vec3::new(a1.cos() * r0, 0.0, a1.sin() * r0),
        );
        let (t0, t1) = (
            Vec3::new(a0.cos() * r1, PINE_TRUNK_H, a0.sin() * r1),
            Vec3::new(a1.cos() * r1, PINE_TRUNK_H, a1.sin() * r1),
        );
        let col = |v: Vec3| band_color(BAND_TRUNK, v.y);
        s.tri(b0, t0, b1, col, None, 0.0);
        s.tri(b1, t0, t1, col, None, 0.0);
    }

    for (wi, (base_y, radius, h, seed)) in PINE_WHORLS.iter().enumerate() {
        let band = if PINE_WHORL_BANDS[wi] == 1 {
            BAND_SKIRT
        } else {
            BAND_CROWN
        };
        let col = move |v: Vec3| band_color(band, v.y);
        let apex = Vec3::new(0.0, base_y + h, 0.0);
        // The volume the canopy's normals are pulled toward: the middle of
        // this whorl's own cone.
        let ctr = Vec3::new(0.0, base_y + h * 0.35, 0.0);

        let rim: Vec<Vec3> = (0..PINE_SEGMENTS)
            .map(|i| {
                let a = i as f32 / PINE_SEGMENTS as f32 * std::f32::consts::TAU;
                // Pull IN by up to PINE_RAGGED of the radius, and hang the
                // rim below its own base plane by up to PINE_DROOP. The hash
                // is over the vertex's own index plus the whorl's seed and
                // the variant, so a shared edge's two ends agree and the seam
                // cannot open.
                let pull =
                    1.0 - PINE_RAGGED * hash01(seed ^ variant.wrapping_mul(0x9e37_79b9), i as u32);
                let droop = PINE_DROOP * hash01(seed ^ 0x5bf0_3635, i as u32);
                let r = radius * pull;
                debug_assert!(r <= PINE_MAX_R);
                Vec3::new(a.cos() * r, base_y - droop, a.sin() * r)
            })
            .collect();

        let base_ctr = Vec3::new(0.0, *base_y, 0.0);
        for i in 0..PINE_SEGMENTS {
            let (p, q) = (rim[i], rim[(i + 1) % PINE_SEGMENTS]);
            // The cone's side.
            s.tri(p, apex, q, col, Some(ctr), PINE_NORMAL_BLEND);
            // The underside. The rim droops and the centre does not, so it is
            // a shallow bowl rather than a plate — the face `ART.md` §5 says
            // every judge catches.
            s.tri(q, base_ctr, p, col, Some(ctr), PINE_NORMAL_BLEND);
        }
    }

    s.mesh()
}

/// A faceted blob on an icosahedron's vertices, jittered per seed. Stands in
/// for every rock, node and bush — the shapes that were `DodecahedronGeometry`
/// in the browser.
fn blob_mesh(radius: f32, jitter: f32, seed: u32, hex: u32) -> Mesh {
    // Icosahedron vertices.
    let t = (1.0 + 5.0f32.sqrt()) * 0.5;
    let raw = [
        [-1.0, t, 0.0],
        [1.0, t, 0.0],
        [-1.0, -t, 0.0],
        [1.0, -t, 0.0],
        [0.0, -1.0, t],
        [0.0, 1.0, t],
        [0.0, -1.0, -t],
        [0.0, 1.0, -t],
        [t, 0.0, -1.0],
        [t, 0.0, 1.0],
        [-t, 0.0, -1.0],
        [-t, 0.0, 1.0],
    ];
    const FACES: [[usize; 3]; 20] = [
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
    let verts: Vec<Vec3> = raw
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let d = Vec3::from_array(*v).normalize();
            d * radius * (1.0 - jitter * hash01(seed, i as u32))
        })
        .collect();

    let mut s = Soup::default();
    let base = linear(hex);
    for (fi, f) in FACES.iter().enumerate() {
        // Per-facet value break-up: a rock whose twenty faces share one value
        // is rule 1's flat surface with extra steps.
        let v = 0.82 + 0.36 * hash01(seed ^ 0x2c1b_3c6d, fi as u32);
        let col = move |_: Vec3| [base[0] * v, base[1] * v, base[2] * v, 1.0];
        s.tri(verts[f[0]], verts[f[1]], verts[f[2]], col, None, 0.0);
    }
    s.mesh()
}

/// A box massing, for the two authored structures. Each entry is
/// `(centre, half-extent, hex)`.
fn boxes_mesh(parts: &[([f32; 3], [f32; 3], u32)]) -> Mesh {
    let mut s = Soup::default();
    for (c, h, hex) in parts {
        let c = Vec3::from_array(*c);
        let h = Vec3::from_array(*h);
        let base = linear(*hex);
        let corner = |sx: f32, sy: f32, sz: f32| c + Vec3::new(h.x * sx, h.y * sy, h.z * sz);
        // Six faces, each two triangles, each face at its own value so the
        // massing reads as a solid rather than a silhouette.
        let faces: [([Vec3; 4], f32); 6] = [
            (
                [
                    corner(-1., -1., 1.),
                    corner(1., -1., 1.),
                    corner(1., 1., 1.),
                    corner(-1., 1., 1.),
                ],
                0.95,
            ),
            (
                [
                    corner(1., -1., -1.),
                    corner(-1., -1., -1.),
                    corner(-1., 1., -1.),
                    corner(1., 1., -1.),
                ],
                0.78,
            ),
            (
                [
                    corner(1., -1., 1.),
                    corner(1., -1., -1.),
                    corner(1., 1., -1.),
                    corner(1., 1., 1.),
                ],
                0.88,
            ),
            (
                [
                    corner(-1., -1., -1.),
                    corner(-1., -1., 1.),
                    corner(-1., 1., 1.),
                    corner(-1., 1., -1.),
                ],
                0.84,
            ),
            (
                [
                    corner(-1., 1., 1.),
                    corner(1., 1., 1.),
                    corner(1., 1., -1.),
                    corner(-1., 1., -1.),
                ],
                1.0,
            ),
            (
                [
                    corner(-1., -1., -1.),
                    corner(1., -1., -1.),
                    corner(1., -1., 1.),
                    corner(-1., -1., 1.),
                ],
                0.62,
            ),
        ];
        for (q, v) in faces {
            let col = move |_: Vec3| [base[0] * v, base[1] * v, base[2] * v, 1.0];
            s.tri(q[0], q[1], q[2], col, None, 0.0);
            s.tri(q[0], q[2], q[3], col, None, 0.0);
        }
    }
    s.mesh()
}

pub fn assets(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> PropAssets {
    let surface = |rough: f32, refl: f32, materials: &mut Assets<StandardMaterial>| {
        materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: rough,
            reflectance: refl,
            ..default()
        })
    };
    PropAssets {
        pines: (0..PINE_MESH_POOL)
            .map(|v| meshes.add(pine_mesh(v as u32 + 1)))
            .collect(),
        blob: meshes.add(blob_mesh(1.0, 0.28, 0x51ed_270b, 0x9c968a)),
        boulder: meshes.add(blob_mesh(1.5, 0.32, 0x1b87_3593, 0x8e887c)),
        bush: meshes.add(blob_mesh(0.7, 0.42, 0x2545_f491, 0x2c5f2e)),
        barrel: meshes.add(Cylinder::new(0.45, 0.95).mesh().resolution(10).build()),
        crate_box: meshes.add(boxes_mesh(&[([0., 0., 0.], [0.55, 0.4, 0.4], 0x6b5334)])),
        cache_box: meshes.add(boxes_mesh(&[([0., 0., 0.], [0.45, 0.275, 0.35], 0x6a5940)])),
        // The pad's greybox: a walled block with a tower. Not a kit of
        // wall-sized slots — one slot, one structure (`terrain.rs`
        // `Occupant::HavenShelter`).
        shelter: meshes.add(boxes_mesh(&[
            ([0.0, 0.15, 0.0], [3.5, 0.15, 3.5], 0x6f6a60),
            ([0.0, 1.6, -3.2], [3.5, 1.45, 0.3], 0x8a8479),
            ([-3.2, 1.6, 0.0], [0.3, 1.45, 3.5], 0x8a8479),
            ([3.2, 1.6, 0.0], [0.3, 1.45, 3.5], 0x8a8479),
            ([-2.35, 1.6, 3.2], [1.15, 1.45, 0.3], 0x8a8479),
            ([2.35, 1.6, 3.2], [1.15, 1.45, 0.3], 0x8a8479),
            ([0.0, 2.75, 3.2], [1.2, 0.3, 0.3], 0x8a8479),
            ([0.0, 3.2, 0.0], [3.6, 0.25, 3.6], 0x5f5b53),
            ([1.8, 4.4, -1.8], [1.1, 1.2, 1.1], 0x8a8479),
        ])),
        // The lesser tier's: an open roof on four posts, deliberately NOT the
        // pad's building at 0.6 scale — under half its height, squatter.
        canopy: meshes.add(boxes_mesh(&[
            ([-1.5, 0.9, -1.5], [0.12, 0.9, 0.12], 0x6a5940),
            ([1.5, 0.9, -1.5], [0.12, 0.9, 0.12], 0x6a5940),
            ([-1.5, 0.9, 1.5], [0.12, 0.9, 0.12], 0x6a5940),
            ([1.5, 0.9, 1.5], [0.12, 0.9, 0.12], 0x6a5940),
            ([0.0, 1.95, 0.0], [1.9, 0.14, 1.9], 0x7b6a4f),
            ([0.0, 1.0, -1.62], [1.6, 0.9, 0.1], 0x6a5940),
        ])),
        foliage: surface(0.86, 0.10, materials),
        rock: surface(0.88, 0.20, materials),
        ore_stone: surface(0.80, 0.24, materials),
        ore_metal: surface(0.55, 0.42, materials),
        ore_sulfur: surface(0.78, 0.22, materials),
        wood: surface(0.85, 0.14, materials),
        metal: surface(0.50, 0.45, materials),
    }
}

/// Spawn the scatter for the near ring, one chunk per frame.
#[allow(clippy::too_many_arguments)]
pub fn stream(
    mut commands: Commands,
    mut ring: ResMut<PropRing>,
    mut store: Local<Option<PropAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    world: Res<WorldId>,
    eye: Res<Eye>,
) {
    let a = store.get_or_insert_with(|| assets(&mut meshes, &mut materials));

    let cx = (eye.pos.x / CHUNK_M).floor() as i32;
    let cz = (eye.pos.z / CHUNK_M).floor() as i32;

    let mut dropped = 0usize;
    ring.built.retain(|(bx, bz), e| {
        if dropped >= 1 || ((*bx - cx).abs() <= NEAR_RADIUS && (*bz - cz).abs() <= NEAR_RADIUS) {
            return true;
        }
        dropped += 1;
        commands.entity(*e).despawn();
        false
    });

    for dz in -NEAR_RADIUS..=NEAR_RADIUS {
        for dx in -NEAR_RADIUS..=NEAR_RADIUS {
            let key = (cx + dx, cz + dz);
            if ring.built.contains_key(&key) {
                continue;
            }
            let parent = commands
                .spawn((Transform::IDENTITY, Visibility::default()))
                .id();
            // A 64 m chunk is exactly 8 scatter cells a side (CELL_SIZE 8 m).
            let cells = (CHUNK_M / terrain::CELL_SIZE) as i32;
            for iz in 0..cells {
                for ix in 0..cells {
                    let cell_x = key.0 * cells + ix;
                    let cell_z = key.1 * cells + iz;
                    let slot =
                        terrain::scatter(world.seed, &world.table, &world.haven, cell_x, cell_z);
                    if slot.occupant == Occupant::None {
                        continue;
                    }
                    spawn_slot(&mut commands, parent, a, &slot);
                }
            }
            ring.built.insert(key, parent);
            // One chunk of scatter per frame, same budget as the ground.
            return;
        }
    }
}

fn spawn_slot(commands: &mut Commands, parent: Entity, a: &PropAssets, slot: &terrain::Slot) {
    let yaw = slot.yaw as f32 / 256.0 * std::f32::consts::TAU;
    // Which mesh, which material, and how far the mesh's own origin sits
    // above the ground — the browser's `lift`, kept because these meshes are
    // centred and the slot's y is the surface.
    let (mesh, material, lift) = match slot.occupant {
        Occupant::Tree => {
            let v = (slot.yaw as usize) % a.pines.len();
            (a.pines[v].clone(), a.foliage.clone(), 0.0)
        }
        Occupant::StoneNode => (a.blob.clone(), a.ore_stone.clone(), 0.5),
        Occupant::MetalNode => (a.blob.clone(), a.ore_metal.clone(), 0.5),
        Occupant::SulfurNode => (a.blob.clone(), a.ore_sulfur.clone(), 0.5),
        Occupant::Bush => (a.bush.clone(), a.foliage.clone(), 0.45),
        Occupant::Rock => (a.boulder.clone(), a.rock.clone(), 0.55),
        Occupant::BarrelSlot => (a.barrel.clone(), a.metal.clone(), 0.5),
        Occupant::CrateSlot => (a.crate_box.clone(), a.wood.clone(), 0.4),
        Occupant::CacheSlot => (a.cache_box.clone(), a.wood.clone(), 0.275),
        Occupant::HavenShelter => (a.shelter.clone(), a.rock.clone(), 0.0),
        Occupant::WaystationCanopy => (a.canopy.clone(), a.wood.clone(), 0.0),
        Occupant::None => return,
    };
    // Rule 2: nothing sits ON the ground, everything sits IN it. Sinking the
    // lift slightly is the cheapest half of that; the crowding half is the
    // clutter skirt, which `sim_core` already places.
    let sink = 0.06;
    commands.entity(parent).with_child((
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform {
            translation: Vec3::new(slot.x, slot.y + lift * slot.scale - sink, slot.z),
            rotation: Quat::from_rotation_y(yaw),
            scale: Vec3::splat(slot.scale),
        },
    ));
}
