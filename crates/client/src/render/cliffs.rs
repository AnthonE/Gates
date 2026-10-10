//! Cliffs: the rock skin over every face too steep to walk
//! (`cliff_skin.rs`) and the grass on its shelves; the crags — rock ledges
//! stepping down the faces, and teeth standing in a face just under its lip,
//! breaking the outline the walkable top draws against the sky; and the
//! scree lying at the faces' feet.
//!
//! A scarp is the smooth heightfield, so from any distance it read as one pale
//! sheet with a photograph on it (`NOW.md` §0rf items 2–3). The skin breaks
//! the whole face into planes, strata, shelves and overhangs; the crags are
//! fractured blocks ([`boulders::rock_block`], on the ground's own material,
//! so a ledge is the same granite as the face it juts from), and the skin
//! falls back round each one so it stands out of the face rather than being
//! swallowed by it.
//!
//! **The sim places and collides the crags** (`sim_core::cliff`, since
//! 2026-10-07: a player walked through a tooth, because a body may walk down
//! and along a face). This only draws each crag, its foot buried in the DRAWN
//! face (`terrain_mesh::near_drawn_y`), which the cliff relief moves off the
//! sim's height. A ledge is drawn down the face by the relief's offset there,
//! as the face is, so a body on the face is as far off the ledge as off the
//! rock beside it; a tooth's top is the lip's, which the relief never moves.
//! Its edges are chipped by at most [`CRAG_CHISEL_MAX_M`], so what is drawn is
//! what is stood on. The skin and the scree are the client's alone: the skin
//! stands out only over faces too steep to climb, and the scree is
//! ankle-high, like the clutter a body already wades through.

use bevy::light::NotShadowCaster;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};
use sim_core::cliff::{self, h01, Spot, CRAG_REACH_M, LEDGE_BURY_M, SPOT_M};
use sim_core::terrain::{self, Haven};

use super::boulders::{rock_block, RockShape, RockSoup};
use super::cliff_skin::{self, CRAG_FADE_M};
use super::clutter::ClutterRing;
use super::props::Soup;
use super::terrain_mesh::{near_drawn_y, Ring};
use super::{Eye, WorldEntity, WorldId};

/// One streamed cell's side, metres: a whole number of the sim's spots.
pub const CLIFF_CELL_M: f32 = 32.0;
/// Cells drawn either side of the eye's: ~220 m.
pub const CLIFF_RING: i32 = 7;
/// Cells handed to the pool per frame; each is the skin's half-metre lattice
/// where the cell is steep, and 64 spots of a few ground taps.
const QUEUES_PER_FRAME: usize = 2;
/// Finished cells landed per frame.
const LANDS_PER_FRAME: usize = 2;
/// A ledge's value against the face's: the same granite, freshly broken.
const LEDGE_VALUE: f32 = 0.92;
/// Where scree may lie: walkable ground at most this steep…
const SCREE_SLOPE_MAX: f32 = 0.95;
/// …with a cliff this many metres uphill of it.
const SCREE_REACH_M: f32 = 6.0;
/// The share of foot spots that carry a pile.
const SCREE_SHARE: f32 = 0.8;
/// How lush the turf on a mossy crag's top is (`RockShape::moss`).
const CRAG_MOSS: f32 = 1.5;
/// Chisel depth as a share of a crag's smallest extent…
const CRAG_CHISEL_SHARE: f32 = 0.5;
/// …and never deeper, metres: a crag is solid, and a corner cut off is air a
/// body stands on.
const CRAG_CHISEL_MAX_M: f32 = 0.4;

const _: () = assert!(CLIFF_CELL_M % SPOT_M == 0.0);

/// A built cell's two meshes: its rock, and the grass on its shelves.
type CellMeshes = (Option<Mesh>, Option<Mesh>);

#[derive(Resource, Default)]
pub struct CliffRing {
    seed: Option<u64>,
    built: HashMap<(i32, i32), Option<Entity>>,
    /// Cells being built on the pool.
    tasks: HashMap<(i32, i32), Task<CellMeshes>>,
    /// The eye's cell, once a walk around it found every cell of the ring
    /// built or queued: until the eye leaves it, the retains and the probe
    /// have nothing to do and are skipped (`boulders::RockRing::settled`).
    /// Landing is not — a queued cell still lands into `built`, which keeps
    /// it in the ring.
    settled: Option<(i32, i32)>,
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
        ring.settled = None;
    }
    let cx = (eye.pos.x / CLIFF_CELL_M).floor() as i32;
    let cz = (eye.pos.z / CLIFF_CELL_M).floor() as i32;
    let settled = ring.settled == Some((cx, cz));
    if !settled {
        ring.settled = None;
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
    }

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

    if settled {
        return;
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
    ring.settled = Some((cx, cz));
}

/// A new world: forget the cells (their entities are `WorldEntity`).
pub fn teardown(mut ring: ResMut<CliffRing>) {
    *ring = CliffRing::default();
}

/// Every crag and scree pile of one cell and the skin over its faces, as one
/// rock soup, and the grass on the skin's shelves.
pub(super) fn cell_soup(seed: u64, haven: &Haven, cx: i32, cz: i32) -> (RockSoup, Soup) {
    let mut soup = RockSoup::default();
    let mut tufts = Soup::default();
    let n = (CLIFF_CELL_M / SPOT_M) as i32;
    let mut lat = terrain::Lattice::new();
    if cliff_skin::steep_cell(&mut lat, seed, haven, CLIFF_CELL_M, cx, cz) {
        // Every crag that reaches within the skin's fade of the cell, its
        // neighbours' included, so the skin falls back round a crag on
        // either side of a cell edge alike.
        let r = ((CRAG_REACH_M + CRAG_FADE_M) / SPOT_M).ceil() as i32;
        let mut crags = Vec::new();
        for j in cz * n - r..(cz + 1) * n + r {
            for i in cx * n - r..(cx + 1) * n + r {
                let c = cliff::crag(&mut lat, seed, haven, i, j);
                if c.is_some() {
                    crags.push(c);
                }
            }
        }
        cliff_skin::skin(
            &mut soup,
            &mut tufts,
            seed,
            haven,
            &crags,
            CLIFF_CELL_M,
            cx,
            cz,
        );
    }
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
    (soup, tufts)
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
            moss: if c.moss { CRAG_MOSS } else { 0.0 },
            chisel: (CRAG_CHISEL_SHARE * c.hx.min(c.hz).min(y1 - y0)).min(CRAG_CHISEL_MAX_M),
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
                moss: 0.0,
                chisel: 0.0,
                min_seg: 1,
                value: LEDGE_VALUE * 0.8,
            },
        );
    }
}
