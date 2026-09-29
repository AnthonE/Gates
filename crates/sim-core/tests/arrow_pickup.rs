//! A stopped arrow becomes a loose stack you can see and take back
//! (`spent.rs`, `grounditem.rs`; `NOW.md` §5 item 2), driven through
//! `World::tick` — the only path a player has. `tests/arrow_recovery.rs`
//! gates the stop, the lodge and the fall below this.

#![allow(clippy::disallowed_macros)]

use sim_core::backpack::BackpackContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::spent::{feet_mm, SpentRec};
use sim_core::world::{Command, World, EV_GATHER};

/// The round, at an index of its own.
const ARROW: u16 = 7;

/// A stack ceiling small enough to fill by hand.
const QUIVER: u16 = 4;

const P: usize = 0;

/// A world with one player standing on the island, arrows stackable, and
/// the loose-stack ladder armed (inert content lays nothing down).
fn world_with_archer() -> Box<World> {
    let mut w = Box::new(World::new(20260731));
    let mut g = GatherContent::EMPTY;
    g.stack_max[ARROW as usize] = QUIVER;
    g.item_count = ARROW + 1;
    w.gather = g;
    w.backpack = BackpackContent::probe_fixture();
    w.tick(&[Command::Join { id: 1 }]);
    w
}

/// An arrow in nothing, `dx` millimetres east of the player and a metre
/// up, as `ranged` hands one on.
fn stop(w: &mut World, dx: i32) {
    let (x, y, z) = feet_mm(&w.players[P].body);
    w.spent.lodge(SpentRec {
        qx: x + dx,
        qy: y + 1_000,
        qz: z,
        round: ARROW,
        ready_at: w.tick,
        ..SpentRec::default()
    });
}

fn carried(w: &World, item: u16) -> u16 {
    w.players[P]
        .inv
        .iter()
        .filter(|s| s.count > 0 && s.item == item)
        .map(|s| s.count)
        .sum()
}

fn gathers(w: &World) -> Vec<(u16, u16)> {
    w.events
        .entries()
        .iter()
        .filter(|e| e.code == EV_GATHER)
        .map(|e| ((e.b >> 16) as u16, e.b as u16))
        .collect()
}

#[test]
fn a_stopped_arrow_lies_on_the_ground_and_comes_back() {
    let mut w = world_with_archer();
    stop(&mut w, 0);
    w.tick(&[]);

    assert!(w.spent.is_empty(), "it left the stopped-arrow store");
    assert_eq!(w.ground_items.len(), 1, "and lies on the ground");
    let g = w.ground_items.entries()[0];
    assert_eq!(
        (g.stack.item, g.stack.count),
        (ARROW, 1),
        "one of the round, never the weapon"
    );
    let feet = w.players[P].body;
    assert_eq!(
        (g.qx, g.qz),
        (feet.qx, feet.qz),
        "straight down from where it stopped"
    );
    assert!(
        (g.qy - feet.qy).abs() <= 2,
        "on the surface the player stands on (y {} vs feet {})",
        g.qy,
        feet.qy
    );

    w.tick(&[Command::Pickup { id: 1 }]);
    assert_eq!(carried(&w, ARROW), 1, "the round entered the quiver");
    assert!(w.ground_items.is_empty(), "and left the ground");
    assert_eq!(gathers(&w), vec![(ARROW, 1)], "announced as a gather");
}

/// Mutant: a `rest_at` that ignored the sea lays arrows on the seabed.
#[test]
fn an_arrow_that_falls_into_deep_water_is_lost() {
    let mut w = world_with_archer();
    // The island's corner is open sea.
    let floor = sim_core::terrain::ground(w.seed, &w.haven, 5.0, 5.0);
    assert!(
        floor < sim_core::terrain::SEA_LEVEL - sim_core::grounditem::SINK_DEPTH_M,
        "the fixture's point must be deep water (floor {floor} m)"
    );
    w.spent.lodge(SpentRec {
        qx: 5_000,
        qy: 2_000,
        qz: 5_000,
        round: ARROW,
        ready_at: w.tick,
        ..SpentRec::default()
    });
    w.tick(&[]);
    assert!(w.spent.is_empty());
    assert!(w.ground_items.is_empty(), "nothing lies on the seabed");
}

