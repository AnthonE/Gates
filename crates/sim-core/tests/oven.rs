//! The oven: a campfire that lights, burns, banks its byproduct and cooks
//! what is on it (`sim-core/oven.rs`, oven v0).
//!
//! Four things a fire can get wrong that no verb before it could, and this
//! file is mostly about those:
//!
//! 1. **It is a container with a second life.** The contents live in the
//!    box store, so every move rule already holds — but the oven's own
//!    state rides an index-aligned array beside it, and a swap-remove that
//!    moved one half and not the other would leave a box burning and a
//!    fire inert. `a_removal_moves_both_halves` is that check, and it is
//!    the same shape as the `bag_ready` invariant one store up.
//! 2. **It spends on a clock the player is not pressing.** A tick sweep
//!    that runs every oven every tick is unbounded work; one with a cursor
//!    is state to persist. Ours steps each oven once per
//!    `OVEN_PERIOD_TICKS` by index, so what the tests assert is *rates* —
//!    a fuel unit lasts its content seconds and not "some ticks".
//! 3. **It pays into itself.** An output that does not fit must not be
//!    destroyed and must not be paid twice. Both directions are asserted,
//!    because either one alone is satisfied by doing nothing.
//! 4. **It refuses.** A press on a cold empty fire is `REFUSE_D_FUEL`, and
//!    a move of anything the oven has no use for is `REFUSE_M_OVEN` — the
//!    reference's own 2025 rule, and what keeps a campfire from being a
//!    cheaper box than a box.

use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::deploy::{box_key, DeployContent, ARCH_FIRE, REFUSE_D_FUEL};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::inventory::{CONT_BOX, CONT_SELF, REFUSE_M_OVEN};
use sim_core::limits::{BOX_SLOTS, TICK_HZ};
use sim_core::oven::{CookContent, OVEN_PERIOD_TICKS};
use sim_core::world::{Command, World, EV_DEPLOY_REFUSED, EV_MOVE_REFUSED, EV_OVEN};

const SEED: u64 = 0x0FEE_0FEE;
const PLAYER: u32 = 3;
/// `DeployContent::probe_fixture` row 4 is the fire, and it costs one unit
/// of item 6 to place.
const FIRE_ROW: u16 = 4;
const FIRE_ITEM: u16 = 6;
/// `CookContent::probe_fixture`: item 0 burns for 30 ticks and leaves item
/// 5 at half a unit each; item 1 cooks into item 6 in 60 ticks on a fire.
const FUEL: u16 = 0;
const RAW: u16 = 1;
const CHAR: u16 = 5;
const COOKED: u16 = 6;

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

fn buildable_cell(seed: u64) -> (u16, u16) {
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (512 + dx).clamp(0, 1023) as u16;
                let cz = (512 + dz).clamp(0, 1023) as u16;
                let (x, z) = cell_center(cx, cz);
                if foundation_terrain_ok(seed, x, z) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell within 64 cells — the generator changed under this test");
}

/// A world with one player standing beside a placed, cold, empty fire.
fn fire_world() -> (World, u32, u16, u16) {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.cook = CookContent::probe_fixture();

    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: PLAYER }]);
    w.players[0].body = sim_core::movement::Body::at(SEED, x, z);
    w.players[0].inv[0] = ItemStack {
        item: FIRE_ITEM,
        count: 1,
        cond: 0,
    };
    w.players[0].inv[1] = ItemStack {
        item: FUEL,
        count: 40,
        cond: 0,
    };
    w.players[0].inv[2] = ItemStack {
        item: RAW,
        count: 4,
        cond: 0,
    };
    w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row: FIRE_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.deploys.len(), 1, "the fixture needs its fire placed");
    assert_eq!(
        w.deploys.boxes().len(),
        1,
        "a fire is a container: placing one stands a record up in the box store"
    );
    assert!(
        w.deploys.oven_states()[0].burns(),
        "and that record's state says oven, not box"
    );
    (w, box_key(cx, cz, 0), cx, cz)
}

