//! What a blow landed on: which way the surface faces and what it is made of.
//!
//! The wire says only *ground / world / built* and a point (`ranged::SURF_*`).
//! Everything else an effect needs is read here out of state the client
//! already holds and already trusts — worldgen (`terrain::splat`, the
//! scatter), the piece mirror and the deploy mirror — and resolved with the
//! sim's own queries (`collide::shot_hit`, `terrain::slot_blocks`) rather
//! than a second opinion about the geometry. Nothing here decides whether a
//! blow landed; it only says where to draw the answer and in what colour.
//!
//! One resolver, every reader: `impact::contacts` calls it once per blow and
//! the particles, the decal and the sound all take its answer, so a wooden
//! wall cannot throw stone sparks while playing a wood thock.

use bevy::prelude::*;

use super::impact::Matter;
use super::WorldId;
use client_core::core::ClientCore;
use sim_core::build::{self, BUILD_CELL_M};
use sim_core::collide::{self, ColIndex, PieceHit, PLANE_THICKNESS_M};
use sim_core::deploy::{
    ARCH_BAG, ARCH_DOOR, ARCH_FIRE, ARCH_FURNACE, ARCH_GARAGE_DOOR, ARCH_LOCK, ARCH_RECYCLER,
    ARCH_RESEARCH, ARCH_WINDOW_BARS, ARCH_WINDOW_GLASS, ARCH_WORKBENCH2, ARCH_WORKBENCH3,
};
use sim_core::limits::{MAX_BUILD_COORD, MAX_BUILD_SOCKETS};
use sim_core::ranged::{SURF_BUILT, SURF_GROUND, SURF_WORLD};
use sim_core::terrain::{self, Occupant, Slot};

/// The radius the material probes use, metres — an arrowhead's order, so a
/// probe meets the one piece the impact point is on and not its neighbour.
const PROBE_R_M: f32 = 0.05;

/// Which way the surface at `(x, y, z)` faces, given what the sim said it is.
///
/// - **Ground** is exact: the carved terrain's gradient by central
///   differences on `terrain::ground`, the function the mesh is built from.
/// - **A world occupant** is exact too: a trunk's axis is its slot's own
///   `x`/`z`, so the outward normal is the horizontal from that axis.
/// - **A built piece** asks [`plane_face`] whether the altitude puts it on a
///   slab's face; otherwise a wall lies on its cell's edge, and the offset
///   from the cell centre points at that edge.
pub fn normal_at(world: &WorldId, cols: &ColIndex, x: f32, y: f32, z: f32, surf: u8) -> Vec3 {
    match surf {
        SURF_GROUND => {
            // One arrow-radius of separation: closer is float noise, wider
            // is the hill rather than the ground under the mark.
            const H: f32 = 0.25;
            let dx = terrain::ground(world.seed, &world.haven, x + H, z)
                - terrain::ground(world.seed, &world.haven, x - H, z);
            let dz = terrain::ground(world.seed, &world.haven, x, z + H)
                - terrain::ground(world.seed, &world.haven, x, z - H);
            Vec3::new(-dx, 2.0 * H, -dz).normalize_or(Vec3::Y)
        }
        SURF_WORLD => match world_slot(world, Vec3::new(x, y, z)) {
            // Horizontal: the side of a standing thing. `normalize_or`
            // covers an impact on the axis itself (a shot from straight up).
            Some(slot) => Vec3::new(x - slot.x, 0.0, z - slot.z).normalize_or(Vec3::Y),
            None => Vec3::Y,
        },
        _ => {
            if let Some(n) = plane_face(world, cols, x, y, z) {
                return n;
            }
            let cell = BUILD_CELL_M;
            let ox = x - (x / cell).floor() * cell - cell * 0.5;
            let oz = z - (z / cell).floor() * cell - cell * 0.5;
            if ox.abs() >= oz.abs() {
                Vec3::new(ox.signum(), 0.0, 0.0)
            } else {
                Vec3::new(0.0, 0.0, oz.signum())
            }
        }
    }
}

