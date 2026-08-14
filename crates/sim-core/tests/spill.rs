//! The spill: what a full pack no longer destroys.
//!
//! `gather::inv_add` used to drop the remainder on the floor in the
//! figurative sense only — the items ceased to exist, and the only trace
//! was an `EV_GATHER` reporting a smaller number than the node paid. The
//! backpack module's own header named this as the last thing v0 did not
//! do. It does it now: a node's yield and a finished craft go into a tick
//! buffer the caller owns, and `world.rs` stands a bag up where the body
//! is standing.
//!
//! Everything here comes off the probe fixtures — `GatherContent` (all
//! items stack to 100), `BackpackContent` (90-tick floor, 360 for items
//! 0..3). Nothing invents a number.

use sim_core::backpack::{BackpackContent, Backpacks, LOOT_REACH_M};
use sim_core::craft::inv_count;
use sim_core::gather::{GatherContent, ItemStack, SWING_INTERVAL_TICKS};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::INV_SLOTS;
use sim_core::movement::POS_XZ_Q;
use sim_core::terrain::{self, Occupant, ScatterTable, CELL_SIZE};
use sim_core::world::{Command, EventQueue, World, EV_BAG_DROPPED};

/// The gather suite's seed, deliberately: the node this file swings at is
/// the node `tests/gather.rs` already proves pays its hand row, so a
/// failure here is about the spill and never about the scatter.
const SEED: u64 = 20_260_731;

/// A fixture item with the short 90-tick lifetime and no tool row, used to
/// wall off every inventory slot. Not the tree's output.
const JUNK: u16 = 7;
const STACK_MAX: u16 = 100;

/// A gatherable slot, the walkable point 1.2 m west of it, and the yaw
/// that best faces it. Ported from `tests/gather.rs::find_isolated`, which
/// is a test helper and not a public API; the isolation test is what makes
/// the swing target unambiguous. Panics rather than skipping — a seed that
/// offers no isolated node is a setup failure, and a skipped case that
/// reports success is the worst bug class (CLAUDE.md).
fn find_isolated(seed: u64, want: Occupant) -> ((f32, f32), u16) {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);
    for cz in 40..216i32 {
        for cx in 40..216i32 {
            let s = terrain::scatter(seed, &table, &haven, cx, cz);
            if s.occupant != want {
                continue;
            }
            let (px, pz) = (s.x - 1.2, s.z);
            let py = terrain::height(seed, px, pz);
            if (s.y - py).max(py - s.y) > 1.0 || py < 1.0 {
                continue; // node on a ledge or in the sea
            }
            let pcx = (px / CELL_SIZE) as i32;
            let pcz = (pz / CELL_SIZE) as i32;
            let mut rivals = 0;
            for dz in -1..=1i32 {
                for dx in -1..=1i32 {
                    let n = terrain::scatter(seed, &table, &haven, pcx + dx, pcz + dz);
                    if sim_core::gather::node_index(n.occupant).is_some() {
                        let d2 = (n.x - px) * (n.x - px) + (n.z - pz) * (n.z - pz);
                        if d2 <= 6.25 && (n.x != s.x || n.z != s.z) {
                            rivals += 1;
                        }
                    }
                }
            }
            if rivals > 0 {
                continue;
            }
            // Best of the 256 LUT headings toward the node. The heading
            // lives in the TOP byte of the u16 — `hi << 8`, not `hi`, or
            // every candidate is the same bearing to within a fraction of
            // a degree and the argmax returns whatever it started at.
            let (dx, dz) = (s.x - px, s.z - pz);
            let mut best_yaw = 0u16;
            let mut best_dot = f32::MIN;
            for hi in 0..=255u16 {
                let yaw = hi << 8;
                let (fx, fz) = sim_core::yaw_dir(yaw);
                let dot = fx * dx + fz * dz;
                if dot > best_dot {
                    best_dot = dot;
                    best_yaw = yaw;
                }
            }
            return ((px, pz), best_yaw);
        }
    }
    panic!("seed {seed} offers no isolated {want:?} — test setup failure");
}

fn hold_primary(yaw: u16, seq: u16) -> Command {
    Command::Input {
        id: 1,
        frame: InputFrame {
            seq,
            buttons: BTN_PRIMARY,
            yaw,
            pitch: 0,
            move_x: 0,
            move_z: 0,
            sel: 0,
        },
    }
}

