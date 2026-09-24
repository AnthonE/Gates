//! The loose-stack store (`grounditem.rs`): where a scattered stack lands,
//! how long it lies there, what the cap does when the island is littered,
//! and what a take gives you.
//!
//! `tests/loot.rs` drives the barrel end to end and therefore proves the
//! *path*; this file proves the *store*, because three of its rules cannot
//! be reached through a barrel with the probe fixture at all — two stacks
//! from one container (the fixture's two rolls usually merge into one
//! slot), the eviction branch (256 stacks), and the zero-ceiling item.
//!
//! Every number comes from a `probe_fixture` (wall 7).

use sim_core::backpack::{BackpackContent, LOOT_REACH_M};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::grounditem::{rest_spot, GroundItems, SCATTER_R_M};
use sim_core::limits::{INV_SLOTS, MAX_GROUND_ITEMS};
use sim_core::movement::{quant_xz, quant_y, Body, POS_XZ_Q};
use sim_core::terrain;
use sim_core::world::{Command, EventQueue, Player, World};

const SEED: u64 = 20260804;
const PLAYER: u32 = 1;

/// The solved authored sites for `SEED`, memoized — `terrain::haven` is
/// thousands of height taps and these assertions run in loops.
fn hv() -> &'static terrain::Haven {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Option<&'static terrain::Haven>> = const { RefCell::new(None) };
    }
    if let Some(h) = CACHE.with(|c| *c.borrow()) {
        return h;
    }
    let h: &'static terrain::Haven = Box::leak(Box::new(terrain::haven(SEED)));
    CACHE.with(|c| *c.borrow_mut() = Some(h));
    h
}

fn stack(item: u16, count: u16) -> ItemStack {
    ItemStack {
        item,
        count,
        cond: 0,
        skin: 0,
    }
}

/// A roll with `n` distinct stacks in it.
fn roll(n: usize) -> [ItemStack; INV_SLOTS] {
    let mut items = [ItemStack::default(); INV_SLOTS];
    for (k, slot) in items.iter_mut().enumerate().take(n) {
        *slot = stack(k as u16, (k as u16 + 1) * 3);
    }
    items
}

/// Somewhere on land, in quanta.
fn land_spot() -> (i32, i32) {
    // The island is centred, so its middle is land on every seed
    // (`relief.rs`'s window lesson: start from where the world is).
    let c = terrain::ISLAND_SIZE * 0.5;
    (quant_xz(c), quant_xz(c))
}

// ------------------------------------------------------------ landing

/// **Two stacks out of one container are two objects**, which is the whole
/// of what a bag could not say and the reason this store exists.
#[test]
fn two_stacks_land_on_different_points() {
    let mut g = GroundItems::new();
    let bc = BackpackContent::probe_fixture();
    let (qx, qz) = land_spot();
    let made = g.scatter(&bc, SEED, hv(), 0x1234, qx, qz, &roll(2), 100);
    assert_eq!(made, 2, "both stacks reached the ground");
    let a = g.entries()[0];
    let b = g.entries()[1];
    assert_ne!(
        (a.qx, a.qz),
        (b.qx, b.qz),
        "two stacks out of one container landed on the same point — the \
         scatter is keyed on the container and not on the stack"
    );
    assert_ne!(a.id, b.id, "two stacks share an id");
}

/// Every stack rests on the ground **under it**, not at the height of the
/// thing that paid it. This is the operator's *"fall down"* in v0: a stack
/// whose offset takes it over a lip lies where the lip is.
#[test]
fn every_stack_rests_on_the_ground_under_it() {
    let mut g = GroundItems::new();
    let bc = BackpackContent::probe_fixture();
    let (qx, qz) = land_spot();
    g.scatter(&bc, SEED, hv(), 0x99, qx, qz, &roll(4), 10);
    assert_eq!(g.len(), 4);
    for it in g.entries() {
        let x = it.qx as f32 * POS_XZ_Q;
        let z = it.qz as f32 * POS_XZ_Q;
        assert_eq!(
            it.qy,
            quant_y(terrain::ground(SEED, hv(), x, z)),
            "a stack is floating or buried"
        );
    }
}

/// The scatter stays inside its own radius, swept over many containers
/// rather than one — a modulo that biased one way would pass a single
/// case.
#[test]
fn the_scatter_stays_inside_its_radius() {
    let span = quant_xz(SCATTER_R_M).max(1);
    let (qx, qz) = land_spot();
    let mut seen_neg = false;
    let mut seen_pos = false;
    for key in 0..400u32 {
        for k in 0..3usize {
            let (ix, _, iz) = rest_spot(SEED, hv(), key, k, qx, qz);
            let dx = ix - qx;
            let dz = iz - qz;
            assert!(
                dx.abs() <= span && dz.abs() <= span,
                "offset ({dx}, {dz}) is outside ±{span} quanta"
            );
            if dx < 0 || dz < 0 {
                seen_neg = true;
            }
            if dx > 0 || dz > 0 {
                seen_pos = true;
            }
        }
    }
    // Not vacuous: an offset that was always zero would satisfy the bound.
    assert!(
        seen_neg && seen_pos,
        "the scatter only ever moved one way — the draw is biased or dead"
    );
}

