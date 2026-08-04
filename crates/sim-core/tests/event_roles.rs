//! `test_event_roles` — the gate for what an event's fields *mean*.
//!
//! Wall 6 says the wire never drifts by accident, and
//! `test_protocol_golden` enforces it — for the encoder. It cannot see
//! this: every event is `push(code, a, b, c)` over three untyped `u32`s,
//! and the doc comments in `world.rs` ("`EV_DEATH`: a = the player who
//! died, b = the player who killed them") are the only statement of which
//! is which. Swap two at an emit site and every gate stays green. The
//! golden pins the *encoder's* bytes and an emit site is not the encoder;
//! `state_hash` deliberately excludes the event ring (`world.rs` — derived
//! output, not sim state, and rightly so); and every field is a `u32`, so
//! the swap type-checks. The result is silent and permanent: a kill feed
//! that names the wrong killer, forever.
//!
//! That is not hypothetical. It is the single largest identifiable bug
//! class in the reference ecosystem's own history — 49 commits in
//! `OxideMod/Oxide.Rust` touch a hook's arguments and about 27 correct a
//! payload that had already shipped wrong, four of them more than once.
//! Their patcher pinned an MSIL hash per patched method, which is the
//! exact analogue of our byte-golden, and it caught none of them, because
//! a hash over the *shape* of a payload is blind to the meaning of the
//! fields inside it. `reference/FINDINGS.md` §1 has the receipts.
//!
//! So this file asserts roles, not bytes: drive one known cause through
//! the real `World`, then check each field against the sentence in
//! `world.rs`. Four disciplines make it able to fail —
//!
//! 1. **Every field in a checked event must be mutually distinguishable.**
//!    A check where the attacker id, the victim id and the damage all
//!    happened to be 1 would pass under any permutation. `distinct3`
//!    asserts the fixture keeps them apart, so a later fixture edit that
//!    blinds a check fails loudly instead of quietly passing.
//! 2. **And distinguishable *inside* a packed field.** Half this lane's
//!    payloads are `hi << 16 | lo`, or `level << 16 | loc << 8 | row`, and
//!    a check against one is blind to the pack being reversed whenever two
//!    parts carry the same number. `distinct_halves` and `distinct_triple`
//!    refuse that outright. This is not hypothetical either:
//!    `SurvivalContent::probe_fixture` sets `max_food` and `max_water` both
//!    to 100, so `EV_VITALS.c` reads identically reversed under the stock
//!    fixture — the arrangement here moves them apart, and the assertion
//!    fails loudly if a later edit moves them back.
//! 3. **Exactly one event per code on the tick it lands.** `only` refuses
//!    zero and refuses two, which makes this a double-emit gate as
//!    well — `Removed duplicate OnBonusItemDrop hook` and two rounds of
//!    `Fixed double deprecated hook call with OnActiveItemChange/d` are
//!    the same family of defect over there.
//! 4. **Find the tick, never assume it.** The first cut of this file
//!    asserted on the tick it sent the swing and read an empty ring twice.
//!    The sim auto-repeats a held button, so every swing after the first
//!    resolves *inside* the cooldown, on a tick the test never sent an
//!    input for. `until` steps until the code appears rather than
//!    predicting when. The bound is in sim ticks, which is deterministic
//!    state and not a clock — `CLAUDE.md`'s wall-clock rule is untouched.
//!
//! Coverage is stated, never implied: `coverage_is_stated_not_implied`
//! pins how many of the 25 codes are checked by role, so the gate can
//! never read as "the event lane is covered" while covering thirteen, and
//! a new `EV_*` cannot land without someone classifying it.
//!
//! There are three arrangements. `duel_world` is `combat.rs`'s duel —
//! `dev_spawn` pins both players to the ring's own spawn for id 1, which
//! the spawn selector guarantees is clear of scatter for 4 m, so a swing
//! lands on a person and never on a tree; id 1 is `players[0]` and
//! attacks, id 2 is `players[1]` and dies. `lone_world` is one body under
//! the survival clock. `builder_world` is one body on a cell the sim's own
//! `foundation_terrain_ok` accepts, paid up for the whole structure.
//!
//! Every *content* number here comes from a fixture — damages, spans, the
//! drink, the consumable row, the piece and deploy tables. What this file
//! chooses for itself is only ever **which seat a value sits in**, and
//! always for discipline 2: two maxima that differ, an item index and a
//! slot that are not both zero, two doorways on different edges so
//! `level`/`loc`/`row` are three different numbers. Those are stated at
//! each constant with the blindness they exist to remove. None of them is
//! a knob — no shipping code reads one.

