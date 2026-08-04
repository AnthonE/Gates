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
use sim_core::inventory::{self, CONT_SELF, REFUSE_M_EMPTY};
use sim_core::loot::LootContent;
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::survival::{SurvivalContent, DRINK_REACH_M, REFUSE_C_NOT_FOOD, REFUSE_C_NO_WATER};
use sim_core::terrain;
use sim_core::world::{
    Command, SimEvent, World, EV_BAG_DROPPED, EV_CONSUMED, EV_CONSUME_REFUSED, EV_DEATH,
    EV_DEPLOY_PLACED, EV_DEPLOY_REMOVED, EV_DOOR, EV_DRANK, EV_GATHER, EV_HEALTH, EV_HIT, EV_MAX,
    EV_MOVED, EV_MOVE_REFUSED, EV_PIECE_PLACED, EV_PIECE_REMOVED, EV_SLOT_HARVESTED, EV_STOCK,
    EV_STRUCT_HIT, EV_VITALS, STRUCT_DEPLOY_BIT,
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
/// `BuildContent::probe_fixture`: row 0 is the foundation, row 1 a wood
/// wall, row 2 a stone floor, row 3 the doorway.
/// `DeployContent::probe_fixture`: row 0 is the hearth, row 2 the door.
const PIECE_FOUNDATION: u16 = 0;
const PIECE_WALL: u16 = 1;
const PIECE_FLOOR: u16 = 2;
const PIECE_DOORWAY: u16 = 3;
const DEPLOY_HEARTH: u16 = 0;
const DEPLOY_DOOR: u16 = 2;

/// The fixture's hp, and what the fixture spear takes off in one swing.
/// `CombatContent::probe_fixture` item 0 deals 34 to a structure;
/// `BuildContent::probe_fixture`'s wood wall has 100 hp and
/// `DeployContent::probe_fixture`'s door has 60. Both first swings
/// therefore leave a `damage << 16 | hp left` whose halves differ, which is
/// what `EV_STRUCT_HIT.c` has to be read against.
const STRUCT_DAMAGE: u32 = 34;
const WALL_HP: u32 = 100;
const DOOR_HP: u32 = 60;

/// The two build levels this file uses, and why there are two.
///
/// Everything used to sit on level 0, so the `level` field of all four
/// addressed payloads — `EV_PIECE_PLACED.b`, `EV_DEPLOY_PLACED.b`,
/// `EV_DOOR.b` and `EV_STOCK.c` — was checked against a value that never
/// varied. A field pinned at its own zero is `distinct3`'s blindness in one
/// dimension: a `level` seat that some future edit stopped writing reads
/// identically. So the arrangement stands a storey. `GROUND` carries the
/// foundation that supports it (and the raid checks, which have to be
/// reachable from the ground); `UPPER` carries the doorways, the door, and
/// the hearth, and is the level every addressed check now asserts.
const GROUND: u8 = 0;
const UPPER: u8 = 1;

/// The builder's own id, distinct from `BODY`/`ATTACKER` for one payload's
/// sake: `EV_STOCK` is `a = feeder, b = cell key, c = level`, and with the
/// hearth on `UPPER` a builder numbered 1 would put 1 in both `a` and `c`
/// and `distinct3` would (correctly) refuse the check. Moving the id is the
/// smaller move than flattening the storey back down.
const BUILDER: u32 = 4;

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
///
/// With the arrangement standing a storey (`UPPER`) the two triples read
/// 1/2/3 and 1/3/2: three different numbers each, and no longer a level
/// that is only ever its own zero.
const PIECE_EDGE: u8 = LOC_EDGE_W;
const DOOR_EDGE: u8 = LOC_EDGE_N;

/// Where a raider stands to swing at the arrangement, and which way it
/// faces. One build cell west of the target column, looking back east —
/// `combat.rs`'s own raid rig in the same posture, because the target scan
/// picks the nearest anchor it is aimed at, and the west edge of the cell
/// is what that resolves to from here. Yaw 64/256 of a turn is +x over the
/// 256-entry LUT.
const RAID_YAW: u16 = 64 << 8;
const RAID_OFFSET_CELLS: f32 = 1.0;

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

/// The first cell holding `kind`, scanned off `terrain::scatter` rather
/// than typed in: a cell that held a barrel at one seed and one weight
/// table is a fixture that silently stops meaning what it says. Returns
/// the slot's world position and its cell, because the swinger is stood
/// exactly on it — `POINT_BLANK_M2` bypasses the aim cone, so the swing
/// lands without this fixture also having to reproduce a yaw.
fn scanned_slot(w: &World, kind: terrain::Occupant) -> (f32, f32, u16, u16) {
    let span = (terrain::ISLAND_SIZE / terrain::CELL_SIZE) as i32;
    for cz in 0..span {
        for cx in 0..span {
            let s = terrain::scatter(SEED, &w.scatter, &w.haven, cx, cz);
            if s.occupant == kind {
                return (s.x, s.z, cx as u16, cz as u16);
            }
        }
    }
    panic!("no {kind:?} on this island — the scatter table changed under this gate");
}

/// `EV_SLOT_HARVESTED: a = cell key, b = terrain occupant ordinal`, on a
/// barrel.
///
/// The barrel is the reason the field had to be named rather than
/// counted: it has no row in the gather table at all, so the old
/// "gatherable index" reading had no value it could honestly carry.
#[test]
fn slot_harvested_on_a_barrel_names_the_cell_then_the_occupant() {
    let mut w = duel_world();
    w.loot = LootContent::probe_fixture();
    let (x, z, cx, cz) = scanned_slot(&w, terrain::Occupant::BarrelSlot);
    w.players[0].body = Body::at(SEED, x, z);
    until(&mut w, EV_SLOT_HARVESTED);
    let ev = only(&w, EV_SLOT_HARVESTED);
    assert_ne!(
        ev.a, ev.b,
        "EV_SLOT_HARVESTED carries the same value twice, so this check \
         cannot see a swap"
    );
    assert_eq!(
        ev.b,
        terrain::Occupant::BarrelSlot as u32,
        "EV_SLOT_HARVESTED.b is the occupant ordinal, not the cell"
    );
    assert_eq!(
        ev.a,
        cell_key(cx, cz),
        "EV_SLOT_HARVESTED.a is the cell key, not the occupant"
    );
}

/// The same event on a node, and the reason this test is not redundant
/// with the barrel above: a tree is occupant **1** and gather-table row
/// **0**. The two readings of field `b` differ by exactly one, which is
/// the quietest possible wrong value — every `u32` check passes, the
/// encoder is untouched so `test_protocol_golden` is green, and the ring
/// is outside `state_hash` so `test_replay` is green. Only a fixture that
/// knows which number it wants can see it.
#[test]
fn slot_harvested_on_a_node_names_the_occupant_not_the_table_row() {
    let mut w = duel_world();
    let (x, z, cx, cz) = scanned_slot(&w, terrain::Occupant::Tree);
    w.players[0].body = Body::at(SEED, x, z);
    until(&mut w, EV_SLOT_HARVESTED);
    let ev = only(&w, EV_SLOT_HARVESTED);
    assert_eq!(
        ev.b,
        terrain::Occupant::Tree as u32,
        "EV_SLOT_HARVESTED.b is the occupant ordinal (Tree = 1), not the \
         gather-table row (Tree = 0)"
    );
    assert_ne!(ev.b, 0, "the table row and the ordinal have been confused");
    assert_eq!(
        ev.a,
        cell_key(cx, cz),
        "EV_SLOT_HARVESTED.a is the cell key"
    );
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
///
/// Takes the `World` by `&mut` and never returns one. It used to return
/// `(World, u16, u16)`, and a `World` inside a returned tuple is a second
/// one in the frame: an unoptimized build cannot elide the move out of the
/// tuple, so every test here overflowed its thread stack under a plain
/// `cargo test` while `--release` — what `ci/gates.sh` runs — quietly
/// elided the copy and stayed green on an unmeasured margin. `duel_world`
/// returns a bare `World`, which is constructible in place; a tuple is not.
/// Same discipline `combat.rs` states, and this file has now learned it
/// twice.
fn builder_world(w: &mut World) -> (u16, u16) {
    w.gather = GatherContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.tick(&[Command::Join { id: BUILDER }]);
    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    );
    w.players[0].body = Body::at(SEED, x, z);
    // The fixture's costs: the wood pieces are paid in item 0, the stone
    // ones (the floor this arrangement's second storey stands on) in item
    // 1, the hearth in item 2, the door in item 4. Generous on purpose — a
    // refusal for want of wood would be this fixture's bug, not the sim's.
    for (slot, item) in [(0usize, 0u16), (1, 1), (2, 2), (3, 4)] {
        w.players[0].inv[slot] = ItemStack { item, count: 200 };
    }
    (cx, cz)
}

/// The storey the addressed checks read: a foundation on the ground, a wall
/// on its west edge, and a floor on top of the wall — so `UPPER` is a real,
/// supported level rather than a number the test asserts about an empty
/// column. The wall is not decoration: `build.rs`'s support rule v0 carries
/// a floor on *an edge piece under one of the cell's four sides*, so a
/// foundation alone refuses the storey with `REFUSE_B_SPOT`'s neighbour,
/// `REFUSE_B_SUPPORT`. Leaves the world standing on the tick the floor
/// landed.
fn stand_a_storey(w: &mut World, cx: u16, cz: u16) {
    place_piece(w, PIECE_FOUNDATION, cx, cz, GROUND, LOC_PLANE);
    place_piece(w, PIECE_WALL, cx, cz, GROUND, LOC_EDGE_W);
    place_piece(w, PIECE_FLOOR, cx, cz, UPPER, LOC_PLANE);
}

/// Place a piece and leave the world standing on the tick it landed.
fn place_piece(w: &mut World, row: u16, cx: u16, cz: u16, level: u8, loc: u8) {
    let before = w.pieces.len();
    w.tick(&[Command::Place {
        id: BUILDER,
        row,
        cx,
        cz,
        level,
        loc,
    }]);
    assert_eq!(
        w.pieces.len(),
        before + 1,
        "piece row {row} did not place at ({cx}, {cz}) level {level} loc \
         {loc} — the fixture, not the mechanic"
    );
}

/// Place a deployable and leave the world standing on the tick it landed.
fn place_deploy(w: &mut World, row: u16, cx: u16, cz: u16, level: u8, loc: u8) {
    let before = w.deploys.len();
    w.tick(&[Command::PlaceDeploy {
        id: BUILDER,
        row,
        cx,
        cz,
        level,
        loc,
    }]);
    assert_eq!(
        w.deploys.len(),
        before + 1,
        "deploy row {row} did not place at ({cx}, {cz}) level {level} loc \
         {loc} — the fixture, not the mechanic"
    );
}

/// Move the builder one cell west of the target column and face it, then
/// swing until `code` lands. The raid posture: the target scan wants an
/// anchor it is aimed at, and a body standing *inside* its own cell is not
/// aiming at that cell's edges.
fn raid_until(w: &mut World, cx: u16, cz: u16, code: u8) {
    let (x, z) = (
        (cx as f32 + 0.5 - RAID_OFFSET_CELLS) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    );
    w.players[0].body = Body::at(SEED, x, z);
    let mut seq = 0u16;
    for _ in 0..MAX_STEPS {
        w.tick(&[Command::Input {
            id: BUILDER,
            frame: InputFrame {
                seq,
                buttons: BTN_PRIMARY,
                yaw: RAID_YAW,
                pitch: 128,
                move_x: 0,
                move_z: 0,
                sel: 0,
            },
        }]);
        seq = seq.wrapping_add(1);
        if count(w, code) > 0 {
            return;
        }
    }
    panic!("event code {code} never landed in {MAX_STEPS} sim ticks of raiding");
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
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);
    stand_a_storey(&mut w, cx, cz);
    place_piece(&mut w, PIECE_DOORWAY, cx, cz, UPPER, PIECE_EDGE);

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
        level, UPPER as u32,
        "EV_PIECE_PLACED.b's high field is LEVEL, and this doorway is on \
         the second storey — a check that reads 0 here is reading a field \
         nobody wrote"
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
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);
    stand_a_storey(&mut w, cx, cz);
    place_piece(&mut w, PIECE_DOORWAY, cx, cz, UPPER, DOOR_EDGE);
    place_deploy(&mut w, DEPLOY_DOOR, cx, cz, UPPER, DOOR_EDGE);

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
        level, UPPER as u32,
        "EV_DEPLOY_PLACED.b's high field is LEVEL, and this door hangs on \
         the second storey"
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
        d.c, BUILDER,
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
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);
    stand_a_storey(&mut w, cx, cz);
    place_piece(&mut w, PIECE_DOORWAY, cx, cz, UPPER, DOOR_EDGE);
    place_deploy(&mut w, DEPLOY_DOOR, cx, cz, UPPER, DOOR_EDGE);

    // A door places locked *and* closed, and placement announces nothing —
    // only the verbs do. Both bits therefore read 1 and 0 together on an
    // owner's first toggle (a locked door still opens for its owner), and
    // a check taken there could not tell the two bits apart. So drive them
    // one at a time and read both ticks: the unlock, then the open.
    w.tick(&[Command::Lock {
        id: BUILDER,
        cx,
        cz,
        level: UPPER,
        loc: DOOR_EDGE,
        locked: false,
    }]);
    let unlocked = only(&w, EV_DOOR);
    let (_, _, unlocked_state) = unpack(unlocked.b);

    w.tick(&[Command::Use {
        id: BUILDER,
        cx,
        cz,
        level: UPPER,
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
    assert_eq!(level, UPPER as u32, "EV_DOOR.b's high field is LEVEL");
    assert_eq!(loc, DOOR_EDGE as u32, "EV_DOOR.b's middle field is LOC");
    assert_eq!(open, 1, "EV_DOOR.b bit 0 is OPEN, and the toggle opened it");
    assert_eq!(
        locked, 0,
        "EV_DOOR.b bit 1 is LOCKED, and the unlock before this cleared it"
    );
    assert_eq!(
        d.c, BUILDER,
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
/// `c` is the level, and it used to be unfalsifiable: a hearth is
/// `PLACE_FOUNDATION`, the whole arrangement sat on level 0, so this seat
/// could only ever read its own zero and a `c` nobody wrote would have
/// passed. `PLACE_FOUNDATION` wants *a plane piece at that level*, and a
/// floor is one — so the hearth now stands on `UPPER` and `c` has a value
/// to be wrong about.
#[test]
fn stock_names_the_feeder_first_and_the_cell_second() {
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);
    stand_a_storey(&mut w, cx, cz);
    place_deploy(&mut w, DEPLOY_HEARTH, cx, cz, UPPER, LOC_PLANE);
    w.tick(&[Command::Feed {
        id: BUILDER,
        cx,
        cz,
        level: UPPER,
    }]);

    let s = only(&w, EV_STOCK);
    distinct3(s, "EV_STOCK");
    distinct_halves(s.b, "EV_STOCK.b (the cell key)");
    assert_eq!(
        s.a, BUILDER,
        "EV_STOCK.a is the FEEDER — this event is the lane's one inversion, \
         and a cell key here means someone made it match its neighbours"
    );
    assert_eq!(
        s.b,
        cell_key(cx, cz),
        "EV_STOCK.b is the hearth's CELL KEY, not the feeder"
    );
    assert_eq!(
        s.c, UPPER as u32,
        "EV_STOCK.c is the LEVEL fed, and this hearth is upstairs"
    );
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
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);
    stand_a_storey(&mut w, cx, cz);
    place_deploy(&mut w, DEPLOY_HEARTH, cx, cz, UPPER, LOC_PLANE);
    let placed = only(&w, EV_DEPLOY_PLACED);
    w.tick(&[Command::Feed {
        id: BUILDER,
        cx,
        cz,
        level: UPPER,
    }]);
    let stocked = only(&w, EV_STOCK);

    assert_eq!(
        placed.a,
        cell_key(cx, cz),
        "the placement leads with the cell"
    );
    assert_eq!(stocked.a, BUILDER, "the feed leads with the player");
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

/// `EV_STRUCT_HIT: a = build cell key, b = STRUCT_DEPLOY_BIT | level << 16
/// | loc << 8 | row, c = damage dealt << 16 | hp left`.
///
/// The raid's progress bar, and the widest payload in the lane: four
/// distinct meanings over three fields, two of them packed. `c` is the pair
/// a swap would hide most expensively — `damage << 16 | left` reversed
/// draws a wall gaining health as it is beaten, and the fixture keeps the
/// halves apart (34 dealt, 66 left) so this check can see it.
///
/// The target is a wood wall on the *west edge* of the ground storey, which
/// is what makes `b` readable: a foundation would address 0/0/0 and a check
/// against three zeroes is blind to every permutation of them.
#[test]
fn struct_hit_names_the_cell_then_the_address_then_damage_over_hp_left() {
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);
    place_piece(&mut w, PIECE_FOUNDATION, cx, cz, GROUND, LOC_PLANE);
    place_piece(&mut w, PIECE_WALL, cx, cz, GROUND, PIECE_EDGE);
    raid_until(&mut w, cx, cz, EV_STRUCT_HIT);

    let h = only(&w, EV_STRUCT_HIT);
    distinct3(h, "EV_STRUCT_HIT");
    let (level, loc, row) = unpack(h.b & !STRUCT_DEPLOY_BIT);
    distinct_triple(level, loc, row, "EV_STRUCT_HIT.b");
    distinct_halves(h.c, "EV_STRUCT_HIT.c (damage over hp left)");
    distinct_halves(h.a, "EV_STRUCT_HIT.a (the cell key)");

    assert_eq!(
        h.a,
        cell_key(cx, cz),
        "EV_STRUCT_HIT.a is the CELL KEY, not the raider"
    );
    assert_eq!(
        h.b & STRUCT_DEPLOY_BIT,
        0,
        "a PIECE was hit, so STRUCT_DEPLOY_BIT is clear — set it here and \
         the client looks the address up in the wrong store"
    );
    assert_eq!(
        level, GROUND as u32,
        "EV_STRUCT_HIT.b's high field is LEVEL"
    );
    assert_eq!(
        loc, PIECE_EDGE as u32,
        "EV_STRUCT_HIT.b's middle field is LOC"
    );
    assert_eq!(
        row, PIECE_WALL as u32,
        "EV_STRUCT_HIT.b's low field is the piece ROW"
    );
    assert_eq!(
        h.c >> 16,
        STRUCT_DAMAGE,
        "EV_STRUCT_HIT.c's HIGH half is the damage DEALT, not the hp left"
    );
    assert_eq!(
        h.c & 0xffff,
        WALL_HP - STRUCT_DAMAGE,
        "EV_STRUCT_HIT.c's LOW half is the hp LEFT, not the damage dealt — \
         reversed, a raided wall appears to heal under the swing"
    );
}

/// The deployable half of the same code, and the bit that tells the two
/// apart.
///
/// `damage_deploy` sets `STRUCT_DEPLOY_BIT` and `damage_piece` does not,
/// and that single bit is the whole of how a client knows which store the
/// address names. A door and a doorway sit at the *same* address, so
/// dropping the bit does not merely mislabel the hit — it points the client
/// at the piece the door hangs in. Nothing else in the payload could
/// disambiguate them.
#[test]
fn struct_hit_on_a_deployable_sets_the_deploy_bit() {
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);
    place_piece(&mut w, PIECE_FOUNDATION, cx, cz, GROUND, LOC_PLANE);
    place_piece(&mut w, PIECE_DOORWAY, cx, cz, GROUND, PIECE_EDGE);
    place_deploy(&mut w, DEPLOY_DOOR, cx, cz, GROUND, PIECE_EDGE);
    raid_until(&mut w, cx, cz, EV_STRUCT_HIT);

    let h = only(&w, EV_STRUCT_HIT);
    distinct3(h, "EV_STRUCT_HIT (deployable)");
    distinct_halves(h.c, "EV_STRUCT_HIT.c (damage over hp left)");
    let (level, loc, row) = unpack(h.b & !STRUCT_DEPLOY_BIT);

    assert_eq!(
        h.b & STRUCT_DEPLOY_BIT,
        STRUCT_DEPLOY_BIT,
        "the DOOR takes the swing, not the doorway it hangs in, so \
         STRUCT_DEPLOY_BIT is set — clear it and the client charges the \
         damage to the piece at the same address"
    );
    assert_eq!(h.a, cell_key(cx, cz), "EV_STRUCT_HIT.a is the CELL KEY");
    assert_eq!(level, GROUND as u32, "the level, under the bit");
    assert_eq!(loc, PIECE_EDGE as u32, "the loc, under the bit");
    assert_eq!(
        row, DEPLOY_DOOR as u32,
        "the low field is the DEPLOY row, read against the deploy table"
    );
    assert_eq!(
        h.c >> 16,
        STRUCT_DAMAGE,
        "the damage DEALT is the high half here too"
    );
    assert_eq!(
        h.c & 0xffff,
        DOOR_HP - STRUCT_DAMAGE,
        "and the hp LEFT the low half — the door's 60, not the wall's 100"
    );
}

/// `EV_PIECE_REMOVED: a = build cell key, b = level << 16 | loc << 8 | row`.
///
/// The same address the hit carried, one swing later. Worth checking as its
/// own event rather than trusting the hit: this is a *different* emit site
/// (`drop_piece`, which decay also reaches), and the pair is where the two
/// could drift apart — a removal that named a different cell than the hits
/// that caused it would leave the piece drawn forever on every client that
/// saw the raid.
#[test]
fn piece_removed_names_the_cell_then_the_address_it_was_hit_at() {
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);
    place_piece(&mut w, PIECE_FOUNDATION, cx, cz, GROUND, LOC_PLANE);
    place_piece(&mut w, PIECE_WALL, cx, cz, GROUND, PIECE_EDGE);
    raid_until(&mut w, cx, cz, EV_STRUCT_HIT);
    let hit = only(&w, EV_STRUCT_HIT);
    raid_until(&mut w, cx, cz, EV_PIECE_REMOVED);

    let r = only(&w, EV_PIECE_REMOVED);
    let (level, loc, row) = unpack(r.b);
    distinct_triple(level, loc, row, "EV_PIECE_REMOVED.b");
    distinct_halves(r.a, "EV_PIECE_REMOVED.a (the cell key)");

    assert_eq!(
        r.a,
        cell_key(cx, cz),
        "EV_PIECE_REMOVED.a is the CELL KEY, not the raider who felled it"
    );
    assert_eq!(
        level, GROUND as u32,
        "EV_PIECE_REMOVED.b's high field is LEVEL"
    );
    assert_eq!(
        loc, PIECE_EDGE as u32,
        "EV_PIECE_REMOVED.b's middle field is LOC"
    );
    assert_eq!(
        row, PIECE_WALL as u32,
        "EV_PIECE_REMOVED.b's low field is the piece ROW"
    );
    assert_eq!(
        r.c, 0,
        "EV_PIECE_REMOVED states no role for c, and the emit site passes 0"
    );

    // The address the removal names is the address the hits named. Checked
    // as a relationship, because each test above passes on its own while
    // the two sites disagree.
    assert_eq!(r.a, hit.a, "the removal and the hits name one cell");
    assert_eq!(
        r.b,
        hit.b & !STRUCT_DEPLOY_BIT,
        "and one address — the removal carries no store bit, the hit does"
    );
    assert!(
        w.pieces
            .entries()
            .iter()
            .take(w.pieces.len())
            .all(|p| !(p.cx == cx && p.cz == cz && p.level == GROUND && p.loc == PIECE_EDGE)),
        "the wall is announced gone but still in the store"
    );
}

