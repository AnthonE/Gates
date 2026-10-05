//! Cliff outcrops: rock ledges stepping down every face too steep to walk,
//! and the scree lying at their feet.
//!
//! A scarp is the smooth heightfield, so from any distance it read as one pale
//! sheet with a photograph on it (`NOW.md` §0rf items 2–3). This stands
//! fractured blocks in the face — [`boulders::rock_block`], on the ground's own
//! material, so a ledge is the same granite as the face it juts from. Each
//! block is buried uphill and shows a vertical downhill face with a short shelf
//! on top, so a cliff reads as stacked ledges rather than a ramp, and the
//! ledges throw shadow down the face.
//!
//! The blocks sit on the DRAWN face (`terrain_mesh::near_drawn_y`), which the
//! cliff relief moves off the sim's height; the slope tests stay the sim's.
//! The relief cannot touch the walkable top, so the lip's outline against the
//! sky is broken by low crag teeth standing in the face just under it, every
//! corner of each on ground too steep to walk.
//!
//! **Drawn, not decided: nothing here collides.** A block stands only where
//! the sim already refuses a body — slope past [`terrain::CLIFF_SLOPE_RATIO`]
//! at the block AND just below its face — and stands proud of the slope by at
//! most [`LEDGE_FACE_MAX_M`] / `CLIFF_SLOPE_RATIO` horizontally, so nobody can
//! walk into one. The scree is ankle-high, like the clutter a body already
//! wades through.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use sim_core::terrain::{self, Haven};

use super::boulders::{rock_block, RockShape, RockSoup};
use super::terrain_mesh::{near_drawn_y, Ring};
use super::{Eye, WorldEntity, WorldId};

/// One streamed cell's side, metres.
pub const CLIFF_CELL_M: f32 = 32.0;
/// Cells drawn either side of the eye's: ~220 m.
pub const CLIFF_RING: i32 = 7;
/// Candidate spacing inside a cell, metres.
const STEP_M: f32 = 4.0;
/// Cells built per frame; each is ~64 candidates of a few ground taps.
const BUILDS_PER_FRAME: usize = 2;
/// The share of steep candidates that grow a ledge where the crag field is
/// full; it falls to nothing where the field is empty, so a face carries
/// crags and smooth slabs between them rather than an even brick pattern.
const LEDGE_SHARE: f32 = 0.9;
/// The crag field's wavelength, metres.
const CRAG_M: f32 = 22.0;
/// Tallest visible downhill face, metres.
const LEDGE_FACE_MAX_M: f32 = 3.2;
/// Shortest visible downhill face, metres.
const LEDGE_FACE_MIN_M: f32 = 0.8;
/// A ledge's value against the face's: the same granite, freshly broken.
const LEDGE_VALUE: f32 = 0.92;
/// How far a ledge's foot is buried below the face, metres.
const LEDGE_BURY_M: f32 = 0.7;
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

#[derive(Resource, Default)]
pub struct CliffRing {
    seed: Option<u64>,
    built: HashMap<(i32, i32), Option<Entity>>,
}

