//! Cliffs: the rock skin over every face too steep to walk
//! (`cliff_skin.rs`) and the grass on its shelves, the crags breaking its
//! lip against the sky, and the scree lying at its foot.
//!
//! A scarp is the smooth heightfield, so from any distance it read as one pale
//! sheet with a photograph on it (`NOW.md` §0rf items 2–3). Blocks stood in
//! the face read as bricks on plaster; the skin breaks the whole face into
//! planes, strata, shelves and overhangs instead.
//!
//! The skin cannot touch the walkable top, so the lip's outline against the
//! sky is broken by low crag teeth standing in the face just under it
//! ([`boulders::rock_block`], on the ground's own material and chiselled into
//! facets), every corner of each on ground too steep to walk. They sit on
//! the DRAWN face (`terrain_mesh::near_drawn_y`), which the cliff relief
//! moves off the sim's height; the slope tests stay the sim's.
//!
//! **Drawn, not decided: nothing here collides.** The skin and the teeth
//! stand only where the sim already refuses a body. The scree is ankle-high,
//! like the clutter a body already wades through.

use bevy::light::NotShadowCaster;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};
use sim_core::terrain::{self, Haven};

use super::boulders::{rock_block, RockShape, RockSoup};
use super::cliff_skin;
use super::clutter::ClutterRing;
use super::props::Soup;
use super::terrain_mesh::{near_drawn_y, Ring};
use super::{Eye, WorldEntity, WorldId};

/// One streamed cell's side, metres.
pub const CLIFF_CELL_M: f32 = 32.0;
/// Cells drawn either side of the eye's: ~220 m.
pub const CLIFF_RING: i32 = 7;
/// Candidate spacing inside a cell, metres.
const STEP_M: f32 = 4.0;
/// Cells handed to the pool per frame; each is the skin's half-metre lattice
/// where the cell is steep, and ~64 candidates of a few ground taps.
const QUEUES_PER_FRAME: usize = 2;
/// Finished cells landed per frame.
const LANDS_PER_FRAME: usize = 2;
/// The crag field's wavelength, metres.
const CRAG_M: f32 = 22.0;
/// A crag's value against the face's: the same granite, freshly broken.
const CRAG_VALUE: f32 = 0.92;
/// How far a tooth's foot is buried below the face, metres.
const TOOTH_BURY_M: f32 = 0.7;
/// Where scree may lie: walkable ground at most this steep…
const SCREE_SLOPE_MAX: f32 = 0.95;
/// …with a cliff this many metres uphill of it.
const SCREE_REACH_M: f32 = 6.0;
/// The share of foot candidates that carry a pile.
const SCREE_SHARE: f32 = 0.8;
/// How far below the lip a crag tooth may stand, metres along the fall line.
const TOOTH_REACH_M: f32 = 3.5;
/// The share of candidates under a lip that grow a tooth where the crag
/// field is full.
const TOOTH_SHARE: f32 = 0.6;
/// The gap kept between a tooth's uphill face and the lip, metres.
const TOOTH_CLEAR_M: f32 = 0.25;
/// Lowest and highest a tooth's head stands above the lip, metres.
const TOOTH_RISE_MIN_M: f32 = 0.3;
const TOOTH_RISE_MAX_M: f32 = 1.5;
/// Tallest a tooth may be from its buried foot, metres: a face that drops
/// away faster than this under the lip would make it a free-standing pillar.
const TOOTH_TALL_MAX_M: f32 = 5.0;

/// The share of teeth in the green band with turf on top.
const TOOTH_MOSS_SHARE: f32 = 0.8;
/// How lush that turf is (`RockShape::moss`).
const TOOTH_MOSS: f32 = 1.7;
/// Chisel depth as a share of a cliff rock's smallest extent.
const CHISEL_SHARE: f32 = 0.85;
/// Deepest a chisel plane cuts into a cliff rock, metres.
const CHISEL_MAX_M: f32 = 2.0;

/// A built cell's two meshes: its rock, and the grass on its shelves.
type CellMeshes = (Option<Mesh>, Option<Mesh>);

#[derive(Resource, Default)]
pub struct CliffRing {
    seed: Option<u64>,
    built: HashMap<(i32, i32), Option<Entity>>,
    /// Cells being built on the pool.
    tasks: HashMap<(i32, i32), Task<CellMeshes>>,
}

