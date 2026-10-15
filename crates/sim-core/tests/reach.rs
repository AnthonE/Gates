//! **A hand's reach is the same storey and a clear line** (`reach.rs`).
//!
//! Every container verb measured reach on the flat alone, which passed walls
//! and floors: a box in a walled 1×1 emptied from outside it, a furnace
//! worked through a wall, a bag on the floor above taken from below. These
//! hold the two halves that close it, against a collision index built by
//! hand so each case is exactly one piece.

use sim_core::build::{self, LOC_EDGE_ZLO, LOC_PLANE, SHAPE_FLOOR, SHAPE_WALL};
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