/// One in you rides you for the lodge and falls out at your feet, wherever
/// you have walked.
#[test]
fn an_arrow_in_you_falls_out_at_your_feet_when_the_lodge_ends() {
    let mut w = world_with_archer();
    let ready = w.tick + 5;
    let (x, y, z) = feet_mm(&w.players[P].body);
    w.spent.lodge(SpentRec {
        qx: x,
        qy: y + 1_300,
        qz: z,
        round: ARROW,
        ready_at: ready,
        host: w.players[P].id,
        life: u64::from(w.players[P].deaths),
    });
    w.tick(&[]);
    assert_eq!(w.spent.len(), 1, "still in the body");
    assert!(w.ground_items.is_empty());

    // Walked two metres (moved by hand: the arrow reads the body, not the
    // path it took).
    w.players[P].body.qx += (2.0 / POS_XZ_Q) as i32;
    while w.tick < ready {
        w.tick(&[]);
    }
    w.tick(&[]);
    assert!(w.spent.is_empty(), "the lodge ran out");
    assert_eq!(w.ground_items.len(), 1);
    let g = w.ground_items.entries()[0];
    assert_eq!(
        (g.qx, g.qz),
        (w.players[P].body.qx, w.players[P].body.qz),
        "it fell out where the body is now, not where it was hit"
    );
}

#[test]
fn an_arrow_in_a_body_that_dies_falls_out_at_once() {
    let mut w = world_with_archer();
    let (x, y, z) = feet_mm(&w.players[P].body);
    w.spent.lodge(SpentRec {
        qx: x,
        qy: y + 1_300,
        qz: z,
        round: ARROW,
        ready_at: w.tick + 10_000,
        host: w.players[P].id,
        life: u64::from(w.players[P].deaths),
    });
    w.tick(&[]);
    assert_eq!(w.spent.len(), 1, "riding");
    w.players[P].dead = true;
    w.players[P].deaths += 1;
    w.tick(&[]);
    assert!(w.spent.is_empty(), "a corpse carries nothing");
    assert_eq!(w.ground_items.len(), 1, "it fell where the body died");
}

/// Two arrows at once rest as two stacks, and the verb takes the nearer.
#[test]
fn the_nearest_arrow_is_the_one_that_comes_back() {
    let mut w = world_with_archer();
    stop(&mut w, 3_000);
    stop(&mut w, 300);
    w.tick(&[]);
    assert_eq!(w.ground_items.len(), 2);
    w.tick(&[Command::Pickup { id: 1 }]);
    assert_eq!(w.ground_items.len(), 1, "one taken");
    let far = (feet_mm(&w.players[P].body).0 + 3_000) as f32 / 1000.0;
    let left = w.ground_items.entries()[0].qx as f32 * POS_XZ_Q;
    assert!(
        (left - far).max(far - left) < 0.05,
        "the far one is what is left"
    );
}

#[test]
fn a_full_quiver_spills_the_arrow_at_your_feet() {
    let mut w = world_with_archer();
    for s in w.players[P].inv.iter_mut() {
        *s = ItemStack {
            item: ARROW,
            count: QUIVER,
            cond: 0,
            skin: 0,
        };
    }
    stop(&mut w, 0);
    w.tick(&[]);
    w.tick(&[Command::Pickup { id: 1 }]);
    assert!(w.ground_items.is_empty(), "the stack left the ground");
    assert_eq!(
        w.backpacks.len(),
        1,
        "and what did not fit spilled into a bag"
    );
}

#[test]
fn the_dead_do_not_pick_up() {
    let mut w = world_with_archer();
    stop(&mut w, 0);
    w.tick(&[]);
    w.players[P].dead = true;
    w.tick(&[Command::Pickup { id: 1 }]);
    assert_eq!(w.ground_items.len(), 1, "the arrow is untouched");
    assert!(gathers(&w).is_empty());
}

/// The y the loose stack rests at is the body's own quanta.
#[test]
fn a_resting_arrow_is_in_body_quanta() {
    let mut w = world_with_archer();
    stop(&mut w, 0);
    w.tick(&[]);
    let g = w.ground_items.entries()[0];
    let y = g.qy as f32 * POS_Y_Q;
    let feet = w.players[P].body.qy as f32 * POS_Y_Q;
    assert!(
        (y - feet).max(feet - y) < 0.05,
        "{y} m against feet at {feet} m"
    );
}