/// Put `count` of `item` straight into oven slot `slot`, bypassing the
/// move verb. The verb has its own tests below; the burn tests want the
/// oven loaded without asserting the loading.
fn load(w: &mut World, key: u32, slot: usize, item: u16, count: u16) {
    let i = w.deploys.box_index(key).expect("the fire is a container");
    w.deploys.set_box_slot(
        i,
        slot,
        ItemStack {
            item,
            count,
            cond: 0,
        },
    );
}

fn slot(w: &World, key: u32, s: usize) -> ItemStack {
    let i = w.deploys.box_index(key).expect("the fire is a container");
    w.deploys.box_slot(i, s)
}

fn total(w: &World, key: u32, item: u16) -> u32 {
    let i = w.deploys.box_index(key).expect("the fire is a container");
    (0..BOX_SLOTS)
        .map(|s| w.deploys.box_slot(i, s))
        .filter(|st| st.item == item)
        .map(|st| st.count as u32)
        .sum()
}

fn lit(w: &World, key: u32) -> bool {
    let i = w.deploys.box_index(key).expect("the fire is a container");
    w.deploys.oven_states()[i].lit
}

fn press(w: &mut World, cx: u16, cz: u16) {
    w.tick(&[Command::Use {
        id: PLAYER,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
}

fn events(w: &World, code: u8) -> usize {
    w.events.entries().iter().filter(|e| e.code == code).count()
}

fn refusal(w: &World, code: u8) -> Option<u32> {
    w.events
        .entries()
        .iter()
        .find(|e| e.code == code)
        .map(|e| e.b)
}

/// Run `ticks` ticks with nothing happening.
fn idle(w: &mut World, ticks: u64) {
    for _ in 0..ticks {
        w.tick(&[]);
    }
}

/// A match lights what is laid; it does not lay it. The refusal is the
/// whole of the reference's ignite condition and it is announced, never
/// silent.
#[test]
fn a_cold_empty_fire_refuses_the_match() {
    let (mut w, key, cx, cz) = fire_world();
    press(&mut w, cx, cz);
    assert!(!lit(&w, key), "an empty fire must not light");
    assert_eq!(
        refusal(&w, EV_DEPLOY_REFUSED),
        Some(REFUSE_D_FUEL),
        "and it must say why"
    );
    assert_eq!(events(&w, EV_OVEN), 0, "a refusal is not an announcement");
}

/// Fuel in, match, fire. And the press the other way puts it out — one
/// verb, both directions, the way a door's use key works.
#[test]
fn fuel_makes_the_match_work_and_the_press_toggles() {
    let (mut w, key, cx, cz) = fire_world();
    load(&mut w, key, 0, FUEL, 4);

    press(&mut w, cx, cz);
    assert!(lit(&w, key), "a fire with wood in it lights");
    assert_eq!(events(&w, EV_OVEN), 1, "and announces it");

    press(&mut w, cx, cz);
    assert!(!lit(&w, key), "the same press puts it out");
    assert_eq!(events(&w, EV_OVEN), 1, "and announces that too");
}

/// The rate, not the tick: one fuel unit lasts the content's seconds.
/// Asserted as a *span* rather than an exact tick because the sweep steps
/// each oven once per period — which is the design, and a test that pinned
/// a tick would be pinning the sweep's phase instead of the fuel's life.
#[test]
fn a_fuel_unit_burns_for_its_content_seconds() {
    let (mut w, key, cx, cz) = fire_world();
    let cc = CookContent::probe_fixture();
    load(&mut w, key, 0, FUEL, 2);
    press(&mut w, cx, cz);

    // The first step takes a unit and starts burning it.
    idle(&mut w, OVEN_PERIOD_TICKS * 2);
    assert_eq!(
        total(&w, key, FUEL),
        1,
        "one unit is on the fire and one is waiting"
    );

    // Somewhere inside the next unit's life the second one is taken; by
    // two units' worth of ticks plus a period of slack, both are gone and
    // the fire has snuffed itself for want of a third.
    idle(&mut w, cc.fuel_ticks as u64 * 2 + OVEN_PERIOD_TICKS * 2);
    assert_eq!(total(&w, key, FUEL), 0, "both units burned");
    assert!(!lit(&w, key), "and a fire with nothing left goes out");
}

/// The self-snuff is announced with **no actor**, which is the one field
/// separating it from a hand on the switch.
///
/// Ticked one at a time and read each tick, because the event ring is the
/// tick's and not the world's: an `idle` that ran past the snuff would
/// find an empty ring and report "never announced" for an event that did.
#[test]
fn a_fire_that_runs_dry_announces_itself_with_no_hand_on_it() {
    let (mut w, key, cx, cz) = fire_world();
    let cc = CookContent::probe_fixture();
    load(&mut w, key, 0, FUEL, 1);
    press(&mut w, cx, cz);
    let lit_ev = w
        .events
        .entries()
        .iter()
        .find(|e| e.code == EV_OVEN)
        .expect("the light announces");
    assert_eq!(lit_ev.c, PLAYER, "a hand lit it");

    let mut out = None;
    for _ in 0..(cc.fuel_ticks as u64 + OVEN_PERIOD_TICKS * 2) {
        w.tick(&[]);
        if let Some(e) = w
            .events
            .entries()
            .iter()
            .find(|e| e.code == EV_OVEN && e.b & 1 == 0)
        {
            out = Some(*e);
            break;
        }
    }
    let out = out.expect("running dry announces");
    assert_eq!(out.c, 0, "nobody put it out — it ran out");
    assert!(!lit(&w, key));
}

/// The byproduct is banked and paid whole: the fixture's 50% pays one unit
/// per two burned, exactly, with nothing random in it. Two runs of the
/// same world must therefore agree — which is wall 5 in one assertion.
#[test]
fn the_byproduct_is_banked_and_paid_at_the_content_rate() {
    let burn = |units: u16| -> u32 {
        let (mut w, key, cx, cz) = fire_world();
        let cc = CookContent::probe_fixture();
        load(&mut w, key, 0, FUEL, units);
        press(&mut w, cx, cz);
        idle(
            &mut w,
            cc.fuel_ticks as u64 * units as u64 + OVEN_PERIOD_TICKS * 2,
        );
        assert_eq!(total(&w, key, FUEL), 0, "all of it burned");
        total(&w, key, CHAR)
    };
    assert_eq!(burn(2), 1, "50% of two units is one, paid whole");
    assert_eq!(burn(8), 4, "and the bank does not drift over eight");
    assert_eq!(burn(8), 4, "twice, from the same seed and the same script");
}

/// What is on the fire cooks, one unit at a time, into the oven's own
/// slots — and the input stack shrinks by exactly what the output gained.
#[test]
fn what_is_on_the_fire_cooks_into_the_fire() {
    let (mut w, key, cx, cz) = fire_world();
    let cc = CookContent::probe_fixture();
    load(&mut w, key, 0, FUEL, 20);
    load(&mut w, key, 1, RAW, 2);
    press(&mut w, cx, cz);

    let row = cc
        .row_for(ARCH_FIRE, RAW)
        .copied()
        .expect("the fixture cooks");
    idle(&mut w, row.ticks as u64 + OVEN_PERIOD_TICKS * 2);
    assert_eq!(slot(&w, key, 1).count, 1, "one unit left the input stack");
    assert_eq!(total(&w, key, COOKED), 1, "and one arrived cooked");

    idle(&mut w, row.ticks as u64 + OVEN_PERIOD_TICKS * 2);
    assert_eq!(total(&w, key, COOKED), 2, "the second follows the first");
    assert_eq!(slot(&w, key, 1), ItemStack::default(), "the input is spent");
}

/// A fire that is out does not cook. The progress is frozen rather than
/// discarded, so relighting finishes what was started — the kinder reading
/// of the two, and the one that cannot lose food to a misclick.
#[test]
fn a_cold_fire_cooks_nothing_and_forgets_nothing() {
    let (mut w, key, cx, cz) = fire_world();
    let cc = CookContent::probe_fixture();
    let row = cc
        .row_for(ARCH_FIRE, RAW)
        .copied()
        .expect("the fixture cooks");
    load(&mut w, key, 0, FUEL, 20);
    load(&mut w, key, 1, RAW, 1);

    idle(&mut w, row.ticks as u64 * 2);
    assert_eq!(total(&w, key, COOKED), 0, "an unlit fire cooks nothing");

    // Lit for most of a row, then out, then lit again: the total time on
    // the fire is what finishes it, not the time on the clock.
    press(&mut w, cx, cz);
    idle(&mut w, row.ticks as u64 - OVEN_PERIOD_TICKS * 2);
    press(&mut w, cx, cz);
    assert_eq!(total(&w, key, COOKED), 0, "not done when it went out");
    idle(&mut w, row.ticks as u64);
    assert_eq!(total(&w, key, COOKED), 0, "and cold time does not count");
    press(&mut w, cx, cz);
    idle(&mut w, OVEN_PERIOD_TICKS * 4);
    assert_eq!(total(&w, key, COOKED), 1, "relighting finishes it");
}

/// A full oven holds what it made rather than destroying it: the
/// conversion does not run, the input is untouched, and nothing is paid
/// twice when room appears.
#[test]
fn a_full_oven_loses_nothing() {
    let (mut w, key, cx, cz) = fire_world();
    let cc = CookContent::probe_fixture();
    let row = cc
        .row_for(ARCH_FIRE, RAW)
        .copied()
        .expect("the fixture cooks");
    let cap = GatherContent::probe_fixture().stack_max_of(COOKED);
    // Fuel in slot 0, the input in slot 1, and every other slot filled
    // with something the output cannot join or displace.
    load(&mut w, key, 0, FUEL, 20);
    load(&mut w, key, 1, RAW, 2);
    for s in 2..BOX_SLOTS {
        load(&mut w, key, s, CHAR, cap);
    }
    press(&mut w, cx, cz);
    idle(&mut w, row.ticks as u64 * 2);
    assert_eq!(
        slot(&w, key, 1).count,
        2,
        "with nowhere to put the output, the input stays whole"
    );
    assert_eq!(total(&w, key, COOKED), 0, "and nothing was made");

    // Make room. The very next steps pay it out — once.
    //
    // Two slots, not one, and the reason is a real ordering: the fuel half
    // of a step runs before the cook half, so the charcoal the fire banked
    // while it had nowhere to put it is paid into the first slot that
    // opens. One freed slot would be taken by that debt and the check
    // below would be measuring the byproduct's priority rather than the
    // conversion's correctness.
    load(&mut w, key, 2, CHAR, 0);
    load(&mut w, key, 3, CHAR, 0);
    idle(&mut w, OVEN_PERIOD_TICKS * 4);
    assert_eq!(total(&w, key, COOKED), 1, "exactly one unit, not two");
    assert_eq!(slot(&w, key, 1).count, 1, "and exactly one was consumed");
}

/// The move verb's oven rule: fuel and inputs go in, junk bounces, and
/// what the oven made comes back out. Announced, never silent, and never a
/// dropped session — `inventory.rs`'s whole posture, applied to a third
/// kind of destination.
#[test]
fn an_oven_takes_fuel_and_what_it_cooks_and_nothing_else() {
    let (mut w, key, cx, _cz) = fire_world();
    let _ = cx;
    // Slot 3 holds something the fire has no use for.
    w.players[0].inv[3] = ItemStack {
        item: 2,
        count: 5,
        cond: 0,
    };

    let mv = |w: &mut World, from_slot: u8, count: u16| {
        w.tick(&[Command::Move {
            id: PLAYER,
            cont: key,
            from_kind: CONT_SELF,
            from_slot,
            to_kind: CONT_BOX,
            to_slot: 0,
            count,
        }]);
    };

    mv(&mut w, 1, 10); // fuel
    assert_eq!(total(&w, key, FUEL), 10, "fuel goes in");

    mv(&mut w, 2, 4); // a cook input
    assert_eq!(total(&w, key, RAW), 4, "so does what it cooks");

    mv(&mut w, 3, 5); // junk
    assert_eq!(
        refusal(&w, EV_MOVE_REFUSED),
        Some(REFUSE_M_OVEN),
        "and a fire is not a storage box"
    );
    assert_eq!(total(&w, key, 2), 0, "nothing of it landed");

    // What the fire made comes back out through the same door it went in.
    load(&mut w, key, 5, CHAR, 3);
    w.tick(&[Command::Move {
        id: PLAYER,
        cont: key,
        from_kind: CONT_BOX,
        from_slot: 5,
        to_kind: CONT_SELF,
        to_slot: 10,
        count: 3,
    }]);
    assert_eq!(
        w.players[0].inv[10],
        ItemStack {
            item: CHAR,
            count: 3,
            cond: 0,
        },
        "the charcoal is takeable"
    );
}

/// The index-aligned halves move together. A swap-remove that moved the
/// contents and left the state would leave the surviving container burning
/// somebody else's fuel — silently, and in the state hash.
#[test]
fn a_removal_moves_both_halves() {
    let (mut w, key, cx, cz) = fire_world();
    load(&mut w, key, 0, FUEL, 4);
    press(&mut w, cx, cz);
    assert!(lit(&w, key));

    // A second container beside the first, so the removal below actually
    // swaps rather than truncating: place a fire one cell over.
    let (bx, bz) = (cx + 1, cz);
    w.players[0].inv[0] = ItemStack {
        item: FIRE_ITEM,
        count: 1,
        cond: 0,
    };
    w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row: FIRE_ROW,
        cx: bx,
        cz: bz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    let second = box_key(bx, bz, 0);
    if w.deploys.box_index(second).is_none() {
        // The neighbouring cell's terrain refused it. Nothing about the
        // alignment can be asserted without two records, and inventing a
        // second placement site would be inventing terrain — so say so
        // rather than passing on a check that did not run.
        panic!("the fixture needs a second container: cell ({bx}, {bz}) refused the placement");
    }
    assert!(!lit(&w, second), "the new fire is cold");

    // Break the first one down to nothing, through the verb a raider
    // actually uses: swings at the structure until it is gone. The
    // fixture's spear takes 34 off a 40 hp fire, so two land it.
    let i = w
        .deploys
        .entries()
        .iter()
        .position(|d| d.cx == cx && d.cz == cz && d.loc == LOC_PLANE)
        .expect("the burning fire is standing");
    let dc = w.deploy;
    let mut pieces = std::mem::take(&mut w.pieces);
    for _ in 0..8 {
        if w.deploys.box_index(key).is_none() {
            break;
        }
        sim_core::deploy::damage_deploy(&dc, &mut pieces, &mut w.deploys, i, 34, &mut w.events);
    }
    assert!(
        w.deploys.box_index(key).is_none(),
        "the burning fire came apart"
    );
    let j = w
        .deploys
        .box_index(second)
        .expect("the survivor is still a container");
    assert!(
        !w.deploys.oven_states()[j].lit,
        "and it did not inherit the dead fire's flame"
    );
}

/// A fire survives a restart, burning. The world save is the only
/// non-command path into `World`, so the state that makes a fire a fire
/// has to be in it — otherwise every shard restart is a silent snuff.
///
/// Asserted field by field rather than by comparing `state_hash`, and the
/// reason is a rule this test would otherwise trip over: **every body in a
/// loaded world is asleep** (`worldsave.rs`), so a world with a player
/// standing in it never hashes equal to its own reload. `tests/worldsave.rs`
/// owns that whole-world equality; what is owed here is the oven's half of
/// it, which is the array the file learned to carry in this slice.
#[test]
fn a_burning_fire_survives_a_restart() {
    let (mut w, key, cx, cz) = fire_world();
    load(&mut w, key, 0, FUEL, 6);
    load(&mut w, key, 1, RAW, 2);
    press(&mut w, cx, cz);
    idle(&mut w, OVEN_PERIOD_TICKS * 3);
    let state = w.deploys.oven_states()[0];
    assert!(state.lit && state.burn > 0, "the fixture needs a live fire");

    let mut blob = vec![0u8; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut blob).expect("the world encodes");

    let mut back = World::new(SEED);
    back.gather = GatherContent::probe_fixture();
    back.build = BuildContent::probe_fixture();
    back.deploy = DeployContent::probe_fixture();
    back.cook = CookContent::probe_fixture();
    back.load(&blob[..n]).expect("and decodes");

    assert_eq!(
        back.deploys.oven_states(),
        w.deploys.oven_states(),
        "the fire came back a different fire"
    );
    assert_eq!(
        back.deploys.boxes()[0].items,
        w.deploys.boxes()[0].items,
        "and what was on it came back with it"
    );
    assert!(
        lit(&back, key),
        "the fire is still burning after the restart"
    );

    // And it goes on burning, on the same clock: the restored `burn` is
    // ticks remaining and not a reset, so the reload does not hand the
    // owner a free fuel unit.
    let before = back.deploys.oven_states()[0].burn;
    idle(&mut back, OVEN_PERIOD_TICKS * 2);
    let after = back.deploys.oven_states()[0];
    assert!(
        after.lit && (after.burn < before || total(&back, key, FUEL) < 6),
        "a reloaded fire must keep spending, not idle"
    );
}

/// The rate the shipped content states, in the units a player feels.
/// Fixture-free: this is arithmetic over `oven.rs`'s own constants, and it
/// is here so that a period changed for the sweep's sake cannot quietly
/// change how long a fire lasts.
#[test]
fn the_period_divides_a_second_without_remainder() {
    assert_eq!(
        TICK_HZ as u64 % OVEN_PERIOD_TICKS,
        0,
        "an oven step that does not divide the second makes every content \
         `seconds` value approximate"
    );
}

// ---------------------------------------------------------------------------
// The recycler (recycler v0) — the third archetype on this code path, and
// the two things it does differently.
//
// It is an oven that does not burn, so the gates it owes are the two
// deltas and nothing else: the switch must not ask for fuel, and a
// conversion must pay EVERY row over its input or none of them. The rest
// of this file already covers what the two share, because they are one
// sweep.

/// `DeployContent::probe_fixture` row 6 is the recycler; it costs one unit
/// of item 8 to place. `CookContent::probe_fixture` rows 1 and 2 are its
/// table: item 2 pays 2 × item 6 AND 1 × item 7, both on one 30-tick clock.
const RECYCLER_ROW: u16 = 6;
const RECYCLER_ITEM: u16 = 8;
const SALVAGE: u16 = 2;
const YIELD_A: u16 = 6;
const YIELD_B: u16 = 7;
const RECYCLE_TICKS: u64 = 30;

/// `fire_world`'s shape with the other archetype: one player beside a
/// placed, switched-off, empty recycler.
fn recycler_world() -> (World, u32, u16, u16) {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.cook = CookContent::probe_fixture();

    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: PLAYER }]);
    w.players[0].body = sim_core::movement::Body::at(SEED, x, z);
    w.players[0].inv[0] = ItemStack {
        item: RECYCLER_ITEM,
        count: 1,
        cond: 0,
    };
    w.players[0].inv[1] = ItemStack {
        item: SALVAGE,
        count: 8,
        cond: 0,
    };
    w.players[0].inv[2] = ItemStack {
        item: FUEL,
        count: 40,
        cond: 0,
    };
    w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row: RECYCLER_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.deploys.len(), 1, "the fixture needs its recycler placed");
    assert_eq!(
        w.deploys.boxes().len(),
        1,
        "a recycler is a container: placing one stands a record up in the box store"
    );
    let st = w.deploys.oven_states()[0];
    assert!(
        st.is_converter() && !st.burns(),
        "and that record's state says converter-that-does-not-burn"
    );
    (w, box_key(cx, cz, 0), cx, cz)
}