/// `EV_DEPLOY_REMOVED: a = build cell key, b = level << 16 | loc << 8 |
/// row`.
///
/// Byte-for-byte the same shape as `EV_PIECE_REMOVED` and a different code,
/// which is exactly the pair most worth checking together: the two removals
/// address two different stores, they are one line apart in `world.rs`, and
/// nothing in a payload distinguishes them but the code itself. A door
/// removed under its own doorway's code deletes the doorway on every client
/// that hears it.
#[test]
fn deploy_removed_names_the_cell_and_the_deploy_row_not_the_piece_under_it() {
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);
    place_piece(&mut w, PIECE_FOUNDATION, cx, cz, GROUND, LOC_PLANE);
    place_piece(&mut w, PIECE_DOORWAY, cx, cz, GROUND, PIECE_EDGE);
    place_deploy(&mut w, DEPLOY_DOOR, cx, cz, GROUND, PIECE_EDGE);
    raid_until(&mut w, cx, cz, EV_DEPLOY_REMOVED);

    let r = only(&w, EV_DEPLOY_REMOVED);
    let (level, loc, row) = unpack(r.b);
    distinct_halves(r.a, "EV_DEPLOY_REMOVED.a (the cell key)");

    assert_eq!(
        r.a,
        cell_key(cx, cz),
        "EV_DEPLOY_REMOVED.a is the CELL KEY, not the raider"
    );
    assert_eq!(
        level, GROUND as u32,
        "EV_DEPLOY_REMOVED.b's high field is LEVEL"
    );
    assert_eq!(
        loc, PIECE_EDGE as u32,
        "EV_DEPLOY_REMOVED.b's middle field is LOC"
    );
    assert_eq!(
        row, DEPLOY_DOOR as u32,
        "EV_DEPLOY_REMOVED.b's low field is the DEPLOY row — read against \
         the piece table it would name the doorway"
    );
    assert_eq!(
        r.b & STRUCT_DEPLOY_BIT,
        0,
        "the removal carries no store bit: its own code already says which \
         store this is, and setting it here would corrupt the level field"
    );
    assert_eq!(
        r.c, 0,
        "EV_DEPLOY_REMOVED states no role for c, and the emit site passes 0"
    );

    // The doorway is still standing. The two removals are separate codes
    // over separate stores, and the door falling must not take its frame.
    assert_eq!(w.deploys.len(), 0, "the door fell");
    assert!(
        w.pieces
            .entries()
            .iter()
            .take(w.pieces.len())
            .any(|p| p.cx == cx && p.cz == cz && p.level == GROUND && p.loc == PIECE_EDGE),
        "the doorway the door hung in went with it — that is the two \
         removals crossed, and the piece store is the one that lost"
    );
    assert_eq!(
        count(&w, EV_PIECE_REMOVED),
        0,
        "a deployable fell and the PIECE removal code was emitted for it"
    );
}