use sim_core::backpack::BackpackContent;
use sim_core::build::{
    foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_EDGE_N, LOC_EDGE_W, LOC_PLANE,
};
use sim_core::combat::CombatContent;
use sim_core::deploy::DeployContent;
use sim_core::gather::{cell_key, GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::survival::{SurvivalContent, DRINK_REACH_M, REFUSE_C_NOT_FOOD, REFUSE_C_NO_WATER};
use sim_core::terrain;
use sim_core::world::{
    Command, SimEvent, World, EV_BAG_DROPPED, EV_CONSUMED, EV_CONSUME_REFUSED, EV_DEATH,
    EV_DEPLOY_PLACED, EV_DOOR, EV_DRANK, EV_GATHER, EV_HEALTH, EV_HIT, EV_PIECE_PLACED, EV_STOCK,
    EV_VITALS,
};
use sim_core::yaw_dir;

const SEED: u64 = 20260802;
/// The fixture's item 0: 34 damage, 2 m reach — three swings to kill.
const SPEAR: u16 = 0;
const DAMAGE: u32 = 34;
const FIXTURE_HP: u32 = 100;
/// A fixture item with no weapon row, used where a stack must be
/// distinguishable from a player id and from its own count.
const JUNK: u16 = 7;
const JUNK_COUNT: u16 = 3;

/// The attacker and the victim, by id. Kept apart on purpose — an event
/// carrying two player ids cannot be checked with one player.
const ATTACKER: u32 = 1;
const VICTIM: u32 = 2;

/// Which way the attacker faces, and how far in front the victim stands
/// (inside the fixture spear's 2 m reach).
const YAW: u16 = 0;
const REACH_M: f32 = 1.0;

/// Deterministic step bound: three swings at the fixture's swing interval
/// is well under this, and a cause that has not fired by here is broken
/// rather than slow. Sim ticks, not milliseconds.
const MAX_STEPS: u32 = 600;

/// The survival arrangement's lone body — the same id `duel_world` gives
/// the attacker, because `spawn_pos(1)` is the ring position both pin to.
/// One body on purpose: the join door announces vitals per player, so a
/// second one would put two `EV_VITALS` on the same tick and `only` would
/// (correctly) refuse them both.
const BODY: u32 = ATTACKER;

/// The two maxima the survival fixtures run under.
///
/// `SurvivalContent::probe_fixture` sets `max_food` and `max_water` both to
/// 100, which packs the *same number into both halves* of `EV_VITALS.c` —
/// a check against it would read identically if the two were reversed at
/// the emit site. These keep them apart. This is `distinct3`'s discipline
/// carried inside a packed field, and nothing else about the fixture moves:
/// the spans, the drink and the consumable row are all still its own.
const MAX_FOOD: u16 = 90;
const MAX_WATER: u16 = 70;

/// Meter readings put on the body before a vitals check, chosen distinct
/// from each other, from both maxima and from the player id, so all four
/// halves of the two packed fields are individually identifiable.
const FOOD_NOW: u16 = 45;
const WATER_NOW: u16 = 23;

/// The fixture's consumable row, re-indexed off zero and placed off slot
/// zero. `EV_CONSUMED.b` is `item << 16 | slot`, and an item 0 in slot 0
/// packs to 0 — the one value that survives every possible mis-pack. The
/// *row itself* is still the fixture's; only its index moves.
const FOOD_ITEM: u16 = 5;
const FOOD_SLOT: u8 = 3;

/// The build/deploy fixture's rows, by name rather than by digit.
/// `BuildContent::probe_fixture`: row 0 is the foundation, row 3 the
/// doorway. `DeployContent::probe_fixture`: row 0 is the hearth, row 2 the
/// door.
const PIECE_FOUNDATION: u16 = 0;
const PIECE_DOORWAY: u16 = 3;
const DEPLOY_HEARTH: u16 = 0;
const DEPLOY_DOOR: u16 = 2;

/// The single build level everything here sits on. A hearth is
/// `PLACE_FOUNDATION` and a foundation is level 0, so the whole
/// arrangement is pinned there and `EV_STOCK.c` can only ever read 0 —
/// stated at that check rather than papered over.
const LEVEL: u8 = 0;

/// Which edges carry which check, and why they differ.
///
/// `EV_PIECE_PLACED` and `EV_DEPLOY_PLACED` both pack `level << 16 | loc
/// << 8 | row`, and a check is blind to any pair of those three being
/// swapped when two of them hold the same number. The doorway *piece* is
/// row 3, so it goes on the west edge (`LOC_EDGE_W` = 2) to read 0/2/3;
/// the door *deployable* is row 2, so it goes on the north edge
/// (`LOC_EDGE_N` = 3) to read 0/3/2. Same discipline as `distinct_halves`,
/// one field wider — and it is why there are two doorways here rather
/// than one.
const PIECE_EDGE: u8 = LOC_EDGE_W;
const DOOR_EDGE: u8 = LOC_EDGE_N;

/// How many events of `code` are in the tick just run.
fn count(w: &World, code: u8) -> u32 {
    let mut n = 0;
    for e in w.events.entries() {
        if e.code == code {
            n += 1;
        }
    }
    n
}

/// Exactly one event of `code` in the tick just run. Zero means the cause
/// never fired and every role check below it would be vacuous; two means
/// something emits it twice, which is its own defect class.
///
/// Reads the live ring rather than a snapshot, deliberately: `until` stops
/// *on* the tick its code landed and ticks no further, so two calls after
/// one `until` are two reads of the same tick — and nothing here copies a
/// `World`-sized value into the frame.
fn only(w: &World, code: u8) -> SimEvent {
    let n = count(w, code);
    assert_eq!(
        n, 1,
        "expected exactly one event code {code} on this tick, saw {n} \
         (zero = the cause never fired, so the role checks are vacuous; \
         two = something emits it twice)"
    );
    for e in w.events.entries() {
        if e.code == code {
            return *e;
        }
    }
    unreachable!()
}

/// Two players on one point, the attacker armed with the fixture spear.
///
/// One `World` per test and never a second one in the frame: it is a large
/// fixed-capacity value and an unoptimized build puts a construction
/// temporary beside every live one, so a wrapper struct holding it
/// overflows a test thread's stack (`combat.rs` says the same, and this
/// file learned it the hard way).
fn duel_world() -> World {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    w.dev_spawn = Some(w.spawn_pos(ATTACKER));
    w.tick(&[Command::Join { id: ATTACKER }, Command::Join { id: VICTIM }]);
    w.players[0].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
    };
    let (fx, fz) = yaw_dir(YAW);
    let a = w.players[0].body;
    let (ax, az) = (a.qx as f32 * POS_XZ_Q, a.qz as f32 * POS_XZ_Q);
    w.players[1].body = Body::at(SEED, ax + fx * REACH_M, az + fz * REACH_M);
    w
}