/// The first delta. A fire with nothing in it refuses the match
/// (`a_cold_empty_fire_refuses_the_match`, above); a recycler has no fuel
/// to lay, so the same press on the same empty container must WORK — a
/// refusal for want of something it never consumes is a rule with nothing
/// behind it.
#[test]
fn an_empty_recycler_switches_on_where_an_empty_fire_would_not() {
    let (mut w, key, cx, cz) = recycler_world();
    assert!(!lit(&w, key), "it starts off");
    press(&mut w, cx, cz);
    assert!(lit(&w, key), "the switch is just a switch");
    assert_eq!(
        refusal(&w, EV_DEPLOY_REFUSED),
        None,
        "and nothing refused: REFUSE_D_FUEL is the burner's rule"
    );
    assert_eq!(
        events(&w, EV_OVEN),
        1,
        "the state is announced like a fire's"
    );
    press(&mut w, cx, cz);
    assert!(!lit(&w, key), "and it toggles back off");
}

/// It also never snuffs itself. A fire that runs dry goes out with no hand
/// on it; a recycler spends nothing, so the only thing that stops one is
/// the switch — even after long enough that a fire would have burned 40
/// units of fuel.
#[test]
fn a_running_recycler_spends_nothing_and_stays_on() {
    let (mut w, key, cx, cz) = recycler_world();
    press(&mut w, cx, cz);
    let before = w.deploys.oven_states()[w.deploys.box_index(key).unwrap()];
    idle(&mut w, 40 * RECYCLE_TICKS);
    assert!(lit(&w, key), "nothing ran out, because nothing was spent");
    let after = w.deploys.oven_states()[w.deploys.box_index(key).unwrap()];
    assert_eq!(after.burn, before.burn, "and the burn timer never moved");
    assert_eq!(after.bank, 0, "nor did the byproduct bank");
}