/// `EV_MOVED`: a = player id, b = the move's address, c = count << 16 |
/// the item that left the source slot.
///
/// The address is four parts in one field — from kind, from slot, to kind,
/// to slot — which is precisely the shape `reference/FINDINGS.md` §1 counts
/// ~27 corrections of over there: the right value in the wrong position.
/// So all four are held apart in the fixture, and so are the two halves of
/// `c`. A transposed pair fails here rather than shipping a panel that
/// rolls back the wrong slot forever.
#[test]
fn moved_names_the_address_and_what_moved() {
    let mut w = duel_world();
    w.players[0].inv[0] = ItemStack {
        item: JUNK,
        count: 30,
    };
    w.players[0].inv[9] = ItemStack::default();

    const FROM_SLOT: u8 = 0;
    const TO_SLOT: u8 = 9;
    const COUNT: u16 = 12;
    assert_ne!(FROM_SLOT, TO_SLOT, "the two slots must be distinguishable");
    w.tick(&[Command::Move {
        id: ATTACKER,
        bag: 0,
        from_kind: CONT_SELF,
        from_slot: FROM_SLOT,
        to_kind: CONT_SELF,
        to_slot: TO_SLOT,
        count: COUNT,
    }]);

    let e = only(&w, EV_MOVED);
    assert_eq!(e.a, ATTACKER, "a is the player who moved it");
    assert_eq!(
        e.b,
        inventory::addr(CONT_SELF, FROM_SLOT, CONT_SELF, TO_SLOT),
        "b is the address, from before to"
    );
    // The pack, read back part by part — an assertion against the whole
    // word alone would agree with `addr` even if both were reversed.
    assert_eq!(e.b >> 24, CONT_SELF as u32, "b[31:24] is the from kind");
    assert_eq!(
        (e.b >> 16) & 0xff,
        FROM_SLOT as u32,
        "b[23:16] is the from slot"
    );
    assert_eq!(
        (e.b >> 8) & 0xff,
        CONT_SELF as u32,
        "b[15:8] is the to kind"
    );
    assert_eq!(e.b & 0xff, TO_SLOT as u32, "b[7:0] is the to slot");
    distinct_halves(e.c, "EV_MOVED.c");
    assert_eq!(e.c >> 16, COUNT as u32, "c's high half is the count");
    assert_eq!(e.c & 0xffff, JUNK as u32, "c's low half is the item");
    // And the world actually did it — a role check against an event whose
    // cause did nothing is a check on a lie.
    assert_eq!(w.players[0].inv[TO_SLOT as usize].count, COUNT);
}