/// Give the victim exactly one distinguishable stack, so the bag it leaves
/// is worth dropping and the loot announces one thing.
fn arm_victim_with_junk(w: &mut World) {
    for s in w.players[1].inv.iter_mut() {
        *s = ItemStack::default();
    }
    w.players[1].inv[0] = ItemStack {
        item: JUNK,
        count: JUNK_COUNT,
    };
}

/// One tick with the swing held — which is what a player holding the
/// button actually sends, and what makes the sim repeat the swing.
fn step(w: &mut World, seq: &mut u16) {
    w.tick(&[Command::Input {
        id: ATTACKER,
        frame: InputFrame {
            seq: *seq,
            buttons: BTN_PRIMARY,
            yaw: YAW,
            pitch: 128,
            move_x: 0,
            move_z: 0,
            sel: 0,
        },
    }]);
    *seq = seq.wrapping_add(1);
}

/// Step until `code` lands, leaving the world standing on that tick.
fn until(w: &mut World, code: u8) {
    let mut seq = 0u16;
    for _ in 0..MAX_STEPS {
        step(w, &mut seq);
        if count(w, code) > 0 {
            return;
        }
    }
    panic!("event code {code} never landed in {MAX_STEPS} sim ticks");
}

/// `distinct3`, carried inside a packed field.
///
/// Half of this lane's payloads are `hi << 16 | lo`, and a check against
/// one is blind to the pack being reversed whenever the two halves happen
/// to carry the same number — `EV_VITALS.c` under the stock fixture is
/// exactly that, `max_food` and `max_water` both 100. Assert the halves
/// differ and the reversal becomes visible; leave it out and the
/// assertion below it is decoration.
fn distinct_halves(packed: u32, what: &str) {
    assert!(
        packed >> 16 != packed & 0xffff,
        "{what} packs {} into both halves, so this check cannot see the \
         pack reversed. Move the fixture, not the assertion.",
        packed >> 16
    );
}

/// `distinct_halves`, one field wider: the addressed events pack
/// `level << 16 | loc << 8 | row` into `b`, and a check against that is
/// blind to any two of the three being swapped whenever they carry the
/// same number. The fixture must keep all three apart.
fn distinct_triple(level: u32, loc: u32, row: u32, what: &str) {
    assert!(
        level != loc && loc != row && level != row,
        "{what} packs level {level}, loc {loc}, row {row} — two are equal, \
         so this check cannot see them swapped. Move the fixture, not the \
         assertion."
    );
}

/// The fixture must keep a checked event's three fields apart, or a
/// permutation of them would satisfy every assertion. This is the check
/// that keeps the checks honest.
fn distinct3(e: SimEvent, what: &str) {
    assert!(
        e.a != e.b && e.b != e.c && e.a != e.c,
        "{what} carries {}, {}, {} — two fields are equal, so this check \
         cannot see a swap. Move the fixture, not this assertion.",
        e.a,
        e.b,
        e.c
    );
}

/// `EV_HIT: a = attacker player id, b = victim player id, c = damage`.
///
/// The sharpest case in the lane: `a` and `b` are the same kind of thing,
/// so nothing but the values distinguishes them.
#[test]
fn hit_names_the_attacker_then_the_victim_then_the_damage() {
    let mut w = duel_world();
    until(&mut w, EV_HIT);
    let hit = only(&w, EV_HIT);
    distinct3(hit, "EV_HIT");
    assert_eq!(hit.a, ATTACKER, "EV_HIT.a is the ATTACKER, not the victim");
    assert_eq!(hit.b, VICTIM, "EV_HIT.b is the VICTIM, not the attacker");
    assert_eq!(hit.c, DAMAGE, "EV_HIT.c is the damage dealt");
}

/// `EV_HEALTH: a = player id, b = hp after the change, c = max hp`.
///
/// `b` and `c` are both hp readings, and a swap would draw a full bar on a
/// wounded body — so the check must run on a tick where they differ.
#[test]
fn health_names_the_player_then_hp_then_max() {
    let mut w = duel_world();
    until(&mut w, EV_HEALTH);
    let health = only(&w, EV_HEALTH);
    distinct3(health, "EV_HEALTH");
    assert_eq!(health.a, VICTIM, "EV_HEALTH.a is whose body this is");
    assert_eq!(
        health.b,
        FIXTURE_HP - DAMAGE,
        "EV_HEALTH.b is hp AFTER the change, not the max"
    );
    assert_eq!(health.c, FIXTURE_HP, "EV_HEALTH.c is max hp, not current");
}

/// `EV_HIT` and `EV_HEALTH` ride the same strike, and the pair is where a
/// cross-event swap would hide: both carry a player id in `a`, and they
/// are deliberately *different* players — attacker for the hitmarker,
/// victim for the bar.
#[test]
fn the_hit_and_the_health_name_opposite_players() {
    let mut w = duel_world();
    until(&mut w, EV_HIT);
    let hit = only(&w, EV_HIT);
    let health = only(&w, EV_HEALTH);
    assert_eq!(hit.a, ATTACKER);
    assert_eq!(health.a, VICTIM);
    assert_ne!(
        hit.a, health.a,
        "EV_HIT is the attacker's fact and EV_HEALTH is the victim's — if \
         they ever name the same player, one of them is emitting the wrong id"
    );
}