/// The second delta, and the reason `CookRow` grew a `count`: one input
/// pays EVERY row that names it, together, off one clock.
#[test]
fn a_conversion_pays_every_row_over_its_input() {
    let (mut w, key, cx, cz) = recycler_world();
    load(&mut w, key, 0, SALVAGE, 3);
    press(&mut w, cx, cz);

    idle(&mut w, RECYCLE_TICKS);
    assert_eq!(slot(&w, key, 0).count, 2, "one unit went in");
    assert_eq!(total(&w, key, YIELD_A), 2, "and paid the first row's 2");
    assert_eq!(total(&w, key, YIELD_B), 1, "and the second row's 1");

    idle(&mut w, RECYCLE_TICKS * 2);
    assert_eq!(total(&w, key, SALVAGE), 0, "the hopper empties");
    assert_eq!(total(&w, key, YIELD_A), 6, "3 units × 2");
    assert_eq!(total(&w, key, YIELD_B), 3, "3 units × 1");
}

/// And it is all-or-nothing. With room for the first row's pay and not the
/// second's, NOTHING may happen: the input is not consumed, neither output
/// lands, and the progress holds at the line. A per-row check would pass
/// the first payment and strand the conversion half-applied, which is a
/// dupe or a loss depending on which half you count.
#[test]
fn a_conversion_that_cannot_pay_it_all_pays_none_of_it() {
    let (mut w, key, cx, cz) = recycler_world();
    let cap = GatherContent::probe_fixture().stack_max_of(YIELD_A);
    // One slot of salvage, and every other slot filled with something the
    // recycler's outputs cannot top up — so `YIELD_A` has exactly one
    // slot's worth of room and `YIELD_B` has none at all.
    load(&mut w, key, 0, SALVAGE, 4);
    load(&mut w, key, 1, YIELD_A, cap - 2);
    for s in 2..BOX_SLOTS {
        load(&mut w, key, s, FUEL, cap);
    }
    press(&mut w, cx, cz);
    idle(&mut w, RECYCLE_TICKS * 3);

    assert_eq!(
        total(&w, key, SALVAGE),
        4,
        "the input is untouched: a conversion that cannot complete does not start"
    );
    assert_eq!(
        total(&w, key, YIELD_A),
        cap as u32 - 2,
        "and the row that WOULD have fit paid nothing — that is the whole point"
    );
    assert_eq!(
        total(&w, key, YIELD_B),
        0,
        "the row that could not fit, likewise"
    );

    // Free the room and the same held conversion pays out whole.
    load(&mut w, key, 2, 0, 0);
    idle(&mut w, RECYCLE_TICKS);
    assert_eq!(total(&w, key, SALVAGE), 3, "now it lands");
    assert_eq!(total(&w, key, YIELD_A), cap as u32, "both rows paid,");
    assert_eq!(total(&w, key, YIELD_B), 1, "in the same step");
}