/// Same seed, same container, same tick ⇒ same landing spots. Wall 1 and
/// wall 5 in one assertion: a replay has to put the loot back where it was.
#[test]
fn the_same_container_scatters_identically_twice() {
    let bc = BackpackContent::probe_fixture();
    let (qx, qz) = land_spot();
    let mut a = GroundItems::new();
    let mut b = GroundItems::new();
    a.scatter(&bc, SEED, hv(), 77, qx, qz, &roll(3), 500);
    b.scatter(&bc, SEED, hv(), 77, qx, qz, &roll(3), 500);
    assert_eq!(a.entries(), b.entries(), "two runs scattered differently");
}

// ------------------------------------------------------------ lifetime

/// The despawn ladder is **the bag's**, asked about one item instead of a
/// container's worth — so a rare stack in the grass lives exactly as long
/// as a rare bag, and neither can be gamed against the other.
#[test]
fn the_despawn_ladder_is_the_bags() {
    let bc = BackpackContent::probe_fixture();
    let mut g = GroundItems::new();
    let (qx, qz) = land_spot();
    // Item 0 is the fixture's rare half (4× the floor), item 8 common.
    let mut items = [ItemStack::default(); INV_SLOTS];
    items[0] = stack(0, 1);
    items[1] = stack(8, 1);
    g.scatter(&bc, SEED, hv(), 5, qx, qz, &items, 1_000);
    let rare = g.entries()[0];
    let common = g.entries()[1];
    assert!(
        rare.expires > common.expires,
        "a rare stack does not outlive a common one"
    );
    // And each is exactly what a bag holding only that item would get.
    for it in [rare, common] {
        let mut one = [ItemStack::default(); INV_SLOTS];
        one[0] = it.stack;
        assert_eq!(
            it.expires,
            1_000 + bc.lifetime_ticks(&one) as u64,
            "the loose ladder and the bag ladder disagree about item {}",
            it.stack.item
        );
    }
}

/// The sweep retires a stack on the tick its own `expires` names — not one
/// early, not one late.
#[test]
fn a_stack_leaves_on_the_tick_it_named() {
    let bc = BackpackContent::probe_fixture();
    let mut g = GroundItems::new();
    let (qx, qz) = land_spot();
    g.scatter(&bc, SEED, hv(), 1, qx, qz, &roll(1), 0);
    let due = g.entries()[0].expires;
    assert!(due > 0);
    assert_eq!(g.expire_due(due - 1), 0, "it left a tick early");
    assert_eq!(g.len(), 1);
    assert_eq!(g.expire_due(due), 1, "it did not leave on its own tick");
    assert!(g.is_empty());
}

/// Inert content disarms the store whole, exactly as it disarms the bag: a
/// shard whose ladder was never authored gets no litter with no lifetime.
#[test]
fn inert_content_scatters_nothing() {
    let mut g = GroundItems::new();
    let (qx, qz) = land_spot();
    let made = g.scatter(&BackpackContent::EMPTY, SEED, hv(), 1, qx, qz, &roll(2), 10);
    assert_eq!(made, 0);
    assert!(g.is_empty());
}

// ------------------------------------------------------------ the cap

/// **The cap evicts the stack nearest its own despawn**, and this is the
/// branch no barrel can reach — 256 stacks is over a hundred barrels.
#[test]
fn the_cap_evicts_the_stack_nearest_its_despawn() {
    let bc = BackpackContent::probe_fixture();
    let mut g = GroundItems::new();
    let (qx, qz) = land_spot();
    // Fill it, one stack at a time, each a tick later than the last so
    // the expiries are strictly ordered and the victim is knowable.
    for t in 0..MAX_GROUND_ITEMS as u64 {
        g.scatter(&bc, SEED, hv(), t as u32, qx, qz, &roll(1), t);
    }
    assert_eq!(g.len(), MAX_GROUND_ITEMS, "the store did not fill");
    let doomed = g
        .entries()
        .iter()
        .min_by_key(|e| e.expires)
        .copied()
        .expect("full");
    let id_before = g.next_id();

    g.scatter(&bc, SEED, hv(), 9_999, qx, qz, &roll(1), 10_000);

    assert_eq!(g.len(), MAX_GROUND_ITEMS, "the cap did not hold");
    assert!(
        g.index_of_id(doomed.id).is_none(),
        "the evicted stack was not the one nearest its despawn"
    );
    assert!(
        g.index_of_id(id_before).is_some(),
        "the fresh stack was refused instead of the doomed one being evicted \
         — a barrel somebody paid three swings for was eaten by the cap"
    );
}

// ------------------------------------------------------------- the take