/// `EV_DEATH: a = the player who died, b = the player who killed them`.
///
/// Both fields are player ids. Swap them and every kill feed credits the
/// corpse.
#[test]
fn death_names_the_dead_then_the_killer() {
    let mut w = duel_world();
    until(&mut w, EV_DEATH);
    let death = only(&w, EV_DEATH);
    assert_ne!(
        death.a, death.b,
        "EV_DEATH carries the same id twice, so this check cannot see a swap"
    );
    assert_eq!(death.a, VICTIM, "EV_DEATH.a is the player who DIED");
    assert_eq!(death.b, ATTACKER, "EV_DEATH.b is the player who KILLED");
    assert_eq!(w.players[1].deaths, 1, "and the body actually died");
}

/// `EV_BAG_DROPPED: a = backpack id, b = the player whose body it came off`.
///
/// Two small integers from different spaces — bag ids and player ids both
/// start at 1, which is exactly the shape that hides a swap.
#[test]
fn bag_dropped_names_the_bag_then_the_body() {
    let mut w = duel_world();
    arm_victim_with_junk(&mut w);
    until(&mut w, EV_BAG_DROPPED);
    let bag = only(&w, EV_BAG_DROPPED);
    assert_ne!(
        bag.a, bag.b,
        "EV_BAG_DROPPED carries the same value twice, so this check cannot \
         see a swap"
    );
    assert_eq!(
        bag.b, VICTIM,
        "EV_BAG_DROPPED.b is the body the bag came off, not the bag id"
    );
    assert_eq!(
        bag.a,
        w.backpacks.next_id() - 1,
        "EV_BAG_DROPPED.a is the backpack id, not the body"
    );
}

/// `EV_GATHER: a = player id, b = item index << 16 | units actually added`.
///
/// The packed field is the risk rather than a swap of two roles: high half
/// is the item, low half is the count, and nothing but this says which way
/// round.
#[test]
fn gather_names_the_player_then_item_over_count() {
    let mut w = duel_world();
    arm_victim_with_junk(&mut w);
    until(&mut w, EV_BAG_DROPPED);
    // The attacker is standing on the bag it just made.
    w.tick(&[Command::Loot { id: ATTACKER }]);

    let got = only(&w, EV_GATHER);
    assert_eq!(got.a, ATTACKER, "EV_GATHER.a is who gained the items");
    assert_eq!(
        got.b >> 16,
        JUNK as u32,
        "EV_GATHER.b's HIGH half is the item index"
    );
    assert_eq!(
        got.b & 0xffff,
        JUNK_COUNT as u32,
        "EV_GATHER.b's LOW half is the count actually added"
    );
    assert_ne!(
        got.b >> 16,
        got.b & 0xffff,
        "the fixture packs the same value into both halves, so this check \
         cannot see a swapped pack"
    );
}

// ---------------------------------------------------------------------
// The survival lane: EV_VITALS, EV_CONSUMED, EV_DRANK, EV_CONSUME_REFUSED.
//
// `NOW.md` orders the remaining codes by *swap silence* rather than by
// code order, and this is the head of that order. `EV_DRANK` carries two
// small integers that are both plausible in either position (25 units of
// water restored, 20 hp paid for it); `EV_VITALS` carries two packed
// fields of the same shape, meter over ceiling, where a reversal draws a
// full bar on a starving body.
// ---------------------------------------------------------------------

/// One body, under a fixture whose two maxima are distinguishable.
///
/// The maxima are set *before* the join tick because `survival::grant`
/// reads them there — a body is granted its meters at the ceiling, so a
/// fixture edited afterwards would leave the player above its own max.
fn lone_world() -> World {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    // Without the combat fixture `hp_max` is zero, which makes the hp half
    // of every announcement meaningless — `survival.rs`'s own `lone_world`
    // installs it for the same reason.
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.survival.max_food = MAX_FOOD;
    w.survival.max_water = MAX_WATER;
    // The fixture's own consumable row, at an index and a slot that are
    // not zero. See `FOOD_ITEM`.
    w.survival.consumable[FOOD_ITEM as usize] = w.survival.consumable[0];
    w.dev_spawn = Some(w.spawn_pos(BODY));
    w.tick(&[Command::Join { id: BODY }]);
    w
}

/// Step with no command at all until `code` lands.
///
/// The clock is the cause here, not a held button, and a quiet tick is
/// what keeps `only` meaningful: `survival::step` announces vitals when a
/// meter moved, and a swing on the same tick would add a second event
/// from a second cause. Bounded in sim ticks, like `until`.
fn until_quiet(w: &mut World, code: u8) {
    for _ in 0..MAX_STEPS {
        w.tick(&[]);
        if count(w, code) > 0 {
            return;
        }
    }
    panic!("event code {code} never landed in {MAX_STEPS} quiet sim ticks");
}

/// Land the body somewhere it can drink: above the waterline with sea
/// inside `DRINK_REACH_M`. The same scan `survival.rs`'s own tests use —
/// a *shore*, deliberately, because standing on the sea floor is not the
/// case the verb is for.
fn stand_at_the_shore(w: &mut World) {
    let r = DRINK_REACH_M;
    let mut x = 0.0f32;
    while x < terrain::ISLAND_SIZE {
        let mut z = 0.0f32;
        while z < terrain::ISLAND_SIZE {
            let h = terrain::height(SEED, x, z);
            if (terrain::SEA_LEVEL..terrain::BEACH_MAX_H).contains(&h)
                && (terrain::height(SEED, x + r, z) < terrain::SEA_LEVEL
                    || terrain::height(SEED, x - r, z) < terrain::SEA_LEVEL
                    || terrain::height(SEED, x, z + r) < terrain::SEA_LEVEL
                    || terrain::height(SEED, x, z - r) < terrain::SEA_LEVEL)
            {
                w.players[0].body = Body::at(SEED, x, z);
                return;
            }
            z += 4.0;
        }
        x += 4.0;
    }
    panic!("this island has no coast — the generator changed under this test");
}