/// The move verb's half of the archetype split. A fire admits fuel because
/// it burns it; a recycler must not, or the rule that keeps a campfire
/// from being a cheap wood chest hands the job to the recycler instead.
#[test]
fn a_recycler_refuses_the_fuel_a_fire_admits() {
    let cc = CookContent::probe_fixture();
    assert!(cc.accepts(ARCH_FIRE, FUEL), "a fire burns it");
    assert!(
        !cc.accepts(sim_core::deploy::ARCH_RECYCLER, FUEL),
        "a recycler has no use for it and must not hold it"
    );
    assert!(
        !cc.accepts(sim_core::deploy::ARCH_RECYCLER, CHAR),
        "nor the byproduct of a burn it never performs"
    );
    assert!(
        cc.accepts(sim_core::deploy::ARCH_RECYCLER, SALVAGE),
        "its own input, though,"
    );
    for out in [YIELD_A, YIELD_B] {
        assert!(
            cc.accepts(sim_core::deploy::ARCH_RECYCLER, out),
            "and what it made comes back out through its own door"
        );
    }

    // The refusal is the live one, through the verb.
    let (mut w, key, _, _) = recycler_world();
    w.tick(&[Command::Move {
        id: PLAYER,
        cont: key,
        from_kind: CONT_SELF,
        from_slot: 2,
        to_kind: CONT_BOX,
        to_slot: 0,
        count: 1,
    }]);
    assert_eq!(
        refusal(&w, EV_MOVE_REFUSED),
        Some(REFUSE_M_OVEN),
        "the move verb says why"
    );
    assert_eq!(total(&w, key, FUEL), 0, "and nothing went in");
}

