//! The pickup verb — an arrow on the ground becomes an arrow in the quiver
//! (`reference/PROJECTILES.md` §9.7 pieces 3 and 4, arrow recovery v1).
//!
//! **This is the half `arrow_recovery.rs` said it could not gate.** That
//! suite's header names the gap in its own words — *"no player can press
//! anything here"* — and everything it does cover (the store, the break
//! roll, the lodge, the cap, the save) is what the verb stands on. This
//! file is the verb: `Command::Pickup` through `World::tick`, which is the
//! only path a player has.
//!
//! **What the operator asked for.** `DECISIONS.md` 2026-08-10, *"why cant
//! we get arrows back? why cant it be like rust?"* — answered eighteen
//! days later, because §9.7's four pieces were split across two passes on
//! purpose.
//!
//! **Every assertion below was run against a mutant**, and two of them
//! were worth writing for it. The pack-full case passes trivially under
//! the take-then-discover shape unless you assert the arrow is *still
//! there*; and the reach test passes under a 2D compare unless the arrow
//! is put overhead. Both mutants are named in the test that catches them.

#![allow(clippy::disallowed_macros)]

use sim_core::gather::{GatherContent, ItemStack, NO_ITEM};
use sim_core::limits::INV_SLOTS;
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::spent::{SpentRec, PICKUP_REACH_M};
use sim_core::world::{Command, World, EV_GATHER};

/// The round the fixture recovers. Not 0 and not 1: an item index that is
/// also a plausible count is a coincidence waiting to read as a pass.
const ARROW: u16 = 7;

/// What a quiver holds. Small on purpose — the pack-full case has to be
/// reachable without filling thirty slots by hand.
const QUIVER: u16 = 4;

/// A world with an arrow item the ladder can size, and one player standing
/// at a known place with an empty pack.
fn world_with_archer() -> Box<World> {
    let mut w = Box::new(World::new(20260731));
    let mut g = GatherContent::EMPTY;
    g.stack_max[ARROW as usize] = QUIVER;
    g.item_count = ARROW + 1;
    w.gather = g;
    w.tick(&[Command::Join { id: 1 }]);
    w
}

/// The fixture's archer is the first joiner, so slot 0 — `gather.rs`'s
/// convention, and the reason nothing here needs a private lookup.
const P: usize = 0;

/// Where the fixture's archer stands, in the arrow store's millimetres.
/// Read off the player rather than restated, because the conversion is the
/// thing under test in `a_lodged_arrow_overhead_is_out_of_reach`.
fn body_mm(w: &World) -> (i32, i32, i32) {
    let p = &w.players[P];
    (
        p.body.qx * (POS_XZ_Q * 1000.0) as i32,
        p.body.qy * (POS_Y_Q * 1000.0) as i32,
        p.body.qz * (POS_XZ_Q * 1000.0) as i32,
    )
}

/// Lay an arrow down at an offset in millimetres from the archer's body.
fn lay(w: &mut World, dx: i32, dy: i32, dz: i32, ready_at: u64) {
    let (bx, by, bz) = body_mm(w);
    w.spent.lodge(SpentRec {
        qx: bx + dx,
        qy: by + dy,
        qz: bz + dz,
        round: ARROW,
        ready_at,
    });
}

/// How many of `item` the player is carrying, across every slot.
fn carried(w: &World, item: u16) -> u16 {
    let p = &w.players[P];
    p.inv
        .iter()
        .filter(|s| s.count > 0 && s.item == item)
        .map(|s| s.count)
        .sum()
}

/// Every `EV_GATHER` in the queue, as `(item, units)`.
fn gathers(w: &World) -> Vec<(u16, u16)> {
    w.events
        .entries()
        .iter()
        .filter(|e| e.code == EV_GATHER)
        .map(|e| ((e.b >> 16) as u16, e.b as u16))
        .collect()
}