/// Whether the impact at (`x`, `y`, `z`) landed on the **face** of a built
/// plane, and which way that face points — `None` when the altitude cannot
/// say so without guessing.
///
/// A plane's top is `column_floor_y(cell, plate) + level_y(level)` — the
/// predictor's own `ColIndex` — and the slab hangs `PLANE_THICKNESS_M` below
/// it. Within `EDGE_M` of the cell boundary a rim hit is as reachable as a
/// face hit and nothing distinguishes them, so it declines there and the
/// caller's edge snap answers.
pub fn plane_face(world: &WorldId, cols: &ColIndex, x: f32, y: f32, z: f32) -> Option<Vec3> {
    const EDGE_M: f32 = 0.25;
    // An arrow stops at the first sample inside the band, so its height
    // overshoots by up to one tap.
    const SLAB_FACE_TOL_M: f32 = 0.25;

    let (bx, bz) = (build::build_cell_of(x), build::build_cell_of(z));
    if bx < 0 || bz < 0 || bx >= MAX_BUILD_COORD as i32 || bz >= MAX_BUILD_COORD as i32 {
        return None;
    }
    let (bx, bz) = (bx as u16, bz as u16);
    let m = cols.get(bx, bz);
    let tris = m.tri_xlo_zlo | m.tri_xhi_zlo | m.tri_xlo_zhi | m.tri_xhi_zhi;
    if m.planes == 0 && tris == 0 {
        return None;
    }
    let cxm = bx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5;
    let czm = bz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5;
    if (x - cxm).abs() > BUILD_CELL_M * 0.5 - EDGE_M
        || (z - czm).abs() > BUILD_CELL_M * 0.5 - EDGE_M
    {
        return None;
    }
    let base = build::column_floor_y(
        world.seed,
        &world.haven,
        bx,
        bz,
        cols.plate(bx, bz).unwrap_or(0),
    );
    for level in 0..MAX_BUILD_SOCKETS {
        let bit = 1u16 << level;
        if (m.planes | tris) & bit == 0 {
            continue;
        }
        let top = base + build::level_y(level as u8);
        if y > top + SLAB_FACE_TOL_M {
            continue;
        }
        // Level 0 is solid to the ground: a top face and no underside.
        if level == 0 {
            if y >= top - SLAB_FACE_TOL_M {
                return Some(Vec3::Y);
            }
            continue;
        }
        let bottom = top - PLANE_THICKNESS_M;
        if y < bottom - SLAB_FACE_TOL_M {
            continue;
        }
        // Inside the band: the NEARER face, so the answer does not depend on
        // which test was written first.
        return Some(if top - y <= y - bottom {
            Vec3::Y
        } else {
            Vec3::NEG_Y
        });
    }
    None
}

/// What the surface at `at` is made of. `n` is [`normal_at`]'s answer for the
/// same point — the built probe crosses the surface along it.
pub fn matter_at(world: &WorldId, core: &ClientCore, at: Vec3, surf: u8, n: Vec3) -> Matter {
    match surf {
        SURF_GROUND => ground_matter(
            terrain::splat(world.seed, at.x, at.z),
            at.y <= terrain::SEA_LEVEL,
        ),
        SURF_WORLD => world_matter(world, at),
        SURF_BUILT => built_matter(world, core, at, n),
        _ => Matter::Dirt,
    }
}

/// The ground's matter from its splat weights — `sound::steps::surface_cue`'s
/// reading exactly (sand · grass · forest litter · rock, argmax), so the dust
/// a bullet kicks up and the footstep on the same spot agree. Under the sea
/// the answer is the water above it.
pub fn ground_matter(splat: [u8; 4], below_sea: bool) -> Matter {
    if below_sea {
        return Matter::Water;
    }
    let mut best = 0usize;
    for (i, w) in splat.iter().enumerate() {
        if *w > splat[best] {
            best = i;
        }
    }
    match best {
        0 => Matter::Sand,
        1 => Matter::Grass,
        2 => Matter::Dirt,
        _ => Matter::Stone,
    }
}