/// Keep the cliff cells within [`CLIFF_RING`] of the eye built, nearest
/// first, and drop the ones that fall out of it.
pub fn stream(
    mut commands: Commands,
    mut ring: ResMut<CliffRing>,
    mut meshes: ResMut<Assets<Mesh>>,
    ground: Res<Ring>,
    world: Res<WorldId>,
    eye: Res<Eye>,
) {
    let Some(material) = ground.ground_material() else {
        return;
    };
    if ring.seed != Some(world.seed) {
        for (_, e) in ring.built.drain() {
            if let Some(e) = e {
                commands.entity(e).despawn();
            }
        }
        ring.seed = Some(world.seed);
    }
    let cx = (eye.pos.x / CLIFF_CELL_M).floor() as i32;
    let cz = (eye.pos.z / CLIFF_CELL_M).floor() as i32;
    ring.built.retain(|&(x, z), e| {
        let keep = (x - cx).abs() <= CLIFF_RING + 1 && (z - cz).abs() <= CLIFF_RING + 1;
        if !keep {
            if let Some(e) = e {
                commands.entity(*e).despawn();
            }
        }
        keep
    });
    let mut budget = BUILDS_PER_FRAME;
    for r in 0..=CLIFF_RING {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let key = (cx + dx, cz + dz);
                if ring.built.contains_key(&key) {
                    continue;
                }
                let soup = cell_soup(world.seed, &world.haven, key.0, key.1);
                let e = if soup.is_empty() {
                    None
                } else {
                    Some(
                        commands
                            .spawn((
                                WorldEntity,
                                Mesh3d(meshes.add(soup.mesh())),
                                MeshMaterial3d(material.clone()),
                                Transform::IDENTITY,
                            ))
                            .id(),
                    )
                };
                ring.built.insert(key, e);
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

/// Every ledge and scree pile of one cell, as one rock soup.
pub fn cell_soup(seed: u64, haven: &Haven, cx: i32, cz: i32) -> RockSoup {
    let mut soup = RockSoup::default();
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
                let toothed = lip.is_some_and(|lip| {
                    h(5) < TOOTH_SHARE * crag
                        && tooth(&mut soup, &mut lat, seed, haven, (x, z), lip, (dx, dz), k)
                });
                if !toothed && h(3) < LEDGE_SHARE * crag {
                    ledge(&mut soup, &mut lat, seed, haven, x, z, s, (dx, dz), k);
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
    soup
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
) -> bool {
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
            return false;
        }
        foot = foot.min(near_drawn_y(lat, seed, haven, px, pz));
    }
    let rise = TOOTH_RISE_MIN_M + (TOOTH_RISE_MAX_M - TOOTH_RISE_MIN_M) * h(4) * h(4) * h(4);
    if y_lip + rise - (foot - LEDGE_BURY_M) > TOOTH_TALL_MAX_M {
        return false;
    }
    let moss = y_lip > terrain::BEACH_MAX_H + 1.5 && y_lip < terrain::TREELINE_H && h(5) < 0.4;
    rock_block(
        soup,
        &RockShape {
            x: cx,
            z: cz,
            hx,
            hz,
            dir: (bx, bz),
            y0: foot - LEDGE_BURY_M,
            y1: y_lip + rise,
            tx: (h(6) - 0.5) * 0.5,
            // The top falls away toward the drop, as a weathered rim does.
            tz: -0.1 - 0.25 * h(7),
            key: k ^ 0x7007_7a11,
            moss,
            min_seg: 2,
            value: LEDGE_VALUE * (0.85 + 0.3 * h(8)),
        },
    );
    true
}

/// One ledge whose downhill face stands at `(x, z)`.
#[allow(clippy::too_many_arguments)]
fn ledge(
    soup: &mut RockSoup,
    lat: &mut terrain::Lattice,
    seed: u64,
    haven: &Haven,
    x: f32,
    z: f32,
    s: f32,
    (dx, dz): (f32, f32),
    k: u32,
) {
    let h = |c: u32| h01(k, c + 16);
    // Most ledges are modest and a few are big, the way a crag breaks.
    let face = LEDGE_FACE_MIN_M + (LEDGE_FACE_MAX_M - LEDGE_FACE_MIN_M) * h(3) * h(3);
    let hx = 1.4 + 3.6 * h(1) * h(1) + face * 0.5;
    // Tops lean along the contour as bedding planes do, which raises one end
    // of the face by up to `|tx| hx`.
    let tx = (h(5) - 0.5) * 0.4;
    let tallest = face + tx.abs() * hx;
    // The ground below the face, out past where the ledge stands proud of
    // the slope, must refuse a body too, or the ledge would stand on
    // walkable foot-slope where a player could reach it.
    let reach = tallest / s + 1.0;
    let (fx, fz) = (x + dx * reach, z + dz * reach);
    if terrain::ground_slope(seed, haven, fx, fz) < terrain::CLIFF_SLOPE_RATIO {
        return;
    }
    // Deep enough that the shelf's uphill end is buried: the ground climbs
    // `2 hz s` across it, against the face's height.
    let hz = (tallest / (2.0 * s) + 0.4).max(0.8 + 0.8 * h(2));
    // Off the fall line a little, so no two ledges share a bearing.
    let a = (h(7) - 0.5) * 0.7;
    let (sa, ca) = a.sin_cos();
    let (dx, dz) = (dx * ca - dz * sa, dx * sa + dz * ca);
    // The block's local +Z points downhill; its centre sits `hz` uphill of
    // the face.
    let (ccx, ccz) = (x - dx * hz, z - dz * hz);
    // The contour either side of the face: a block over a gully must reach
    // down to the lowest of them, or it hangs in the air at one end.
    let (ax, az) = (-dz, dx);
    let y = near_drawn_y(lat, seed, haven, x, z);
    let yl = near_drawn_y(lat, seed, haven, x + ax * hx, z + az * hx);
    let yr = near_drawn_y(lat, seed, haven, x - ax * hx, z - az * hx);
    // A ledge across a gully would show its whole side; leave that ground
    // to the face.
    if y - yl.min(yr) > face {
        return;
    }
    let y0 = y.min(yl).min(yr) - LEDGE_BURY_M;
    let y1 = y + face;
    // The relief can cut the face back behind the block; a block whose back
    // would stand a metre out of the rock is left out.
    let (bx, bz) = (x - dx * 2.0 * hz, z - dz * 2.0 * hz);
    if near_drawn_y(lat, seed, haven, bx, bz) < y1 - 1.0 {
        return;
    }
    let moss = y > terrain::BEACH_MAX_H + 1.5 && y < terrain::TREELINE_H && h(4) < 0.35;
    rock_block(
        soup,
        &RockShape {
            x: ccx,
            z: ccz,
            hx,
            hz,
            dir: (dx, dz),
            y0,
            y1,
            tx,
            tz: (h(6) - 0.5) * 0.25,
            key: k ^ 0x51ed_270b,
            moss,
            min_seg: 2,
            value: LEDGE_VALUE * (0.85 + 0.3 * h(8)),
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
                moss: false,
                min_seg: 1,
                value: LEDGE_VALUE * 0.8,
            },
        );
    }
}