/// The plain case, and the one the operator asked for: an arrow lying at
/// your feet is an arrow in your quiver.
///
/// Mutant: dispatching `Command::Pickup` to a no-op leaves the store at 1
/// and the quiver at 0 — both asserted, so the verb cannot be silently
/// unwired the way `take_near` was for eighteen days.
#[test]
fn an_arrow_at_your_feet_comes_back() {
    let mut w = world_with_archer();
    lay(&mut w, 0, 0, 0, 0);
    assert_eq!(w.spent.len(), 1, "fixture laid the arrow");

    w.tick(&[Command::Pickup { id: 1 }]);

    assert_eq!(carried(&w, ARROW), 1, "the round entered the quiver");
    assert_eq!(w.spent.len(), 0, "and left the ground");
    assert_eq!(
        gathers(&w),
        vec![(ARROW, 1)],
        "announced as a gather — one item, one unit"
    );
}

/// The round comes back, never the bow. `SpentRec::round` is the ammo and
/// `Arrow::item` is the weapon, and this is the assertion that the pickup
/// reads the right one of the two.
#[test]
fn what_comes_back_is_the_round_and_not_the_weapon() {
    let mut w = world_with_archer();
    // A bow index that is deliberately *also* a valid stackable item, so
    // reading the wrong field would look like a success rather than a zero.
    const BOW: u16 = 3;
    w.gather.stack_max[BOW as usize] = 1;
    lay(&mut w, 0, 0, 0, 0);

    w.tick(&[Command::Pickup { id: 1 }]);

    assert_eq!(carried(&w, ARROW), 1);
    assert_eq!(carried(&w, BOW), 0, "a pickup never mints the weapon");
}

/// Reach is measured in **three** dimensions, which is the one thing this
/// verb's pick does that `loot_nearest`'s does not.
///
/// Mutant, and the reason the arrow is directly overhead: drop `dy` from
/// `peek_near`'s distance and this arrow is at horizontal distance zero —
/// a 2D compare takes it from thirty metres up. Every other placement in
/// this file passes under that mutant.
#[test]
fn a_lodged_arrow_overhead_is_out_of_reach() {
    let mut w = world_with_archer();
    let up = (PICKUP_REACH_M * 1000.0) as i32 + 1_000; // a metre past reach
    lay(&mut w, 0, up, 0, 0);

    w.tick(&[Command::Pickup { id: 1 }]);

    assert_eq!(carried(&w, ARROW), 0, "straight up is still distance");
    assert_eq!(w.spent.len(), 1, "and it stays in the tree");
    assert!(gathers(&w).is_empty(), "nothing owed, nothing announced");
}

/// An arrow further away than a foundation is placeable is not reachable.
/// `PICKUP_REACH_M` is `BUILD_REACH_M` by `pub use`, and this is what makes
/// that alias a claim rather than a comment.
#[test]
fn an_arrow_past_the_reach_stays_where_it_is() {
    let mut w = world_with_archer();
    let far = (PICKUP_REACH_M * 1000.0) as i32 + 500;
    lay(&mut w, far, 0, 0, 0);
    w.tick(&[Command::Pickup { id: 1 }]);
    assert_eq!(w.spent.len(), 1, "out of reach");

    // ...and one that is inside it, from the same fixture, so the test
    // cannot pass by refusing everything.
    let mut w = world_with_archer();
    let near = (PICKUP_REACH_M * 1000.0) as i32 - 500;
    lay(&mut w, near, 0, 0, 0);
    w.tick(&[Command::Pickup { id: 1 }]);
    assert_eq!(w.spent.len(), 0, "inside it");
}

/// An arrow that drew blood may not be collected during the fight. The
/// lodge is the store's rule and this is the verb honouring it — the sim
/// clock decides, so a client that presses early gets nothing.
#[test]
fn the_lodge_survives_the_verb() {
    let mut w = world_with_archer();
    let ready = w.tick + 5;
    lay(&mut w, 0, 0, 0, ready);

    w.tick(&[Command::Pickup { id: 1 }]);
    assert_eq!(w.spent.len(), 1, "still lodged");
    assert_eq!(carried(&w, ARROW), 0);

    while w.tick < ready {
        w.tick(&[]);
    }
    w.tick(&[Command::Pickup { id: 1 }]);
    assert_eq!(w.spent.len(), 0, "ready, and taken");
    assert_eq!(carried(&w, ARROW), 1);
}