/// Land the body where no water is in reach in any direction: the same
/// scan, the other verdict, so the dry refusal is driven against real
/// ground rather than against a disarmed table.
fn stand_inland(w: &mut World) {
    let r = DRINK_REACH_M;
    let mut x = 0.0f32;
    while x < terrain::ISLAND_SIZE {
        let mut z = 0.0f32;
        while z < terrain::ISLAND_SIZE {
            let dry = [(0.0, 0.0), (r, 0.0), (-r, 0.0), (0.0, r), (0.0, -r)]
                .into_iter()
                .all(|(dx, dz)| terrain::height(SEED, x + dx, z + dz) >= terrain::BEACH_MAX_H);
            if dry {
                w.players[0].body = Body::at(SEED, x, z);
                return;
            }
            z += 4.0;
        }
        x += 4.0;
    }
    panic!("this island is all coast — the generator changed under this test");
}

/// `EV_VITALS: a = player id, b = food << 16 | water, c = max food << 16 |
/// max water`.
///
/// Two packed fields of identical shape, and the reversal that matters is
/// not `b` against `c` but the halves *inside* each: swap them and a
/// starving body draws a full bar, or a full one draws empty. The meters
/// are read back off the world on the tick the event landed rather than
/// pinned to a constant — the clock is still draining, so the only honest
/// statement is that the event agrees with the sim it came from.
#[test]
fn vitals_names_the_player_then_the_meters_then_their_maxima() {
    let mut w = lone_world();
    // The join grants both meters at their ceiling, which would make
    // `b == c` and blind the whole check. Move them apart first.
    w.players[0].food = FOOD_NOW;
    w.players[0].water = WATER_NOW;
    until_quiet(&mut w, EV_VITALS);
    let v = only(&w, EV_VITALS);
    distinct3(v, "EV_VITALS");
    distinct_halves(v.b, "EV_VITALS.b");
    distinct_halves(v.c, "EV_VITALS.c");
    assert_eq!(v.a, BODY, "EV_VITALS.a is whose meters these are");
    assert_eq!(
        v.b >> 16,
        w.players[0].food as u32,
        "EV_VITALS.b's HIGH half is FOOD, and it must agree with the sim"
    );
    assert_eq!(
        v.b & 0xffff,
        w.players[0].water as u32,
        "EV_VITALS.b's LOW half is WATER, and it must agree with the sim"
    );
    assert_eq!(
        v.c >> 16,
        MAX_FOOD as u32,
        "EV_VITALS.c's HIGH half is MAX food, not max water"
    );
    assert_eq!(
        v.c & 0xffff,
        MAX_WATER as u32,
        "EV_VITALS.c's LOW half is MAX water, not max food"
    );
}

/// `EV_DRANK: a = player id, b = water units restored, c = hp the drink
/// cost`.
///
/// Two small integers, either of which is plausible in the other's place —
/// swap them and the client thanks the sea for the twenty points it just
/// took. Both come from the fixture, never from this file.
#[test]
fn drank_names_the_player_then_the_water_then_the_hp_it_cost() {
    let mut w = lone_world();
    let sc = SurvivalContent::probe_fixture();
    stand_at_the_shore(&mut w);
    // A full meter refuses rather than paying hp, so drain it first.
    w.players[0].water = WATER_NOW;
    w.tick(&[Command::Drink { id: BODY }]);
    let d = only(&w, EV_DRANK);
    distinct3(d, "EV_DRANK");
    assert_eq!(d.a, BODY, "EV_DRANK.a is who drank");
    assert_eq!(
        d.b, sc.drink_water as u32,
        "EV_DRANK.b is the WATER restored, not the hp it cost"
    );
    assert_eq!(
        d.c, sc.drink_hp_cost as u32,
        "EV_DRANK.c is the HP the drink cost, not the water it gave"
    );
}

/// `EV_CONSUMED: a = player id, b = item index << 16 | inventory slot`.
///
/// The packed field is the risk: high half is what was eaten, low half is
/// where it was eaten from, and nothing but this says which way round.
#[test]
fn consumed_names_the_player_then_item_over_slot() {
    let mut w = lone_world();
    w.players[0].food = FOOD_NOW;
    w.players[0].water = WATER_NOW;
    w.players[0].inv[FOOD_SLOT as usize] = ItemStack {
        item: FOOD_ITEM,
        count: 1,
    };
    w.tick(&[Command::Consume {
        id: BODY,
        slot: FOOD_SLOT,
    }]);
    let c = only(&w, EV_CONSUMED);
    distinct3(c, "EV_CONSUMED");
    distinct_halves(c.b, "EV_CONSUMED.b");
    assert_eq!(c.a, BODY, "EV_CONSUMED.a is who ate");
    assert_eq!(
        c.b >> 16,
        FOOD_ITEM as u32,
        "EV_CONSUMED.b's HIGH half is the ITEM index"
    );
    assert_eq!(
        c.b & 0xffff,
        FOOD_SLOT as u32,
        "EV_CONSUMED.b's LOW half is the inventory SLOT"
    );
}

