//! **A hand's reach is the same storey and a clear line** (`reach.rs`).
//!
//! Every container verb measured reach on the flat alone, which passed walls
//! and floors: a box in a walled 1×1 emptied from outside it, a furnace
//! worked through a wall, a bag on the floor above taken from below. These
//! hold the two halves that close it, against a collision index built by
//! hand so each case is exactly one piece.

use sim_core::build::{self, LOC_EDGE_ZLO, LOC_PLANE, SHAPE_FLOOR, SHAPE_FOUNDATION, SHAPE_WALL};
use sim_core::collide::ColIndex;
use sim_core::reach::{self, Target, LOOT_REACH_DY_M};
use sim_core::terrain;

const SEED: u64 = 0x6000_0C1E;
const CX: u16 = 512;
const CZ: u16 = 512;

fn hv(seed: u64) -> &'static terrain::Haven {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static terrain::Haven = Box::leak(Box::new(terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

/// The cell's floor, its low-z edge, and the centre a box would stand at.
fn cell() -> (f32, f32, f32) {
    let base = build::column_floor_y(SEED, hv(SEED), CX, CZ, 0);
    let z_lo = CZ as f32 * build::BUILD_CELL_M;
    let x_mid = (CX as f32 + 0.5) * build::BUILD_CELL_M;
    (base, z_lo, x_mid)
}

/// A wall on the cell's low-z edge stops a hand reaching from outside it to
/// a box inside; with no wall the same reach is clear.
#[test]
fn a_wall_between_the_eye_and_the_box_blocks_the_hand() {
    let (base, z_lo, x) = cell();
    let eye = (x, base + 1.6, z_lo - 1.0);
    let boxed = (x, base + 0.4, z_lo + 1.5);
    let open = Box::new(ColIndex::new());
    let mut walled = Box::new(ColIndex::new());
    walled.add(CX, CZ, 0, LOC_EDGE_ZLO, SHAPE_WALL, 0);
    assert!(
        reach::sight(SEED, hv(SEED), &open, 0, eye, boxed),
        "open ground"
    );
    assert!(
        !reach::sight(SEED, hv(SEED), &walled, 0, eye, boxed),
        "a hand reached through a wall"
    );
    // Inside the room, the wall behind the player is no obstacle.
    let inside = (x, base + 1.6, z_lo + 0.6);
    assert!(reach::sight(SEED, hv(SEED), &walled, 0, inside, boxed));
}

/// A floor between two storeys stops a hand reaching up through it.
#[test]
fn a_floor_between_storeys_blocks_the_hand() {
    let (base, z_lo, x) = cell();
    let z = z_lo + 1.5;
    let upstairs = base + build::level_y(1);
    let eye = (x + 0.5, base + 1.6, z);
    let bag = (x, upstairs + reach::LIFT_M, z);
    let mut floored = Box::new(ColIndex::new());
    floored.add(CX, CZ, 1, LOC_PLANE, SHAPE_FLOOR, 0);
    assert!(reach::sight(SEED, hv(SEED), &ColIndex::new(), 0, eye, bag));
    assert!(
        !reach::sight(SEED, hv(SEED), &floored, 0, eye, bag),
        "a hand reached through the floor above"
    );
}

/// The storey rule on its own, with nothing built at all: a storey up is
/// out of reach, a step up is not.
#[test]
fn a_storey_up_is_out_of_reach_and_a_step_is_not() {
    let (base, z_lo, x) = cell();
    let open = Box::new(ColIndex::new());
    let feet = base;
    let eye = (x, feet + 1.6, z_lo + 0.5);
    let at = |base_y: f32| Target {
        x,
        z: z_lo + 1.5,
        base_y,
        mid: reach::LIFT_M,
        storey: true,
    };
    assert!(reach::clear_from(
        SEED,
        hv(SEED),
        &open,
        0,
        eye,
        feet,
        at(base + 1.0)
    ));
    assert!(
        build::level_y(1) > LOOT_REACH_DY_M,
        "a storey is past the rule"
    );
    assert!(!reach::clear_from(
        SEED,
        hv(SEED),
        &open,
        0,
        eye,
        feet,
        at(base + build::level_y(1))
    ));
    assert!(!reach::clear_from(
        SEED,
        hv(SEED),
        &open,
        0,
        eye,
        feet,
        at(base - build::level_y(1))
    ));
}

