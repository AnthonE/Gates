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
//! pins how many of the lane's `EV_MAX` codes are checked by role, so the
//! gate can never read as "the event lane is covered" while covering
//! thirteen, and a new `EV_*` cannot land without someone classifying it.
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

use sim_core::backpack::{BackpackContent, BAG_GONE_DESPAWN, BAG_GONE_EMPTIED};
use sim_core::build::{
    foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE,
    REFUSE_B_COST, REFUSE_B_PIECE,
};
use sim_core::combat::{AmmoDef, CombatContent, RangedDef};
use sim_core::craft::{CraftContent, REFUSE_INPUTS, REFUSE_RECIPE};
use sim_core::deploy::{box_key, DeployContent, REFUSE_D_KIND, REFUSE_D_SPOT};
use sim_core::gather::{cell_key, weak_mark8, GatherContent, ItemStack, NO_ITEM};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::inventory::{self, CONT_SELF, REFUSE_M_EMPTY};
use sim_core::limits::TICK_HZ;
use sim_core::loot::LootContent;
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
use sim_core::oven::CookContent;
use sim_core::ranged::SURF_GROUND;
use sim_core::survival::{SurvivalContent, DRINK_REACH_M, REFUSE_C_NOT_FOOD, REFUSE_C_NO_WATER};
use sim_core::terrain;
use sim_core::world::{
    Command, SimEvent, World, DEATH_BY_MAX, EV_AUTH, EV_BAG_DROPPED, EV_BAG_REMOVED,
    EV_BUILD_REFUSED, EV_CHARGE_PLACED, EV_CONSUMED, EV_CONSUME_REFUSED, EV_CRAFT_DONE,
    EV_CRAFT_REFUSED, EV_DEATH, EV_DEPLOY_PLACED, EV_DEPLOY_REFUSED, EV_DEPLOY_REMOVED, EV_DOOR,
    EV_DRANK, EV_GATHER, EV_GATHER_REFUSED, EV_HEALTH, EV_HIT, EV_IMPACT, EV_KNOCK, EV_KNOWN,
    EV_MAX, EV_MOVED, EV_MOVE_REFUSED, EV_OVEN, EV_PIECE_PLACED, EV_PIECE_REMOVED,
    EV_PIECE_REPAIRED, EV_RESEARCH, EV_RESEARCH_REFUSED, EV_RESPAWN, EV_SHOT, EV_SLOT_HARVESTED,
    EV_SLOT_RESPAWNED, EV_STOCK, EV_STRUCT_HIT, EV_VITALS, EV_WEAK_MARK, STRUCT_DEPLOY_BIT,
};
use sim_core::yaw_dir;

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
/// Row 4 is the fire (oven v0). Ground placement, so it stands on the
/// `GROUND` level the storey is built from — and that is what makes the
/// level field of `EV_OVEN.b` worth asserting: it is checked against a
/// level the door's own event does *not* use.
const DEPLOY_FIRE: u16 = 4;
/// Row 5 is the code lock (lock v1), item 7, placement class `door`.
const DEPLOY_LOCK: u16 = 5;

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

/// A table row no baked table has, for the refusal family's first cause.
///
/// Every one of the three refusal codes checked below leads its validation
/// chain with a range test on the row it was handed — `row >= piece_count`,
/// `row >= def_count`, `recipe >= recipe_count` — so one out-of-range value
/// drives the first refusal in all three without depending on terrain, on
/// reach, or on what the fixture happens to hold. It is deliberately far
/// past every fixture's count rather than one past it: a table that grows
/// must not quietly turn this cause into a *successful* placement, which
/// would leave the test asserting on an event that never fired.
const NO_SUCH_ROW: u16 = 9999;

/// Which edges carry which check, and why they differ.
///
/// `EV_PIECE_PLACED` and `EV_DEPLOY_PLACED` both pack `level << 16 | loc
/// << 8 | row`, and a check is blind to any pair of those three being
/// swapped when two of them hold the same number. The doorway *piece* is
/// row 3, so it goes on the low-x edge (`LOC_EDGE_XLO` = 2) to read 0/2/3;
/// the door *deployable* is row 2, so it goes on the low-z edge
/// (`LOC_EDGE_ZLO` = 3) to read 0/3/2. Same discipline as `distinct_halves`,
/// one field wider — and it is why there are two doorways here rather
/// than one.
///
/// With the arrangement standing a storey (`UPPER`) the two triples read
/// 1/2/3 and 1/3/2: three different numbers each, and no longer a level
/// that is only ever its own zero.
const PIECE_EDGE: u8 = LOC_EDGE_XLO;
const DOOR_EDGE: u8 = LOC_EDGE_ZLO;

/// Where a raider stands to swing at the arrangement, and which way it
/// faces. One build cell at −x of the target column, looking back +x —
/// `combat.rs`'s own raid rig in the same posture, because the target scan
/// picks the nearest anchor it is aimed at, and the low-x edge of the cell
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
        cond: 0,
    };
    let (fx, fz) = yaw_dir(YAW);
    let a = w.players[0].body;
    let (ax, az) = (a.qx as f32 * POS_XZ_Q, a.qz as f32 * POS_XZ_Q);
    w.players[1].body = Body::at(SEED, hv(SEED), ax + fx * REACH_M, az + fz * REACH_M);
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
        cond: 0,
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