/// Player 1 at `pos`, gather and backpack fixtures armed, and every one of
/// the 30 inventory slots walled off with junk at the ceiling — so the
/// very first swing has nowhere to pay. Boxed for the reason
/// `tests/gather.rs` states: a `World` through a test frame overflows the
/// default test-thread stack.
fn full_pack_world(pos: (f32, f32)) -> Box<World> {
    let mut w = Box::new(World::new(SEED));
    w.gather = GatherContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    // The weak spot is off for the same reason the base-pay gather test
    // turns it off: a stationary swinger lands in the roaming mark's
    // sector on some seeds and doubles a payout, which would make the
    // spilled count seed-dependent.
    for n in w.gather.nodes.iter_mut() {
        n.weak_pct = 0;
    }
    w.dev_spawn = Some(pos);
    w.tick(&[Command::Join { id: 1 }]);
    for s in w.players[0].inv.iter_mut() {
        *s = ItemStack {
            item: JUNK,
            count: STACK_MAX,
        };
    }
    w
}

/// The whole point. A swing at a node with no room for its yield used to
/// pay nothing and destroy the wood; it now stands a bag up underfoot
/// holding exactly what the node paid.
#[test]
fn a_full_pack_drops_the_yield_at_your_feet() {
    let (pos, yaw) = find_isolated(SEED, Occupant::Tree);
    let mut w = full_pack_world(pos);
    let tree = w.gather.nodes[0];
    let (px, pz) = (w.players[0].body.qx, w.players[0].body.qz);

    assert_eq!(w.backpacks.len(), 0, "no bag before the swing");
    w.tick(&[hold_primary(yaw, 0)]);

    assert_eq!(
        inv_count(&w.players[0].inv, tree.output),
        0,
        "the pack was full: nothing reached the hands"
    );
    assert_eq!(w.backpacks.len(), 1, "the yield stood a bag up instead");
    let bag = w.backpacks.entries()[0];
    assert_eq!(
        inv_count(&bag.items, tree.output),
        tree.hand_yield as u32,
        "the bag holds exactly what the node paid, not a rounded share"
    );
    assert_eq!((bag.qx, bag.qz), (px, pz), "it fell where the swinger is");
    assert_eq!(bag.owner, 1, "credited to the swinger");
    assert!(
        w.events
            .entries()
            .iter()
            .any(|e| e.code == EV_BAG_DROPPED && e.a == bag.id),
        "a new container on the ground announces itself"
    );
}

/// The bound. A player farming at a full pack pays a swing every
/// `SWING_INTERVAL_TICKS` — a bag apiece would churn `MAX_BACKPACKS` in
/// minutes and evict other people's death bags to do it. Standing still
/// costs one bag however long you swing, because the spill merges into the
/// bag already in reach.
#[test]
fn swinging_on_at_a_full_pack_grows_one_bag_not_many() {
    let (pos, yaw) = find_isolated(SEED, Occupant::Tree);
    let mut w = full_pack_world(pos);
    let tree = w.gather.nodes[0];

    // Every swing the node has in it, at the fixed cadence.
    for seq in 0..(SWING_INTERVAL_TICKS * (tree.hits as u64 + 1)) {
        w.tick(&[hold_primary(yaw, seq as u16)]);
        assert!(
            w.backpacks.len() <= 1,
            "a second bag was minted at tick {} — the merge is what bounds this",
            w.tick
        );
    }
    assert_eq!(w.backpacks.len(), 1, "one bag caught the whole node");
    assert_eq!(
        inv_count(&w.backpacks.entries()[0].items, tree.output),
        (tree.hits as u32) * tree.hand_yield as u32,
        "and it holds the node's whole yield — the total gather.rs pins \
         for a pack that had room"
    );
}