/// **A full quiver leaves the arrow on the ground and says so.**
///
/// This is the assertion the look-then-take split in `spent.rs` exists
/// for, and the mutant is the shape that was easiest to write: take the
/// arrow first, then call `inv_add`, and drop the return on the floor.
/// Under it `carried` is unchanged and no gather is announced — both of
/// which this test would still see as correct — while `w.spent.len()` goes
/// to 0 and a player's ammunition is deleted with no evidence anywhere.
/// The store length is the assertion that catches it.
#[test]
fn a_full_quiver_leaves_the_arrow_lying_there() {
    let mut w = world_with_archer();
    {
        for s in 0..INV_SLOTS {
            w.players[P].inv[s] = ItemStack {
                item: ARROW,
                count: QUIVER,
                cond: 0,
            };
        }
    }
    let before = carried(&w, ARROW);
    lay(&mut w, 0, 0, 0, 0);

    w.tick(&[Command::Pickup { id: 1 }]);

    assert_eq!(w.spent.len(), 1, "THE ARROW IS STILL THERE");
    assert_eq!(carried(&w, ARROW), before, "and nothing was minted");
    assert_eq!(
        gathers(&w),
        vec![(ARROW, 0)],
        "the zero is owed: an arrow was in reach and the pack refused it"
    );
}

/// An item the stack ladder cannot size cannot be picked up — `inv_add`'s
/// stated hazard, guarded the way `loot_nearest` guards it. Without the
/// guard `inv_add` returns 0 and the arrow is announced as a refusal it
/// can never recover from, once per press, forever.
#[test]
fn a_round_with_no_stack_ceiling_is_not_taken() {
    let mut w = world_with_archer();
    w.gather.stack_max[ARROW as usize] = 0;
    lay(&mut w, 0, 0, 0, 0);

    w.tick(&[Command::Pickup { id: 1 }]);

    assert_eq!(w.spent.len(), 1);
    assert!(
        gathers(&w).is_empty(),
        "an item that can never be carried owes no pack-full line"
    );
}

/// The nearest ready arrow, not the first one laid. Insertion order is an
/// artefact and the player is reaching for the one under their hand.
#[test]
fn the_nearest_arrow_is_the_one_that_comes_back() {
    let mut w = world_with_archer();
    lay(&mut w, 2_000, 0, 0, 0); // 2 m — laid first
    lay(&mut w, 200, 0, 0, 0); //  0.2 m — laid second

    let (bx, _, _) = body_mm(&w);
    w.tick(&[Command::Pickup { id: 1 }]);

    assert_eq!(w.spent.len(), 1, "one taken");
    assert_eq!(
        w.spent.entries()[0].qx,
        bx + 2_000,
        "the far one is what is left"
    );
}

/// A press with nothing in reach is silent. `Loot`'s posture — the verb
/// that finds nothing has nothing to report, and an unowed `EV_GATHER`
/// zero would turn the client's "pack full" line into a lie (that event's
/// own doc says so).
#[test]
fn a_press_over_bare_ground_says_nothing() {
    let mut w = world_with_archer();
    w.tick(&[Command::Pickup { id: 1 }]);
    assert!(gathers(&w).is_empty());
    assert_eq!(carried(&w, NO_ITEM), 0);
}

/// A corpse does not reach. The dispatch guards on `live_slot_of`, the
/// same guard every verb but `Respawn` uses, and this is the case that
/// would otherwise let a dead player farm the field they died in.
#[test]
fn the_dead_do_not_pick_up() {
    let mut w = world_with_archer();
    lay(&mut w, 0, 0, 0, 0);
    {
        w.players[P].dead = true;
    }

    w.tick(&[Command::Pickup { id: 1 }]);

    assert_eq!(w.spent.len(), 1, "the arrow is untouched");
    assert!(gathers(&w).is_empty());
}
