//! The survival clock, at the level the module cannot see: what a death by
//! hunger or thirst does to the *world* — the death count, the beach the
//! body wakes up on, and the fresh pair it wakes up with.
//!
//! `crates/sim-core/src/survival.rs`'s own unit tests own the arithmetic
//! (the span is exact, the meters stack, the ramp arrives). This file owns
//! the consequence, because the consequence lives in `World::respawn` and
//! in `spawn_pos_n`, neither of which `survival::step` can reach.
//!
//! Nothing here invents a number: every rate comes from
//! `SurvivalContent::probe_fixture`, whose spans are seconds precisely so a
//! whole death fits inside a test.

use sim_core::combat::CombatContent;
use sim_core::limits::TICK_HZ;
use sim_core::survival::SurvivalContent;
use sim_core::world::{Command, World};

const SEED: u64 = 20260803;

/// One player on the ring's own spawn, with the clock armed and hp granted
/// from the combat fixture — an inert `CombatContent` grants zero hp, and a
/// body that starts at zero can never *reach* zero, so the clock would have
/// nothing to kill.
fn lone_world() -> World {
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.tick(&[Command::Join { id: 1 }]);
    w
}

/// Run until the clock kills the only body *again* — the count is read
/// against where it started, so a second call cannot be satisfied by the
/// first call's death — then answer the death screen with the beach, which
/// is what a clock death used to do by itself before wire v16 put the
/// choice in the player's hands (`bag_respawn.rs` owns the choice; this
/// file is about the clock). Returns the tick the death landed on.
fn run_until_death(w: &mut World, ticks: u32) -> Option<u32> {
    let before = w.players[0].deaths;
    for t in 0..ticks {
        w.tick(&[]);
        if w.players[0].deaths > before {
            assert!(w.players[0].dead, "a clock death did not raise the screen");
            w.tick(&[Command::Respawn {
                id: 1,
                on_bag: false,
            }]);
            return Some(t);
        }
    }
    None
}

/// A death by the clock is a death: it counts, exactly like a death by
/// somebody's hatchet. Until this landed, `deaths` moved only on combat's
/// kill path, so a starved body respawned at generation zero of the spawn
/// ring — the identical beach — to starve on the same ground.
#[test]
fn a_clock_death_counts_and_moves_the_beach() {
    let mut w = lone_world();
    let before = (w.players[0].body.qx, w.players[0].body.qz);
    assert_eq!(w.players[0].deaths, 0, "a fresh body has died zero times");

    let died_at = run_until_death(&mut w, 60 * TICK_HZ);
    assert!(
        died_at.is_some(),
        "an untended body did not die inside a minute at fixture rates"
    );
    assert_eq!(w.players[0].deaths, 1, "the clock's death counted once");

    // The consequence the count exists for: generation 1 of the ring is a
    // different beach, and it is the one the body is actually standing on.
    let after = (w.players[0].body.qx, w.players[0].body.qz);
    assert_ne!(
        after, before,
        "the starved body respawned on the beach it starved on"
    );
    let (x1, z1) = w.spawn_pos_n(1, 1);
    let expect = sim_core::movement::Body::at(SEED, x1, z1);
    assert_eq!(
        after,
        (expect.qx, expect.qz),
        "the respawn did not walk the spawn ring forward by one generation"
    );
}

/// Two deaths are two generations, not the same beach twice. This is the
/// assertion that would still fire if the count were incremented somewhere
/// that resets it — the ring is read with `deaths`, so a stuck count is a
/// stuck spawn.
#[test]
fn a_second_clock_death_walks_the_ring_again() {
    let mut w = lone_world();
    assert!(
        run_until_death(&mut w, 60 * TICK_HZ).is_some(),
        "first death"
    );
    let first = (w.players[0].body.qx, w.players[0].body.qz);
    // The respawn granted a fresh pair, so the second death takes another
    // full span — the same window is enough.
    assert!(
        run_until_death(&mut w, 60 * TICK_HZ).is_some(),
        "the respawned body never died again — the grant did not re-arm"
    );
    assert_eq!(w.players[0].deaths, 2);
    let second = (w.players[0].body.qx, w.players[0].body.qz);
    assert_ne!(second, first, "two deaths, one beach");
}

/// The respawn hands back a full pair. Without it the body wakes up at zero
/// on both meters and dies again inside a few ticks, which is a spawn loop
/// rather than a game.
#[test]
fn the_respawned_body_is_fed() {
    let mut w = lone_world();
    assert!(run_until_death(&mut w, 60 * TICK_HZ).is_some(), "died once");
    let sc = SurvivalContent::probe_fixture();
    assert_eq!(
        (w.players[0].food, w.players[0].water),
        (sc.max_food, sc.max_water),
        "a respawn must grant a full pair"
    );
    assert!(w.players[0].hp > 0, "and a body with hp in it");
}