/// `EV_CONSUME_REFUSED: a = player id, b = survival::REFUSE_C_*`.
///
/// `c` is always zero here, so `distinct3` cannot apply and a different
/// discipline has to: drive *two different causes* and require `b` to
/// move between them. A field pinned against one reason code would pass
/// just as well if the emit site hard-coded it; requiring two proves `b`
/// is the reason channel rather than a constant, and that `a` is the
/// player in both.
#[test]
fn consume_refused_names_the_player_then_why() {
    let mut w = lone_world();

    // Cause one: an empty slot is not food.
    w.players[0].inv[FOOD_SLOT as usize] = ItemStack::default();
    w.tick(&[Command::Consume {
        id: BODY,
        slot: FOOD_SLOT,
    }]);
    let not_food = only(&w, EV_CONSUME_REFUSED);
    assert_eq!(not_food.a, BODY, "EV_CONSUME_REFUSED.a is who was refused");
    assert_eq!(
        not_food.b, REFUSE_C_NOT_FOOD,
        "EV_CONSUME_REFUSED.b is the reason, and an empty hand is NOT_FOOD"
    );

    // Cause two: dry ground has nothing to drink.
    stand_inland(&mut w);
    w.players[0].water = WATER_NOW;
    w.tick(&[Command::Drink { id: BODY }]);
    let no_water = only(&w, EV_CONSUME_REFUSED);
    assert_eq!(no_water.a, BODY, "EV_CONSUME_REFUSED.a is who was refused");
    assert_eq!(
        no_water.b, REFUSE_C_NO_WATER,
        "EV_CONSUME_REFUSED.b is the reason, and dry ground is NO_WATER"
    );

    assert_ne!(
        not_food.b, no_water.b,
        "two different refusals reported the same reason code — `b` is not \
         carrying the cause, so pinning it against one constant proves nothing"
    );
}

// ---------------------------------------------------------------------
// The build/deploy lane: EV_PIECE_PLACED, EV_DEPLOY_PLACED, EV_STOCK,
// EV_DOOR.
//
// This is where the reference's own history is loudest. `EV_STOCK` is the
// one code in the lane whose `a`/`b` convention is *inverted* relative to
// every addressed neighbour — the player is `a` and the cell key is `b`,
// where `EV_PIECE_PLACED`, `EV_DEPLOY_PLACED`, `EV_DOOR` and
// `EV_STRUCT_HIT` all put the cell key in `a`. A reader tidying the doc
// comments into consistency would "correct" it into a bug, and nothing
// downstream would object: both fields are `u32`, the encoder's bytes do
// not move, and the ring is not in `state_hash`.
// ---------------------------------------------------------------------

/// A cell whose center takes a foundation, found by asking the sim's own
/// rule rather than by typing a coordinate that held at one seed. Skips
/// cells where `cx == cz`, because the cell key is `cx << 16 | cz` and a
/// key with equal halves cannot show the pack reversed.
fn buildable_cell(seed: u64) -> (u16, u16) {
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (170 + dx).clamp(0, 1023) as u16;
                let cz = (170 + dz).clamp(0, 1023) as u16;
                if cx == cz {
                    continue;
                }
                let (x, z) = (
                    (cx as f32 + 0.5) * BUILD_CELL_M,
                    (cz as f32 + 0.5) * BUILD_CELL_M,
                );
                if foundation_terrain_ok(seed, x, z) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell within 64 cells — the generator changed under this test");
}

/// One builder, standing on a buildable cell with the fixture's build and
/// deploy tables installed and enough of every input item to pay for the
/// whole arrangement. Returns the cell it is standing on.
fn builder_world() -> (World, u16, u16) {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.tick(&[Command::Join { id: BODY }]);
    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    );
    w.players[0].body = Body::at(SEED, x, z);
    // The fixture's costs: pieces are paid in item 0, the hearth in item
    // 2, the door in item 4. Generous on purpose — a refusal for want of
    // wood would be this fixture's bug, not the sim's.
    for (slot, item) in [(0usize, 0u16), (1, 2), (2, 4)] {
        w.players[0].inv[slot] = ItemStack { item, count: 200 };
    }
    (w, cx, cz)
}

/// Place a piece and leave the world standing on the tick it landed.
fn place_piece(w: &mut World, row: u16, cx: u16, cz: u16, loc: u8) {
    let before = w.pieces.len();
    w.tick(&[Command::Place {
        id: BODY,
        row,
        cx,
        cz,
        level: LEVEL,
        loc,
    }]);
    assert_eq!(
        w.pieces.len(),
        before + 1,
        "piece row {row} did not place at ({cx}, {cz}) loc {loc} — the \
         fixture, not the mechanic"
    );
}

/// Place a deployable and leave the world standing on the tick it landed.
fn place_deploy(w: &mut World, row: u16, cx: u16, cz: u16, loc: u8) {
    let before = w.deploys.len();
    w.tick(&[Command::PlaceDeploy {
        id: BODY,
        row,
        cx,
        cz,
        level: LEVEL,
        loc,
    }]);
    assert_eq!(
        w.deploys.len(),
        before + 1,
        "deploy row {row} did not place at ({cx}, {cz}) loc {loc} — the \
         fixture, not the mechanic"
    );
}

/// The three sub-fields of an addressed event's packed `b`.
fn unpack(b: u32) -> (u32, u32, u32) {
    (b >> 16, (b >> 8) & 0xff, b & 0xff)
}