/// A thing lying where it fell answers to the line alone. A bag left hanging
/// where a floor was broken under it is a storey up and nothing is in the
/// way: it is reached from below. With the floor still there, it is not.
#[test]
fn a_bag_left_hanging_where_a_floor_broke_is_reached_from_below() {
    let (base, z_lo, x) = cell();
    let feet = base;
    let eye = (x + 0.5, feet + 1.6, z_lo + 1.5);
    let hanging = Target {
        x,
        z: z_lo + 1.5,
        base_y: base + build::level_y(1),
        mid: reach::LIFT_M,
        storey: false,
    };
    assert!(reach::clear_from(
        SEED,
        hv(SEED),
        &ColIndex::new(),
        0,
        eye,
        feet,
        hanging
    ));
    let mut floored = Box::new(ColIndex::new());
    floored.add(CX, CZ, 1, LOC_PLANE, SHAPE_FLOOR, 0);
    assert!(!reach::clear_from(
        SEED,
        hv(SEED),
        &floored,
        0,
        eye,
        feet,
        hanging
    ));
}

/// An arrow stuck in the underside of a floor is aimed just off where it went
/// in, back along its flight, so the floor it is stuck in is not in the way
/// of the hand reaching up to it.
#[test]
fn an_arrow_stuck_in_a_ceiling_is_reached_from_the_room_below() {
    let (base, z_lo, x) = cell();
    let mut floored = Box::new(ColIndex::new());
    floored.add(CX, CZ, 1, LOC_PLANE, SHAPE_FLOOR, 0);
    let ceiling = base + build::level_y(1);
    let q = |m: f32, step: f32| (m / step) as i32;
    let stuck = (
        q(x, sim_core::movement::POS_XZ_Q),
        q(ceiling - 0.37, sim_core::movement::POS_Y_Q),
        q(z_lo + 1.5, sim_core::movement::POS_XZ_Q),
    );
    let t = Target::stuck(stuck, [0, 127, 0]);
    let eye = (x + 0.8, base + 1.6, z_lo + 1.5);
    assert!(reach::clear_from(SEED, hv(SEED), &floored, 0, eye, base, t));
}

/// A foundation laid over a bag lifts the hand's aim onto its top: the bag
/// is not buried out of reach under the slab.
#[test]
fn a_foundation_laid_over_a_bag_lifts_the_aim_onto_it() {
    let (base, z_lo, x) = cell();
    let z = z_lo + 1.5;
    let ground = terrain::ground(SEED, hv(SEED), x, z);
    let q = (
        (x / sim_core::movement::POS_XZ_Q) as i32,
        (ground / sim_core::movement::POS_Y_Q) as i32,
        (z / sim_core::movement::POS_XZ_Q) as i32,
    );
    let mut founded = Box::new(ColIndex::new());
    founded.add(CX, CZ, 0, LOC_PLANE, SHAPE_FOUNDATION, 0);
    let t = Target::lying(SEED, hv(SEED), &founded, q);
    assert!(
        t.base_y >= base - 0.01,
        "aimed at the slab's top, not under it"
    );
    let eye = (x + 1.0, t.base_y + 1.6, z);
    assert!(reach::clear_from(
        SEED,
        hv(SEED),
        &founded,
        0,
        eye,
        t.base_y,
        t
    ));
}

/// Nearest first, ties to the lower index, and no more sight walks than the
/// cap: a pile of blocked candidates finds nothing rather than walking them
/// all.
#[test]
fn the_nearest_clear_candidate_wins_and_the_walks_are_capped() {
    let mut cand = [(3.0f32, 0u16), (1.0, 1), (1.0, 2), (2.0, 3)];
    assert_eq!(reach::nearest_clear(&mut cand, |_| true), Some(1));
    let mut cand = [(3.0f32, 0u16), (1.0, 1), (1.0, 2), (2.0, 3)];
    assert_eq!(
        reach::nearest_clear(&mut cand, |i| i != 1 && i != 2),
        Some(3)
    );
    let mut blocked = [(0.0f32, 0u16); 64];
    for (i, c) in blocked.iter_mut().enumerate() {
        *c = (i as f32, i as u16);
    }
    let mut walks = 0;
    assert_eq!(
        reach::nearest_clear(&mut blocked, |_| {
            walks += 1;
            false
        }),
        None
    );
    assert_eq!(walks, sim_core::limits::HAND_SIGHT_TESTS_MAX);
}

