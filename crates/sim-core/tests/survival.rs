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
/// first call's death. Returns the tick the death landed on.
fn run_until_death(w: &mut World, ticks: u32) -> Option<u32> {
    let before = w.players[0].deaths;
    for t in 0..ticks {
        w.tick(&[]);
        if w.players[0].deaths > before {
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