/// `EV_PIECE_PLACED: a = build cell key (cx << 16 | cz), b = level << 16 |
/// loc << 8 | piece row`.
///
/// The cell key is `a` here — the convention every addressed event in this
/// lane keeps except `EV_STOCK`.
#[test]
fn piece_placed_names_the_cell_then_level_over_loc_over_row() {
    let (mut w, cx, cz) = builder_world();
    place_piece(&mut w, PIECE_FOUNDATION, cx, cz, LOC_PLANE);
    place_piece(&mut w, PIECE_DOORWAY, cx, cz, PIECE_EDGE);

    let p = only(&w, EV_PIECE_PLACED);
    let (level, loc, row) = unpack(p.b);
    distinct_triple(level, loc, row, "EV_PIECE_PLACED.b");
    distinct_halves(p.a, "EV_PIECE_PLACED.a (the cell key)");
    assert_eq!(
        p.a,
        cell_key(cx, cz),
        "EV_PIECE_PLACED.a is the CELL KEY, not the builder"
    );
    assert_eq!(p.a >> 16, cx as u32, "the cell key's HIGH half is cx");
    assert_eq!(p.a & 0xffff, cz as u32, "the cell key's LOW half is cz");
    assert_eq!(
        level, LEVEL as u32,
        "EV_PIECE_PLACED.b's high field is LEVEL"
    );
    assert_eq!(
        loc, PIECE_EDGE as u32,
        "EV_PIECE_PLACED.b's middle field is LOC"
    );
    assert_eq!(
        row, PIECE_DOORWAY as u32,
        "EV_PIECE_PLACED.b's low field is the piece ROW"
    );
}

/// `EV_DEPLOY_PLACED: a = build cell key, b = level << 16 | loc << 8 |
/// row, c = owner player id`.
///
/// Same address shape as the piece, plus an owner in `c` — and the owner
/// is the field that makes this event worth checking, because `a` and `c`
/// are both small integers and a swap puts a cell key where a player id
/// belongs.
#[test]
fn deploy_placed_names_the_cell_then_the_address_then_the_owner() {
    let (mut w, cx, cz) = builder_world();
    place_piece(&mut w, PIECE_FOUNDATION, cx, cz, LOC_PLANE);
    place_piece(&mut w, PIECE_DOORWAY, cx, cz, DOOR_EDGE);
    place_deploy(&mut w, DEPLOY_DOOR, cx, cz, DOOR_EDGE);

    let d = only(&w, EV_DEPLOY_PLACED);
    distinct3(d, "EV_DEPLOY_PLACED");
    let (level, loc, row) = unpack(d.b);
    distinct_triple(level, loc, row, "EV_DEPLOY_PLACED.b");
    assert_eq!(
        d.a,
        cell_key(cx, cz),
        "EV_DEPLOY_PLACED.a is the CELL KEY, not the owner"
    );
    assert_eq!(
        level, LEVEL as u32,
        "EV_DEPLOY_PLACED.b's high field is LEVEL"
    );
    assert_eq!(
        loc, DOOR_EDGE as u32,
        "EV_DEPLOY_PLACED.b's middle field is LOC"
    );
    assert_eq!(
        row, DEPLOY_DOOR as u32,
        "EV_DEPLOY_PLACED.b's low field is the deploy ROW"
    );
    assert_eq!(
        d.c, BODY,
        "EV_DEPLOY_PLACED.c is the OWNER player id, not part of the address"
    );
}

/// `EV_DOOR: a = build cell key, b = level << 16 | loc << 8 | locked << 1
/// | open, c = the player whose action changed it`.
///
/// The door's whole state, absolute. `locked` and `open` are two adjacent
/// bits in the same byte and are exactly the pair a swap would hide — so
/// this drives the toggle and requires the two bits to disagree, which is
/// the one-bit form of `distinct_halves`.
#[test]
fn door_names_the_cell_then_its_whole_state_then_who_moved_it() {
    let (mut w, cx, cz) = builder_world();
    place_piece(&mut w, PIECE_FOUNDATION, cx, cz, LOC_PLANE);
    place_piece(&mut w, PIECE_DOORWAY, cx, cz, DOOR_EDGE);
    place_deploy(&mut w, DEPLOY_DOOR, cx, cz, DOOR_EDGE);

    // A door places locked *and* closed, and placement announces nothing —
    // only the verbs do. Both bits therefore read 1 and 0 together on an
    // owner's first toggle (a locked door still opens for its owner), and
    // a check taken there could not tell the two bits apart. So drive them
    // one at a time and read both ticks: the unlock, then the open.
    w.tick(&[Command::Lock {
        id: BODY,
        cx,
        cz,
        level: LEVEL,
        loc: DOOR_EDGE,
        locked: false,
    }]);
    let unlocked = only(&w, EV_DOOR);
    let (_, _, unlocked_state) = unpack(unlocked.b);

    w.tick(&[Command::Use {
        id: BODY,
        cx,
        cz,
        level: LEVEL,
        loc: DOOR_EDGE,
    }]);
    let d = only(&w, EV_DOOR);
    let (level, loc, state) = unpack(d.b);
    let (locked, open) = ((state >> 1) & 1, state & 1);

    assert_ne!(
        locked, open,
        "the fixture has locked and open holding the same bit, so this \
         check cannot see them swapped. Move the fixture, not the assertion."
    );
    assert_eq!(
        d.a,
        cell_key(cx, cz),
        "EV_DOOR.a is the CELL KEY, not the player who moved it"
    );
    assert_eq!(level, LEVEL as u32, "EV_DOOR.b's high field is LEVEL");
    assert_eq!(loc, DOOR_EDGE as u32, "EV_DOOR.b's middle field is LOC");
    assert_eq!(open, 1, "EV_DOOR.b bit 0 is OPEN, and the toggle opened it");
    assert_eq!(
        locked, 0,
        "EV_DOOR.b bit 1 is LOCKED, and the unlock before this cleared it"
    );
    assert_eq!(
        d.c, BODY,
        "EV_DOOR.c is the player whose action changed it, not the cell"
    );

    // The two bits moved independently, one verb each: the unlock left the
    // door shut, and the toggle opened it without re-locking. Crossing the
    // two bits at the emit site cannot produce this pair.
    assert_eq!(
        unlocked_state, 0,
        "the unlock should leave the door clear of both bits — shut and \
         unlocked — and it read {unlocked_state}"
    );
    assert_eq!(
        state, 1,
        "and the toggle should set the open bit alone, leaving {state} = 1"
    );
}

