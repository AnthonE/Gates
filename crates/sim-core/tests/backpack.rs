//! The death backpack, the sim half: what a body leaves behind, who can
//! pick it up, and when it stops being there. The content half — that the
//! despawn ladder bakes out of `content/balance.toml` and rises with
//! rarity — is `crates/content/tests/content.rs`; nothing here invents a
//! number, every one comes out of `BackpackContent::probe_fixture` or
//! `GatherContent::probe_fixture`.
//!
//! The arrangement is the duel from `combat.rs`: `dev_spawn` pins both
//! players to the ring's own spawn for id 1, which the spawn selector
//! guarantees is clear of scatter for 4 m — so a swing lands on a person
//! and never on a tree, and a bag dropped there is the only bag in reach.

use sim_core::backpack::{BackpackContent, BAG_GONE_DESPAWN, BAG_GONE_EMPTIED, LOOT_REACH_M};
use sim_core::combat::CombatContent;
use sim_core::gather::{GatherContent, ItemStack, SWING_INTERVAL_TICKS};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::INV_SLOTS;
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::world::{Command, World, EV_BAG_DROPPED, EV_BAG_REMOVED, EV_GATHER};
use sim_core::yaw_dir;

const SEED: u64 = 20260802;
/// The fixture's item 0: 34 damage, 2 m reach — three swings to kill.
/// It is also one of the fixture's long-lived items (360 ticks).
const SPEAR: u16 = 0;
/// A fixture item with no weapon row and the short 90-tick lifetime.
const JUNK: u16 = 7;
/// Every fixture item stacks to 100 (`GatherContent::probe_fixture`).
const STACK_MAX: u16 = 100;

fn duel_world() -> World {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
    for p in w.players.iter_mut().take(2) {
        p.inv[0] = ItemStack {
            item: SPEAR,
            count: 1,
        };
    }
    w
}

fn swing_frame(seq: u16, yaw: u16) -> InputFrame {
    InputFrame {
        seq,
        buttons: BTN_PRIMARY,
        yaw,
        pitch: 128,
        move_x: 0,
        move_z: 0,
        sel: 0,
    }
}

/// Put player `b` `dist` metres along `yaw` from player `a`.
fn place_in_front(w: &mut World, a: usize, b: usize, yaw: u16, dist: f32) {
    let (fx, fz) = yaw_dir(yaw);
    let body = w.players[a].body;
    let (ax, az) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
    w.players[b].body = Body::at(SEED, ax + fx * dist, az + fz * dist);
}

/// Hold the swing on player 2 until they die. The button stays down —
/// `frame` is sim state, so cadence, not re-pressing, is what paces the
/// swings, and every tick is checked because a kill can land on any of
/// them. Returns the tick the kill landed on and where the body was
/// standing when it did.
fn kill_player_two(w: &mut World) -> (u64, i32, i32) {
    let yaw = 0u16;
    let deaths_before = w.players[1].deaths;
    for seq in 0..(SWING_INTERVAL_TICKS as u16 * 8) {
        place_in_front(w, 0, 1, yaw, 1.0);
        let at = w.tick;
        let (vx, vz) = (w.players[1].body.qx, w.players[1].body.qz);
        w.tick(&[Command::Input {
            id: 1,
            frame: swing_frame(seq, yaw),
        }]);
        if w.players[1].deaths > deaths_before {
            return (at, vx, vz);
        }
    }
    panic!("three fixture spear hits must kill inside eight swing intervals");
}

fn ev_codes(w: &World) -> Vec<(u8, u32, u32)> {
    w.events
        .entries()
        .iter()
        .map(|e| (e.code, e.a, e.b))
        .collect()
}

#[test]
fn a_kill_leaves_what_the_body_carried_on_the_ground() {
    let mut w = duel_world();
    // Give the victim something worth taking, past the spear in slot 0.
    w.players[1].inv[1] = ItemStack {
        item: JUNK,
        count: 42,
    };
    let victim_id = w.players[1].id;
    let (_, vx, vz) = kill_player_two(&mut w);

    assert_eq!(w.backpacks.len(), 1, "the death dropped exactly one bag");
    let bag = w.backpacks.entries()[0];
    assert_eq!(bag.owner, victim_id);
    assert_eq!(
        (bag.qx, bag.qz),
        (vx, vz),
        "the bag is where the body was, not where the killer stood"
    );
    let held: Vec<(u16, u16)> = bag
        .items
        .iter()
        .filter(|s| s.count > 0)
        .map(|s| (s.item, s.count))
        .collect();
    assert_eq!(held, vec![(SPEAR, 1), (JUNK, 42)]);
    assert!(
        w.players[1].inv.iter().all(|s| s.count == 0),
        "and the body itself wakes up naked"
    );
}