/// The scatter slot a world-surface impact stopped on: the one whose volume
/// holds the point, over the 3×3 cells around it (a trunk near a cell edge
/// belongs to the neighbour's slot), else the nearest occupied slot.
pub fn world_slot(world: &WorldId, at: Vec3) -> Option<Slot> {
    let (feet, r, h) = (at.y - PROBE_R_M * 3.0, PROBE_R_M * 3.0, PROBE_R_M * 6.0);
    let (pcx, pcz) = (
        (at.x / terrain::CELL_SIZE).floor() as i32,
        (at.z / terrain::CELL_SIZE).floor() as i32,
    );
    let mut nearest: Option<(f32, Slot)> = None;
    for dz in -terrain::OCCUPANT_PROBE_CELLS..=terrain::OCCUPANT_PROBE_CELLS {
        for dx in -terrain::OCCUPANT_PROBE_CELLS..=terrain::OCCUPANT_PROBE_CELLS {
            let slot = terrain::scatter(world.seed, &world.table, &world.haven, pcx + dx, pcz + dz);
            if slot.occupant == Occupant::None {
                continue;
            }
            if terrain::slot_blocks(&slot, at.x, at.z, feet, r, h) {
                return Some(slot);
            }
            let d = (slot.x - at.x).powi(2) + (slot.z - at.z).powi(2);
            if nearest.is_none_or(|(best, _)| d < best) {
                nearest = Some((d, slot));
            }
        }
    }
    nearest.map(|(_, s)| s)
}

/// What stands at a world-surface impact: [`world_slot`]'s occupant, or a
/// depot's concrete, or dirt when nothing can be named.
pub fn world_matter(world: &WorldId, at: Vec3) -> Matter {
    let (feet, r, h) = (at.y - PROBE_R_M * 3.0, PROBE_R_M * 3.0, PROBE_R_M * 6.0);
    if sim_core::depot::blocks(&world.haven, at.x, at.z, feet, r, h) {
        return Matter::Stone;
    }
    world_slot(world, at).map_or(Matter::Dirt, |s| Matter::of_occupant(s.occupant as u8))
}

/// What a built surface is made of: the piece the sim's own shot query finds
/// when it crosses the surface at `at` along `n`, and that piece's tier — or
/// the deployable standing there. Twig is wood; an unresolvable surface is
/// wood too, because every piece enters the world as twig.
pub fn built_matter(world: &WorldId, core: &ClientCore, at: Vec3, n: Vec3) -> Matter {
    let cols = core.pieces.cols();
    // Across the surface along the normal, then against it (a normal that
    // points the wrong way still crosses the same edge), then the point.
    let probes = [
        (at + n * 0.4, at - n * 0.15),
        (at - n * 0.4, at + n * 0.15),
        (at, at),
    ];
    for (a, b) in probes {
        let Some((hit, insert)) = collide::shot_hit(
            world.seed,
            &world.haven,
            cols,
            a.x,
            a.z,
            b.x,
            b.z,
            at.y,
            PROBE_R_M,
        ) else {
            continue;
        };
        // An insert (a door in its frame) is the deployable at the address.
        let found = if insert {
            deploy_matter(core, hit).or_else(|| piece_matter(core, hit))
        } else {
            piece_matter(core, hit).or_else(|| deploy_matter(core, hit))
        };
        if let Some(m) = found {
            return m;
        }
    }
    collide::deploy_stop(world.seed, &world.haven, cols, at.x, at.z, at.y, PROBE_R_M)
        .and_then(|hit| deploy_matter(core, hit))
        .unwrap_or(Matter::Wood)
}

fn piece_matter(core: &ClientCore, hit: PieceHit) -> Option<Matter> {
    let r =
        core.pieces.entries().iter().find(|r| {
            r.cx == hit.cx && r.cz == hit.cz && r.level == hit.level && r.loc == hit.loc
        })?;
    ((r.row as u16) < core.piece_defs_have)
        .then(|| Matter::of_piece(core.piece_defs.pieces[r.row as usize].material))
}