// ---------------------------------------------------------------------------
// The drink verb (wire v15). Thirst's real answer, and the first verb in
// this module that reads the world rather than the inventory — so this is
// where it has to be tested: `survival::drink` can be handed a body, but
// only `World` can put that body on a shoreline, walk its spawn ring when
// the salt kills it, and grant it a fresh pair on the other side.
// ---------------------------------------------------------------------------

/// A standable point with water inside `DRINK_REACH_M`, found by scanning
/// the heightfield rather than hard-coded: a hard-coded coast is a number
/// that goes stale the first time the generator's constants move, and this
/// costs nothing because `terrain::height` is the same pure function the
/// verb itself asks.
///
/// The scan is deliberately for a *shore* — land above the waterline with
/// sea within reach — and not simply for water, because standing on the
/// sea floor is not the case the verb is for.
fn shoreline(seed: u64) -> (f32, f32) {
    let r = sim_core::survival::DRINK_REACH_M;
    let mut x = 0.0f32;
    while x < sim_core::terrain::ISLAND_SIZE {
        let mut z = 0.0f32;
        while z < sim_core::terrain::ISLAND_SIZE {
            let h = sim_core::terrain::height(seed, x, z);
            if (sim_core::terrain::SEA_LEVEL..sim_core::terrain::BEACH_MAX_H).contains(&h)
                && (sim_core::terrain::height(seed, x + r, z) < sim_core::terrain::SEA_LEVEL
                    || sim_core::terrain::height(seed, x - r, z) < sim_core::terrain::SEA_LEVEL
                    || sim_core::terrain::height(seed, x, z + r) < sim_core::terrain::SEA_LEVEL
                    || sim_core::terrain::height(seed, x, z - r) < sim_core::terrain::SEA_LEVEL)
            {
                return (x, z);
            }
            z += 4.0;
        }
        x += 4.0;
    }
    panic!("this island has no coast — the generator changed under this test");
}

/// A point with no water inside reach in any direction: the same scan, the
/// other verdict. Inland, so the dry refusal is tested against real ground
/// rather than against a disarmed table.
fn inland(seed: u64) -> (f32, f32) {
    let r = sim_core::survival::DRINK_REACH_M;
    let mut x = 0.0f32;
    while x < sim_core::terrain::ISLAND_SIZE {
        let mut z = 0.0f32;
        while z < sim_core::terrain::ISLAND_SIZE {
            let dry = [(0.0, 0.0), (r, 0.0), (-r, 0.0), (0.0, r), (0.0, -r)]
                .into_iter()
                .all(|(dx, dz)| {
                    sim_core::terrain::height(seed, x + dx, z + dz)
                        >= sim_core::terrain::BEACH_MAX_H
                });
            if dry {
                return (x, z);
            }
            z += 4.0;
        }
        x += 4.0;
    }
    panic!("this island is all coast — the generator changed under this test");
}

fn stand(w: &mut World, x: f32, z: f32) {
    w.players[0].body = sim_core::movement::Body::at(w.seed, x, z);
}

fn count(w: &World, code: u8) -> usize {
    w.events.entries().iter().filter(|e| e.code == code).count()
}

/// The verb's whole point: a player standing at the sea can drink from it,
/// and the meter moves by exactly what the content says.
#[test]
fn a_body_at_the_shore_can_drink() {
    let mut w = lone_world();
    let (x, z) = shoreline(SEED);
    stand(&mut w, x, z);
    let sc = SurvivalContent::probe_fixture();
    // Drain the meter first: a full one refuses, which is its own test.
    w.players[0].water = 10;
    let hp_before = w.players[0].hp;
    w.tick(&[Command::Drink { id: 1 }]);
    assert_eq!(
        count(&w, sim_core::world::EV_DRANK),
        1,
        "a drink at the shore must announce itself"
    );
    assert!(
        w.players[0].water > 10,
        "the meter did not move: {} of {}",
        w.players[0].water,
        sc.max_water
    );
    assert_eq!(
        hp_before - w.players[0].hp,
        sc.drink_hp_cost,
        "the sea is salt — the drink must cost exactly what the content says"
    );
}

