//! The town's safe zone: who it protects, who it does not, and that it is
//! not a vault for a sleeping body.
use sim_core::combat::{self, HOSTILE_TICKS};
use sim_core::kit;
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::town;
use sim_core::world::{Command, Player, World, SAFE_SLEEP_TICKS};

const SEED: u64 = 20_260_731;

fn body(safe: bool, hostile: u16) -> Player {
    Player {
        active: true,
        safe,
        hostile,
        ..Player::default()
    }
}

#[test]
fn nobody_hurts_from_inside_and_nobody_inside_is_hurt_unless_hostile() {
    // Attacker outside, victim inside and peaceful: shielded.
    let mut a = body(false, 0);
    assert!(combat::shielded(&mut a, &body(true, 0)));
    // Trying counts: the attacker is hostile now.
    assert_eq!(a.hostile, HOSTILE_TICKS);
    // Attacker inside: shielded whoever the victim is.
    let mut a = body(true, 0);
    assert!(combat::shielded(&mut a, &body(false, 0)));
    assert!(combat::shielded(&mut a, &body(true, HOSTILE_TICKS)));
    // A hostile body that ran inside is fair game from outside.
    let mut a = body(false, 0);
    assert!(!combat::shielded(&mut a, &body(true, 5)));
    // Out in the open, nothing changes.
    let mut a = body(false, 0);
    assert!(!combat::shielded(&mut a, &body(false, 0)));
}

#[test]
fn the_tick_marks_the_zone_runs_hostility_down_and_moves_sleepers_out() {
    let mut w = World::new(SEED);
    let t = w.haven.town;
    assert!(t.live);
    let (x, z) = kit::to_world(&t.placed(), 0.0, 20.0);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: 1 }]);
    let slot = w
        .players
        .iter()
        .position(|p| p.active && p.id == 1)
        .unwrap();
    w.players[slot].body = Body::at(SEED, &w.haven, x, z);
    w.players[slot].hostile = 3;
    w.tick(&[]);
    assert!(w.players[slot].safe, "a body in the market is in the zone");
    assert_eq!(w.players[slot].hostile, 2);
    // Asleep in the zone past the limit: moved out of the main gate.
    w.players[slot].sleeping = true;
    w.players[slot].slept_at = 0;
    w.tick = SAFE_SLEEP_TICKS + 30 - (SAFE_SLEEP_TICKS + 30) % 30;
    w.tick(&[]);
    let p = &w.players[slot];
    let (px, pz) = (p.body.qx as f32 * POS_XZ_Q, p.body.qz as f32 * POS_XZ_Q);
    assert!(
        !town::safe(&t, px, pz),
        "the sleeper is still inside at {px},{pz}"
    );
    assert!(!p.safe);
}