/// `EV_STOCK: a = feeder player id, b = hearth cell key, c = level`.
///
/// **The inverted one.** Every other addressed event in this lane puts the
/// cell key in `a`; this puts the player there. It is documented that way
/// (`world.rs`) and the server decodes it that way, so the asymmetry is
/// correct — but it is the single likeliest field in the sim to be
/// "tidied" into consistency by someone reading the doc comments in a
/// block, which is precisely how the reference ecosystem shipped ~27 of
/// these.
///
/// `c` is the level, and a hearth is `PLACE_FOUNDATION`, so it can only
/// read 0 here. That is checked by value and stated rather than dressed
/// up: this check sees `a`/`b` crossed, and does not see a level that was
/// never anything but zero.
#[test]
fn stock_names_the_feeder_first_and_the_cell_second() {
    let (mut w, cx, cz) = builder_world();
    place_piece(&mut w, PIECE_FOUNDATION, cx, cz, LOC_PLANE);
    place_deploy(&mut w, DEPLOY_HEARTH, cx, cz, LOC_PLANE);
    w.tick(&[Command::Feed {
        id: BODY,
        cx,
        cz,
        level: LEVEL,
    }]);

    let s = only(&w, EV_STOCK);
    distinct3(s, "EV_STOCK");
    distinct_halves(s.b, "EV_STOCK.b (the cell key)");
    assert_eq!(
        s.a, BODY,
        "EV_STOCK.a is the FEEDER — this event is the lane's one inversion, \
         and a cell key here means someone made it match its neighbours"
    );
    assert_eq!(
        s.b,
        cell_key(cx, cz),
        "EV_STOCK.b is the hearth's CELL KEY, not the feeder"
    );
    assert_eq!(s.c, LEVEL as u32, "EV_STOCK.c is the level fed");
}

/// The inversion, asserted as a relationship rather than left implicit in
/// two separate tests.
///
/// `EV_DEPLOY_PLACED` and `EV_STOCK` describe the *same hearth on the same
/// cell*, and they disagree about which field holds what on purpose. If a
/// later edit makes them agree, both of the tests above still pass
/// individually — one of them would simply be asserting the wrong
/// convention. This is the pair check that catches that, and it is
/// `the_hit_and_the_health_name_opposite_players` applied one lane over.
#[test]
fn the_placement_and_the_feed_disagree_about_field_a_on_purpose() {
    let (mut w, cx, cz) = builder_world();
    place_piece(&mut w, PIECE_FOUNDATION, cx, cz, LOC_PLANE);
    place_deploy(&mut w, DEPLOY_HEARTH, cx, cz, LOC_PLANE);
    let placed = only(&w, EV_DEPLOY_PLACED);
    w.tick(&[Command::Feed {
        id: BODY,
        cx,
        cz,
        level: LEVEL,
    }]);
    let stocked = only(&w, EV_STOCK);

    assert_eq!(
        placed.a,
        cell_key(cx, cz),
        "the placement leads with the cell"
    );
    assert_eq!(stocked.a, BODY, "the feed leads with the player");
    assert_ne!(
        placed.a, stocked.a,
        "EV_DEPLOY_PLACED.a and EV_STOCK.a now hold the same kind of thing \
         for one hearth on one cell. One of them has been changed to match \
         the other, and `world.rs` says they differ."
    );
    // And the mirror: the cell key and the player id appear in both
    // events, in opposite seats.
    assert_eq!(
        placed.c, stocked.a,
        "the owner and the feeder are one player"
    );
    assert_eq!(placed.a, stocked.b, "and both name one cell");
}

/// Coverage, stated rather than implied.
///
/// A gate that checks five of twenty-five codes and says nothing about the
/// other twenty reads, to the next person, as "the event lane is covered."
/// It is not. This pins the ledger so the number cannot drift silently and
/// a new `EV_*` cannot land without someone deciding whether it needs a
/// role check. Closing the remaining twenty is queued work
/// (`reference/FINDINGS.md` §1), not a reason to leave the five unchecked.
#[test]
fn coverage_is_stated_not_implied() {
    /// Every code `world.rs` can emit is 1..=EV_MAX.
    const EV_MAX: u8 = 25;
    /// Driven through a real cause and asserted field by field above.
    const COVERED: [u8; 13] = [
        EV_GATHER,
        EV_HIT,
        EV_HEALTH,
        EV_DEATH,
        EV_BAG_DROPPED,
        EV_VITALS,
        EV_CONSUMED,
        EV_CONSUME_REFUSED,
        EV_DRANK,
        EV_PIECE_PLACED,
        EV_DEPLOY_PLACED,
        EV_DOOR,
        EV_STOCK,
    ];
    /// What is knowingly still byte-golden only. Change this number in the
    /// same commit that changes `COVERED`, never on its own.
    const UNCOVERED: usize = 12;

    let mut counted = 0usize;
    for code in 1..=EV_MAX {
        if !COVERED.contains(&code) {
            counted += 1;
        }
    }
    assert_eq!(
        counted, UNCOVERED,
        "the coverage ledger is stale: {counted} of the {EV_MAX} event codes \
         have no role check, but this test claims {UNCOVERED}. A new EV_* \
         landed, or one gained a check without the ledger moving."
    );
    assert_eq!(
        COVERED.len() + UNCOVERED,
        EV_MAX as usize,
        "the ledger does not add up to the code range"
    );
}