#[test]
fn the_kill_is_announced_as_a_bag_the_whole_shard_can_see() {
    let mut w = duel_world();
    w.players[1].inv[1] = ItemStack {
        item: JUNK,
        count: 3,
    };
    let victim_id = w.players[1].id;
    kill_player_two(&mut w);
    let bag_id = w.backpacks.entries()[0].id;
    assert!(
        ev_codes(&w).contains(&(EV_BAG_DROPPED, bag_id, victim_id)),
        "the drop rides the same tick as the death"
    );
}

#[test]
fn the_killer_takes_it_and_the_bag_is_gone() {
    let mut w = duel_world();
    w.players[1].inv[1] = ItemStack {
        item: JUNK,
        count: 42,
    };
    kill_player_two(&mut w);
    let bag_id = w.backpacks.entries()[0].id;

    // The killer never moved; the bag is at their feet.
    w.tick(&[Command::Loot { id: 1 }]);
    assert!(w.backpacks.is_empty(), "an emptied bag leaves immediately");
    let mine: Vec<(u16, u16)> = w.players[0]
        .inv
        .iter()
        .filter(|s| s.count > 0)
        .map(|s| (s.item, s.count))
        .collect();
    assert_eq!(
        mine,
        vec![(SPEAR, 2), (JUNK, 42)],
        "the killer's own spear topped up, the rest landed beside it"
    );
    let ev = ev_codes(&w);
    assert!(
        ev.contains(&(EV_BAG_REMOVED, bag_id, BAG_GONE_EMPTIED)),
        "and the shard hears why it went"
    );
    assert!(
        ev.iter().filter(|(c, _, _)| *c == EV_GATHER).count() == 2,
        "loot pays in the toast gathering pays in — one per slot moved"
    );
}

#[test]
fn a_bag_out_of_reach_stays_shut() {
    let mut w = duel_world();
    w.players[1].inv[1] = ItemStack {
        item: JUNK,
        count: 42,
    };
    kill_player_two(&mut w);
    let bag = w.backpacks.entries()[0];

    // Step the looter just past the reach they share with every other
    // world interaction, along the axis the bag is not on.
    let far = bag.qx as f32 * POS_XZ_Q + LOOT_REACH_M + 0.5;
    w.players[0].body = Body::at(SEED, far, bag.qz as f32 * POS_XZ_Q);
    w.tick(&[Command::Loot { id: 1 }]);
    assert_eq!(w.backpacks.len(), 1, "the bag is untouched");
    assert_eq!(w.backpacks.entries()[0].items[1].count, 42);
    assert!(w.players[0].inv.iter().all(|s| s.item != JUNK));

    // And one step inside it opens.
    let near = bag.qx as f32 * POS_XZ_Q + LOOT_REACH_M - 0.5;
    w.players[0].body = Body::at(SEED, near, bag.qz as f32 * POS_XZ_Q);
    w.tick(&[Command::Loot { id: 1 }]);
    assert!(
        w.backpacks.is_empty(),
        "reach is the only thing that gated it"
    );
}

#[test]
fn what_does_not_fit_stays_in_the_bag() {
    let mut w = duel_world();
    w.players[1].inv[1] = ItemStack {
        item: JUNK,
        count: 42,
    };
    // Fill the killer to the brim with something else, leaving one slot
    // that already holds the spear (which can still top up by 1).
    for s in w.players[0].inv.iter_mut().skip(1) {
        *s = ItemStack {
            item: 1,
            count: STACK_MAX,
        };
    }
    kill_player_two(&mut w);
    let bag_id = w.backpacks.entries()[0].id;

    w.tick(&[Command::Loot { id: 1 }]);
    assert_eq!(w.backpacks.len(), 1, "a bag with anything left stands");
    let left = w.backpacks.find(bag_id).expect("still there");
    assert_eq!(left.items[1].count, 42, "the junk had nowhere to go");
    assert_eq!(
        w.players[0].inv[0],
        ItemStack {
            item: SPEAR,
            count: 2
        },
        "but the stack that could top up did"
    );
    assert!(
        left.items[0].count == 0,
        "and the bag lost exactly what was taken"
    );
}

