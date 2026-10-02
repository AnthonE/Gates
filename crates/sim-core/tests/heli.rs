//! The attack helicopter (`heli.rs`): unarmed it never comes; armed, it
//! arrives on schedule, finds a player standing in the open, shoots them,
//! and when its patrol is over flies back out to sea and is gone.

use sim_core::combat::CombatContent;
use sim_core::heli::{Heli, HeliDef, HELI_SLOT};
use sim_core::mob;
use sim_core::movement::POS_Y_Q;
use sim_core::world::{Command, World, EV_HURT, EV_SHOT};

/// `content/mobs.toml`'s shipped `[heli]` at 30 Hz, but arriving after one
/// second instead of five minutes.
fn def() -> HeliDef {
    HeliDef {
        first_ticks: 30,
        every_ticks: 1_200 * 30,
        patrol_ticks: 300 * 30,
        speed_mmpt: 22_000 / 30,
        engage_speed_mmpt: 10_000 / 30,
        cruise_mm: 45_000,
        engage_mm: 26_000,
        orbit_mm: 32_000,
        detect_mm: 130_000,
        lose_ticks: 8 * 30,
        range_mm: 90_000,
        damage: 18,
        burst: 8,
        rate_ticks: 3,
        gap_ticks: 54,
        spread_pm: 18,
    }
}

#[test]
fn unarmed_it_never_comes() {
    let mut w = World::new(11);
    w.combat = CombatContent::probe_fixture();
    w.tick(&[Command::Join { id: 1 }]);
    for _ in 0..600 {
        w.tick(&[]);
    }
    assert!(!w.mobs.m[HELI_SLOT].alive);
    assert_eq!(w.heli, Heli::default());
}

#[test]
fn it_hunts_a_player_in_the_open_then_leaves() {
    let mut w = World::new(11);
    w.combat = CombatContent::probe_fixture();
    // The spawn ring: open beach, nothing between the sky and the player.
    w.tick(&[Command::Join { id: 1 }]);
    w.heli_def = def();
    let heli_id = mob::mob_id(HELI_SLOT);

    let (mut arrived, mut shots, mut hurt) = (false, 0u32, 0u32);
    let mut lowest_clearance = f32::MAX;
    let mut departed = false;
    for _ in 0..20_000 {
        w.tick(&[]);
        let m = &w.mobs.m[HELI_SLOT];
        if m.alive {
            arrived = true;
            let x = m.body.qx as f32 * sim_core::movement::POS_XZ_Q;
            let z = m.body.qz as f32 * sim_core::movement::POS_XZ_Q;
            let ground =
                sim_core::terrain::ground(w.seed, &w.haven, x, z).max(sim_core::terrain::SEA_LEVEL);
            lowest_clearance = lowest_clearance.min(m.body.qy as f32 * POS_Y_Q - ground);
            assert!(m.body.qy <= sim_core::heli::Y_CEIL_Q);
        } else if arrived {
            departed = true;
            break;
        }
        for e in w.events.entries() {
            if e.code == EV_SHOT && e.a == heli_id {
                shots += 1;
            }
            if e.code == EV_HURT && e.a == w.players[0].id {
                hurt += 1;
            }
        }
    }
    assert!(arrived, "the heli never arrived");
    assert!(shots > 0, "the heli never fired");
    assert!(
        hurt > 0,
        "{shots} rounds and not one landed on a player in the open"
    );
    assert!(
        w.players[0].hp < 100 || w.players[0].dead || w.players[0].wounded,
        "the player is untouched"
    );
    assert!(
        lowest_clearance >= 5.0,
        "the heli came down to {lowest_clearance:.1} m over the ground"
    );
    assert!(departed, "the patrol ended and the heli never left");
    assert!(w.heli.next_at > w.tick, "no next visit is scheduled");
}