/// A bag that grows keeps the longer clock. This is the case that makes
/// the `if want > expires` guard load-bearing rather than decorative: a
/// **partly looted** bag still carries the expiry its rare contents bought,
/// and recomputing the clock on a merge would pull it in by the difference
/// between the ladder's rungs — the bag would despawn early because
/// somebody dropped wood in it.
///
/// Driven against `spill_at` directly. Going through a swing cannot reach
/// it: whatever the node pays is still inside, so the recomputed clock is
/// always the later one and the guard never has to say no.
#[test]
fn a_merge_never_pulls_a_partly_looted_bag_s_clock_in() {
    let bc = BackpackContent::probe_fixture();
    let gc = GatherContent::probe_fixture();
    let mut bp = Backpacks::new();
    let mut ev = EventQueue::default();

    // A bag holding one rare item (360 ticks) and one junk (90).
    let mut held = [ItemStack::default(); INV_SLOTS];
    held[0] = ItemStack { item: 0, count: 1 };
    held[1] = ItemStack {
        item: JUNK,
        count: 1,
    };
    let id = bp
        .stand_up(&bc, 0, 0, 0, 1, &held, 100, &mut ev)
        .expect("the fixture ladder is armed");
    assert_eq!(bp.entries()[0].expires, 100 + 360, "the rare item set it");

    // The move verb takes the rare item out — the ordinary partly-looted
    // bag. Its clock does not shorten, by design (nothing recomputes it).
    bp.set_slot(0, 0, ItemStack::default());
    assert_eq!(bp.entries()[0].expires, 460);

    // Now spill junk into it. Recomputed from scratch this would be
    // 200 + 90 = 290, a quarter of an hour early at 30 Hz.
    let mut spill = [ItemStack::default(); INV_SLOTS];
    spill[0] = ItemStack {
        item: JUNK,
        count: 5,
    };
    assert_eq!(
        bp.spill_at(&bc, &gc, 0, 0, 0, 1, &mut spill, 200, &mut ev),
        Some(id),
        "same address: merged into the standing bag, not a new one"
    );
    assert_eq!(bp.len(), 1, "merged rather than minted");
    assert_eq!(
        bp.entries()[0].expires,
        460,
        "the clock must stand, never be recomputed downward"
    );
    assert_eq!(inv_count(&bp.entries()[0].items, JUNK), 6, "1 + 5 merged");
    assert!(
        spill.iter().all(|s| s.count == 0),
        "the caller's buffer is drained by what the bag took"
    );
}

/// Out of reach is out of reach: a spill more than `LOOT_REACH_M` from a
/// standing bag mints its own rather than teleporting into one across the
/// clearing.
#[test]
fn a_spill_out_of_reach_stands_its_own_bag_up() {
    let bc = BackpackContent::probe_fixture();
    let gc = GatherContent::probe_fixture();
    let mut bp = Backpacks::new();
    let mut ev = EventQueue::default();

    let mut held = [ItemStack::default(); INV_SLOTS];
    held[0] = ItemStack {
        item: JUNK,
        count: 1,
    };
    let near = bp.stand_up(&bc, 0, 0, 0, 1, &held, 100, &mut ev).unwrap();

    // Two reaches away, in the movement quantum the store speaks.
    let far_q = ((LOOT_REACH_M * 2.0) / POS_XZ_Q) as i32;
    let mut spill = [ItemStack::default(); INV_SLOTS];
    spill[0] = ItemStack {
        item: JUNK,
        count: 3,
    };
    let made = bp
        .spill_at(&bc, &gc, far_q, 0, 0, 1, &mut spill, 110, &mut ev)
        .unwrap();
    assert_ne!(made, near, "a second bag, not the first one moved");
    assert_eq!(bp.len(), 2);
    assert_eq!(
        inv_count(&bp.entries()[0].items, JUNK),
        1,
        "the far bag is untouched"
    );
}

/// The disarm survives. `base_ticks == 0` is content that never armed the
/// backpack module, and its stated meaning is "the world before this
/// slice" — so overflow is destroyed exactly as it used to be, and no bag
/// appears on a shard whose content asked for none.
#[test]
fn an_inert_ladder_still_destroys_the_overflow() {
    let (pos, yaw) = find_isolated(SEED, Occupant::Tree);
    let mut w = full_pack_world(pos);
    w.backpack = BackpackContent::EMPTY;
    let tree = w.gather.nodes[0];

    for seq in 0..(SWING_INTERVAL_TICKS * 3) {
        w.tick(&[hold_primary(yaw, seq as u16)]);
    }
    assert_eq!(w.backpacks.len(), 0, "an inert ladder stands nothing up");
    assert_eq!(
        inv_count(&w.players[0].inv, tree.output),
        0,
        "and the yield is gone, as it was before the spill lane"
    );
}

/// Wall 5 applied to the new state: the bag the spill mints, its id, its
/// address and its clock are all sim state, so two runs of the same
/// commands must hash identically.
#[test]
fn the_spill_replays_bit_identically() {
    let (pos, yaw) = find_isolated(SEED, Occupant::Tree);
    let run = || {
        let mut w = full_pack_world(pos);
        for seq in 0..(SWING_INTERVAL_TICKS * 2) {
            w.tick(&[hold_primary(yaw, seq as u16)]);
        }
        (w.state_hash(), w.backpacks.len())
    };
    let (a, bags_a) = run();
    let (b, bags_b) = run();
    assert_eq!(bags_a, 1, "the run actually spilled something");
    assert_eq!(bags_a, bags_b);
    assert_eq!(a, b, "the spill must replay bit-identically");
}
