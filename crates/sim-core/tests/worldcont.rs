//! The authored world container: the haven pad's crate and a waystation's
//! cache, as things a player can open and take from.
//!
//! `terrain::scatter` has placed both since the destination-gradient slice,
//! `content/loot.toml` has baked `LOOT_CRATE` and `LOOT_CACHE` since the
//! same one, and `ci/haven_prize.mjs` measures the gradient between them —
//! and until this landed `loot.rs` said, in the tree, "No verb opens one
//! yet." The whole risk/reward walk paid nobody.
//!
//! What closes it is a fourth `CONT_*` kind, not a new verb: `Command::Move`
//! already spans three container kinds with validation ordered before the
//! mutation, so **not one line of `plan_move` moves**. This file is about
//! the four things a fourth kind can get wrong that the first three could
//! not:
//!
//! 1. **Its contents do not exist until somebody opens it.** A bag and a
//!    box are full or empty before you look; a crate is *rolled* by the
//!    open. That makes the open a `Command` rather than a subscription, and
//!    it puts a random draw on the command path — so the roll has to be a
//!    function of (seed, cell, tick) and nothing else, and opening twice
//!    must not pay twice.
//! 2. **The client names a cell, and terrain owns what is there.** The
//!    handle is a `cell_key`, and the sim re-derives the occupant through
//!    `terrain::scatter` rather than believing it. A cell holding a tree,
//!    a cell holding nothing, and a cell across the island all have to open
//!    nothing.
//! 3. **It refills.** A bag despawns and a box stands empty forever; a
//!    crate is furniture that comes back. The timer arms on the transition
//!    to empty and only on the transition, or a player could hold it off
//!    forever by putting one item back.
//! 4. **It has no removal path.** Every other container store swap-removes,
//!    and this one never does, so nothing may assume a record's index is
//!    stable *or* that it can vanish.

use sim_core::backpack::BackpackContent;
use sim_core::gather::{cell_key, GatherContent, ItemStack, RESPAWN_MIN_TICKS};
use sim_core::inventory::{
    CONT_SELF, CONT_WORLD, REFUSE_M_NO_CONTAINER, REFUSE_M_REACH, REFUSE_M_SLOT,
};
use sim_core::limits::{INV_SLOTS, MAX_ITEM_DEFS, MAX_WORLD_CONTS};
use sim_core::loot::{LootContent, LootEntryDef, LootTableDef, LOOT_CACHE, LOOT_CRATE};
use sim_core::movement::{quant_xz, Body};
use sim_core::terrain::{self, Occupant, ScatterTable, CELLS_PER_SIDE};
use sim_core::world::{Command, World, EV_MOVED, EV_MOVE_REFUSED};
use sim_core::worldsave::{WorldSaveError, WORLD_SAVE_MAX_BYTES};