/// Keep the cliff cells within [`CLIFF_RING`] of the eye built, nearest
/// first, and drop the ones that fall out of it. A cell is built on
/// `AsyncComputeTaskPool` — a steep one is a few milliseconds of skin — and
/// landed here.
pub fn stream(
    mut commands: Commands,
    mut ring: ResMut<CliffRing>,
    mut meshes: ResMut<Assets<Mesh>>,
    ground: Res<Ring>,
    clutter: Res<ClutterRing>,
    world: Res<WorldId>,
    eye: Res<Eye>,
) {
    let Some(material) = ground.ground_material() else {
        return;
    };
    // The shelves' grass wears the meadow's cards, made by the clutter
    // ring's first fill.
    let Some(card_material) = clutter.card_material() else {
        return;
    };
    if ring.seed != Some(world.seed) {
        for (_, e) in ring.built.drain() {
            if let Some(e) = e {
                commands.entity(e).despawn();
            }
        }
        // Dropping a `Task` cancels it.
        ring.tasks.clear();
        ring.seed = Some(world.seed);
    }
    let cx = (eye.pos.x / CLIFF_CELL_M).floor() as i32;
    let cz = (eye.pos.z / CLIFF_CELL_M).floor() as i32;
    let near =
        |x: i32, z: i32| (x - cx).abs() <= CLIFF_RING + 1 && (z - cz).abs() <= CLIFF_RING + 1;
    ring.built.retain(|&(x, z), e| {
        let keep = near(x, z);
        if !keep {
            if let Some(e) = e {
                commands.entity(*e).despawn();
            }
        }
        keep
    });
    ring.tasks.retain(|&(x, z), _| near(x, z));

    // Cells that finished, a bounded few a frame: `meshes.add` uploads.
    for _ in 0..LANDS_PER_FRAME {
        let Some(key) = ring
            .tasks
            .iter()
            .find(|(_, t)| t.is_finished())
            .map(|(k, _)| *k)
        else {
            break;
        };
        let Some(mut task) = ring.tasks.remove(&key) else {
            break;
        };
        let Some((rock, grass)) = block_on(future::poll_once(&mut task)) else {
            // Finished but not ready: put it back, since dropping it would
            // cancel it.
            ring.tasks.insert(key, task);
            break;
        };
        let e = rock.map(|rock| {
            let e = commands
                .spawn((
                    WorldEntity,
                    Mesh3d(meshes.add(rock)),
                    MeshMaterial3d(material.clone()),
                    Transform::IDENTITY,
                ))
                .id();
            if let Some(grass) = grass {
                // No shadow: a masked card in the shadow pass is an alpha
                // test per texel, as the meadow's far tiles (`clutter.rs`)
                // say.
                commands.spawn((
                    Mesh3d(meshes.add(grass)),
                    MeshMaterial3d(card_material.clone()),
                    NotShadowCaster,
                    Transform::IDENTITY,
                    ChildOf(e),
                ));
            }
            e
        });
        ring.built.insert(key, e);
    }

    let pool = AsyncComputeTaskPool::get();
    let (seed, haven) = (world.seed, world.haven);
    let mut budget = QUEUES_PER_FRAME;
    for r in 0..=CLIFF_RING {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let key = (cx + dx, cz + dz);
                if ring.built.contains_key(&key) || ring.tasks.contains_key(&key) {
                    continue;
                }
                ring.tasks.insert(
                    key,
                    pool.spawn(async move {
                        let (soup, tufts) = cell_soup(seed, &haven, key.0, key.1);
                        (
                            (!soup.is_empty()).then(|| soup.mesh()),
                            (!tufts.is_empty()).then(|| tufts.mesh()),
                        )
                    }),
                );
                budget -= 1;
                if budget == 0 {
                    return;
                }
            }
        }
    }
}