/// The ziggurat's stone stops a hand though nothing is built there: the
/// sanctum's elite crate from the ledge outside its west wall, against the
/// same reach from inside the room.
#[test]
fn the_ziggurat_stone_blocks_a_hand() {
    let seed = 20_260_731;
    let z = hv(seed).ziggurat;
    assert!(z.live);
    let at = |lx: f32, dy: f32, lz: f32| {
        let (x, wz) = sim_core::kit::to_world(&z.placed(), lx, lz);
        (x, z.floor_y + dy, wz)
    };
    let crate_mid = at(-12.0, 15.4, -4.0);
    let open = ColIndex::new();
    assert!(
        !reach::sight(seed, hv(seed), &open, 0, at(-14.6, 16.6, -4.0), crate_mid),
        "a hand reached through the sanctum wall"
    );
    assert!(reach::sight(
        seed,
        hv(seed),
        &open,
        0,
        at(-10.0, 16.6, -4.0),
        crate_mid
    ));
}

/// Every authored crate the stone could now hide — the ziggurat's eight
/// and every landmark's — still has a place to stand within arm's reach,
/// on its own floor, clear of the stone, with a clear line to it.
#[test]
fn every_authored_crate_can_still_be_reached() {
    use sim_core::terrain::{scatter, ScatterTable, CELL_SIZE};
    use sim_core::worldcont::{stand_q, table_of};
    let offsets: [(f32, f32); 12] = [
        (1.2, 0.0),
        (-1.2, 0.0),
        (0.0, 1.2),
        (0.0, -1.2),
        (2.0, 0.0),
        (-2.0, 0.0),
        (0.0, 2.0),
        (0.0, -2.0),
        (1.4, 1.4),
        (-1.4, 1.4),
        (1.4, -1.4),
        (-1.4, -1.4),
    ];
    let table = ScatterTable::alpha_default();
    let mut checked = 0;
    for seed in [20_260_731u64, SEED, 42] {
        let h = hv(seed);
        let mut sites: Vec<(f32, f32)> = Vec::new();
        if h.ziggurat.live {
            for k in 0..sim_core::monument::CRATES.len() {
                let (x, _, wz, _, _) = sim_core::monument::crate_world(&h.ziggurat, k).unwrap();
                sites.push((x, wz));
            }
        }
        for m in h.marks.iter().filter(|m| m.live) {
            let mut k = 0;
            while let Some((x, wz, _, _)) = sim_core::landmark::anchor(m, k) {
                sites.push((x, wz));
                k += 1;
            }
        }
        let open = ColIndex::new();
        for (x, wz) in sites {
            let s = scatter(
                seed,
                &table,
                h,
                (x / CELL_SIZE) as i32,
                (wz / CELL_SIZE) as i32,
            );
            if table_of(s.occupant).is_none() {
                continue;
            }
            let t = Target::world_cont(stand_q(s.x, s.y, s.z), s.occupant);
            let reached = offsets.iter().any(|&(dx, dz)| {
                let (px, pz) = (s.x + dx, s.z + dz);
                let r = sim_core::collide::CAPSULE_RADIUS_M;
                let tall = sim_core::collide::CAPSULE_HEIGHT_M;
                let walled = sim_core::monument::blocks(&h.ziggurat, 0, px, pz, s.y, r, tall)
                    || sim_core::landmark::blocks(&h.marks, px, pz, s.y, r, tall);
                !walled && reach::clear_from(seed, h, &open, 0, (px, s.y + 1.6, pz), s.y, t)
            });
            assert!(
                reached,
                "seed {seed}: the {:?} at ({:.1}, {:.1}, {:.1}) cannot be reached",
                s.occupant, s.x, s.y, s.z
            );
            checked += 1;
        }
    }
    assert!(checked >= 8, "the ziggurat's crates at least ({checked})");
}

