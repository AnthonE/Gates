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
        reach::sight(SEED, hv(SEED), &open, eye, boxed),
        "open ground"
    );
    assert!(
        !reach::sight(SEED, hv(SEED), &walled, eye, boxed),
        "a hand reached through a wall"
    );
    // Inside the room, the wall behind the player is no obstacle.
    let inside = (x, base + 1.6, z_lo + 0.6);
    assert!(reach::sight(SEED, hv(SEED), &walled, inside, boxed));
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
    assert!(reach::sight(SEED, hv(SEED), &ColIndex::new(), eye, bag));
    assert!(
        !reach::sight(SEED, hv(SEED), &floored, eye, bag),
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
        eye,
        feet,
        at(base + build::level_y(1))
    ));
    assert!(!reach::clear_from(
        SEED,
        hv(SEED),
        &open,
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
    assert!(reach::clear_from(SEED, hv(SEED), &floored, eye, base, t));
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