/// Dry ground refuses, and says why. A press that vanishes silently is
/// indistinguishable from a broken key, which is the posture every other
/// refusal in this sim keeps.
#[test]
fn a_body_inland_is_refused_and_told_why() {
    let mut w = lone_world();
    let (x, z) = inland(SEED);
    stand(&mut w, x, z);
    w.players[0].water = 10;
    let hp_before = w.players[0].hp;
    let water_before = w.players[0].water;
    w.tick(&[Command::Drink { id: 1 }]);
    assert_eq!(
        count(&w, sim_core::world::EV_DRANK),
        0,
        "there is no water here to drink"
    );
    let refused = w
        .events
        .entries()
        .iter()
        .filter(|e| {
            e.code == sim_core::world::EV_CONSUME_REFUSED
                && e.b == sim_core::survival::REFUSE_C_NO_WATER
        })
        .count();
    assert_eq!(refused, 1, "a dry press must be announced as a dry press");
    assert_eq!(w.players[0].hp, hp_before, "a refusal costs nothing");
    assert_eq!(w.players[0].water, water_before, "and gives nothing");
}

/// A full meter refuses rather than charging hp for nothing — the eat
/// verb's `REFUSE_C_FULL` case, on the other verb.
#[test]
fn a_full_meter_refuses_rather_than_paying_hp() {
    let mut w = lone_world();
    let (x, z) = shoreline(SEED);
    stand(&mut w, x, z);
    let sc = SurvivalContent::probe_fixture();
    w.players[0].water = sc.max_water;
    let hp_before = w.players[0].hp;
    w.tick(&[Command::Drink { id: 1 }]);
    assert_eq!(count(&w, sim_core::world::EV_DRANK), 0, "nothing was drunk");
    assert_eq!(
        w.players[0].hp, hp_before,
        "a refused drink must not take the hp its cost would have"
    );
    let refused = w
        .events
        .entries()
        .iter()
        .filter(|e| {
            e.code == sim_core::world::EV_CONSUME_REFUSED
                && e.b == sim_core::survival::REFUSE_C_FULL
        })
        .count();
    assert_eq!(refused, 1, "and it must say which refusal it was");
}

/// The salt can kill you, and when it does it is a death like any other:
/// counted, announced, and answered by the spawn ring. This is the reason
/// `Command::Drink` goes through `World::respawn` at all — a verb that can
/// take the last point of hp and then leave the body standing there at zero
/// would be a corpse the world never notices.
#[test]
fn drinking_yourself_to_death_is_a_death() {
    let mut w = lone_world();
    let (x, z) = shoreline(SEED);
    stand(&mut w, x, z);
    let sc = SurvivalContent::probe_fixture();
    // One point of hp and a meter with room: the next mouthful is the last.
    w.players[0].hp = 1;
    w.players[0].water = 0;
    let deaths_before = w.players[0].deaths;
    w.tick(&[Command::Drink { id: 1 }]);
    assert_eq!(
        w.players[0].deaths,
        deaths_before + 1,
        "the salt death was not counted — a body that dies uncounted respawns \
         on the identical beach"
    );
    let self_death = w
        .events
        .entries()
        .iter()
        .filter(|e| e.code == sim_core::world::EV_DEATH && e.a == e.b)
        .count();
    assert_eq!(self_death, 1, "a death by the world names one id twice");
    // The sea is its own sentence on the death screen: `DEATH_BY_SALT` and
    // not the clock's code, for `EV_DRANK`'s reason one shelf over — a
    // death you pressed a key for is not a death that happened to you.
    assert!(w.players[0].dead, "the salt death did not raise the screen");
    assert_eq!(
        w.players[0].death_cause,
        sim_core::world::DEATH_BY_SALT,
        "the sea was recorded as the clock"
    );
    w.tick(&[Command::Respawn {
        id: 1,
        on_bag: false,
    }]);
    assert_eq!(
        (w.players[0].food, w.players[0].water),
        (sc.max_food, sc.max_water),
        "the respawn must grant a fresh pair, exactly as a starve death does"
    );
    assert!(w.players[0].hp > 0, "and a body with hp in it");
}

/// Content that authors no drink refuses the press rather than swallowing
/// it — the same posture as an eat against a table with no food row, and
/// the reason `validate::structural` can treat `drink_water = 0` as "the
/// verb is disarmed" instead of as "the verb is broken".
#[test]
fn a_disarmed_drink_refuses_out_loud() {
    let mut w = lone_world();
    let (x, z) = shoreline(SEED);
    stand(&mut w, x, z);
    w.survival.drink_water = 0;
    w.players[0].water = 0;
    w.tick(&[Command::Drink { id: 1 }]);
    assert_eq!(count(&w, sim_core::world::EV_DRANK), 0, "nothing was drunk");
    assert_eq!(
        count(&w, sim_core::world::EV_CONSUME_REFUSED),
        1,
        "a disarmed verb still answers the press"
    );
}