#[test]
fn a_bag_despawns_on_the_ladder_the_content_declares() {
    let mut w = duel_world();
    // Junk only: the fixture's short 90-tick lifetime, not the spear's.
    w.players[1].inv[0] = ItemStack {
        item: JUNK,
        count: 5,
    };
    let (killed_at, _, _) = kill_player_two(&mut w);
    let bag = w.backpacks.entries()[0];
    let bag_id = bag.id;
    assert_eq!(
        bag.expires,
        killed_at + 90,
        "an all-junk bag rides the fixture's base"
    );

    while w.tick < bag.expires {
        w.tick(&[]);
        assert!(w.backpacks.find(bag_id).is_some(), "not a tick early");
    }
    w.tick(&[]);
    assert!(w.backpacks.is_empty(), "and not a tick late");
    assert!(ev_codes(&w).contains(&(EV_BAG_REMOVED, bag_id, BAG_GONE_DESPAWN)));
}

#[test]
fn one_long_lived_item_keeps_the_whole_bag_standing() {
    let mut w = duel_world();
    w.players[1].inv[1] = ItemStack {
        item: JUNK,
        count: 5,
    };
    let (killed_at, _, _) = kill_player_two(&mut w);
    // Slot 0 still holds the spear — a fixture "rare", 360 ticks.
    assert_eq!(
        w.backpacks.entries()[0].expires,
        killed_at + 360,
        "the bag lives as long as the best thing in it"
    );
}

#[test]
fn an_inert_ladder_destroys_exactly_as_it_did_before() {
    let mut w = duel_world();
    w.backpack = BackpackContent::EMPTY;
    w.players[1].inv[1] = ItemStack {
        item: JUNK,
        count: 42,
    };
    kill_player_two(&mut w);
    assert!(
        w.backpacks.is_empty(),
        "content that never armed the module changes nothing"
    );
    assert!(w.players[1].inv.iter().all(|s| s.count == 0));
}

#[test]
fn a_naked_death_leaves_no_litter() {
    let mut w = duel_world();
    for s in w.players[1].inv.iter_mut() {
        *s = ItemStack::default();
    }
    kill_player_two(&mut w);
    assert!(
        w.backpacks.is_empty(),
        "a body with nothing on it drops nothing"
    );
    assert!(!ev_codes(&w).iter().any(|(c, _, _)| *c == EV_BAG_DROPPED));
}

#[test]
fn looting_nothing_is_not_an_error() {
    let mut w = duel_world();
    let inv_before = w.players[0].inv;
    w.tick(&[Command::Loot { id: 1 }]);
    assert!(w.backpacks.is_empty());
    assert_eq!(
        w.players[0].inv, inv_before,
        "a reach that finds no bag takes nothing"
    );
    assert!(
        !ev_codes(&w)
            .iter()
            .any(|(c, _, _)| *c == EV_GATHER || *c == EV_BAG_REMOVED),
        "and says nothing"
    );
}

/// One duel, fought to the kill, hashed. A function rather than two live
/// worlds on purpose: `World` is 450 kB and two of them plus the
/// constructor's temporaries overflow a test thread's stack.
fn hash_after_a_kill_carrying(count: u16) -> (u64, usize) {
    let mut w = duel_world();
    w.players[1].inv[1] = ItemStack { item: JUNK, count };
    kill_player_two(&mut w);
    let slots = w.backpacks.entries()[0].items.len();
    (w.state_hash(), slots)
}

#[test]
fn a_bag_is_hashed_state() {
    // A replay that lost one unit out of a bag must not be able to call
    // itself identical (CLAUDE.md wall 5).
    let (h42, slots) = hash_after_a_kill_carrying(42);
    let (h41, _) = hash_after_a_kill_carrying(41);
    assert_ne!(h42, h41, "one unit of loot moves the state hash");
    assert_eq!(slots, INV_SLOTS, "the whole inventory, one entity");
}