/// The **last** unit in a full oven, which used to be a deadlock and is
/// fixed as a side effect of the scratch commit — gated because it is a
/// behaviour change on a shipped path, not because it was asked for.
///
/// The old room check ran before the input was consumed, so it could not
/// see the slot the conversion itself was about to free. An oven with one
/// unit left in its last non-full slot therefore asked for room that only
/// existed after the step it was blocking: it held at the line forever,
/// with a player watching a fire that would never finish. Consuming into a
/// scratch copy and asking whether the result fits asks the honest
/// question instead.
#[test]
fn the_last_unit_cooks_into_the_slot_it_frees() {
    let (mut w, key, cx, cz) = fire_world();
    let cc = CookContent::probe_fixture();
    let row = cc
        .row_for(ARCH_FIRE, RAW)
        .copied()
        .expect("the fixture cooks");
    let cap = GatherContent::probe_fixture().stack_max_of(COOKED);
    // Fuel, then ONE raw unit, then every remaining slot full of something
    // the output can neither join nor displace. The only room in the whole
    // container is the raw unit's own slot.
    load(&mut w, key, 0, FUEL, 20);
    load(&mut w, key, 1, RAW, 1);
    for s in 2..BOX_SLOTS {
        load(&mut w, key, s, CHAR, cap);
    }
    press(&mut w, cx, cz);
    idle(&mut w, row.ticks as u64 * 2);

    assert_eq!(total(&w, key, RAW), 0, "the last unit went in");
    assert_eq!(
        total(&w, key, COOKED),
        1,
        "and came out into the slot it vacated — the old check could not \
         see that room and held here forever"
    );
}
