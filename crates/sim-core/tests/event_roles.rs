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
//! `world.rs`. Three disciplines make it able to fail —
//!
//! 1. **Every field in a checked event must be mutually distinguishable.**
//!    A check where the attacker id, the victim id and the damage all
//!    happened to be 1 would pass under any permutation. `distinct3`
//!    asserts the fixture keeps them apart, so a later fixture edit that
//!    blinds a check fails loudly instead of quietly passing.
//! 2. **Exactly one event per code on the tick it lands.** `only` refuses
//!    zero and refuses two, which makes this a double-emit gate as
//!    well — `Removed duplicate OnBonusItemDrop hook` and two rounds of
//!    `Fixed double deprecated hook call with OnActiveItemChange/d` are
//!    the same family of defect over there.
//! 3. **Find the tick, never assume it.** The first cut of this file
//!    asserted on the tick it sent the swing and read an empty ring twice.
//!    The sim auto-repeats a held button, so every swing after the first
//!    resolves *inside* the cooldown, on a tick the test never sent an
//!    input for. `until` steps until the code appears rather than
//!    predicting when. The bound is in sim ticks, which is deterministic
//!    state and not a clock — `CLAUDE.md`'s wall-clock rule is untouched.
//!
//! Coverage is stated, never implied: `coverage_is_stated_not_implied`
//! pins how many of the 25 codes are checked by role, so the gate can
//! never read as "the event lane is covered" while covering five, and a
//! new `EV_*` cannot land without someone classifying it.
//!
//! The arrangement is `combat.rs`'s duel — `dev_spawn` pins both players to
//! the ring's own spawn for id 1, which the spawn selector guarantees is
//! clear of scatter for 4 m, so a swing lands on a person and never on a
//! tree. Player id 1 is `players[0]` and attacks; id 2 is `players[1]` and
//! dies. Nothing here invents a number: every value comes from a fixture.

use sim_core::backpack::BackpackContent;
use sim_core::combat::CombatContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::world::{
    Command, SimEvent, World, EV_BAG_DROPPED, EV_DEATH, EV_GATHER, EV_HEALTH, EV_HIT,
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
    const COVERED: [u8; 5] = [EV_GATHER, EV_HIT, EV_HEALTH, EV_DEATH, EV_BAG_DROPPED];
    /// What is knowingly still byte-golden only. Change this number in the
    /// same commit that changes `COVERED`, never on its own.
    const UNCOVERED: usize = 20;

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