/// A card door counts as it stands now: a bag lying in the green room's
/// doorway is reached while the door is open, and a player standing in the
/// open doorway reaches past it; shut, the leaf is stone like the wall.
#[test]
fn a_card_doorway_is_stone_only_while_its_door_is_shut() {
    let seed = 20_260_731;
    let z = hv(seed).ziggurat;
    assert!(z.live);
    let at = |lx: f32, dy: f32, lz: f32| {
        let (x, wz) = sim_core::kit::to_world(&z.placed(), lx, lz);
        (x, z.floor_y + dy, wz)
    };
    let open = ColIndex::new();
    let door_1 = 1 << 0;
    let bag = at(0.0, reach::LIFT_M, 11.0);
    let inside = at(0.0, 1.6, 8.5);
    assert!(reach::sight(seed, hv(seed), &open, door_1, inside, bag));
    assert!(
        !reach::sight(seed, hv(seed), &open, 0, inside, bag),
        "a shut leaf is in the way"
    );
    let in_the_doorway = at(0.0, 1.6, 11.0);
    assert!(reach::sight(
        seed,
        hv(seed),
        &open,
        door_1,
        in_the_doorway,
        at(0.0, reach::LIFT_M, 9.0)
    ));
}

/// An arrow landing on a ziggurat ledge sinks into its top step (the arrow
/// ladder passes through it, as a body steps onto it), where the hand's
/// point test reads it as inside the stone. It is aimed just above the top
/// instead, and reached from the ledge beside it.
#[test]
fn an_arrow_sunk_into_a_ledge_top_is_reached_from_the_ledge() {
    let seed = 20_260_731;
    let h = hv(seed);
    let z = h.ziggurat;
    assert!(z.live);
    let (x, wz) = sim_core::kit::to_world(&z.placed(), -3.0, 4.0);
    let top = sim_core::monument::ground(&z, 0, x, wz, z.floor_y + 4.8, 0.0);
    assert!(top > z.floor_y + 4.0, "the blue room's floor is stone here");
    let q = |m: f32, step: f32| (m / step) as i32;
    let sunk = (
        q(x, sim_core::movement::POS_XZ_Q),
        q(top - 0.05, sim_core::movement::POS_Y_Q),
        q(wz, sim_core::movement::POS_XZ_Q),
    );
    let dir = [127, -20, 0];
    let open = ColIndex::new();
    let (ex, ewz) = sim_core::kit::to_world(&z.placed(), -1.5, 4.0);
    let eye = (ex, top + 1.6, ewz);
    assert!(
        !reach::clear_from(seed, h, &open, 0, eye, top, Target::stuck(sunk, dir)),
        "the raw stuck point is inside the stone"
    );
    assert!(reach::clear_from(
        seed,
        h,
        &open,
        0,
        eye,
        top,
        Target::stuck_in(h, 0, sunk, dir)
    ));
}

/// A door is worked from either side of its doorway, shut leaf and all, but
/// not from the room above it, and not from behind another wall (the
/// 2026-10-19 playtest opened doors "from like above").
#[test]
fn a_door_is_reached_from_its_own_storey_and_not_through_a_floor() {
    use sim_core::build::{LOC_EDGE_ZLO as EDGE, SHAPE_DOORWAY};
    use sim_core::deploy::DeployRec;
    let (_, z_lo, x) = cell();
    let rec = DeployRec {
        cx: CX,
        cz: CZ,
        level: 0,
        loc: EDGE,
        ..DeployRec::default()
    };
    let mut cols = Box::new(ColIndex::new());
    cols.add(CX, CZ, 0, EDGE, SHAPE_DOORWAY, 0);
    cols.set_door(CX, CZ, 0, EDGE, true);
    // A floor over the room, and a wall a cell further out on the outside.
    cols.add(CX, CZ, 1, LOC_PLANE, SHAPE_FLOOR, 0);
    cols.add(CX, CZ - 1, 0, EDGE, SHAPE_WALL, 0);
    let hv = hv(SEED);
    let base = Target::door(SEED, hv, &cols, &rec, (x, z_lo)).base_y;
    let from = |z: f32, feet: f32| {
        let eye = (x + 0.4, feet + 1.6, z);
        let t = Target::door(SEED, hv, &cols, &rec, (eye.0, eye.2));
        reach::clear_from(SEED, hv, &cols, 0, eye, feet, t)
    };
    assert!(from(z_lo - 1.5, base), "outside, in front of the shut door");
    assert!(from(z_lo + 1.5, base), "inside, behind it");
    assert!(
        !from(z_lo + 1.5, base + build::level_y(1)),
        "from the room above, through its floor"
    );
    assert!(
        !from(z_lo - build::BUILD_CELL_M - 0.5, base),
        "from behind another wall"
    );
}