fn deploy_matter(core: &ClientCore, hit: PieceHit) -> Option<Matter> {
    let r =
        core.deploys.entries().iter().find(|r| {
            r.cx == hit.cx && r.cz == hit.cz && r.level == hit.level && r.loc == hit.loc
        })?;
    if (r.row as u16) >= core.deploy_defs_have {
        return None;
    }
    let def = &core.deploy_defs.defs[r.row as usize];
    Some(arch_matter(def.arch, def.hp))
}

/// A deployable's matter by archetype. `DeployDef` carries no material, and
/// the one archetype that comes in two (the door) is told apart by its hp:
/// `content/deployables.toml` gives the wooden door 200 and the metal 250.
pub fn arch_matter(arch: u8, hp: u16) -> Matter {
    match arch {
        ARCH_FIRE | ARCH_FURNACE | ARCH_WINDOW_GLASS => Matter::Stone,
        ARCH_LOCK | ARCH_RECYCLER | ARCH_RESEARCH | ARCH_WORKBENCH2 | ARCH_WORKBENCH3
        | ARCH_WINDOW_BARS | ARCH_GARAGE_DOOR => Matter::Metal,
        ARCH_DOOR if hp >= 250 => Matter::Metal,
        ARCH_BAG => Matter::Dirt,
        _ => Matter::Wood,
    }
}

