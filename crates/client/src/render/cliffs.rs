//! Cliff outcrops: rock ledges stepping down every face too steep to walk,
//! and the scree lying at their feet.
//!
//! A scarp is the smooth heightfield, so from any distance it read as one pale
//! sheet with a photograph on it (`NOW.md` §0rf items 2–3). This stands
//! fractured blocks in the face — [`boulders::rock_block`], on the ground's own
//! material, so a ledge is the same granite as the face it juts from. Each
//! block is buried uphill and shows a vertical downhill face with a short shelf
//! on top, so a cliff reads as stacked ledges rather than a ramp, and the
//! ledges throw shadow down the face. Low crag teeth standing in the face just
//! under the lip break the outline the walkable top draws against the sky.
//!
//! **The sim places and collides them** (`sim_core::cliff`, since 2026-10-07:
//! a player walked through a tooth, because a body may walk down and along a
//! face). This only draws each crag, its foot buried in the DRAWN face
//! (`terrain_mesh::near_drawn_y`), which the cliff relief moves off the sim's
//! height. A ledge is drawn down the face by the relief's offset there, as the
//! face is, so a body on the face is as far off the ledge as off the rock
//! beside it; a tooth's top is the lip's, which the relief never moves. The
//! scree is ankle-high and the client's alone, like the clutter a body already
//! wades through.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use sim_core::cliff::{self, h01, Spot, LEDGE_BURY_M, SPOT_M};
use sim_core::terrain::{self, Haven};

use super::boulders::{rock_block, RockShape, RockSoup};
use super::terrain_mesh::{near_drawn_y, Ring};
use super::{Eye, WorldEntity, WorldId};

/// One streamed cell's side, metres: a whole number of the sim's spots.
pub const CLIFF_CELL_M: f32 = 32.0;
/// Cells drawn either side of the eye's: ~220 m.
pub const CLIFF_RING: i32 = 7;
/// Cells built per frame; each is 64 spots of a few ground taps.
const BUILDS_PER_FRAME: usize = 2;
/// A ledge's value against the face's: the same granite, freshly broken.
const LEDGE_VALUE: f32 = 0.92;
/// Where scree may lie: walkable ground at most this steep…
const SCREE_SLOPE_MAX: f32 = 0.95;
/// …with a cliff this many metres uphill of it.
const SCREE_REACH_M: f32 = 6.0;
/// The share of foot spots that carry a pile.
const SCREE_SHARE: f32 = 0.8;

const _: () = assert!(CLIFF_CELL_M % SPOT_M == 0.0);

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

/// Every crag and scree pile of one cell, as one rock soup.
pub fn cell_soup(seed: u64, haven: &Haven, cx: i32, cz: i32) -> RockSoup {
    let mut soup = RockSoup::default();
    let n = (CLIFF_CELL_M / SPOT_M) as i32;
    let mut lat = terrain::Lattice::new();
    for j in 0..n {
        for i in 0..n {
            let sp = cliff::spot(&mut lat, seed, haven, cx * n + i, cz * n + j);
            if sp.crag.is_some() {
                crag(&mut soup, &mut lat, seed, haven, &sp);
            } else if sp.s > 0.0 && sp.s <= SCREE_SLOPE_MAX && h01(sp.key, 4) < SCREE_SHARE {
                // A cliff uphill: the stones that fell off it.
                let (ux, uz) = (sp.x - sp.dx * SCREE_REACH_M, sp.z - sp.dz * SCREE_REACH_M);
                if terrain::ground_slope(seed, haven, ux, uz) >= terrain::CLIFF_SLOPE_RATIO
                    && cliff::open_ground(haven, sp.x, sp.z)
                {
                    scree(
                        &mut soup,
                        &mut lat,
                        seed,
                        haven,
                        sp.x,
                        sp.z,
                        (sp.dx, sp.dz),
                        sp.key,
                    );
                }
            }
        }
    }
    soup
}

/// The sim's crag at `sp`, drawn with its foot in the drawn face.
fn crag(soup: &mut RockSoup, lat: &mut terrain::Lattice, seed: u64, haven: &Haven, sp: &Spot) {
    let c = &sp.crag;
    let drawn = |lat: &mut terrain::Lattice, lx: f32, lz: f32| {
        let (x, z) = c.to_world(lx, lz);
        near_drawn_y(lat, seed, haven, x, z)
    };
    let (y0, y1) = if c.tooth {
        // Buried under the lowest of the face beneath it.
        let corners = [
            (1.0, 1.0),
            (1.0, -1.0),
            (-1.0, 1.0),
            (-1.0, -1.0),
            (0.0, -1.0),
        ];
        let foot = corners
            .iter()
            .map(|&(u, v)| drawn(lat, u * c.hx, v * c.hz))
            .fold(f32::INFINITY, f32::min);
        (foot - LEDGE_BURY_M, c.y1)
    } else {
        // The face it juts from, and the contour either side of it.
        let y = drawn(lat, 0.0, c.hz);
        let yl = drawn(lat, c.hx, c.hz);
        let yr = drawn(lat, -c.hx, c.hz);
        let (fx, fz) = c.to_world(0.0, c.hz);
        let relief = y - terrain::ground(seed, haven, fx, fz);
        (y.min(yl).min(yr) - LEDGE_BURY_M, c.y1 + relief)
    };
    rock_block(
        soup,
        &RockShape {
            x: c.x,
            z: c.z,
            hx: c.hx,
            hz: c.hz,
            dir: c.dir,
            y0,
            y1,
            tx: c.tx,
            tz: c.tz,
            key: sp.key ^ if c.tooth { 0x7007_7a11 } else { 0x51ed_270b },
            moss: c.moss,
            min_seg: 2,
            value: LEDGE_VALUE * (0.85 + 0.3 * h01(sp.key, 40)),
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
