//! Fall damage: a short drop is free, a long one hurts, a very long one
//! puts the body down with the fall's own cause.

use sim_core::combat::CombatContent;
use sim_core::input::{InputFrame, BTN_JUMP};
use sim_core::movement::{self, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::Scratch;
use sim_core::terrain;
use sim_core::world::{Command, World};

const SEED: u64 = 11;

/// The body lifted `metres` over its spawn and let go, nothing ticked yet.
fn lifted(metres: f32) -> World {
    let mut w = World::new(SEED);
    w.combat = CombatContent::probe_fixture();
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[Command::Join { id: 1 }]);
    w.tick(&[]);
    let b = &mut w.players[0].body;
    assert!(b.grounded, "the spawn stands on the ground");
    b.qy += (metres / POS_Y_Q) as i32;
    b.grounded = false;
    b.qvy = 0;
    w
}

fn dropped(metres: f32) -> World {
    let mut w = lifted(metres);
    for _ in 0..(10 * 30) {
        w.tick(&[]);
        if w.players[0].body.grounded {
            break;
        }
    }
    w
}

#[test]
fn a_short_drop_is_free() {
    let w = dropped(3.0);
    assert!(w.players[0].body.grounded);
    assert_eq!(w.players[0].hp, 100);
}

#[test]
fn a_long_drop_hurts_and_a_longer_one_downs() {
    let w = dropped(8.0);
    let hp = w.players[0].hp;
    assert!(hp < 100 && hp > 0, "an 8 m drop left {hp} hp");

    let w = dropped(30.0);
    let p = &w.players[0];
    assert!(p.wounded || p.dead, "a 30 m drop left the body standing");
}

/// A throttle tick steps two frames (`Command::InputPair`). The older one
/// landing the body and the newer jumping it straight back off used to
/// cancel the landing: one check across both steps saw only the jump. Both
/// parities, so the landing falls in each of the pair's steps once.
#[test]
fn a_landing_inside_a_throttle_pair_still_hurts() {
    for shift in 0..2 {
        let mut w = lifted(8.0);
        for _ in 0..shift {
            w.tick(&[]);
        }
        let mut seq = 1u16;
        for _ in 0..(4 * 30) {
            let prev = InputFrame {
                seq,
                ..InputFrame::default()
            };
            let frame = InputFrame {
                seq: seq + 1,
                buttons: BTN_JUMP,
                ..InputFrame::default()
            };
            seq += 2;
            w.tick(&[Command::InputPair {
                id: 1,
                prev,
                frame,
                favour: 0,
            }]);
        }
        let hp = w.players[0].hp;
        assert!(
            hp < 100,
            "shift {shift}: an 8 m drop landed in a pair left {hp} hp"
        );
    }
}

/// Logging off mid-fall is not a parachute: the sleeper lands as hard.
#[test]
fn a_body_whose_owner_leaves_mid_fall_lands_as_hard() {
    let mut w = lifted(8.0);
    w.tick(&[Command::Leave { id: 1 }]);
    assert!(w.players[0].sleeping);
    for _ in 0..(3 * 30) {
        w.tick(&[]);
    }
    let hp = w.players[0].hp;
    assert!(hp < 100, "a sleeper's 8 m drop left {hp} hp");
}

/// Nor is being down: a downed body's 8 m drop finishes it.
#[test]
fn a_downed_body_lands_as_hard() {
    let mut w = lifted(8.0);
    let p = &mut w.players[0];
    p.wounded = true;
    p.wound_until = u64::MAX / 2;
    p.hp = sim_core::wound::WOUNDED_HP;
    for _ in 0..(3 * 30) {
        w.tick(&[]);
    }
    assert!(w.players[0].dead, "a downed body survived an 8 m drop");
}

/// A face stops a fall dead and slides the body rather than standing it
/// (`movement::face_fall`), so a landing read off `grounded` alone let a
/// drop onto any cliff land free. Dropped 10 m onto faces gentle enough to
/// hold a slide, the first touch is the impact and it hurts.
#[test]
fn a_drop_onto_a_face_hurts() {
    let mut w = lifted(0.0);
    let haven = w.haven;
    let mut sc = Scratch::live(SEED);
    let mut hit = 0;
    let mut x = 300.0f32;
    'scan: while x < terrain::ISLAND_SIZE - 300.0 {
        let mut z = 300.0f32;
        while z < terrain::ISLAND_SIZE - 300.0 {
            z += 17.0;
            let slope = terrain::ground_slope(SEED, &haven, x, z);
            let g = terrain::ground(SEED, &haven, x, z);
            if !(terrain::CLIFF_SLOPE_RATIO * 1.3..2.0).contains(&slope)
                || g < terrain::SEA_LEVEL + 5.0
                || sim_core::town::safe(&haven.town, x, z)
                || movement::face_fall(&mut sc.occupants(), SEED, &haven, x, z).is_none()
            {
                continue;
            }
            let p = &mut w.players[0];
            p.hp = 100;
            p.body.qx = movement::quant_xz(x);
            p.body.qz = movement::quant_xz(z);
            p.body.qy = movement::quant_y(g + 10.0);
            p.body.qvy = 0;
            p.body.grounded = false;
            // Down to the first touch: the tick that stopped the fall.
            for _ in 0..60 {
                w.tick(&[]);
                if w.players[0].body.qvy == 0 {
                    break;
                }
            }
            let p = &w.players[0];
            // Only a touch on the face itself counts; a boulder or a shelf
            // stood the body up, which was always a landing.
            if p.body.qvy != 0 || p.body.grounded || p.dead || p.wounded {
                continue;
            }
            let (bx, bz) = (p.body.qx as f32 * POS_XZ_Q, p.body.qz as f32 * POS_XZ_Q);
            assert!(
                p.hp < 100,
                "a 10 m drop onto the face at ({bx:.1}, {bz:.1}) left {} hp",
                p.hp
            );
            hit += 1;
            if hit == 4 {
                break 'scan;
            }
        }
        x += 97.0;
    }
    assert!(
        hit > 0,
        "no drop ever touched a face: the test drops nowhere"
    );
}