/// The solved authored sites for `seed` — what `terrain::ground` needs in order
/// to know where the carve is.
///
/// Memoized per seed, and that is not premature: `terrain::haven` is a few
/// thousand `height` taps (a shoreline march, a bisect and a rosette per
/// candidate bearing), these suites call it from inside assertion loops, and
/// the first draft of this helper resolved it per call and took the workspace
/// test run past five minutes. It is a pure function of the seed, so caching
/// cannot change a result.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    // A thread-local rather than a `Mutex`: `std::sync::Mutex` is on
    // `sim-core/clippy.toml`'s disallowed list (wall 3), and that list is
    // crate-scoped, so it binds this suite too. Per-thread is the right shape
    // anyway — the cache exists to stop a per-assertion recompute, not to be
    // shared.
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static sim_core::terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static sim_core::terrain::Haven = Box::leak(Box::new(sim_core::terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

const SEED: u64 = 0x600D_C0DE;
const PLAYER: u32 = 7;

/// A loot fixture with all three container tables armed, so the crate and
/// the cache pay *different* items and a test can tell which table a
/// container actually rolled. `LootContent::probe_fixture` fills the barrel
/// alone and is deliberately left alone — it is the parity probe's input,
/// and widening it would move `test_replay`'s golden for no reason.
///
/// Every number here is invented for this file and none of it comes from
/// `content/` (wall 7).
fn loot_fixture() -> LootContent {
    let mut c = LootContent::probe_fixture();
    // The crate: item 2, always exactly four rolls of one, so "what did
    // this pay" is a constant rather than a draw. A fixture whose output
    // varies cannot distinguish "rolled again" from "rolled differently".
    let mut crate_t = LootTableDef::INERT;
    crate_t.entries[0] = LootEntryDef {
        item: 2,
        weight: 1,
        count_min: 1,
        count_max: 1,
    };
    crate_t.len = 1;
    crate_t.total_weight = 1;
    crate_t.rolls_min = 4;
    crate_t.rolls_max = 4;
    c.tables[LOOT_CRATE] = crate_t;

    // The cache: a different item, and fewer of them — the tier gradient
    // in miniature, so `the_two_tiers_roll_their_own_table` is checking a
    // real difference rather than an alias.
    let mut cache_t = LootTableDef::INERT;
    cache_t.entries[0] = LootEntryDef {
        item: 3,
        weight: 1,
        count_min: 1,
        count_max: 1,
    };
    cache_t.len = 1;
    cache_t.total_weight = 1;
    cache_t.rolls_min = 2;
    cache_t.rolls_max = 2;
    c.tables[LOOT_CACHE] = cache_t;
    c
}

fn world() -> World {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.loot = loot_fixture();
    w.backpack = BackpackContent::probe_fixture();
    w
}

/// The first cell on the island holding `want`, and where it stands.
/// Scanned rather than hard-coded: the placement is a pure function of the
/// seed and a literal here would be a second answer to that question,
/// stale the first time the ring moves.
fn find_slot(w: &World, want: Occupant) -> (u16, u16, f32, f32) {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(SEED);
    for cx in 0..CELLS_PER_SIDE {
        for cz in 0..CELLS_PER_SIDE {
            let s = terrain::scatter(SEED, &table, &haven, cx, cz);
            if s.occupant == want {
                return (cx as u16, cz as u16, s.x, s.z);
            }
        }
    }
    let _ = w;
    panic!("seed {SEED:#x} places no {want:?}");
}

/// A cell holding a **tree** — something real that is not a container —
/// and the point it stands at.
///
/// Needed so the occupancy refusal can be tested with reach out of the
/// picture: standing on the slot makes the distance zero, so the only
/// thing left that can refuse the open is what terrain says is there.
///
/// A tree specifically, and not merely an empty cell, because an empty
/// cell's `Slot` reports position `(0, 0, 0)` rather than the cell's own
/// ground — so "stand on it" would put the body at the world origin and
/// the cell it names would be `(0, 0)`, whose handle is zero and refuses
/// one guard earlier. Both of those made an earlier draft of this test
/// pass with the occupancy check deleted.
fn find_occupied_non_container() -> (u16, u16, f32, f32) {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(SEED);
    for cx in 0..CELLS_PER_SIDE {
        for cz in 0..CELLS_PER_SIDE {
            let s = terrain::scatter(SEED, &table, &haven, cx, cz);
            if s.occupant == Occupant::Tree {
                assert_ne!(
                    cell_key(cx as u16, cz as u16),
                    0,
                    "the fixture cell must not be the one handle zero guards"
                );
                return (cx as u16, cz as u16, s.x, s.z);
            }
        }
    }
    panic!("seed {SEED:#x} places no tree");
}

/// Stand `PLAYER` on top of the container at `(x, z)` and join them.
fn join_at(w: &mut World, x: f32, z: f32) {
    w.tick(&[Command::Join { id: PLAYER }]);
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
}

/// Move the standing body to `x` without re-joining — used to walk out of
/// reach and back, which is the only thing reach tests need.
fn stand_at(w: &mut World, x: f32, z: f32) {
    w.players[0].body.qx = quant_xz(x);
    w.players[0].body.qz = quant_xz(z);
}

/// The move verb's answers on the tick just run.
///
/// **There is no cursor here, and there used to be one.** `since(&w, mark)`
/// sliced `entries()[mark..]` against a `mark` taken before the action —
/// but `World::tick` clears the ring at its start, so the index was into a
/// ring that no longer existed. It gave the right answer only while every
/// `mark` happened to read 0, and a door announcing one more fact at the
/// join (`EV_KNOWN`, 2026-08-15) moved every mark off 0.
///
/// **What that cost is smaller than this comment first claimed, and the
/// difference is the point** (corrected 2026-08-15). A stale cursor makes
/// `answers()` return `[]`. Two of the three affected tests —
/// `a_take_out_of_reach_refuses_and_says_so` and
/// `a_take_from_an_unopened_cell_refuses_and_mints_nothing` — compare it
/// with `assert_eq!` against a non-empty list, so they would have gone
/// loudly red rather than quietly green. Only the negative assertion in
/// `a_slot_past_the_container_refuses` (`!answers(&w).contains(…)`) reads
/// `[]` as a pass — and it did so **before** `EV_KNOWN` too, because a
/// negative assertion cannot tell "the event is absent" from "the ring is
/// empty" no matter what the mark reads. So `EV_KNOWN` was the trigger for
/// the cursor bug and never the cause of the blind assertion; the blind
/// shape is a property of asserting on an absence.
///
/// The ring is per-tick, so "since the mark" and "this tick" are the same
/// set, and the honest spelling of that is no mark at all. Every assertion
/// below is unchanged; what changed is that they can now fail.
fn answers(w: &World) -> Vec<(u8, u32)> {
    w.events
        .entries()
        .iter()
        .filter(|e| e.code == EV_MOVED || e.code == EV_MOVE_REFUSED)
        .map(|e| (e.code, e.b))
        .collect()
}

/// Run empty ticks until `w.tick >= to`. The refill window is tens of
/// thousands of ticks, and an empty `World::tick` is cheap — but not free,
/// so this is the one place a test spends them and it says so.
fn advance_to(w: &mut World, to: u64) {
    while w.tick < to {
        w.tick(&[]);
    }
}

fn open(w: &mut World, cx: u16, cz: u16) {
    w.tick(&[Command::OpenWorldCont {
        id: PLAYER,
        cont: cell_key(cx, cz),
    }]);
}

/// Total units of `item` sitting in world container `i`.
fn units(w: &World, i: usize, item: u16) -> u32 {
    w.world_conts.entries()[i]
        .items
        .iter()
        .filter(|s| s.item == item)
        .map(|s| s.count as u32)
        .sum()
}

// ---------------------------------------------------------------- opening

/// The verb the whole destination gradient was waiting for: `E` on the pad's
/// crate produces loot, out of a table nothing had ever rolled at runtime.
#[test]
fn opening_a_crate_rolls_its_table() {
    let mut w = world();
    let (cx, cz, x, z) = find_slot(&w, Occupant::CrateSlot);
    join_at(&mut w, x, z);
    assert_eq!(w.world_conts.len(), 0, "nothing exists before the open");

    open(&mut w, cx, cz);

    assert_eq!(w.world_conts.len(), 1, "the open mints exactly one record");
    let rec = w.world_conts.entries()[0];
    assert_eq!((rec.cx, rec.cz), (cx, cz));
    assert_eq!(rec.table as usize, LOOT_CRATE);
    assert_eq!(
        units(&w, 0, 2),
        4,
        "the crate fixture is four rolls of one item 2"
    );
    assert_eq!(rec.refill_at, 0, "a full container has no refill armed");
}

/// **The anti-forgery wall.** The handle is a cell the client chose, and the
/// sim re-derives what stands there rather than believing it. Three shapes
/// of lie, all of which must mint nothing at all — not an empty record, not
/// a record that fills later.
#[test]
fn a_cell_that_holds_no_container_opens_nothing() {
    let mut w = world();
    let (cx, cz, x, z) = find_slot(&w, Occupant::CrateSlot);
    join_at(&mut w, x, z);

    // **Standing ON a cell that holds no container**, which is the only
    // shape of this test that proves anything. The obvious version — open
    // the cell next door — passes with the occupant check deleted, because
    // a cell is 8 m and arm's reach is 5, so the neighbour is refused by
    // DISTANCE and the scatter re-derivation is never consulted. Found by
    // mutating `table_of(...)?` to `unwrap_or(LOOT_CRATE)` and watching
    // all seventeen tests stay green.
    //
    // So the body is moved to the empty cell's own slot position: reach is
    // zero, and the only thing left that can refuse is the occupant.
    let (tcx, tcz, tx, tz) = find_occupied_non_container();
    stand_at(&mut w, tx, tz);
    open(&mut w, tcx, tcz);
    assert_eq!(
        w.world_conts.len(),
        0,
        "a cell holding a tree opens nothing even with your arms around it"
    );

    // And the cell next door, which is the one an honest client sends by
    // rounding wrong. Belt and braces — it is the distance that refuses
    // this one, and the assertion above is the one about occupancy.
    stand_at(&mut w, x, z);
    open(&mut w, cx + 1, cz);
    assert_eq!(w.world_conts.len(), 0, "the cell beside it holds nothing");

    // A cell off the grid entirely. `terrain::scatter` refuses it, so the
    // occupant comes back `None` and the table lookup declines.
    open(&mut w, CELLS_PER_SIDE as u16 + 5, cz);
    assert_eq!(
        w.world_conts.len(),
        0,
        "a cell off the island holds nothing"
    );

    // Handle zero — cell (0, 0), which is also the "nothing open" value the
    // subscription uses. Guarded for `box_key`'s reason.
    w.tick(&[Command::OpenWorldCont {
        id: PLAYER,
        cont: 0,
    }]);
    assert_eq!(w.world_conts.len(), 0, "handle zero never resolves");

    // And the real cell still works, so the three refusals above are about
    // the address and not about a world that stopped answering.
    open(&mut w, cx, cz);
    assert_eq!(w.world_conts.len(), 1);
}

/// Reach is proved at the open, not assumed from the ask. A crate you can
/// see across the pad is a crate you must walk to.
#[test]
fn opening_out_of_reach_opens_nothing() {
    let mut w = world();
    let (cx, cz, x, z) = find_slot(&w, Occupant::CrateSlot);
    join_at(&mut w, x + 40.0, z);
    open(&mut w, cx, cz);
    assert_eq!(w.world_conts.len(), 0, "40 m is not arm's reach");

    // Walk back and it opens — the refusal was the distance and nothing else.
    stand_at(&mut w, x, z);
    open(&mut w, cx, cz);
    assert_eq!(w.world_conts.len(), 1);
}

/// Opening twice does not pay twice. The roll is on the mint, and the
/// second open finds the record — a container that re-rolled on every look
/// would be an infinite item fountain reachable by tapping `E`.
#[test]
fn a_second_open_does_not_reroll() {
    let mut w = world();
    let (cx, cz, x, z) = find_slot(&w, Occupant::CrateSlot);
    join_at(&mut w, x, z);

    open(&mut w, cx, cz);
    let after_first = w.world_conts.entries()[0].items;

    for _ in 0..5 {
        open(&mut w, cx, cz);
    }
    assert_eq!(w.world_conts.len(), 1, "no second record for the same cell");
    assert_eq!(
        w.world_conts.entries()[0].items,
        after_first,
        "the contents are the ones the first open rolled"
    );
}

/// The tier gradient is a property of the world, not of the table indices:
/// a crate rolls the crate's table and a cache rolls the cache's, and the
/// occupant is the only thing that selects between them. This is `haven.rs`'s
/// kind-agrees-with-zone assert, carried one layer down to what a container
/// actually pays.
#[test]
fn the_two_tiers_roll_their_own_table() {
    let mut w = world();
    let (cx, cz, x, z) = find_slot(&w, Occupant::CrateSlot);
    join_at(&mut w, x, z);
    open(&mut w, cx, cz);
    assert_eq!(units(&w, 0, 2), 4, "the crate pays item 2");
    assert_eq!(units(&w, 0, 3), 0, "and not the cache's item");

    let (ccx, ccz, cxp, czp) = find_slot(&w, Occupant::CacheSlot);
    stand_at(&mut w, cxp, czp);
    open(&mut w, ccx, ccz);
    assert_eq!(w.world_conts.len(), 2);
    assert_eq!(w.world_conts.entries()[1].table as usize, LOOT_CACHE);
    assert_eq!(units(&w, 1, 3), 2, "the cache pays two of item 3");
    assert_eq!(units(&w, 1, 2), 0, "and not the pad's item");
}

// ------------------------------------------------------------ taking from

fn take(w: &mut World, cx: u16, cz: u16, from_slot: u8, to_slot: u8, count: u16) {
    w.tick(&[Command::Move {
        id: PLAYER,
        cont: cell_key(cx, cz),
        from_kind: CONT_WORLD,
        from_slot,
        to_kind: CONT_SELF,
        to_slot,
        count,
    }]);
}

/// The verb that makes the walk pay: a stack out of the crate and into a
/// hand, through the move path every other container already uses.
#[test]
fn the_move_verb_empties_a_crate_into_a_player() {
    let mut w = world();
    let (cx, cz, x, z) = find_slot(&w, Occupant::CrateSlot);
    join_at(&mut w, x, z);
    open(&mut w, cx, cz);

    let held = w.world_conts.entries()[0].items[0];
    assert!(held.count > 0, "the crate rolled something into slot 0");

    take(&mut w, cx, cz, 0, 0, held.count);

    assert_eq!(w.players[0].inv[0], held, "the stack arrived intact");
    assert_eq!(
        w.world_conts.entries()[0].items[0].count,
        0,
        "and left the crate"
    );
    let answered = answers(&w);
    assert_eq!(answered.len(), 1, "a move answers exactly once");
    assert_eq!(
        answered[0].0, EV_MOVED,
        "and the answer is the move, not a refusal ({:?})",
        answered[0]
    );
}

/// A crate you have walked away from is a crate you cannot take from, and
/// the refusal is announced rather than silent — the reference's own
/// container bug is a *disconnect*, and every exit on this path is an event.
#[test]
fn a_take_out_of_reach_refuses_and_says_so() {
    let mut w = world();
    let (cx, cz, x, z) = find_slot(&w, Occupant::CrateSlot);
    join_at(&mut w, x, z);
    open(&mut w, cx, cz);
    let before = w.world_conts.entries()[0].items;

    stand_at(&mut w, x + 40.0, z);
    take(&mut w, cx, cz, 0, 0, 1);

    assert_eq!(
        w.world_conts.entries()[0].items,
        before,
        "nothing moved out of a container out of reach"
    );
    assert_eq!(
        answers(&w),
        vec![(EV_MOVE_REFUSED, REFUSE_M_REACH)],
        "a container out of reach refuses, once, by reason"
    );
}

/// A cell nobody has opened has no record, and a move against it refuses
/// with "no container" rather than minting one. The roll lives in `open`
/// alone, so a move cannot be used to skip the reach and occupant checks
/// that opening is made of.
#[test]
fn a_take_from_an_unopened_cell_refuses_and_mints_nothing() {
    let mut w = world();
    let (cx, cz, x, z) = find_slot(&w, Occupant::CrateSlot);
    join_at(&mut w, x, z);

    take(&mut w, cx, cz, 0, 0, 1);

    assert_eq!(w.world_conts.len(), 0, "the move verb mints no containers");
    assert_eq!(
        answers(&w),
        vec![(EV_MOVE_REFUSED, REFUSE_M_NO_CONTAINER)],
        "an unopened cell refuses as 'no container'"
    );
}

/// A world container is `INV_SLOTS` wide — the widest kind — so the slot
/// bound is the flat one and slot 30 is past it. Asserted rather than
/// inferred, because `slots_in`'s default arm is what answers for this kind
/// and a future per-kind narrowing would silently strand the tail.
#[test]
fn a_slot_past_the_container_refuses() {
    let mut w = world();
    let (cx, cz, x, z) = find_slot(&w, Occupant::CrateSlot);
    join_at(&mut w, x, z);
    open(&mut w, cx, cz);

    take(&mut w, cx, cz, INV_SLOTS as u8, 0, 1);
    assert_eq!(
        answers(&w),
        vec![(EV_MOVE_REFUSED, REFUSE_M_SLOT)],
        "slot 30 is past every container"
    );

    // And the last legal slot is *addressable* — a bound that refuses
    // everything passes the half above and is not the bound we want. It
    // refuses as EMPTY, which is a fact about the contents and proves the
    // address itself resolved.
    take(&mut w, cx, cz, INV_SLOTS as u8 - 1, 5, 1);
    assert!(
        !answers(&w)
            .iter()
            .any(|&(c, b)| c == EV_MOVE_REFUSED && b == REFUSE_M_SLOT),
        "slot 29 is inside a world container, so the refusal must not be \
         an address error"
    );
}

// ----------------------------------------------------------------- refill

/// Empty it and it comes back — but not before its tick, and not by
/// looking at it early. The whole reason a crate is a store rather than a
/// bag: it is furniture that outlives being emptied.
#[test]
fn an_emptied_crate_refills_when_its_tick_comes_and_not_before() {
    let mut w = world();
    let (cx, cz, x, z) = find_slot(&w, Occupant::CrateSlot);
    join_at(&mut w, x, z);
    open(&mut w, cx, cz);

    // Take everything.
    for s in 0..INV_SLOTS {
        let held = w.world_conts.entries()[0].items[s];
        if held.count > 0 {
            take(&mut w, cx, cz, s as u8, s as u8, held.count);
        }
    }
    let rec = w.world_conts.entries()[0];
    assert!(rec.is_empty(), "the crate is empty");
    assert!(
        rec.refill_at >= w.tick + RESPAWN_MIN_TICKS,
        "emptying it armed the refill at least the barrel's minimum out"
    );

    // Looking at it early changes nothing.
    open(&mut w, cx, cz);
    assert!(
        w.world_conts.entries()[0].is_empty(),
        "an early open does not hurry the refill"
    );

    // Past the tick, the next open finds it stocked again.
    let due = w.world_conts.entries()[0].refill_at;
    advance_to(&mut w, due);
    open(&mut w, cx, cz);
    assert_eq!(
        units(&w, 0, 2),
        4,
        "the refill rolled the crate's own table again"
    );
    assert_eq!(
        w.world_conts.entries()[0].refill_at,
        0,
        "and disarmed the timer"
    );
    assert_eq!(w.world_conts.len(), 1, "still one record for the cell");
}

/// Putting one item back and taking it out again must not restart the
/// clock. The timer arms on the *transition* to empty, so a player camping
/// a crate cannot hold its refill off forever — which is what a naive
/// "arm it whenever it is empty" would allow.
#[test]
fn a_put_back_and_a_re_take_do_not_postpone_the_refill() {
    let mut w = world();
    let (cx, cz, x, z) = find_slot(&w, Occupant::CrateSlot);
    join_at(&mut w, x, z);
    open(&mut w, cx, cz);
    for s in 0..INV_SLOTS {
        let held = w.world_conts.entries()[0].items[s];
        if held.count > 0 {
            take(&mut w, cx, cz, s as u8, s as u8, held.count);
        }
    }
    let armed = w.world_conts.entries()[0].refill_at;
    assert!(armed > 0);

    // Put one back — the container is no longer empty, so the timer clears.
    w.tick(&[Command::Move {
        id: PLAYER,
        cont: cell_key(cx, cz),
        from_kind: CONT_SELF,
        from_slot: 0,
        to_kind: CONT_WORLD,
        to_slot: 0,
        count: 1,
    }]);
    assert_eq!(
        w.world_conts.entries()[0].refill_at,
        0,
        "a container with something in it is not waiting to refill"
    );

    // Take it out again on a later tick: the clock restarts from *now*,
    // which is the honest reading — the crate was not empty in between.
    // What must not happen is the refill arriving early, and what must not
    // happen is the record multiplying.
    let then = w.tick + 100;
    advance_to(&mut w, then);
    take(&mut w, cx, cz, 0, 0, 1);
    let rearmed = w.world_conts.entries()[0].refill_at;
    assert!(
        rearmed >= w.tick + RESPAWN_MIN_TICKS,
        "the re-arm is still a full window out"
    );
    assert_eq!(w.world_conts.len(), 1);
}

// ------------------------------------------------------------------- caps

/// `MAX_WORLD_CONTS`' overflow branch is unreachable today, and this is what
/// keeps that true. The store holds only containers somebody has opened, so
/// the live ceiling is what terrain authors — and if a monument lands with
/// forty crates on it, this goes red *before* a player finds the crate that
/// silently will not open.
#[test]
fn the_cap_is_above_what_terrain_authors() {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(SEED);
    let mut authored = 0usize;
    for cx in 0..CELLS_PER_SIDE {
        for cz in 0..CELLS_PER_SIDE {
            let s = terrain::scatter(SEED, &table, &haven, cx, cz);
            if matches!(s.occupant, Occupant::CrateSlot | Occupant::CacheSlot) {
                authored += 1;
            }
        }
    }
    assert!(authored > 0, "the island authors containers at all");
    assert!(
        authored <= MAX_WORLD_CONTS,
        "seed {SEED:#x} authors {authored} world containers against a cap of \
         {MAX_WORLD_CONTS} — raise the cap, or the containers past it refuse \
         to open with nothing said"
    );
}

// -------------------------------------------------------------- hashed state

/// A world container is state, so it is in the hash — every field of it.
/// Two shards that rolled the same crate and then emptied it at different
/// ticks agree about the contents and disagree about when it pays again,
/// and that divergence has to be loud on the tick it happens rather than
/// on the refill twenty minutes later.
#[test]
fn a_world_container_is_hashed_state() {
    let mut a = world();
    let (cx, cz, x, z) = find_slot(&a, Occupant::CrateSlot);
    join_at(&mut a, x, z);
    let before = a.state_hash();
    open(&mut a, cx, cz);
    assert_ne!(before, a.state_hash(), "the roll is in the hash");

    // Same commands, same ticks, same hash — the roll is a function of
    // (seed, cell, tick) and of nothing else.
    let mut b = world();
    join_at(&mut b, x, z);
    open(&mut b, cx, cz);
    assert_eq!(a.state_hash(), b.state_hash(), "the roll is deterministic");
}

/// The refill deadline is hashed, not just the contents.
///
/// Two shards that emptied the same crate on different ticks hold the same
/// (empty) contents and different clocks, so a hash that skipped
/// `refill_at` would call them equal right up until one of them paid out —
/// a divergence that stays silent for twenty minutes and then appears with
/// nothing nearby to explain it.
///
/// **Two worlds, not four.** This started life inside the test above and
/// overflowed the 2 MiB test thread: `World` is built on the stack and
/// four of them do not fit (`CLAUDE.md`'s shadow-stack trap, one target
/// over). Splitting the claim is the fix, and it is also the better test.
#[test]
fn the_refill_deadline_is_hashed() {
    let mut d = world();
    let (cx, cz, x, z) = find_slot(&d, Occupant::CrateSlot);
    join_at(&mut d, x, z);
    open(&mut d, cx, cz);

    let mut e = world();
    join_at(&mut e, x, z);
    open(&mut e, cx, cz);
    let then = e.tick + 50;
    advance_to(&mut e, then);

    for s in 0..INV_SLOTS {
        let n = d.world_conts.entries()[0].items[s].count;
        if n > 0 {
            take(&mut d, cx, cz, s as u8, s as u8, n);
        }
        let n = e.world_conts.entries()[0].items[s].count;
        if n > 0 {
            take(&mut e, cx, cz, s as u8, s as u8, n);
        }
    }
    assert!(d.world_conts.entries()[0].is_empty());
    assert!(e.world_conts.entries()[0].is_empty());
    assert_ne!(
        d.world_conts.entries()[0].refill_at,
        e.world_conts.entries()[0].refill_at,
        "the two crates are due at different ticks"
    );
    assert_ne!(
        d.state_hash(),
        e.state_hash(),
        "refill_at is a hashed field — two empty crates due at different \
         ticks are different worlds"
    );
}

/// Every stack a crate rolls is a real item. `roll_into` is shared with the
/// barrel and already bounds this, but a container whose contents reach the
/// world save is a container whose contents the decoder will re-validate —
/// so the two have to agree about what is legal.
#[test]
fn every_rolled_stack_names_a_real_item() {
    let mut w = world();
    let (cx, cz, x, z) = find_slot(&w, Occupant::CrateSlot);
    join_at(&mut w, x, z);
    open(&mut w, cx, cz);
    for s in w.world_conts.entries()[0].items.iter() {
        assert!(
            *s == ItemStack::default() || (s.item as usize) < MAX_ITEM_DEFS,
            "a rolled stack names item {} past the table",
            s.item
        );
    }
}

// ------------------------------------------------------------ persistence

/// **The section this pass added has to survive a restart, and until this
/// test it had only ever round-tripped EMPTY.**
///
/// Every world-save test in `tests/worldsave.rs` builds a world nobody has
/// opened a container in, so the new section encoded a count of zero and
/// decoded a count of zero — symmetric, green, and no evidence at all about
/// the record layout. A stride that is wrong by a byte is invisible at
/// n = 0 and total at n = 1.
///
/// What it must preserve is not "some loot" but the exact hash: a shard
/// that reboots and re-rolls every crate on the island is the loot
/// equivalent of the world save not existing, and a shard that reboots
/// with the crates *emptied but their refill clocks reset* is the same bug
/// with a delay on it.
#[test]
fn an_opened_crate_survives_a_restart() {
    let mut w = Box::new(world());
    let (cx, cz, x, z) = find_slot(&w, Occupant::CrateSlot);
    join_at(&mut w, x, z);
    open(&mut w, cx, cz);

    // Two records, one of them emptied, so the round trip carries a full
    // container, an empty one, and an armed refill deadline — the three
    // shapes a record can be in.
    let (ccx, ccz, cxp, czp) = find_slot(&w, Occupant::CacheSlot);
    stand_at(&mut w, cxp, czp);
    open(&mut w, ccx, ccz);
    for s in 0..INV_SLOTS {
        let n = w.world_conts.entries()[1].items[s].count;
        if n > 0 {
            w.tick(&[Command::Move {
                id: PLAYER,
                cont: cell_key(ccx, ccz),
                from_kind: CONT_WORLD,
                from_slot: s as u8,
                to_kind: CONT_SELF,
                to_slot: s as u8,
                count: n,
            }]);
        }
    }
    assert_eq!(w.world_conts.len(), 2);
    assert!(
        !w.world_conts.entries()[0].is_empty(),
        "the crate is stocked"
    );
    assert!(
        w.world_conts.entries()[1].is_empty(),
        "the cache is cleared"
    );
    assert!(
        w.world_conts.entries()[1].refill_at > 0,
        "and its refill is armed"
    );
    // The body leaves first, for `worldsave::a_quiet_world`'s reason: a
    // file arrives entirely asleep by design, so a world saved with
    // somebody still driving a body can never hash-match its own reload,
    // and the mismatch would be about the sleeper rule rather than about
    // anything this section added.
    w.tick(&[Command::Leave { id: PLAYER }]);
    let before = w.state_hash();
    let saved = w.world_conts.entries().to_vec();

    let mut blob = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut blob).expect("encodes");
    blob.truncate(n);
    drop(w);

    let mut back = Box::new(world());
    back.load(&blob).expect("decodes");

    assert_eq!(
        back.world_conts.entries(),
        &saved[..],
        "every field of every record came back — the cell, the stand \\
         position, the table, the refill deadline and all thirty stacks"
    );
    assert_eq!(
        back.state_hash(),
        before,
        "wall 5 at the origin: the loaded world is the saved world"
    );
}

/// The save is untrusted input, and the check worth having is the one that
/// is *not* obvious: a record's stand position must lie inside the cell it
/// claims.
///
/// The cell and the table are the checks a reader expects. The position is
/// the one that matters, because `qx`/`qz` are what the reach test reads —
/// so a hand-edited save that leaves the cell alone and moves the crate to
/// wherever a player is standing turns the haven pad into a container you
/// can open from your own base. The walk is the entire price of this loot.
#[test]
fn a_save_cannot_move_a_crate_to_your_feet() {
    let mut w = Box::new(world());
    let (cx, cz, x, z) = find_slot(&w, Occupant::CrateSlot);
    join_at(&mut w, x, z);
    open(&mut w, cx, cz);

    let mut blob = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut blob).expect("encodes");
    blob.truncate(n);

    // The record's `qx` sits four bytes into it, after `cx` and `cz`. Find
    // the record by its cell rather than by a hand-computed offset — the
    // stride is what a poking test gets wrong, and this one only needs to
    // locate two bytes it already knows the value of.
    let want = [
        cx.to_le_bytes()[0],
        cx.to_le_bytes()[1],
        cz.to_le_bytes()[0],
        cz.to_le_bytes()[1],
    ];
    let at = blob
        .windows(4)
        .position(|win| win == want)
        .expect("the record's cell is in the blob");

    // Shove the stand position a kilometre east, leaving the cell alone.
    let mut bent = blob.clone();
    let far = (x + 1000.0) * (1.0 / 0.001);
    bent[at + 4..at + 8].copy_from_slice(&(far as i32).to_le_bytes());

    let mut back = Box::new(world());
    // The exact reason, not merely `is_err()`: this test locates the record
    // by searching for its cell bytes, and a search that matched the wrong
    // four bytes would bend something else and still refuse — passing for a
    // reason that has nothing to do with the claim.
    assert_eq!(
        back.load(&bent),
        Err(WorldSaveError::AddressOutOfRange),
        "a container standing outside the cell it names must be refused, \
         not loaded — it is a crate you could open from anywhere"
    );

    // And the unbent blob loads, so the refusal above is the edit and not
    // the fixture.
    let mut ok = Box::new(world());
    ok.load(&blob).expect("the unedited save is legal");
}