/// `EV_SHOT: a = shooter id, b = yaw << 8 | pitch, c = speed << 16 | drop`.
///
/// The two packed fields are the exposure here. `b` and `c` are each two
/// numbers in one word, so a check that only asserted "b is nonzero" would
/// survive the halves being swapped inside the word — which is the byte-
/// golden hole this file exists for, one level inside a field. Both are
/// unpacked and asserted by half, with a fixture whose halves cannot be
/// mistaken for each other (a yaw that does not fit in a byte, a pitch that
/// is not the level default, a speed and a drop two orders apart).
///
/// The ballistics come from the **round**, not the bow — `PROJECTILES.md`
/// §9.3 — so arming the bow alone must leave the shot unfired, and the
/// fixture below would fail loudly rather than quietly if `ammo_def` ever
/// started answering for the weapon.
#[test]
fn shot_names_the_shooter_then_the_aim_then_the_ballistics() {
    /// Deliberately past a byte, so a swap of the halves of `b` cannot
    /// produce a value that still looks like a plausible yaw.
    const SHOT_YAW: u16 = 4_097;
    /// Not 128 (the level default), so a zeroed pitch fails the check.
    const SHOT_PITCH: u8 = 200;
    const BOW: u16 = 5;
    const ARROW: u16 = 6;
    const SPEED_MMPT: u16 = 1_333;
    const DROP_MMPT2: u16 = 22;

    let mut w = duel_world();
    w.combat.ranged[BOW as usize] = RangedDef {
        damage: 30,
        ammo: [ARROW, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 60,
        range_mm: 60_000,
    };
    w.combat.ammo[ARROW as usize] = AmmoDef {
        speed_mmpt: SPEED_MMPT,
        drop_mmpt2: DROP_MMPT2,
    };
    w.players[0].inv[0] = ItemStack {
        item: BOW,
        count: 1,
        cond: 0,
    };
    w.players[0].inv[1] = ItemStack {
        item: ARROW,
        count: 5,
        cond: 0,
    };
    w.tick(&[Command::Input {
        id: ATTACKER,
        frame: InputFrame {
            seq: 1,
            buttons: BTN_PRIMARY,
            yaw: SHOT_YAW,
            pitch: SHOT_PITCH,
            move_x: 0,
            move_z: 0,
            sel: 0,
        },
    }]);

    let shot = only(&w, EV_SHOT);
    distinct3(shot, "EV_SHOT");
    assert_eq!(
        shot.a, ATTACKER,
        "EV_SHOT.a is who fired, not who was aimed at"
    );
    assert_eq!(
        shot.b >> 8,
        SHOT_YAW as u32,
        "EV_SHOT.b's high half is yaw — the halves are the wrong way round"
    );
    assert_eq!(
        shot.b & 0xff,
        SHOT_PITCH as u32,
        "EV_SHOT.b's low byte is pitch"
    );
    assert_eq!(
        shot.c >> 16,
        SPEED_MMPT as u32,
        "EV_SHOT.c's high half is speed — a tracer flown at the drop would \
         cross the island in a tick"
    );
    assert_eq!(
        shot.c & 0xffff,
        DROP_MMPT2 as u32,
        "EV_SHOT.c's low half is drop"
    );
}

/// `EV_IMPACT: a = SURF_* << 24 | x, b = z, c = y` — all three in the
/// entity lane's quanta, `c` signed.
///
/// **The sharpest positional payload in the lane, and the reason this
/// file exists.** `a`'s low half and `b` are the same kind of number in
/// the same units — two axes of one point — so a transposition at the
/// `events.push` site produces a mark somewhere else on the island with
/// every other gate green: the encoder is untouched (golden green), the
/// event queue is not in `state_hash` (replay green), and both are `u32`
/// (clippy green). That is `reference/FINDINGS.md` §1's trap exactly, and
/// the only thing that can see it is an assertion that knows which axis
/// is which.
///
/// So the fixture stands the shooter somewhere x and z **cannot be
/// confused**, and asserts it: `distinct_axes` below fails loudly rather
/// than letting the checks pass vacuously if the spawn ever moves to a
/// diagonal.
///
/// Straight down (`pitch` 0) for the surface: an arrow dropped from the
/// eye meets the ground under the shooter's own feet, which is a
/// `SURF_GROUND` this test can predict without re-implementing the stop
/// ladder. The other two kinds are `ranged.rs`'s to decide and the
/// wire-domain table's to bound; what is checked here is the *roles*.
#[test]
fn impact_names_the_surface_then_x_then_z_then_y() {
    /// Straight down. `pitch_dir(0)` is the bottom of the 256-entry table
    /// — planar scale ~0, vertical −1 — so the arrow falls from the eye
    /// rather than flying, and what it meets is the ground below.
    const DOWN: u8 = 0;
    const BOW: u16 = 5;
    const ARROW: u16 = 6;

    let mut w = duel_world();
    w.combat.ranged[BOW as usize] = RangedDef {
        damage: 30,
        ammo: [ARROW, NO_ITEM, NO_ITEM, NO_ITEM],
        rate_ticks: 60,
        range_mm: 60_000,
    };
    w.combat.ammo[ARROW as usize] = AmmoDef {
        speed_mmpt: 1_333,
        drop_mmpt2: 22,
    };
    w.players[0].inv[0] = ItemStack {
        item: BOW,
        count: 1,
        cond: 0,
    };
    w.players[0].inv[1] = ItemStack {
        item: ARROW,
        count: 5,
        cond: 0,
    };
    // The victim would be under the falling arrow otherwise, and a body
    // resolves before the world does — `EV_HIT`, not this.
    w.players[1].dead = true;
    w.players[1].active = false;

    let (want_x, want_z) = (w.players[0].body.qx, w.players[0].body.qz);
    assert_ne!(
        want_x, want_z,
        "the shooter stands on a diagonal, so x and z carry the same \
         number and every axis check below is blind to a swap. Move the \
         fixture, not the assertion."
    );

    w.tick(&[Command::Input {
        id: ATTACKER,
        frame: InputFrame {
            seq: 1,
            buttons: BTN_PRIMARY,
            yaw: YAW,
            pitch: DOWN,
            move_x: 0,
            move_z: 0,
            sel: 0,
        },
    }]);
    until(&mut w, EV_IMPACT);
    let im = only(&w, EV_IMPACT);

    assert_eq!(
        im.a >> 24,
        SURF_GROUND as u32,
        "EV_IMPACT.a's high byte is the surface kind, and an arrow dropped \
         onto open ground met the ground"
    );
    // One quantum of slack an axis: the stop point is the sample the loop
    // broke on, not the shooter's own cell, so it may land a quantum
    // either side. Slack this tight still cannot absorb a swap — the two
    // axes are thousands of quanta apart.
    let (got_x, got_z) = ((im.a & 0x00FF_FFFF) as i32, im.b as i32);
    assert!(
        (got_x - want_x).abs() <= 1,
        "EV_IMPACT.a's low 24 bits are the impact's X ({want_x}), got \
         {got_x} — if this reads as the Z ({want_z}), the axes are \
         transposed at the push site"
    );
    assert!(
        (got_z - want_z).abs() <= 1,
        "EV_IMPACT.b is the impact's Z ({want_z}), got {got_z} — if this \
         reads as the X ({want_x}), the axes are transposed at the push site"
    );

    // `c` signed, which no other field on this lane is: the ground under
    // the shooter may be below datum, and the whole point of carrying the
    // two's-complement pattern is that such a mark still lands there.
    let ground_q = (terrain::height(SEED, want_x as f32 * POS_XZ_Q, want_z as f32 * POS_XZ_Q)
        / POS_Y_Q) as i32;
    let got_y = im.c as i32;
    assert!(
        (got_y - ground_q).abs() <= ARROW_STEP_Q,
        "EV_IMPACT.c is the impact's Y in POS_Y_Q quanta ({ground_q} is \
         the ground here), got {got_y}. A y read off the wrong axis, or \
         read unsigned where the ground is below datum, lands here."
    );
}

/// One tick of a falling arrow, in `POS_Y_Q` quanta — the slack the Y
/// check above allows, stated as the sample spacing rather than guessed.
/// The stop point is the first sample *under* the surface, so it may sit
/// up to one segment below it.
const ARROW_STEP_Q: i32 = 200;

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
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
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
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
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

/// `EV_GATHER_REFUSED: a = player id, b = held item index << 16 |
/// gather::REFUSE_G_* reason` (wire v42).
///
/// Driven through the dead-tool cause rather than the wrong-tool one, and
/// that is discipline 2 at work: `REFUSE_G_TOOL` is 1 and the swinger's id
/// is 1, so the wrong-tool arrangement packs the same number into `a` and
/// `b`'s low half and is blind to that exchange. The broken cause puts
/// (1, 7, 2) in the three seats — all pairwise distinct.
///
/// The arrangement mutates the fixture the way every arrangement here
/// does: the junk item is given a condition ceiling so a zero-condition
/// stack of it reads as DEAD (`gather::swing`'s Q4 guard), and the tree's
/// hand row is zeroed so the fallback pays nothing — the shipped content's
/// own shape since 2026-08-15.
#[test]
fn gather_refused_names_the_player_then_item_over_reason() {
    use sim_core::gather::{REFUSE_G_BROKEN, SWING_INTERVAL_TICKS};

    // A tree with a clear stand point, found the way `tests/gather.rs`
    // finds one: scan the scatter for an isolated one.
    let table = sim_core::terrain::ScatterTable::alpha_default();
    let haven = terrain::haven(SEED);
    let mut found = None;
    'scan: for cz in 40..216i32 {
        for cx in 40..216i32 {
            let s = terrain::scatter(SEED, &table, &haven, cx, cz);
            if s.occupant != sim_core::terrain::Occupant::Tree {
                continue;
            }
            let (px, pz) = (s.x - 1.2, s.z);
            let py = terrain::height(SEED, px, pz);
            if (s.y - py).max(py - s.y) > 1.0 || py < 1.0 {
                continue;
            }
            let pcx = (px / sim_core::terrain::CELL_SIZE) as i32;
            let pcz = (pz / sim_core::terrain::CELL_SIZE) as i32;
            let mut rivals = 0;
            for ddz in -1..=1i32 {
                for ddx in -1..=1i32 {
                    let n = terrain::scatter(SEED, &table, &haven, pcx + ddx, pcz + ddz);
                    let aims = sim_core::gather::node_index(n.occupant).is_some()
                        || n.occupant == sim_core::terrain::Occupant::BarrelSlot;
                    if aims && (n.x != s.x || n.z != s.z) {
                        let d2 = (n.x - px) * (n.x - px) + (n.z - pz) * (n.z - pz);
                        if d2 <= 6.25 {
                            rivals += 1;
                        }
                    }
                }
            }
            if rivals > 0 {
                continue;
            }
            let (dx, dz) = (s.x - px, s.z - pz);
            let mut best_yaw = 0u16;
            let mut best_dot = f32::MIN;
            for hi in 0..=255u16 {
                let (fx, fz) = yaw_dir(hi << 8);
                let dot = fx * dx + fz * dz;
                if dot > best_dot {
                    best_dot = dot;
                    best_yaw = hi << 8;
                }
            }
            found = Some(((px, pz), best_yaw));
            break 'scan;
        }
    }
    let ((px, pz), yaw) = found.expect("the seed offers an isolated tree");

    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    for n in w.gather.nodes.iter_mut() {
        n.weak_pct = 0;
        n.hand_yield = 0;
    }
    // The junk item gains a ceiling so a zero-condition stack of it is a
    // DEAD tool rather than a mere non-tool. 123 is distinct from every
    // other value in the check.
    w.gather.cond_max[JUNK as usize] = 123;
    w.dev_spawn = Some((px, pz));
    w.tick(&[Command::Join { id: BODY }]);
    w.players[0].inv[0] = ItemStack {
        item: JUNK,
        count: 1,
        cond: 0,
    };

    let mut seq = 1u16;
    let mut steps = 0u32;
    loop {
        w.tick(&[Command::Input {
            id: BODY,
            frame: InputFrame {
                seq,
                buttons: BTN_PRIMARY,
                yaw,
                pitch: 0,
                move_x: 0,
                move_z: 0,
                sel: 0,
            },
        }]);
        seq = seq.wrapping_add(1);
        if w.events
            .entries()
            .iter()
            .any(|e| e.code == EV_GATHER_REFUSED)
        {
            break;
        }
        steps += 1;
        assert!(
            steps < SWING_INTERVAL_TICKS as u32 * 4,
            "no EV_GATHER_REFUSED within four swing windows — the cause is \
             broken, not slow"
        );
    }

    let got = only(&w, EV_GATHER_REFUSED);
    distinct3(got, "EV_GATHER_REFUSED");
    distinct_halves(got.b, "EV_GATHER_REFUSED.b (item over reason)");
    assert_eq!(got.a, BODY, "EV_GATHER_REFUSED.a is who swung");
    assert_eq!(
        got.b >> 16,
        JUNK as u32,
        "EV_GATHER_REFUSED.b's HIGH half is the HELD item — the sentence \
         names the torch, never bare hands"
    );
    assert_eq!(
        got.b & 0xffff,
        REFUSE_G_BROKEN,
        "EV_GATHER_REFUSED.b's LOW half is the reason, and a dead tool is \
         REFUSE_G_BROKEN"
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
                w.players[0].body = Body::at(SEED, hv(SEED), x, z);
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
                w.players[0].body = Body::at(SEED, hv(SEED), x, z);
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
        cond: 0,
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
// The refusal family: EV_CRAFT_REFUSED, EV_BUILD_REFUSED,
// EV_DEPLOY_REFUSED.
//
// Three codes, one sentence between them — `a = player id, b = the reason
// ordinal, c = 0` — and together they are the largest emit population in
// the lane by a wide margin: 31 sites for `EV_BUILD_REFUSED`, 18 for
// `EV_DEPLOY_REFUSED`, 6 for `EV_CRAFT_REFUSED`, against 103 pushes in all
// of `sim-core`. Until this section they had no role check of any kind,
// which is the wrong way round: a family this repetitive is exactly where
// a copied-and-edited emit line goes in with its two arguments the wrong
// way round, and every wall stays green when it does.
//
// `EV_CONSUME_REFUSED` above already carries the discipline these three
// need, because `c` is always zero here and `distinct3` therefore cannot
// apply: drive **two different causes** and require `b` to move between
// them. A field pinned against one reason constant would pass just as well
// if the emit site hard-coded it; two causes prove `b` is the reason
// channel and `a` is the player in both.
//
// What is new here is the second half of that discipline, and this lane
// needs it in a way the consume lane did not — see `refused`.
// ---------------------------------------------------------------------

/// One refusal, checked in the seat the sentence gives it.
///
/// The `a != b` guard is not decoration and it is not automatic. This
/// file's fixture builder is player id 4, and `REFUSE_B_REACH` and
/// `REFUSE_D_REACH` are both the ordinal **4** — so "place it out of
/// reach", the most obvious refusal in either table to drive, is precisely
/// the one cause where a swapped `a` and `b` satisfy every assertion below
/// and the check reads green over the bug it exists to find. Discipline 1
/// of this file's header, met by choosing a different cause rather than by
/// weakening the assertion.
///
/// The `c == 0` check is the family's third field stated rather than
/// ignored: `world.rs` gives the refusal codes no role for `c`, and a
/// future emit site that starts smuggling the address into it should have
/// to say so here first.
fn refused(e: SimEvent, who: u32, why: u32, what: &str) {
    assert_ne!(
        who, why,
        "{what}: the reason ordinal and the player id are both {who}, so a \
         swapped `a` and `b` would satisfy every check below. Drive a \
         different cause; do not relax the assertion."
    );
    assert_eq!(e.a, who, "{what}: `a` is who was refused");
    assert_eq!(e.b, why, "{what}: `b` is why");
    assert_eq!(
        e.c, 0,
        "{what}: the refusal family states no role for `c` in world.rs, so \
         it must stay zero — a value here is an undocumented field"
    );
}

/// `EV_CRAFT_REFUSED: a = player id, b = craft::REFUSE_*`.
#[test]
fn craft_refused_names_the_player_then_why() {
    let mut w = lone_world();
    w.craft = CraftContent::probe_fixture();
    // Both causes want an empty hand, and the join must not be trusted to
    // have left one: cause two is "you cannot pay the inputs", which a
    // granted stack would turn into a successful enqueue and a vacuous
    // `only`.
    for s in w.players[0].inv.iter_mut() {
        *s = ItemStack::default();
    }

    // Cause one: a recipe row the table does not have.
    w.tick(&[Command::Craft {
        id: BODY,
        recipe: NO_SUCH_ROW,
        count: 1,
    }]);
    let bad_row = only(&w, EV_CRAFT_REFUSED);
    refused(
        bad_row,
        BODY,
        REFUSE_RECIPE,
        "EV_CRAFT_REFUSED (no such recipe)",
    );

    // Cause two: a real recipe, and nothing in hand to pay it with. The
    // fixture's row 0 is station-free and takes three of item 0, so the
    // chain reaches the input test rather than stopping at a station or a
    // full queue.
    w.tick(&[Command::Craft {
        id: BODY,
        recipe: 0,
        count: 1,
    }]);
    let broke = only(&w, EV_CRAFT_REFUSED);
    refused(
        broke,
        BODY,
        REFUSE_INPUTS,
        "EV_CRAFT_REFUSED (cannot pay the inputs)",
    );

    assert_ne!(
        bad_row.b, broke.b,
        "two different craft refusals reported the same reason code — `b` \
         is not carrying the cause, so pinning it against one constant \
         proves nothing"
    );
    assert!(
        w.players[0].jobs.iter().all(|j| j.remaining == 0),
        "a refused craft queued a job anyway — the event under test is not \
         announcing a refusal at all"
    );
}

/// `EV_BUILD_REFUSED: a = player id, b = build::REFUSE_B_*`.
///
/// Neither cause may be the reach test: `REFUSE_B_REACH` is 4 and so is
/// `BUILDER`. See `refused`.
#[test]
fn build_refused_names_the_player_then_why() {
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);

    // Cause one: a piece row the table does not have.
    w.tick(&[Command::Place {
        id: BUILDER,
        row: NO_SUCH_ROW,
        cx,
        cz,
        level: GROUND,
        loc: LOC_PLANE,
    }]);
    let bad_row = only(&w, EV_BUILD_REFUSED);
    refused(
        bad_row,
        BUILDER,
        REFUSE_B_PIECE,
        "EV_BUILD_REFUSED (no such piece row)",
    );

    // Cause two: the placement `builder_world` was arranged to make legal
    // in every other respect — the cell is one `foundation_terrain_ok`
    // accepted and the body is standing on it — with the payment taken
    // away. `REFUSE_B_COST` is the last test in the chain, so reaching it
    // is also a statement that the seven before it passed.
    for s in w.players[0].inv.iter_mut() {
        *s = ItemStack::default();
    }
    w.tick(&[Command::Place {
        id: BUILDER,
        row: PIECE_FOUNDATION,
        cx,
        cz,
        level: GROUND,
        loc: LOC_PLANE,
    }]);
    let broke = only(&w, EV_BUILD_REFUSED);
    refused(
        broke,
        BUILDER,
        REFUSE_B_COST,
        "EV_BUILD_REFUSED (cannot pay)",
    );

    assert_ne!(
        bad_row.b, broke.b,
        "two different build refusals reported the same reason code — `b` \
         is not carrying the cause, so pinning it against one constant \
         proves nothing"
    );
    assert_eq!(
        w.pieces.len(),
        0,
        "a refused placement put a piece in the world — the event under \
         test is not announcing a refusal at all"
    );
}

/// `EV_DEPLOY_REFUSED: a = player id, b = deploy::REFUSE_D_*`.
///
/// Neither cause may be the reach test here either: `REFUSE_D_REACH` is
/// also 4. See `refused`.
#[test]
fn deploy_refused_names_the_player_then_why() {
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);

    // Cause one: a deployable row the table does not have.
    w.tick(&[Command::PlaceDeploy {
        id: BUILDER,
        row: NO_SUCH_ROW,
        cx,
        cz,
        level: GROUND,
        loc: LOC_PLANE,
    }]);
    let bad_row = only(&w, EV_DEPLOY_REFUSED);
    refused(
        bad_row,
        BUILDER,
        REFUSE_D_KIND,
        "EV_DEPLOY_REFUSED (no such deployable row)",
    );

    // Cause two: a real hearth at an address off the top of the build
    // stack. A level past `MAX_BUILD_LEVELS` is a client-driven value the
    // sim must refuse by event rather than by index, which is the half of
    // wall 4 this lane carries.
    w.tick(&[Command::PlaceDeploy {
        id: BUILDER,
        row: DEPLOY_HEARTH,
        cx,
        cz,
        level: u8::MAX,
        loc: LOC_PLANE,
    }]);
    let no_spot = only(&w, EV_DEPLOY_REFUSED);
    refused(
        no_spot,
        BUILDER,
        REFUSE_D_SPOT,
        "EV_DEPLOY_REFUSED (no such address)",
    );

    assert_ne!(
        bad_row.b, no_spot.b,
        "two different deploy refusals reported the same reason code — `b` \
         is not carrying the cause, so pinning it against one constant \
         proves nothing"
    );
    assert_eq!(
        w.deploys.len(),
        0,
        "a refused deployment put a deployable in the world — the event \
         under test is not announcing a refusal at all"
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
                if foundation_terrain_ok(seed, hv(seed), x, z) {
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
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
    // The fixture's costs: the wood pieces are paid in item 0, the stone
    // ones (the floor this arrangement's second storey stands on) in item
    // 1, the hearth in item 2, the door in item 4, the code lock in item 7.
    // Generous on purpose — a refusal for want of wood would be this
    // fixture's bug, not the sim's. Slot 4 is left free on purpose: the
    // oven test below stocks the fire's own item there.
    for (slot, item) in [(0usize, 0u16), (1, 1), (2, 2), (3, 4), (5, 7)] {
        w.players[0].inv[slot] = ItemStack {
            item,
            count: 200,
            cond: 0,
        };
    }
    (cx, cz)
}

/// The storey the addressed checks read: a foundation on the ground, a wall
/// on its low-x edge, and a floor on top of the wall — so `UPPER` is a real,
/// supported level rather than a number the test asserts about an empty
/// column. The wall is not decoration: `build.rs`'s support rule v0 carries
/// a floor on *an edge piece under one of the cell's four sides*, so a
/// foundation alone refuses the storey with `REFUSE_B_SPOT`'s neighbour,
/// `REFUSE_B_SUPPORT`. Leaves the world standing on the tick the floor
/// landed.
fn stand_a_storey(w: &mut World, cx: u16, cz: u16) {
    place_piece(w, PIECE_FOUNDATION, cx, cz, GROUND, LOC_PLANE);
    place_piece(w, PIECE_WALL, cx, cz, GROUND, LOC_EDGE_XLO);
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

/// Bolt the code lock onto the door at the address. Its own helper
/// because a lock mints **no** deploy record — it is a record about one
/// (`lock.rs`) — so `place_deploy`'s "the store grew by one" assertion is
/// the wrong check for it, and the right one is that the lock store grew.
fn bolt_lock(w: &mut World, cx: u16, cz: u16, level: u8, loc: u8) {
    let before = w.deploys.locks().len();
    w.tick(&[Command::PlaceDeploy {
        id: BUILDER,
        row: DEPLOY_LOCK,
        cx,
        cz,
        level,
        loc,
    }]);
    assert_eq!(
        w.deploys.locks().len(),
        before + 1,
        "the code lock did not bolt onto the door at ({cx}, {cz}) level \
         {level} loc {loc} — the fixture, not the mechanic"
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

/// The raid stance: one cell west of the target column. Split out of
/// [`raid_until`] so a fixture can also PLACE from it — a wall's soft side
/// faces its placer (hard/soft v0), and a payload test that wants full
/// damage landing must build the wall from where it will swing.
fn stand_at_raid_stance(w: &mut World, cx: u16, cz: u16) {
    let (x, z) = (
        (cx as f32 + 0.5 - RAID_OFFSET_CELLS) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    );
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
}

/// Move the builder one cell west of the target column and face it, then
/// swing until `code` lands. The raid posture: the target scan wants an
/// anchor it is aimed at, and a body standing *inside* its own cell is not
/// aiming at that cell's edges.
fn raid_until(w: &mut World, cx: u16, cz: u16, code: u8) {
    stand_at_raid_stance(w, cx, cz);
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

/// `EV_OVEN` — the fire at the address went in or out, and who did it.
///
/// Three fields and all three are forgeable by transposition: the cell key
/// and the actor are both `u32` and both plausible in `a`, and the level
/// and the lit bit share `b`. So the fixture separates every one of them —
/// the fire stands on `GROUND` while the builder is id 4, and the two
/// presses are read one after the other so the lit bit is seen set AND
/// clear at the same address.
#[test]
fn oven_names_the_cell_then_its_state_then_who_lit_it() {
    let mut w = World::new(SEED);
    w.cook = CookContent::probe_fixture();
    let (cx, cz) = builder_world(&mut w);
    // The fixture's fire costs item 0 to place and item 6 to hold; the
    // builder's kit above carries neither, so stock both here rather than
    // widening a fixture four other tests read.
    w.players[0].inv[4] = ItemStack {
        item: 6,
        count: 4,
        cond: 0,
    };
    place_deploy(&mut w, DEPLOY_FIRE, cx, cz, GROUND, LOC_PLANE);

    // Fuel goes in through the container the oven IS: one unit of item 0,
    // which `CookContent::probe_fixture` burns. Without it the press is a
    // refusal, not an announcement — which is itself asserted below.
    let key = box_key(cx, cz, GROUND);
    let i = w.deploys.box_index(key).expect("the fire is a container");
    w.deploys.set_box_slot(
        i,
        0,
        ItemStack {
            item: 0,
            count: 2,
            cond: 0,
        },
    );

    w.tick(&[Command::Use {
        id: BUILDER,
        cx,
        cz,
        level: GROUND,
        loc: LOC_PLANE,
    }]);
    let lit = only(&w, EV_OVEN);
    w.tick(&[Command::Use {
        id: BUILDER,
        cx,
        cz,
        level: GROUND,
        loc: LOC_PLANE,
    }]);
    let out = only(&w, EV_OVEN);

    assert_eq!(
        lit.a,
        cell_key(cx, cz),
        "EV_OVEN.a is the CELL KEY, not the player who struck the match"
    );
    assert_eq!(
        lit.b >> 16,
        GROUND as u32,
        "EV_OVEN.b's high field is LEVEL"
    );
    assert_eq!(
        lit.b & 1,
        1,
        "EV_OVEN.b bit 0 is LIT, and this press lit it"
    );
    assert_eq!(
        lit.c, BUILDER,
        "EV_OVEN.c is the hand that pressed, not the cell"
    );
    assert_ne!(
        lit.a, lit.c,
        "the fixture has the cell key and the actor holding the same value,          so this check cannot see them swapped. Move the fixture, not the          assertion."
    );
    // The same address, the other way: one bit moved and nothing else did,
    // which a swapped or constant field could not reproduce.
    assert_eq!(out.a, lit.a, "the second press is the same fire");
    assert_eq!(out.b & 1, 0, "and it put it out");
    assert_eq!(out.b >> 16, GROUND as u32, "at the same level");
    assert_eq!(out.c, BUILDER, "by the same hand");
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

/// `EV_KNOCK: a = build cell key, b = level << 16 | loc << 8, c = the
/// player who knocked` and `EV_AUTH: a = build cell key, b = level << 16 |
/// loc << 8 | grant, c = the player now remembered`.
///
/// Driven together because one cause produces both in sequence — a hand
/// the lock does not know presses (knock), then enters the code (auth) —
/// and because the pair is exactly the swap a byte-golden cannot see:
/// both carry a cell key in `a`, an address in `b` and a player in `c`,
/// so a crossed `a`/`c` at either emit site would encode green
/// (`reference/FINDINGS.md` §1 is this bug class).
#[test]
fn knock_and_auth_name_the_door_then_the_player() {
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);
    stand_a_storey(&mut w, cx, cz);
    place_piece(&mut w, PIECE_DOORWAY, cx, cz, UPPER, DOOR_EDGE);
    place_deploy(&mut w, DEPLOY_DOOR, cx, cz, UPPER, DOOR_EDGE);
    bolt_lock(&mut w, cx, cz, UPPER, DOOR_EDGE);
    w.tick(&[Command::Access {
        id: BUILDER,
        cx,
        cz,
        level: UPPER,
        loc: DOOR_EDGE,
        op: sim_core::deploy::ACCESS_OP_SET_CODE,
        code: 1234,
    }]);

    // A second body at the same door, which the lock has never heard of.
    const STRANGER: u32 = BUILDER + 1;
    w.tick(&[Command::Join { id: STRANGER }]);
    let slot = (0..8)
        .find(|&i| w.players[i].active && w.players[i].id == STRANGER)
        .expect("the stranger joined");
    w.players[slot].body = w.players[0].body;

    w.tick(&[Command::Use {
        id: STRANGER,
        cx,
        cz,
        level: UPPER,
        loc: DOOR_EDGE,
    }]);
    let k = only(&w, EV_KNOCK);
    distinct_halves(k.a, "EV_KNOCK.a (the cell key)");
    assert_eq!(
        k.a,
        cell_key(cx, cz),
        "EV_KNOCK.a is the CELL KEY of the door, not the knocker"
    );
    let (level, loc, rest) = unpack(k.b);
    assert_eq!(level, UPPER as u32, "EV_KNOCK.b's high field is LEVEL");
    assert_eq!(loc, DOOR_EDGE as u32, "EV_KNOCK.b's middle field is LOC");
    assert_eq!(
        rest, 0,
        "EV_KNOCK carries no state — a knock says somebody is at the door \
         and deliberately not who is allowed through it"
    );
    assert_eq!(k.c, STRANGER, "EV_KNOCK.c is the KNOCKER, not the owner");
    assert_ne!(k.a, k.c, "and the cell key is not the knocker's id");

    w.tick(&[Command::Access {
        id: STRANGER,
        cx,
        cz,
        level: UPPER,
        loc: DOOR_EDGE,
        op: sim_core::deploy::ACCESS_OP_ENTER,
        code: 1234,
    }]);
    let a = only(&w, EV_AUTH);
    assert_eq!(
        a.a,
        cell_key(cx, cz),
        "EV_AUTH.a is the CELL KEY of the lock, not the player it remembers"
    );
    let (level, loc, grant) = unpack(a.b);
    assert_eq!(level, UPPER as u32, "EV_AUTH.b's high field is LEVEL");
    assert_eq!(loc, DOOR_EDGE as u32, "EV_AUTH.b's middle field is LOC");
    assert_eq!(
        grant,
        sim_core::lock::GRANT_FULL as u32,
        "EV_AUTH.b's low field is the GRANT, and the main code grants full"
    );
    assert_eq!(
        a.c, STRANGER,
        "EV_AUTH.c is the player now remembered — the SENDER, since this \
         is an own-fact and nobody learns anybody else's rights"
    );
    assert_ne!(
        a.b, a.c,
        "the address and the player are not the same field"
    );
}

/// `EV_DOOR: a = build cell key, b = level << 16 | loc << 8 | has_lock << 2
/// | locked << 1 | open, c = the player whose action changed it`.
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

    // A door places bare and closed (lock v1), so the three bits all read
    // 0 and a check taken here could not tell any of them apart. Drive
    // them one verb at a time: bolt the lock on and arm it (has_lock and
    // locked, leaf still shut), then toggle (open, the other two held).
    w.tick(&[Command::PlaceDeploy {
        id: BUILDER,
        row: DEPLOY_LOCK,
        cx,
        cz,
        level: UPPER,
        loc: DOOR_EDGE,
    }]);
    let bolted = only(&w, EV_DOOR);
    let (_, _, bolted_state) = unpack(bolted.b);
    assert_eq!(
        bolted_state, 4,
        "bolting a lock on sets has_lock ALONE — the leaf did not move and \
         an unarmed lock is not a locked door; it read {bolted_state}"
    );
    w.tick(&[Command::Access {
        id: BUILDER,
        cx,
        cz,
        level: UPPER,
        loc: DOOR_EDGE,
        op: sim_core::deploy::ACCESS_OP_SET_CODE,
        code: 1234,
    }]);
    let armed = only(&w, EV_DOOR);
    let (_, _, armed_state) = unpack(armed.b);
    assert_eq!(
        armed_state,
        4 | 2,
        "arming it sets locked over has_lock, leaf still shut; it read \
         {armed_state}"
    );
    // Unlock again before the toggle, so the reading below has all three
    // bits at different values (1, 0, 1) and no pair of them can be
    // swapped without this test seeing it.
    w.tick(&[Command::Access {
        id: BUILDER,
        cx,
        cz,
        level: UPPER,
        loc: DOOR_EDGE,
        op: sim_core::deploy::ACCESS_OP_UNLOCK,
        code: 0,
    }]);

    w.tick(&[Command::Use {
        id: BUILDER,
        cx,
        cz,
        level: UPPER,
        loc: DOOR_EDGE,
    }]);
    let d = only(&w, EV_DOOR);
    let (level, loc, state) = unpack(d.b);
    let (has_lock, locked, open) = ((state >> 2) & 1, (state >> 1) & 1, state & 1);

    assert_ne!(
        locked, open,
        "the fixture has locked and open holding the same bit, so this \
         check cannot see them swapped. Move the fixture, not the assertion."
    );
    assert_eq!(
        has_lock, 1,
        "EV_DOOR.b bit 2 is HAS_LOCK, and the lock is still bolted on"
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
        "EV_DOOR.b bit 1 is LOCKED, and the unlock before this cleared it \
         — while bit 2 stayed set, because the lock is still bolted on"
    );
    assert_eq!(
        d.c, BUILDER,
        "EV_DOOR.c is the player whose action changed it, not the cell"
    );

    // The three bits moved independently, one verb each: bolt, arm,
    // toggle. Crossing any two of them at the emit site cannot produce
    // this sequence of three readings.
    assert_eq!(
        state,
        4 | 1,
        "and the toggle should add the open bit alone, leaving {state} = 5"
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
/// The target is a wood wall on the *low-x edge* of the ground storey, which
/// is what makes `b` readable: a foundation would address 0/0/0 and a check
/// against three zeroes is blind to every permutation of them.
#[test]
fn struct_hit_names_the_cell_then_the_address_then_damage_over_hp_left() {
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);
    place_piece(&mut w, PIECE_FOUNDATION, cx, cz, GROUND, LOC_PLANE);
    // Place the wall FROM the raid stance (hard/soft v0): a placement's
    // soft side faces the placer, and this fixture's whole point is the
    // full `STRUCT_DAMAGE` landing — the hard side pays 1 and would make
    // the c-half assertion a test of the side rule instead of the payload.
    stand_at_raid_stance(&mut w, cx, cz);
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

/// `EV_PIECE_REPAIRED: a = build cell key, b = level << 16 | loc << 8 |
/// row, c = healed << 16 | hp now`.
///
/// `EV_STRUCT_HIT` read backwards, and checked against the same wall for
/// exactly that reason: the two events describe one number moving in two
/// directions, and if their `c` halves ever disagreed on which end is
/// which, a client would draw a repaired wall as a damaged one. So this
/// asserts the *shape agreement* as well as the positions — `healed` sits
/// where `damage` sits, `hp now` where `left` does.
///
/// The reversal is the expensive swap here. `healed << 16 | hp` flipped
/// draws a wall dropping to the size of its own repair the moment it is
/// paid for, which reads as a bug in the repair verb rather than in the
/// event, and would be chased in the wrong file.
///
/// The wall is damaged by a real raid rather than a poked store, and the
/// hp before the repair is read out of the store rather than assumed, so
/// however many swings the raid took to land the fixture still knows what
/// the payment was owed to buy back.
#[test]
fn piece_repaired_names_the_cell_then_the_address_then_healed_over_hp() {
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);
    place_piece(&mut w, PIECE_FOUNDATION, cx, cz, GROUND, LOC_PLANE);
    place_piece(&mut w, PIECE_WALL, cx, cz, GROUND, PIECE_EDGE);
    raid_until(&mut w, cx, cz, EV_STRUCT_HIT);

    // Let go of the swing first. Held, the button lands another hit on the
    // same tick the repair is applied, and the fixture would be arguing
    // with itself over what the wall's hp was when it was paid for.
    w.tick(&[Command::Input {
        id: BUILDER,
        frame: InputFrame {
            seq: u16::MAX,
            buttons: 0,
            yaw: RAID_YAW,
            pitch: 128,
            move_x: 0,
            move_z: 0,
            sel: 0,
        },
    }]);
    let hurt = w
        .pieces
        .find(cx, cz, GROUND, PIECE_EDGE)
        .expect("the raided wall still stands")
        .hp as u32;
    assert!(
        hurt > 0 && hurt < WALL_HP,
        "the fixture must hand the repair a wall that is damaged and \
         standing, not one at {hurt} of {WALL_HP}"
    );
    w.tick(&[Command::Repair {
        id: BUILDER,
        deploy: false,
        cx,
        cz,
        level: GROUND,
        loc: PIECE_EDGE,
    }]);

    let r = only(&w, EV_PIECE_REPAIRED);
    distinct3(r, "EV_PIECE_REPAIRED");
    assert_eq!(
        r.b & STRUCT_DEPLOY_BIT,
        0,
        "a PIECE was mended, so STRUCT_DEPLOY_BIT is clear — set it here \
         and the client mends the door hanging at the same address"
    );
    let (level, loc, row) = unpack(r.b & !STRUCT_DEPLOY_BIT);
    distinct_triple(level, loc, row, "EV_PIECE_REPAIRED.b");
    distinct_halves(r.c, "EV_PIECE_REPAIRED.c (healed over hp now)");
    distinct_halves(r.a, "EV_PIECE_REPAIRED.a (the cell key)");

    assert_eq!(
        r.a,
        cell_key(cx, cz),
        "EV_PIECE_REPAIRED.a is the CELL KEY, not the payer — it is \
         broadcast, and the id of whoever paid is not what an onlooker \
         needs to redraw a wall"
    );
    assert_eq!(
        level, GROUND as u32,
        "EV_PIECE_REPAIRED.b's high field is LEVEL"
    );
    assert_eq!(
        loc, PIECE_EDGE as u32,
        "EV_PIECE_REPAIRED.b's middle field is LOC"
    );
    assert_eq!(
        row, PIECE_WALL as u32,
        "EV_PIECE_REPAIRED.b's low field is the piece ROW"
    );
    assert_eq!(
        r.c >> 16,
        WALL_HP - hurt,
        "EV_PIECE_REPAIRED.c's HIGH half is what the payment HEALED, the \
         same seat EV_STRUCT_HIT.c gives the damage dealt"
    );
    assert_eq!(
        r.c & 0xffff,
        WALL_HP,
        "EV_PIECE_REPAIRED.c's LOW half is the hp the piece stands at NOW, \
         the same seat EV_STRUCT_HIT.c gives the hp left — reversed, a \
         repaired wall appears to shrink to the size of its own repair"
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

/// The deployable half of the repair code, and the same bit doing the same
/// job on the way back up.
///
/// This is the pair `struct_hit_on_a_deployable_sets_the_deploy_bit` makes
/// on the way down, and it has to exist separately because the emit site is
/// a different one: `build::repair`'s, not `damage_deploy`'s. A door and its
/// doorway share one address exactly, so a repair that dropped the bit would
/// not mislabel anything — it would tell every client to redraw the *piece*
/// at full hp while the door it hangs in is the thing that was actually paid
/// for, and the two disagree until something else resyncs the address.
///
/// The row is checked against the deploy table on purpose: `DEPLOY_DOOR` and
/// `PIECE_DOORWAY` are indices into different tables, so a payload carrying
/// the right number from the wrong one is exactly the class of bug the
/// positional gates exist for.
#[test]
fn repairing_a_deployable_sets_the_deploy_bit() {
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);
    place_piece(&mut w, PIECE_FOUNDATION, cx, cz, GROUND, LOC_PLANE);
    place_piece(&mut w, PIECE_DOORWAY, cx, cz, GROUND, PIECE_EDGE);
    place_deploy(&mut w, DEPLOY_DOOR, cx, cz, GROUND, PIECE_EDGE);
    raid_until(&mut w, cx, cz, EV_STRUCT_HIT);

    // Let go of the swing, for the piece fixture's reason: held, the next
    // tick lands another hit and the fixture argues with itself about what
    // the door's hp was when it was paid for.
    w.tick(&[Command::Input {
        id: BUILDER,
        frame: InputFrame {
            seq: u16::MAX,
            buttons: 0,
            yaw: RAID_YAW,
            pitch: 128,
            move_x: 0,
            move_z: 0,
            sel: 0,
        },
    }]);
    let hurt = w
        .deploys
        .find(cx, cz, GROUND, PIECE_EDGE)
        .expect("the raided door still stands")
        .hp as u32;
    assert!(
        hurt > 0 && hurt < DOOR_HP,
        "the fixture must hand the repair a door that is damaged and \
         standing, not one at {hurt} of {DOOR_HP}"
    );
    w.tick(&[Command::Repair {
        id: BUILDER,
        deploy: true,
        cx,
        cz,
        level: GROUND,
        loc: PIECE_EDGE,
    }]);

    let r = only(&w, EV_PIECE_REPAIRED);
    distinct3(r, "EV_PIECE_REPAIRED (deployable)");
    distinct_halves(r.c, "EV_PIECE_REPAIRED.c (healed over hp now)");
    let (level, loc, row) = unpack(r.b & !STRUCT_DEPLOY_BIT);

    assert_eq!(
        r.b & STRUCT_DEPLOY_BIT,
        STRUCT_DEPLOY_BIT,
        "the DOOR was mended, not the doorway it hangs in, so \
         STRUCT_DEPLOY_BIT is set — clear it and every client redraws the \
         piece at the same address as though it had been paid for"
    );
    assert_eq!(r.a, cell_key(cx, cz), "EV_PIECE_REPAIRED.a is the CELL KEY");
    assert_eq!(level, GROUND as u32, "the level, under the bit");
    assert_eq!(loc, PIECE_EDGE as u32, "the loc, under the bit");
    assert_eq!(
        row, DEPLOY_DOOR as u32,
        "the low field is the DEPLOY row, read against the deploy table"
    );
    assert_eq!(
        r.c >> 16,
        DOOR_HP - hurt,
        "the HIGH half is what the payment HEALED, the door's missing hp"
    );
    assert_eq!(
        r.c & 0xffff,
        DOOR_HP,
        "and the LOW half the hp it stands at NOW — the door's 60, not the \
         doorway's 100"
    );
    assert_eq!(
        w.deploys
            .find(cx, cz, GROUND, PIECE_EDGE)
            .expect("the door still stands")
            .hp as u32,
        DOOR_HP,
        "and the store agrees with the event it just broadcast"
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
    // From the raid stance, so the swings meet the SOFT side and the wall
    // actually falls inside the step budget (hard/soft v0 — the hard side
    // pays 1 a swing, and this test is about the removal payload).
    stand_at_raid_stance(&mut w, cx, cz);
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
        cond: 0,
    };
    w.players[0].inv[9] = ItemStack::default();

    const FROM_SLOT: u8 = 0;
    const TO_SLOT: u8 = 9;
    const COUNT: u16 = 12;
    assert_ne!(FROM_SLOT, TO_SLOT, "the two slots must be distinguishable");
    w.tick(&[Command::Move {
        id: ATTACKER,
        cont: 0,
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
        cont: 0,
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

/// The probe table's throwable: item 3, `structure` 100 and a 4-tick fuse
/// (`CombatContent::probe_fixture`). Its structure damage is exactly
/// `WALL_HP`, so one charge is one wall and the blast lands as a single
/// event rather than a sum this test would have to carry.
const CHARGE_ITEM: u16 = 3;
const CHARGE_SLOT: u8 = 4;
const CHARGE_FUSE: u32 = 4;

/// `EV_CHARGE_PLACED` says where and how long — and then the fuse actually
/// runs out.
///
/// The second half is not decoration on a role check. Every other event in
/// this file is emitted on the tick its cause was commanded; this one
/// announces something that has *not happened yet*, and the only way to
/// know the announcement was true is to keep ticking until the wall falls.
/// A charge that announced a fuse and then never blew would pass a pure
/// field-role assert and be a dead verb.
#[test]
fn charge_placed_names_the_cell_then_the_address_then_the_fuse() {
    let mut w = World::new(SEED);
    let (cx, cz) = builder_world(&mut w);
    place_piece(&mut w, PIECE_FOUNDATION, cx, cz, GROUND, LOC_PLANE);
    place_piece(&mut w, PIECE_WALL, cx, cz, GROUND, PIECE_EDGE);
    w.players[0].inv[CHARGE_SLOT as usize] = ItemStack {
        item: CHARGE_ITEM,
        count: 3,
        cond: 0,
    };
    // Select the charge on its own tick. Buttons stay at zero throughout:
    // item 3 is also a melee row in this fixture, and a held primary would
    // put a swing on the same wall and leave the test arguing with itself
    // about which cause took the hp.
    w.tick(&[Command::Input {
        id: BUILDER,
        frame: InputFrame {
            seq: 1,
            buttons: 0,
            yaw: RAID_YAW,
            pitch: 128,
            move_x: 0,
            move_z: 0,
            sel: CHARGE_SLOT,
        },
    }]);
    w.tick(&[Command::Throw {
        id: BUILDER,
        deploy: false,
        cx,
        cz,
        level: GROUND,
        loc: PIECE_EDGE,
    }]);

    let r = only(&w, EV_CHARGE_PLACED);
    distinct3(r, "EV_CHARGE_PLACED");
    assert_eq!(
        r.b & STRUCT_DEPLOY_BIT,
        0,
        "a PIECE was charged, so STRUCT_DEPLOY_BIT is clear — set it here \
         and the client sticks the fuse on the door hanging at the same \
         address instead of on the wall that is coming down"
    );
    let (level, loc, row) = unpack(r.b & !STRUCT_DEPLOY_BIT);
    distinct_triple(level, loc, row, "EV_CHARGE_PLACED.b");
    distinct_halves(r.a, "EV_CHARGE_PLACED.a (the cell key)");
    assert_eq!(
        r.a,
        cell_key(cx, cz),
        "EV_CHARGE_PLACED.a is the CELL KEY, not the planter — it is \
         broadcast because the defender needs it more than the raider does"
    );
    assert_eq!(
        level, GROUND as u32,
        "EV_CHARGE_PLACED.b's high field is LEVEL"
    );
    assert_eq!(
        loc, PIECE_EDGE as u32,
        "EV_CHARGE_PLACED.b's middle field is LOC"
    );
    assert_eq!(
        row, PIECE_WALL as u32,
        "EV_CHARGE_PLACED.b's low field is the piece ROW"
    );
    assert_eq!(
        r.c, CHARGE_FUSE,
        "EV_CHARGE_PLACED.c is the FUSE IN TICKS — not the tick it fires \
         on, which a client joining mid-fuse could not subtract from"
    );
    assert_ne!(
        r.c, 0,
        "a zero fuse is refused at bake and at both ends of the wire; one \
         reaching the ring means the field is carrying something else"
    );

    // The charge was paid for out of the hand that planted it.
    assert_eq!(
        w.players[0].inv[CHARGE_SLOT as usize].count, 2,
        "planting spends exactly one charge from the held stack"
    );
    assert_eq!(w.charges.len(), 1, "one charge planted, one charge burning");

    // Now let it burn. The wall must actually fall — this is the assert
    // that makes `balance.toml`'s raid ratio a number a player can spend.
    let standing = |w: &World| w.pieces.find(cx, cz, GROUND, PIECE_EDGE).is_some();
    assert!(standing(&w), "the wall stands while the fuse is burning");
    let mut blast = None;
    for _ in 0..=CHARGE_FUSE + 2 {
        w.tick(&[]);
        if let Some(e) = w.events.entries().iter().find(|e| e.code == EV_STRUCT_HIT) {
            blast = Some(*e);
            break;
        }
    }
    let blast = blast.expect("the fuse ran out and the charge went off");
    assert_eq!(
        blast.c >> 16,
        WALL_HP,
        "the blast deals the THROWABLE's structure column, not a melee row's"
    );
    assert!(
        !standing(&w),
        "a wall at exactly one charge's damage comes down"
    );
    assert!(
        w.charges.is_empty(),
        "a detonated charge leaves the store — a fuse that re-armed would \
         raid the same address forever"
    );
}

// ---------------------------------------------------------------------
// The last five: EV_WEAK_MARK, EV_SLOT_RESPAWNED, EV_CRAFT_DONE,
// EV_BAG_REMOVED, EV_RESPAWN — the codes the ledger carried as a stated
// debt (`NOW.md` §4). None of them needed new machinery, only their causes
// driven to the end: a respawn timer leapt to the way `bag_respawn.rs`
// leaps a cooldown, a death answered twice, a bag watched out of the world
// by two different exits.
// ---------------------------------------------------------------------

/// Where the second weak-spot swing stands: on the mark's own heading,
/// 1 m out from the node — inside the fixture spear's 2 m reach, outside
/// `POINT_BLANK_M2` (point blank has no bearing to judge and never
/// bonuses, so a swing from here is the one that can set the bit).
const WEAK_STAND_M: f32 = 1.0;

/// `until`, facing a chosen yaw. For the one cause where where you stand
/// AND where you look are both the fixture: the weak-spot sector compares
/// the swinger's bearing against the mark's heading, and `until`'s fixed
/// north-facing frame cannot stand in an arbitrary sector.
fn until_facing(w: &mut World, yaw: u16, code: u8) {
    let mut seq = 0u16;
    for _ in 0..MAX_STEPS {
        w.tick(&[Command::Input {
            id: ATTACKER,
            frame: InputFrame {
                seq,
                buttons: BTN_PRIMARY,
                yaw,
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
    panic!("event code {code} never landed in {MAX_STEPS} sim ticks");
}

/// `EV_WEAK_MARK: a = player id, b = cell key, c = weak-hit bit << 8 |
/// next mark heading`.
///
/// The packed `c` is the risk: bit 8 says the hit that just landed stood
/// in the mark's sector, the low byte says where the NEXT mark points.
/// Read on two consecutive hits — the first from point blank, which never
/// bonuses and whose mark is only being announced now, then a second from
/// inside the sector that announcement named — so the bit is seen clear
/// AND set at one node while the heading matches `weak_mark8`'s pure
/// function both times. A reversed pack cannot reproduce the sequence:
/// bit 8 as the heading would put the first mark's whole value in one
/// flag, and the second event's low byte would stop tracking the chase.
#[test]
fn weak_mark_names_the_swinger_then_the_cell_then_bit_over_heading() {
    let mut w = duel_world();
    let (x, z, cx, cz) = scanned_slot(&w, terrain::Occupant::Tree);
    let ck = cell_key(cx, cz);
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
    until(&mut w, EV_WEAK_MARK);
    let first = only(&w, EV_WEAK_MARK);
    assert_eq!(first.a, ATTACKER, "EV_WEAK_MARK.a is the SWINGER");
    assert_eq!(
        first.b, ck,
        "EV_WEAK_MARK.b is the CELL KEY of the node being chased"
    );
    assert_eq!(
        first.c >> 8,
        0,
        "the first hit cannot be a weak hit — the mark it would have had \
         to stand in is only being announced by this event"
    );
    let mark = weak_mark8(SEED, cx, cz, ATTACKER, 1);
    assert_eq!(
        first.c & 0xff,
        mark as u32,
        "EV_WEAK_MARK.c's low byte is the NEXT mark heading, \
         `weak_mark8`'s own value after one landed hit"
    );

    // Stand where that heading points and face back at the node — the
    // sector the sim itself just named — then land the second hit.
    let (mx, mz) = yaw_dir((mark as u16) << 8);
    w.players[0].body = Body::at(SEED, hv(SEED), x + mx * WEAK_STAND_M, z + mz * WEAK_STAND_M);
    let back = (((mark as u16) + 128) & 0xff) << 8;
    until_facing(&mut w, back, EV_WEAK_MARK);
    let second = only(&w, EV_WEAK_MARK);
    distinct3(second, "EV_WEAK_MARK");
    assert_eq!(second.a, ATTACKER, "EV_WEAK_MARK.a is still the swinger");
    assert_eq!(second.b, ck, "EV_WEAK_MARK.b is still the same node");
    assert_eq!(
        second.c >> 8,
        1,
        "EV_WEAK_MARK.c's bit 8 is the WEAK-HIT bit, and this swing stood \
         in the sector the previous event named"
    );
    assert_eq!(
        second.c & 0xff,
        weak_mark8(SEED, cx, cz, ATTACKER, 2) as u32,
        "and the low byte is the heading for hit three — the chase moved on"
    );
}

/// `EV_SLOT_RESPAWNED: a = cell key, b = 0.`
///
/// The one code that needed a timer to elapse: the window is 20–45 min of
/// sim ticks, so the clock is leapt to one tick short of the store's own
/// `respawn_at` — `bag_respawn.rs`'s cooldown leap, the same arithmetic
/// the minutes would have done — and the event must then land on exactly
/// the tick the timer names. The swap this catches is quiet: `a` and `b`
/// reversed reads `(0, cell key)`, and with `b` documented as 0 the zero
/// in `a` would address no cell on any client.
#[test]
fn slot_respawned_names_the_cell_that_stood_back_up() {
    let mut w = duel_world();
    let (x, z, cx, cz) = scanned_slot(&w, terrain::Occupant::Tree);
    assert_ne!(
        cx, cz,
        "the scanned cell packs the same value into both halves of its \
         key, so this check cannot see the key pack reversed"
    );
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
    until(&mut w, EV_SLOT_HARVESTED);
    let due = w
        .slot_lives
        .find(cx, cz)
        .expect("the felled tree holds a life record")
        .respawn_at;
    assert!(due > w.tick, "the harvested slot must carry a future timer");

    w.tick = due - 1;
    until_quiet(&mut w, EV_SLOT_RESPAWNED);
    assert_eq!(
        w.tick,
        due + 1,
        "the release landed on some other tick than the one the timer \
         names — respawn_at is on the wire's side of a doc comment too"
    );
    let ev = only(&w, EV_SLOT_RESPAWNED);
    assert_ne!(
        ev.a, 0,
        "the cell key is zero, so this check cannot see a swap against the \
         documented-zero b. Scan a different slot, not the assertion."
    );
    assert_eq!(
        ev.a,
        cell_key(cx, cz),
        "EV_SLOT_RESPAWNED.a is the CELL KEY of the slot that stood up"
    );
    assert_eq!(ev.b, 0, "EV_SLOT_RESPAWNED.b is documented as 0");
    assert_eq!(ev.c, 0, "and c states no role either");
    assert!(
        w.slot_lives.find(cx, cz).is_none(),
        "the slot is announced standing but its life record remains — the \
         event under test did not ride the release"
    );
}

/// The craft fixture's two-input recipe (row 1): 2 × item 1 + 1 × item 2
/// pay one unit of item 3 over 3 ticks. Row 1 rather than row 0 because
/// row 0 pays **2 × item 2** — the same number in both halves of
/// `EV_CRAFT_DONE.b`, which `distinct_halves` rightly refuses.
const RECIPE_PAYS_ONE: u16 = 1;
/// Row 0, used for the other cause: its output can be denied a slot while
/// its inputs still pay, which is what drives the announced-loss zero.
const RECIPE_OVERFLOWS: u16 = 0;

/// `EV_CRAFT_DONE: a = player id, b = item index << 16 | units actually
/// added (0 = full inventory; the loss is announced, never silent)`.
///
/// Two causes because the low half has two meanings to prove: the units
/// that landed when they fit, and the announced zero when nothing did. A
/// reversed pack survives neither — the first reads `1 << 16 | 3`, the
/// second reads `0 << 16 | 2`, and both are checked whole against the
/// fixture's own row.
#[test]
fn craft_done_names_the_crafter_then_item_over_units() {
    let mut w = lone_world();
    w.craft = CraftContent::probe_fixture();
    let def = w.craft.recipes[RECIPE_PAYS_ONE as usize];
    assert_ne!(
        def.output as u32, def.out_count as u32,
        "the recipe pays its own row number of units, so this check cannot \
         see the pack reversed. Use a different row, not a weaker assertion."
    );
    for s in w.players[0].inv.iter_mut() {
        *s = ItemStack::default();
    }
    w.players[0].inv[0] = ItemStack {
        item: 1,
        count: 2,
        cond: 0,
    };
    w.players[0].inv[1] = ItemStack {
        item: 2,
        count: 1,
        cond: 0,
    };
    w.tick(&[Command::Craft {
        id: BODY,
        recipe: RECIPE_PAYS_ONE,
        count: 1,
    }]);
    assert_eq!(
        count(&w, EV_CRAFT_REFUSED),
        0,
        "the enqueue was refused — the fixture, not the mechanic"
    );
    until_quiet(&mut w, EV_CRAFT_DONE);
    let done = only(&w, EV_CRAFT_DONE);
    distinct3(done, "EV_CRAFT_DONE");
    distinct_halves(done.b, "EV_CRAFT_DONE.b");
    assert_eq!(done.a, BODY, "EV_CRAFT_DONE.a is who crafted");
    assert_eq!(
        done.b >> 16,
        def.output as u32,
        "EV_CRAFT_DONE.b's HIGH half is the ITEM index"
    );
    assert_eq!(
        done.b & 0xffff,
        def.out_count as u32,
        "EV_CRAFT_DONE.b's LOW half is the units actually added"
    );
    assert_eq!(
        w.players[0].inv[0],
        ItemStack {
            item: def.output,
            count: def.out_count,
            cond: 0,
        },
        "and the inventory holds what the event announced"
    );

    // Cause two: the output has nowhere to land. The inputs sit in a
    // stack the batch does not empty, every other slot is full of the
    // output at its own ceiling, and the doc's parenthetical is the law
    // under test: the loss is announced, never silent.
    w.players[0].inv[0] = ItemStack {
        item: 0,
        count: 4,
        cond: 0,
    };
    for s in w.players[0].inv.iter_mut().skip(1) {
        *s = ItemStack {
            item: 2,
            count: 100,
            cond: 0,
        };
    }
    w.tick(&[Command::Craft {
        id: BODY,
        recipe: RECIPE_OVERFLOWS,
        count: 1,
    }]);
    until_quiet(&mut w, EV_CRAFT_DONE);
    let lost = only(&w, EV_CRAFT_DONE);
    assert_eq!(
        lost.a, BODY,
        "EV_CRAFT_DONE.a is who crafted, paid and lost"
    );
    assert_eq!(
        lost.b >> 16,
        w.craft.recipes[RECIPE_OVERFLOWS as usize].output as u32,
        "the HIGH half still names the item that was owed"
    );
    assert_eq!(
        lost.b & 0xffff,
        0,
        "EV_CRAFT_DONE.b's LOW half is 0 — a full inventory's loss is \
         announced, never silent"
    );
    assert!(
        w.players[0]
            .inv
            .iter()
            .skip(1)
            .all(|s| s.item == 2 && s.count == 100),
        "the overflow leaked into a stack that was already at its ceiling"
    );
}

/// `EV_BAG_REMOVED: a = backpack id, b = backpack::BAG_GONE_* (despawn,
/// emptied, evicted)`.
///
/// Two of the three exits, driven on two different bags: the first bag
/// times out (`BAG_GONE_DESPAWN`), the second is emptied by a loot
/// (`BAG_GONE_EMPTIED`). Two bags is not decoration — the first bag's id
/// is 1 and so is `BAG_GONE_EMPTIED`, so emptying bag 1 is the one case a
/// swapped `a` and `b` read green; letting it despawn instead (0 against
/// 1) and emptying bag 2 (2 against 1) keeps every pair apart.
#[test]
fn bag_removed_names_the_bag_then_why() {
    let mut w = duel_world();
    arm_victim_with_junk(&mut w);
    until(&mut w, EV_BAG_DROPPED);
    let first_bag = w.backpacks.next_id() - 1;

    // Cause one: the timer. JUNK is item 7, the fixture ladder's
    // short-lived half, so the despawn fits the quiet-step bound.
    until_quiet(&mut w, EV_BAG_REMOVED);
    let gone = only(&w, EV_BAG_REMOVED);
    assert_ne!(
        gone.a, gone.b,
        "EV_BAG_REMOVED carries the same value twice, so this check cannot \
         see a swap"
    );
    assert_eq!(gone.a, first_bag, "EV_BAG_REMOVED.a is the BAG id");
    assert_eq!(
        gone.b, BAG_GONE_DESPAWN,
        "EV_BAG_REMOVED.b is why, and a timer running out is DESPAWN"
    );
    assert_eq!(gone.c, 0, "EV_BAG_REMOVED states no role for c");
    assert!(
        w.backpacks.find(first_bag).is_none(),
        "the bag is announced gone but still in the store"
    );

    // Cause two: emptied. The victim wakes, is re-armed and dies again,
    // so the loot opens a second bag whose id cannot alias the reason.
    w.tick(&[
        Command::Input {
            id: ATTACKER,
            frame: InputFrame {
                seq: u16::MAX,
                buttons: 0,
                yaw: YAW,
                pitch: 128,
                move_x: 0,
                move_z: 0,
                sel: 0,
            },
        },
        Command::Respawn {
            id: VICTIM,
            on_bag: false,
        },
    ]);
    let (fx, fz) = yaw_dir(YAW);
    let a = w.players[0].body;
    let (ax, az) = (a.qx as f32 * POS_XZ_Q, a.qz as f32 * POS_XZ_Q);
    w.players[1].body = Body::at(SEED, hv(SEED), ax + fx * REACH_M, az + fz * REACH_M);
    arm_victim_with_junk(&mut w);
    until(&mut w, EV_BAG_DROPPED);
    let second_bag = w.backpacks.next_id() - 1;

    w.tick(&[Command::Loot { id: ATTACKER }]);
    let emptied = only(&w, EV_BAG_REMOVED);
    assert_ne!(
        emptied.a, emptied.b,
        "the second bag's id equals the reason ordinal, so this check \
         cannot see a swap. Drive another death; do not relax the assertion."
    );
    assert_eq!(emptied.a, second_bag, "EV_BAG_REMOVED.a is the looted bag");
    assert_eq!(
        emptied.b, BAG_GONE_EMPTIED,
        "EV_BAG_REMOVED.b is why, and a loot that takes everything is \
         EMPTIED"
    );
    assert_eq!(emptied.c, 0, "EV_BAG_REMOVED states no role for c");
    assert!(
        w.backpacks.find(second_bag).is_none(),
        "the emptied bag is announced gone but still standing"
    );
    assert_ne!(
        gone.b, emptied.b,
        "two different exits reported the same reason code — `b` is not \
         carrying the cause, so pinning it against one constant proves \
         nothing"
    );
}

/// Row 3 of `DeployContent::probe_fixture` is the ground-class sleeping
/// bag; placing one consumes one unit of its own item 5.
const DEPLOY_BAG: u16 = 3;
const BAG_PLACE_ITEM: u16 = 5;

/// Empty both meters and run until the clock takes the body —
/// `bag_respawn.rs`'s own `die`, restated here because that file's helper
/// is not importable. Bounded in sim ticks, never milliseconds.
fn starve(w: &mut World) {
    let before = w.players[0].deaths;
    w.players[0].food = 0;
    w.players[0].water = 0;
    for _ in 0..120 * TICK_HZ {
        w.tick(&[]);
        if w.players[0].deaths > before {
            return;
        }
    }
    panic!("the clock never killed the body — the survival fixture changed under this test");
}

/// `EV_RESPAWN: a = player id, b = 1 if the body woke on its own sleeping
/// bag, 0 if the spawn ring answered instead`.
///
/// One death answered each way. The body is id 6, and not for variety:
/// `b` is 1 on the bag path, so a player id of 1 — every other test's
/// favourite — is exactly the id where a swapped `a` and `b` read green
/// on the path most worth checking. Each answer is also held against the
/// position the body actually woke at, so `b` is proven to name the
/// anchor that really answered rather than whatever the emit site claims.
#[test]
fn respawn_names_the_player_then_which_anchor_answered() {
    const SLEEPER: u32 = 6;
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.tick(&[Command::Join { id: SLEEPER }]);
    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    );
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);

    // Cause one: the beach button. No bag exists yet either, so both
    // reasons the ring can answer agree about what `b` must say.
    starve(&mut w);
    w.tick(&[Command::Respawn {
        id: SLEEPER,
        on_bag: false,
    }]);
    let beach = only(&w, EV_RESPAWN);
    assert_eq!(beach.a, SLEEPER, "EV_RESPAWN.a is who woke");
    assert_eq!(
        beach.b, 0,
        "EV_RESPAWN.b is 0 when the spawn ring answered — the beach \
         button never claims a bag"
    );
    assert_eq!(beach.c, 0, "EV_RESPAWN states no role for c");
    let (rx, rz) = w.spawn_pos_n(SLEEPER, 1);
    let ring = Body::at(SEED, hv(SEED), rx, rz);
    assert_eq!(
        (w.players[0].body.qx, w.players[0].body.qz),
        (ring.qx, ring.qz),
        "b said the ring answered, so the body must be standing on the ring"
    );

    // Cause two: a bag of the body's own, placed through the real verb,
    // asked for by the button that wants one.
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
    w.players[0].inv[0] = ItemStack {
        item: BAG_PLACE_ITEM,
        count: 1,
        cond: 0,
    };
    let before = w.deploys.len();
    w.tick(&[Command::PlaceDeploy {
        id: SLEEPER,
        row: DEPLOY_BAG,
        cx,
        cz,
        level: GROUND,
        loc: LOC_PLANE,
    }]);
    assert_eq!(
        w.deploys.len(),
        before + 1,
        "the bag did not place at ({cx}, {cz}) — the fixture, not the mechanic"
    );
    starve(&mut w);
    w.tick(&[Command::Respawn {
        id: SLEEPER,
        on_bag: true,
    }]);
    let bagged = only(&w, EV_RESPAWN);
    assert_ne!(
        bagged.a, bagged.b,
        "the sleeper's id equals the bag answer, so this check cannot see \
         a swap. Join a different id; do not relax the assertion."
    );
    assert_eq!(bagged.a, SLEEPER, "EV_RESPAWN.a is still who woke");
    assert_eq!(
        bagged.b, 1,
        "EV_RESPAWN.b is 1 when the body woke on its OWN bag"
    );
    assert_eq!(bagged.c, 0, "EV_RESPAWN states no role for c");
    let on_bag = Body::at(SEED, hv(SEED), x, z);
    assert_eq!(
        (w.players[0].body.qx, w.players[0].body.qz),
        (on_bag.qx, on_bag.qz),
        "b said the bag answered, so the body must be standing on the bag"
    );
    assert_ne!(
        beach.b, bagged.b,
        "two different anchors reported the same answer — `b` is not \
         carrying which one woke you, so pinning it against one constant \
         proves nothing"
    );
}

/// Is `name` read by an `only(&w, …)` call anywhere in this file?
///
/// `only` is this file's universal idiom for "I drove a real cause and
/// exactly one event of this code landed", so a code that appears in one
/// is a code that was actually put through the world — which is precisely
/// the claim the ledger below makes on its behalf. Nothing else in the
/// file is a reliable witness: a bare mention of `EV_FOO` proves only that
/// someone imported it.
///
/// Matches on the line rather than on a built pattern because wall 3
/// disallows `String` and `format!` in this crate and it is right to bind
/// a test too. The trailing `)` is load-bearing: without it `EV_DEPLOY`
/// would match `only(&w, EV_DEPLOY_PLACED)` and a prefix of any code name
/// would inherit its neighbour's coverage.
///
/// One consequence worth stating, because it is a way to make this gate
/// lie: writing `only(&w, EV_FOO)` inside a *doc comment* counts. Do not.
fn read_by_only(src: &str, name: &str) -> bool {
    for line in src.lines() {
        let Some((_, rest)) = line.split_once("only(&w, ") else {
            continue;
        };
        if let Some(tail) = rest.strip_prefix(name) {
            if tail.starts_with(')') {
                return true;
            }
        }
    }
    false
}

/// Every `pub const EV_*: u8 = <literal>;` in `world.rs`, name and value.
///
/// Parsing a source file in a test is not a habit worth spreading, and it
/// is right twice here: the fact under assertion is *what the constant
/// block contains*, and no amount of importing can see a constant the
/// importer was never told about — nor catch a ledger entry that pairs one
/// code's name with another code's value, which is the same
/// right-value-wrong-seat defect this whole file exists for, committed
/// against the gate instead of against the sim.
fn declared_event_codes() -> Vec<(&'static str, u8)> {
    const SRC: &str = include_str!("../src/world.rs");

    // Borrowed out of `SRC`, never built: wall 3 disallows `String` and
    // `format!` in this crate, and it is right to bind a test too — the
    // names here are `'static` slices of the source and copying them would
    // buy nothing but a wall violation.
    let mut seen: Vec<(&str, u8)> = Vec::new();
    for line in SRC.lines() {
        let line = line.trim();
        if !line.starts_with("pub const EV_") {
            continue;
        }
        let rest = &line["pub const ".len()..];
        let Some((name, value)) = rest.split_once(": u8 = ") else {
            continue;
        };
        // `EV_MAX` is the bound itself, and it is defined as `EV_RESPAWN`
        // rather than a digit — it is not one of the codes being bounded.
        if name == "EV_MAX" {
            continue;
        }
        let value = value.trim_end_matches(';');
        let code: u8 = value.parse().unwrap_or_else(|_| {
            panic!(
                "{name} is declared as `{value}`, which is not a literal \
                 code — this parser reads the constant block in `world.rs` \
                 and a non-literal there makes the ledger's range unknowable"
            )
        });
        seen.push((name, code));
    }
    seen
}

/// Coverage, stated rather than implied — and now earned rather than
/// asserted.
///
/// A gate that checks a third of the codes and says nothing about the rest
/// reads, to the next person, as "the event lane is covered." It is not.
/// This pins the ledger so the number cannot drift silently and a new
/// `EV_*` cannot land without someone deciding whether it needs a role
/// check.
///
/// The ledger used to be a bare list of values, and it could lie in two
/// directions that its own arithmetic could not see. Adding `EV_FOO` to
/// `COVERED` bought the claim of coverage without a test behind it —
/// nothing checked that a listed code had ever been driven — and writing a
/// role check *without* listing it left the gate reporting the lane as
/// less covered than it was, which is the milder fault but the same
/// unearned-arithmetic shape. Both are closed here: every code is named as
/// well as numbered, the name and the number are both checked against
/// `world.rs`'s own declaration, and each side of the ledger is checked
/// against whether an `only(&w, …)` call for it actually exists in this
/// file.
///
/// Every code in the lane now carries a role check; `NOT_COVERED` is
/// empty and stays as the seat the next `EV_*` must be classified into.
/// The stronger form — a payload-role table both the emit site and the
/// check read, so a swap is a compile error (`reference/FINDINGS.md` §1)
/// — is still open, and it is a different shape of work than this ledger.
#[test]
fn coverage_is_stated_not_implied() {
    /// Driven through a real cause and asserted field by field above.
    const COVERED: [(&str, u8); 38] = [
        ("EV_GATHER", EV_GATHER),
        ("EV_GATHER_REFUSED", EV_GATHER_REFUSED),
        ("EV_SLOT_HARVESTED", EV_SLOT_HARVESTED),
        ("EV_CRAFT_REFUSED", EV_CRAFT_REFUSED),
        ("EV_PIECE_PLACED", EV_PIECE_PLACED),
        ("EV_BUILD_REFUSED", EV_BUILD_REFUSED),
        ("EV_DEPLOY_PLACED", EV_DEPLOY_PLACED),
        ("EV_DEPLOY_REFUSED", EV_DEPLOY_REFUSED),
        ("EV_PIECE_REMOVED", EV_PIECE_REMOVED),
        ("EV_DEPLOY_REMOVED", EV_DEPLOY_REMOVED),
        ("EV_STOCK", EV_STOCK),
        ("EV_DOOR", EV_DOOR),
        ("EV_HIT", EV_HIT),
        ("EV_HEALTH", EV_HEALTH),
        ("EV_DEATH", EV_DEATH),
        ("EV_BAG_DROPPED", EV_BAG_DROPPED),
        ("EV_STRUCT_HIT", EV_STRUCT_HIT),
        ("EV_VITALS", EV_VITALS),
        ("EV_CONSUMED", EV_CONSUMED),
        ("EV_CONSUME_REFUSED", EV_CONSUME_REFUSED),
        ("EV_DRANK", EV_DRANK),
        ("EV_MOVED", EV_MOVED),
        ("EV_MOVE_REFUSED", EV_MOVE_REFUSED),
        ("EV_PIECE_REPAIRED", EV_PIECE_REPAIRED),
        ("EV_CHARGE_PLACED", EV_CHARGE_PLACED),
        ("EV_OVEN", EV_OVEN),
        ("EV_KNOCK", EV_KNOCK),
        ("EV_AUTH", EV_AUTH),
        ("EV_SLOT_RESPAWNED", EV_SLOT_RESPAWNED),
        ("EV_WEAK_MARK", EV_WEAK_MARK),
        ("EV_CRAFT_DONE", EV_CRAFT_DONE),
        ("EV_BAG_REMOVED", EV_BAG_REMOVED),
        ("EV_RESPAWN", EV_RESPAWN),
        ("EV_RESEARCH", EV_RESEARCH),
        ("EV_RESEARCH_REFUSED", EV_RESEARCH_REFUSED),
        ("EV_SHOT", EV_SHOT),
        ("EV_KNOWN", EV_KNOWN),
        ("EV_IMPACT", EV_IMPACT),
    ];
    /// What is knowingly still byte-golden only: nothing, since the last
    /// five landed. The seat stays — named, not just counted — so the next
    /// `EV_*` has somewhere to be classified while its check is written,
    /// and the arithmetic below still refuses a code that lands in neither
    /// list.
    const NOT_COVERED: [(&str, u8); 0] = [];
    /// Change this number in the same commit that changes `NOT_COVERED`,
    /// never on its own.
    const UNCOVERED: usize = 0;

    const SELF_SRC: &str = include_str!("event_roles.rs");

    assert_eq!(
        NOT_COVERED.len(),
        UNCOVERED,
        "the uncovered ledger lists {} codes but claims {UNCOVERED}",
        NOT_COVERED.len()
    );
    assert_eq!(
        COVERED.len() + UNCOVERED,
        EV_MAX as usize,
        "the ledger does not add up to the code range"
    );

    // Every code in 1..=EV_MAX is classified exactly once.
    for code in 1..=EV_MAX {
        let in_covered = COVERED.iter().filter(|(_, c)| *c == code).count();
        let in_open = NOT_COVERED.iter().filter(|(_, c)| *c == code).count();
        assert_eq!(
            in_covered + in_open,
            1,
            "event code {code} is classified {} times by the ledger — a new \
             EV_* landed unclassified, or one is listed on both sides",
            in_covered + in_open
        );
    }

    // The names and the numbers both come from `world.rs`, so a ledger
    // entry cannot pair one code's name with another's value.
    let declared = declared_event_codes();
    for (name, code) in COVERED.iter().chain(NOT_COVERED.iter()) {
        let found = declared.iter().find(|(n, _)| n == name);
        let Some((_, real)) = found else {
            panic!(
                "the ledger names {name}, which world.rs does not declare — \
                 a code was renamed or removed without the ledger moving"
            )
        };
        assert_eq!(
            real, code,
            "the ledger pairs {name} with {code}, but world.rs declares it \
             as {real} — the ledger has the right value in the wrong seat, \
             which is the defect this whole file exists to catch"
        );
    }

    // The claim of coverage is earned by a real `only(&w, …)` call.
    for (name, _) in COVERED.iter() {
        assert!(
            read_by_only(SELF_SRC, name),
            "the ledger claims {name} is covered, but no `only(&w, {name})` \
             call exists in this file — nothing ever drove that code through \
             a world, so the claim is arithmetic and not a check"
        );
    }

    // And the admission of no coverage is checked the same way, so a role
    // check cannot land without the ledger moving with it.
    for (name, _) in NOT_COVERED.iter() {
        assert!(
            !read_by_only(SELF_SRC, name),
            "{name} is read by an `only(&w, …)` call, so it has a role check \
             — move it to COVERED and drop UNCOVERED by one in the same \
             commit"
        );
    }
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
    // Shared with `coverage_is_stated_not_implied`, which needs the same
    // declarations to check the ledger's names as well as its values.
    let seen = declared_event_codes();

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

/// The same ledger discipline, applied to a **value domain** rather than a
/// code space — and it is the half the byte-golden provably cannot cover.
///
/// `every_event_code_is_in_range` above protects *which events exist*. This
/// protects *which values a live event's field may carry*, and the two fail
/// differently. A new `EV_*` at least changes a subtype the decoder has
/// never seen. A new `DEATH_BY_*` changes nothing the wire can see at all:
/// `EV_DEATH`'s layout is untouched, so `test_protocol_golden` is green;
/// the event ring is not in `state_hash`, so `test_replay` is green; every
/// cause is a `u8`, so clippy is green. The only witness is a runtime
/// `Err(Range)` inside the encoder, on the one path nobody runs twice — a
/// death.
///
/// That gap shipped. On 2026-08-05 a branch added `DEATH_BY_ARROW = 3`
/// against a wire whose `DEATH_CAUSE_MAX` was 2; every arrow kill failed to
/// encode, the server counted the range error and sent nothing, and the
/// victim's client never learned it had died — a corpse parked behind a
/// death screen that never opened. The judge reproduced it as
/// `DEATH_BY_HAND -> Ok(14 bytes)` / `DEATH_BY_ARROW -> Err(Range)` and
/// failed the pass on it. `DEATH_CAUSE_MAX` is now *derived* from
/// `DEATH_BY_MAX` rather than restated, which makes the two ends unable to
/// disagree — but a derived bound is only as honest as the constant it is
/// derived from, and nothing yet stopped `DEATH_BY_MAX` from being left
/// behind by a fourth cause. That is what this parses the file to check.
///
/// Parsing source in a test is the same deliberate choice `SRC` is used for
/// above, and for the same reason stated there: the fact under assertion is
/// *what the constant block contains*, and no amount of importing can see a
/// constant the importer was never told about. `use sim_core::world::*` in
/// a hundred tests would not notice `DEATH_BY_ARROW`; reading the block
/// does.
#[test]
fn death_causes_are_a_closed_ledger() {
    const SRC: &str = include_str!("../src/world.rs");

    // Borrowed out of `SRC`, never built — wall 3's `String`/`format!` ban
    // binds a test too, and the names here are `'static` slices already.
    let mut seen: Vec<(&str, u8)> = Vec::new();
    for line in SRC.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("pub const DEATH_BY_") else {
            continue;
        };
        let Some((name, value)) = rest.split_once(": u8 = ") else {
            continue;
        };
        // `DEATH_BY_MAX` is the bound itself, written as `DEATH_BY_SALT`
        // rather than a digit — it names the ledger, it is not in it.
        if name == "MAX" {
            continue;
        }
        let value = value.trim_end_matches(';');
        let cause: u8 = value.parse().unwrap_or_else(|_| {
            panic!(
                "DEATH_BY_{name} is declared as `{value}`, which is not a \
                 literal cause — this parser reads the constant block in \
                 `world.rs`, and a non-literal there makes the domain's \
                 range unknowable to the wire that has to bound it"
            )
        });
        seen.push((name, cause));
    }

    // The `>= 3` guard is the parser's own liveness. If the block's shape
    // changes — a doc comment reflowed onto the declaration, a type that
    // stops being `u8` — every `strip_prefix` above misses and this test
    // passes while reading nothing at all. A gate that silently stops
    // looking is worse than one that fails.
    assert!(
        seen.len() >= 3,
        "only {} death causes parsed out of world.rs — the constant block's \
         shape changed and this gate is now reading nothing, which is worse \
         than failing",
        seen.len()
    );

    let highest = seen.iter().map(|(_, c)| *c).max().unwrap();
    assert_eq!(
        highest, DEATH_BY_MAX,
        "world.rs declares a death cause {highest} but DEATH_BY_MAX is \
         {DEATH_BY_MAX}. protocol derives DEATH_CAUSE_MAX from DEATH_BY_MAX, \
         so this gap is not cosmetic: `encode_event_death` refuses cause \
         {highest} with Err(Range), the server drops the packet, and a body \
         killed that way never learns it died. Move DEATH_BY_MAX in the same \
         commit as the new cause."
    );
    assert_eq!(
        seen.len(),
        DEATH_BY_MAX as usize + 1,
        "world.rs declares {} death causes but DEATH_BY_MAX is \
         {DEATH_BY_MAX} — the causes are 0..=DEATH_BY_MAX with no gaps and \
         no duplicates, and the wire's range check assumes exactly that.",
        seen.len()
    );

    let mut sorted: Vec<u8> = seen.iter().map(|(_, c)| *c).collect();
    sorted.sort_unstable();
    for (i, cause) in sorted.iter().enumerate() {
        assert_eq!(
            *cause, i as u8,
            "the death causes are not 0..=DEATH_BY_MAX with no gaps: {:?}",
            seen
        );
    }
}

// ---------------------------------------------------------------------------
// Research (research v0) — `EV_RESEARCH` and `EV_RESEARCH_REFUSED`.
//
// Both are own-facts keyed on `a = the player`, which is what lets the
// server route them with `client_slot_of`. `EV_RESEARCH` then carries two
// small integers in `b` and `c` — the recipe and the coin burned — which is
// exactly the positional payload a byte-golden cannot see swapped, since
// each fits the other's field. The fixture's recipe is 2 and its cost is 5
// so the swap is visible.

/// A world with a placed research table (fixture row 7) and a player
/// holding the sample (item 4) and the coin (item 3).
fn table_world(w: &mut World) {
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.craft = sim_core::craft::CraftContent::probe_fixture();
    w.research = sim_core::research::ResearchContent::probe_fixture();
    w.tick(&[Command::Join { id: BUILDER }]);
    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    );
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
    w.players[0].inv[0] = ItemStack {
        item: 10,
        count: 1,
        cond: 0,
    };
    w.tick(&[Command::PlaceDeploy {
        id: BUILDER,
        row: 7,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(
        w.deploys.len(),
        1,
        "the table has to stand or nothing fires"
    );
    w.players[0].inv[0] = ItemStack {
        item: 4,
        count: 1,
        cond: 0,
    };
    w.players[0].inv[1] = ItemStack {
        item: 3,
        count: 20,
        cond: 0,
    };
}

/// `EV_RESEARCH: a = player, b = recipe, c = coin burned`.
#[test]
fn research_names_the_player_then_the_recipe_then_the_price() {
    let mut w = World::new(SEED);
    table_world(&mut w);
    w.tick(&[Command::Research {
        id: BUILDER,
        slot: 0,
    }]);
    let ev = only(&w, EV_RESEARCH);
    assert_eq!(ev.a, BUILDER, "EV_RESEARCH.a is the LEARNER");
    assert_eq!(ev.b, 2, "EV_RESEARCH.b is the RECIPE, not the cost");
    assert_eq!(ev.c, 5, "EV_RESEARCH.c is the COST, not the recipe");
    assert_ne!(
        ev.b, ev.c,
        "the fixture's two fields must differ or this check proves nothing"
    );
}

/// `EV_RESEARCH_REFUSED: a = player, b = reason`.
#[test]
fn research_refused_names_the_player_then_why() {
    let mut w = World::new(SEED);
    table_world(&mut w);
    // An empty slot: a refusal that needs nothing else arranged.
    w.tick(&[Command::Research {
        id: BUILDER,
        slot: 9,
    }]);
    let ev = only(&w, EV_RESEARCH_REFUSED);
    assert_eq!(ev.a, BUILDER, "EV_RESEARCH_REFUSED.a is the ASKER");
    assert_eq!(
        ev.b,
        sim_core::research::REFUSE_R_SLOT,
        "EV_RESEARCH_REFUSED.b is the REASON"
    );
    assert_ne!(
        ev.a, ev.b,
        "and they differ, so a swap shows here rather than passing"
    );
}

/// `EV_KNOWN: a = the player who holds it, b = the mask's low 32 bits,
/// c = its high 32 bits`.
///
/// A `u64` through two `u32` fields, which is this file's packed-field
/// exposure at its widest: a check made with a small mask is blind to `b`
/// and `c` being swapped, because the high half would be 0 either way and
/// zero survives any permutation with itself. The fixture sets bit 3 and
/// bit 40, so the halves are 8 and 256 — different from each other and
/// from the player id, which is what makes the three assertions below able
/// to fail.
///
/// **The cause is a real death, not a hand-set flag**, and that is the
/// second thing this checks. `wake` rebuilds the record from
/// `Player::default()` and names what a body carries through; until
/// 2026-08-15 `known` was not on that list, so every death deleted every
/// blueprint the player had bought with OBOL. The clock kills the body
/// here — `starve` is the same real cause `respawn_names_the_player…`
/// uses — and the mask has to come back out the other side intact.
#[test]
fn known_names_the_holder_then_the_mask_low_half_first() {
    const HOLDER: u32 = 6;
    // Bit 3 and bit 40: `KNOWN_MASK_BITS` is 64, so the high half is
    // reachable, and these two put 8 in `b` and 256 in `c`.
    const WIDE: u64 = 1 << 3 | 1 << 40;

    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.tick(&[Command::Join { id: HOLDER }]);
    // The join stated an empty mask, which is a fact and is checked in
    // `research.rs`. Set the blueprints the body is about to die holding.
    w.players[0].known = WIDE;

    starve(&mut w);
    w.tick(&[Command::Respawn {
        id: HOLDER,
        on_bag: false,
    }]);

    let ev = only(&w, EV_KNOWN);
    distinct3(ev, "EV_KNOWN");
    assert_eq!(ev.a, HOLDER, "EV_KNOWN.a is who holds the blueprints");
    assert_eq!(
        ev.b, WIDE as u32,
        "EV_KNOWN.b is the LOW 32 bits — `encode_event_known` reassembles \
         `b | c << 32`, so the halves reversed here would hand the client \
         a mask naming recipes nobody bought"
    );
    assert_eq!(ev.c, (WIDE >> 32) as u32, "EV_KNOWN.c is the high 32 bits");
    assert_eq!(
        ev.b as u64 | (ev.c as u64) << 32,
        WIDE,
        "the two halves do not reassemble into the mask the body held"
    );

    // And the sim agrees with what it just announced: a death that dropped
    // the mask but announced the old one would satisfy every check above.
    assert_eq!(
        w.players[0].known, WIDE,
        "a real death erased blueprints bought with OBOL — `wake` is not \
         carrying `known` across"
    );
}