/// `EV_MOVE_REFUSED`: a = player id, b = the reason, c = the address.
///
/// **Reason in `b`, address in `c`** — the opposite order from `EV_MOVED`,
/// and deliberately so: every refusal in this lane puts its reason in `b`
/// (`EV_DEPLOY_REFUSED`, `EV_CONSUME_REFUSED`, `EV_BUILD_REFUSED`), so the
/// field a reader reaches for first means the same thing lane-wide. That
/// consistency is exactly the kind of thing a transposition breaks
/// silently, so it is asserted rather than assumed.
#[test]
fn move_refused_names_the_reason_then_the_address() {
    let mut w = duel_world();
    w.players[0].inv[3] = ItemStack::default(); // empty source: a known cause

    const FROM_SLOT: u8 = 3;
    const TO_SLOT: u8 = 14;
    w.tick(&[Command::Move {
        id: ATTACKER,
        bag: 0,
        from_kind: CONT_SELF,
        from_slot: FROM_SLOT,
        to_kind: CONT_SELF,
        to_slot: TO_SLOT,
        count: 1,
    }]);

    let e = only(&w, EV_MOVE_REFUSED);
    distinct3(e, "EV_MOVE_REFUSED");
    assert_eq!(e.a, ATTACKER, "a is the player refused");
    assert_eq!(
        e.b, REFUSE_M_EMPTY,
        "b is the reason — an empty source slot, not the address"
    );
    assert_eq!(
        e.c,
        inventory::addr(CONT_SELF, FROM_SLOT, CONT_SELF, TO_SLOT),
        "c is the address that was asked for, so the client can roll it back"
    );
    assert_eq!(e.c & 0xff, TO_SLOT as u32, "c[7:0] is the to slot");
    assert_eq!(
        (e.c >> 16) & 0xff,
        FROM_SLOT as u32,
        "c[23:16] is the from slot"
    );
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
    /// Driven through a real cause and asserted field by field above.
    const COVERED: [u8; 19] = [
        EV_GATHER,
        EV_SLOT_HARVESTED,
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
        EV_STRUCT_HIT,
        EV_PIECE_REMOVED,
        EV_DEPLOY_REMOVED,
        EV_MOVED,
        EV_MOVE_REFUSED,
    ];
    /// What is knowingly still byte-golden only. Change this number in the
    /// same commit that changes `COVERED`, never on its own.
    const UNCOVERED: usize = 8;

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

/// The ledger's own range is derived, not asserted.
///
/// `coverage_is_stated_not_implied` scans `1..=EV_MAX` and claims that a new
/// `EV_*` cannot land without someone classifying it. That claim was false
/// while `EV_MAX` was a literal `25` in this file: `EV_FOO: u8 = 26` landed
/// with the ledger green and the new code outside the range it scanned, so
/// the gate would have reported full knowledge of a lane it had not looked
/// at. `EV_MAX` now lives in `world.rs` next to the codes — which halves the
/// problem, because it is at least in the file being edited — and this
/// closes the other half by reading the declarations themselves.
///
/// Parsing a source file in a test is not a habit worth spreading, and it is
/// right here: the fact under assertion is *what the constant block
/// contains*, and no amount of importing can see a constant that the
/// importer was never told about.
#[test]
fn every_event_code_is_in_range() {
    const SRC: &str = include_str!("../src/world.rs");

    // Borrowed out of `SRC`, never built: wall 3 disallows `String` and
    // `format!` in this crate, and it is right to bind a test too — the
    // names here are `'static` slices of the source and copying them would
    // buy nothing but a wall violation.
    let mut seen: Vec<(&str, u8)> = Vec::new();
    for line in SRC.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("pub const EV_") else {
            continue;
        };
        let Some((name, value)) = rest.split_once(": u8 = ") else {
            continue;
        };
        // `EV_MAX` is the bound itself, and it is defined as `EV_RESPAWN`
        // rather than a digit — it is not one of the codes being bounded.
        if name == "MAX" {
            continue;
        }
        let value = value.trim_end_matches(';');
        let code: u8 = value.parse().unwrap_or_else(|_| {
            panic!(
                "EV_{name} is declared as `{value}`, which is not a literal \
                 code — this parser reads the constant block in `world.rs` \
                 and a non-literal there makes the ledger's range unknowable"
            )
        });
        seen.push((name, code));
    }

    assert!(
        seen.len() >= 25,
        "only {} event codes parsed out of world.rs — the constant block's \
         shape changed and this gate is now reading nothing, which is worse \
         than failing",
        seen.len()
    );

    let highest = seen.iter().map(|(_, c)| *c).max().unwrap();
    assert_eq!(
        highest, EV_MAX,
        "world.rs declares an event code {highest} but EV_MAX is {EV_MAX}. \
         The coverage ledger scans 1..=EV_MAX, so the codes past it are \
         unclassified while the gate reads green. Move EV_MAX in the same \
         commit as the new code."
    );
    assert_eq!(
        seen.len(),
        EV_MAX as usize,
        "world.rs declares {} event codes but EV_MAX is {EV_MAX} — the \
         range 1..=EV_MAX has a gap or a duplicate in it, and the ledger's \
         arithmetic assumes neither.",
        seen.len()
    );

    let mut sorted: Vec<u8> = seen.iter().map(|(_, c)| *c).collect();
    sorted.sort_unstable();
    for (i, code) in sorted.iter().enumerate() {
        assert_eq!(
            *code,
            i as u8 + 1,
            "the event codes are not 1..=EV_MAX with no gaps: {:?}",
            seen
        );
    }
}