/// The height of whatever a body standing over `at` stands on: the carved
/// ground, or the top of the highest built slab at or below `at`. Where blood
/// under a hit lands.
pub fn floor_below(world: &WorldId, cols: &ColIndex, at: Vec3) -> f32 {
    let ground = terrain::ground(world.seed, &world.haven, at.x, at.z);
    let (bx, bz) = (build::build_cell_of(at.x), build::build_cell_of(at.z));
    if bx < 0 || bz < 0 || bx >= MAX_BUILD_COORD as i32 || bz >= MAX_BUILD_COORD as i32 {
        return ground;
    }
    let (bx, bz) = (bx as u16, bz as u16);
    let m = cols.get(bx, bz);
    let tris = m.tri_xlo_zlo | m.tri_xhi_zlo | m.tri_xlo_zhi | m.tri_xhi_zhi;
    if m.planes | tris == 0 {
        return ground;
    }
    let base = build::column_floor_y(
        world.seed,
        &world.haven,
        bx,
        bz,
        cols.plate(bx, bz).unwrap_or(0),
    );
    let mut best = ground;
    for level in 0..MAX_BUILD_SOCKETS {
        if (m.planes | tris) & (1u16 << level) == 0 {
            continue;
        }
        let top = base + build::level_y(level as u8);
        if top <= at.y + 0.1 && top > best {
            best = top;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seed the shard ships and every capture is shot on.
    const SEED: u64 = 20260731;

    /// A ground normal points broadly up, is unit, and actually follows the
    /// terrain somewhere rather than being `Vec3::Y` with extra steps.
    #[test]
    fn a_ground_normal_takes_the_terrain_it_lies_on() {
        let world = WorldId::new(SEED);
        let empty = ColIndex::new();
        let n = normal_at(&world, &empty, 1024.0, 0.0, 1024.0, SURF_GROUND);
        assert!(
            n.y > 0.9,
            "a ground normal must point broadly up, got {n:?}"
        );
        assert!((n.length() - 1.0).abs() < 1e-3);
        let tilted = (0..64)
            .filter(|i| {
                let (x, z) = (300.0 + *i as f32 * 20.0, 700.0 + *i as f32 * 11.0);
                normal_at(&world, &empty, x, 0.0, z, SURF_GROUND).y < 0.999
            })
            .count();
        assert!(
            tilted > 0,
            "no sample tilted — the terrain is being ignored"
        );
    }

    /// A mark on a standing thing faces out of it, horizontally.
    #[test]
    fn a_world_normal_faces_out_of_the_thing_it_hit() {
        let world = WorldId::new(SEED);
        let empty = ColIndex::new();
        let n = normal_at(&world, &empty, 512.3, 0.0, 733.7, SURF_WORLD);
        assert_eq!(n.y, 0.0, "a trunk's side has no vertical lean");
        assert!((n.length() - 1.0).abs() < 1e-3);
    }

    /// A floor faces up (its underside down), a wall stands up, and a rim at
    /// the cell boundary declines rather than guessing.
    #[test]
    fn a_floor_faces_up_and_a_wall_stands_up() {
        use sim_core::build::{LEVEL_H_M, LOC_PLANE, SHAPE_FLOOR, SHAPE_FOUNDATION};

        let world = WorldId::new(SEED);
        let mut cols = Box::new(ColIndex::new());
        let (cx, cz) = (341u16, 341u16);
        cols.add(cx, cz, 0, LOC_PLANE, SHAPE_FOUNDATION, 0);
        cols.add(cx, cz, 1, LOC_PLANE, SHAPE_FLOOR, 0);
        let base = build::column_floor_y(world.seed, &world.haven, cx, cz, 0);
        let (mx, mz) = (
            cx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
            cz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
        );
        assert_eq!(normal_at(&world, &cols, mx, base, mz, SURF_BUILT), Vec3::Y);
        let up = normal_at(&world, &cols, mx, base + LEVEL_H_M, mz, SURF_BUILT);
        assert_eq!(up, Vec3::Y);
        let down = normal_at(
            &world,
            &cols,
            mx,
            base + LEVEL_H_M - PLANE_THICKNESS_M,
            mz,
            SURF_BUILT,
        );
        assert_eq!(down, Vec3::NEG_Y);
        let wall = normal_at(
            &world,
            &cols,
            mx + 1.0,
            base + LEVEL_H_M * 0.5,
            mz,
            SURF_BUILT,
        );
        assert_eq!(wall.y, 0.0, "between two floors is a wall, got {wall:?}");
        let rim = normal_at(
            &world,
            &cols,
            cx as f32 * BUILD_CELL_M + 0.05,
            base + LEVEL_H_M,
            mz,
            SURF_BUILT,
        );
        assert_eq!(rim.y, 0.0, "a rim keeps the edge snap, got {rim:?}");
        // The floor under a point over the first storey is that storey.
        let f = floor_below(&world, &cols, Vec3::new(mx, base + LEVEL_H_M + 1.2, mz));
        assert!(
            (f - (base + LEVEL_H_M)).abs() < 1e-4,
            "floor {f} vs {}",
            base + LEVEL_H_M
        );
    }

    /// The ground's matter is the footstep's surface: argmax of the splat,
    /// water under the sea.
    #[test]
    fn ground_matter_reads_the_splat_like_a_footstep() {
        assert_eq!(ground_matter([200, 10, 10, 10], false), Matter::Sand);
        assert_eq!(ground_matter([10, 200, 10, 10], false), Matter::Grass);
        assert_eq!(ground_matter([10, 10, 200, 10], false), Matter::Dirt);
        assert_eq!(ground_matter([10, 10, 10, 200], false), Matter::Stone);
        assert_eq!(ground_matter([200, 10, 10, 10], true), Matter::Water);
    }

    /// A door comes in two matters and everything metal is metal.
    #[test]
    fn a_deployable_is_made_of_its_archetype() {
        assert_eq!(arch_matter(ARCH_DOOR, 200), Matter::Wood);
        assert_eq!(arch_matter(ARCH_DOOR, 250), Matter::Metal);
        assert_eq!(arch_matter(ARCH_GARAGE_DOOR, 600), Matter::Metal);
        assert_eq!(arch_matter(ARCH_FURNACE, 500), Matter::Stone);
        assert_eq!(arch_matter(sim_core::deploy::ARCH_BOX, 150), Matter::Wood);
    }
}