fn world() -> World {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    w.tick(&[Command::Join { id: PLAYER }]);
    let c = terrain::ISLAND_SIZE * 0.5;
    w.players[0].body = Body::at(SEED, hv(), c, c);
    w
}

/// The pick is nearest-in-reach, which is what lets the verb carry no
/// payload: nothing aimable at a stack the player is not standing on ever
/// crosses the wire.
#[test]
fn the_take_is_nearest_first() {
    let mut w = world();
    let bc = w.backpack;
    let (pqx, pqz) = (w.players[0].body.qx, w.players[0].body.qz);
    // Two stacks, hand-placed: one at the feet, one four metres away.
    let far = quant_xz(4.0);
    let near_spot = (pqx, pqz);
    let far_spot = (pqx + far, pqz);
    for (i, (qx, qz)) in [near_spot, far_spot].into_iter().enumerate() {
        let mut items = [ItemStack::default(); INV_SLOTS];
        items[0] = stack(i as u16, 1);
        w.ground_items
            .scatter(&bc, SEED, hv(), i as u32 + 1, qx, qz, &items, w.tick);
    }
    assert_eq!(w.ground_items.len(), 2);
    let ids: Vec<u32> = w.ground_items.entries().iter().map(|g| g.id).collect();

    w.tick(&[Command::Pickup { id: PLAYER }]);
    assert_eq!(
        w.ground_items.len(),
        1,
        "the take took nothing, or took two"
    );
    assert!(
        w.ground_items.index_of_id(ids[0]).is_none(),
        "the far stack was taken before the near one"
    );
}

/// A stack out of reach is a stack you have to walk to — the same arm every
/// other world interaction uses.
#[test]
fn a_take_out_of_reach_takes_nothing() {
    let mut w = world();
    let bc = w.backpack;
    let out = quant_xz(LOOT_REACH_M + 3.0);
    let (qx, qz) = (w.players[0].body.qx + out, w.players[0].body.qz);
    w.ground_items
        .scatter(&bc, SEED, hv(), 1, qx, qz, &roll(1), w.tick);
    let before = w.ground_items.entries()[0];

    w.tick(&[Command::Pickup { id: PLAYER }]);

    assert_eq!(w.ground_items.len(), 1, "an out-of-reach stack was taken");
    assert_eq!(w.ground_items.entries()[0], before, "and it did not change");
}

/// A take into a full pack **spills at the feet** rather than destroying
/// what it could not carry — the same drain every other payout uses.
#[test]
fn a_take_into_a_full_pack_spills_rather_than_destroying() {
    let mut w = world();
    let bc = w.backpack;
    // Fill every slot with an item the incoming stack cannot merge into.
    for s in w.players[0].inv.iter_mut() {
        *s = stack(7, 100);
    }
    let (qx, qz) = (w.players[0].body.qx, w.players[0].body.qz);
    let mut items = [ItemStack::default(); INV_SLOTS];
    items[0] = stack(3, 9);
    w.ground_items
        .scatter(&bc, SEED, hv(), 1, qx, qz, &items, w.tick);

    w.tick(&[Command::Pickup { id: PLAYER }]);

    assert!(w.ground_items.is_empty(), "the stack stayed on the ground");
    let in_bags: u32 = w
        .backpacks
        .entries()
        .iter()
        .flat_map(|b| b.items.iter())
        .filter(|s| s.item == 3)
        .map(|s| s.count as u32)
        .sum();
    assert_eq!(
        in_bags, 9,
        "a take into a full pack destroyed what it could not carry"
    );
}

/// An item no stack ladder can size (`stack_max == 0`,
/// `REFUSE_M_UNSTACKABLE`'s condition) **stays on the ground** rather than
/// vanishing off the island by being looked at.
#[test]
fn a_stack_no_ladder_can_size_stays_where_it_is() {
    let mut gc = GatherContent::probe_fixture();
    let ghost = 9u16;
    gc.stack_max[ghost as usize] = 0;
    let bc = BackpackContent::probe_fixture();
    let mut g = GroundItems::new();
    let mut p = Player {
        id: PLAYER,
        ..Default::default()
    };
    let (qx, qz) = land_spot();
    p.body = Body::at(SEED, hv(), qx as f32 * POS_XZ_Q, qz as f32 * POS_XZ_Q);
    let mut items = [ItemStack::default(); INV_SLOTS];
    items[0] = stack(ghost, 1);
    g.scatter(&bc, SEED, hv(), 1, p.body.qx, p.body.qz, &items, 0);
    assert_eq!(g.len(), 1);

    let mut spill = [ItemStack::default(); INV_SLOTS];
    let mut events = EventQueue::default();
    let took = g.take_nearest(&gc, &mut p, &mut spill, &mut events);

    assert!(took.is_none(), "an unstackable item reported a take");
    assert_eq!(g.len(), 1, "it vanished off the island instead of staying");
    assert!(p.inv.iter().all(|s| s.count == 0));
    assert!(spill.iter().all(|s| s.count == 0));
}