/// A new world: forget the cells (their entities are `WorldEntity`).
pub fn teardown(mut ring: ResMut<CliffRing>) {
    *ring = CliffRing::default();
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

/// The ground's height and downhill gradient `(y, gx, gz)` at a point, from
/// central differences at 1 m — the sim's own `ground_slope` taps.
fn ground_at(seed: u64, haven: &Haven, x: f32, z: f32) -> (f32, f32, f32) {
    let y = terrain::ground(seed, haven, x, z);
    let gx =
        (terrain::ground(seed, haven, x + 1.0, z) - terrain::ground(seed, haven, x - 1.0, z)) * 0.5;
    let gz =
        (terrain::ground(seed, haven, x, z + 1.0) - terrain::ground(seed, haven, x, z - 1.0)) * 0.5;
    (y, gx, gz)
}

/// Whether nothing authored owns this ground: sites, the haven and the build
/// pads keep their carved faces clean.
fn open_ground(haven: &Haven, x: f32, z: f32) -> bool {
    !terrain::in_haven(haven, x, z) && terrain::site_sweep(haven, x, z) <= 0.0
}

/// The skin, the teeth and the scree of one cell, as one rock soup, and the
/// grass on the skin's shelves.
pub(super) fn cell_soup(seed: u64, haven: &Haven, cx: i32, cz: i32) -> (RockSoup, Soup) {
    let mut soup = RockSoup::default();
    let mut tufts = Soup::default();
    cliff_skin::skin(&mut soup, &mut tufts, seed, haven, CLIFF_CELL_M, cx, cz);
    let n = (CLIFF_CELL_M / STEP_M) as i32;
    let cell_key = (cx as u32).wrapping_mul(73_856_093) ^ (cz as u32).wrapping_mul(19_349_663);
    let seed_key = seed as u32 ^ (seed >> 32) as u32;
    let mut lat = terrain::Lattice::new();
    for j in 0..n {
        for i in 0..n {
            let k = cell_key ^ ((j * n + i) as u32).wrapping_mul(0x2545_F491) ^ seed_key;
            let h = |c: u32| h01(k, c);
            let x = (cx as f32 + (i as f32 + h(1)) / n as f32) * CLIFF_CELL_M;
            let z = (cz as f32 + (j as f32 + h(2)) / n as f32) * CLIFF_CELL_M;
            let (y, gx, gz) = ground_at(seed, haven, x, z);
            if y < terrain::SEA_LEVEL - 0.5 {
                continue;
            }
            let s = (gx * gx + gz * gz).sqrt();
            if s < 1e-3 {
                continue;
            }
            // Downhill, along the fall line.
            let (dx, dz) = (-gx / s, -gz / s);
            if s >= terrain::CLIFF_SLOPE_RATIO {
                let crag = crag(x, z, seed_key);
                if !open_ground(haven, x, z) {
                    continue;
                }
                // Just under the lip, a tooth standing above it breaks the
                // outline the walkable top would otherwise draw.
                let lip = lip_above(seed, haven, x, z, (dx, dz));
                if let Some(lip) = lip {
                    if h(5) < TOOTH_SHARE * crag {
                        tooth(&mut soup, &mut lat, seed, haven, (x, z), lip, (dx, dz), k);
                    }
                }
            } else if s <= SCREE_SLOPE_MAX && h(4) < SCREE_SHARE {
                // A cliff uphill: the stones that fell off it.
                let (ux, uz) = (x - dx * SCREE_REACH_M, z - dz * SCREE_REACH_M);
                if terrain::ground_slope(seed, haven, ux, uz) >= terrain::CLIFF_SLOPE_RATIO
                    && open_ground(haven, x, z)
                {
                    scree(&mut soup, &mut lat, seed, haven, x, z, (dx, dz), k);
                }
            }
        }
    }
    (soup, tufts)
}

/// Turf on a tooth's top, keyed by `h` in [0, 1): most in the green band
/// carry it, as the rim of a grown-over face does; none on the beach or past
/// the treeline.
fn tooth_moss(y: f32, h: f32) -> f32 {
    if y <= terrain::BEACH_MAX_H + 1.5 || y >= terrain::TREELINE_H || h >= TOOTH_MOSS_SHARE {
        0.0
    } else {
        TOOTH_MOSS
    }
}

/// How deep the planes that break a cliff rock's edges cut, metres: a share
/// of its smallest extent, so a crag loses its corners and not its shape.
fn chisel_m(hx: f32, hz: f32, face: f32) -> f32 {
    (CHISEL_SHARE * hx.min(hz).min(face)).min(CHISEL_MAX_M)
}

/// Where the crags gather, 0..1: smooth value noise at [`CRAG_M`], pushed
/// toward its ends.
fn crag(x: f32, z: f32, seed: u32) -> f32 {
    let (fx, fz) = (x / CRAG_M, z / CRAG_M);
    let (ix, iz) = (fx.floor(), fz.floor());
    let (tx, tz) = (fx - ix, fz - iz);
    let (ux, uz) = (tx * tx * (3.0 - 2.0 * tx), tz * tz * (3.0 - 2.0 * tz));
    let c = |dx: i32, dz: i32| {
        h01(
            ((ix as i32 + dx) as u32).wrapping_mul(73_856_093)
                ^ ((iz as i32 + dz) as u32).wrapping_mul(83_492_791),
            seed ^ 0xc2a9,
        )
    };
    let a = c(0, 0) + (c(1, 0) - c(0, 0)) * ux;
    let b = c(0, 1) + (c(1, 1) - c(0, 1)) * ux;
    let v = a + (b - a) * uz;
    ((v - 0.25) / 0.5).clamp(0.0, 1.0)
}

/// The lip above a steep point: the first walkable ground up the fall line
/// within [`TOOTH_REACH_M`], as `(distance, height)`.
fn lip_above(seed: u64, haven: &Haven, x: f32, z: f32, (dx, dz): (f32, f32)) -> Option<(f32, f32)> {
    let mut u = 0.5;
    while u <= TOOTH_REACH_M {
        let (px, pz) = (x - dx * u, z - dz * u);
        if terrain::ground_slope(seed, haven, px, pz) < terrain::CLIFF_SLOPE_RATIO {
            return Some((u, terrain::ground(seed, haven, px, pz)));
        }
        u += 0.5;
    }
    None
}

/// A crag standing in the top of a face, its head above the lip `lip`
/// (`(distance uphill, height)`) and its whole footprint on ground too steep
/// to walk, so the outline breaks where nobody can reach it.
#[allow(clippy::too_many_arguments)]
fn tooth(
    soup: &mut RockSoup,
    lat: &mut terrain::Lattice,
    seed: u64,
    haven: &Haven,
    (x, z): (f32, f32),
    (ul, y_lip): (f32, f32),
    (dx, dz): (f32, f32),
    k: u32,
) {
    let h = |c: u32| h01(k, c + 32);
    // Broad and low: a rocky rim, not a standing stone.
    let hx = 1.4 + 2.6 * h(1) * h(1);
    let hz = 0.35 + 0.3 * h(2);
    // Off the fall line a little, so a row of teeth does not line up.
    let a = (h(3) - 0.5) * 0.8;
    let (sa, ca) = a.sin_cos();
    let (bx, bz) = (dx * ca - dz * sa, dx * sa + dz * ca);
    // The uphill face keeps clear of the lip.
    let back = (TOOTH_CLEAR_M + hz - ul).max(0.0);
    let (cx, cz) = (x + dx * back, z + dz * back);
    let (ax, az) = (-bz, bx);
    let corners = [
        (1.0, 1.0),
        (1.0, -1.0),
        (-1.0, 1.0),
        (-1.0, -1.0),
        (0.0, -1.0),
    ];
    let mut foot = f32::INFINITY;
    for (u, v) in corners {
        // `v` runs downhill: `-1` is the uphill side.
        let (px, pz) = (
            cx + ax * hx * u + bx * hz * v,
            cz + az * hx * u + bz * hz * v,
        );
        if terrain::ground_slope(seed, haven, px, pz) < terrain::CLIFF_SLOPE_RATIO {
            return;
        }
        foot = foot.min(near_drawn_y(lat, seed, haven, px, pz));
    }
    let rise = TOOTH_RISE_MIN_M + (TOOTH_RISE_MAX_M - TOOTH_RISE_MIN_M) * h(4) * h(4) * h(4);
    if y_lip + rise - (foot - TOOTH_BURY_M) > TOOTH_TALL_MAX_M {
        return;
    }
    let moss = tooth_moss(y_lip, h(5));
    rock_block(
        soup,
        &RockShape {
            x: cx,
            z: cz,
            hx,
            hz,
            dir: (bx, bz),
            y0: foot - TOOTH_BURY_M,
            y1: y_lip + rise,
            tx: (h(6) - 0.5) * 0.5,
            // The top falls away toward the drop, as a weathered rim does.
            tz: -0.1 - 0.25 * h(7),
            key: k ^ 0x7007_7a11,
            moss,
            chisel: chisel_m(hx, hz, y_lip + rise - foot),
            min_seg: 2,
            value: CRAG_VALUE * (0.85 + 0.3 * h(8)),
        },
    );
}

/// A handful of fallen stones around `(x, z)`, spilled down the fall line.
#[allow(clippy::too_many_arguments)]
fn scree(
    soup: &mut RockSoup,
    lat: &mut terrain::Lattice,
    seed: u64,
    haven: &Haven,
    x: f32,
    z: f32,
    (dx, dz): (f32, f32),
    k: u32,
) {
    let count = 2 + (h01(k, 40) * 6.0) as u32;
    for m in 0..count {
        let h = |c: u32| h01(k ^ (m + 1).wrapping_mul(0x9E37_79B9), c + 48);
        // Spread across the slope, run out further down it.
        let across = (h(1) - 0.5) * 3.2;
        let down = (h(2) - 0.3) * 2.4;
        let (px, pz) = (x + dx * down - dz * across, z + dz * down + dx * across);
        let gy = near_drawn_y(lat, seed, haven, px, pz);
        let size = 0.1 + 0.32 * h(3) * h(3);
        let (sn, cs) = (h(4) * std::f32::consts::TAU).sin_cos();
        rock_block(
            soup,
            &RockShape {
                x: px,
                z: pz,
                hx: size,
                hz: size * (0.55 + 0.45 * h(5)),
                dir: (sn, cs),
                y0: gy - size * 0.7,
                y1: gy + size * (0.5 + 0.6 * h(6)),
                tx: (h(7) - 0.5) * 0.6,
                tz: (h(8) - 0.5) * 0.6,
                key: k ^ (m + 1).wrapping_mul(0x85EB_CA6B),
                moss: 0.0,
                chisel: 0.0,
                min_seg: 1,
                value: CRAG_VALUE * 0.8,
            },
        );
    }
}
