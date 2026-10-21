//! The casino's seats (`seat.rs`): a sit puts the feet on the seat's spot,
//! a taken seat or one out of reach refuses, and the first step stands the
//! sitter up — the frames the client predicts, seated or not.

use sim_core::fmath::fabs;
use sim_core::input::InputFrame;
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::seat::SEAT_NONE;
use sim_core::town;
use sim_core::world::{Command, World};

const SEED: u64 = 20260802;
const A: u32 = 4;
const B: u32 = 5;

fn world() -> World {
    let mut w = World::new(SEED);
    w.tick(&[Command::Join { id: A }, Command::Join { id: B }]);
    assert!(w.haven.town.live, "the fixture seed has a town");
    w
}

fn slot(w: &World, id: u32) -> usize {
    w.players
        .iter()
        .position(|p| p.active && p.id == id)
        .unwrap()
}

/// Stand player `id` at world (`x`, `z`) and let the body settle.
fn stand_at(w: &mut World, id: u32, x: f32, z: f32) {
    let i = slot(w, id);
    w.players[i].body = Body::at(SEED, &w.haven, x, z);
    for _ in 0..3 {
        w.tick(&[]);
    }
}

fn at(w: &World, id: u32) -> (f32, f32) {
    let b = w.players[slot(w, id)].body;
    (b.qx as f32 * POS_XZ_Q, b.qz as f32 * POS_XZ_Q)
}

#[test]
fn a_sitter_is_put_on_the_seat_and_stands_up_on_a_step() {
    let mut w = world();
    // The first armchair, sat in from a metre and a half off.
    let k = town::SEATS
        .iter()
        .position(|s| s.kind == town::SeatKind::Armchair)
        .unwrap();
    let (sx, sz) = town::seat_world(&w.haven.town, k).unwrap();
    stand_at(&mut w, A, sx + 1.5, sz);
    w.tick(&[Command::Sit {
        id: A,
        seat: k as u8,
    }]);
    let (x, z) = at(&w, A);
    assert!(
        fabs(x - sx) <= POS_XZ_Q && fabs(z - sz) <= POS_XZ_Q,
        "put on the seat, to the quantum"
    );
    assert_eq!(w.players[slot(&w, A)].seat as usize, k + 1);
    // Sitting still keeps the seat.
    for _ in 0..30 {
        w.tick(&[]);
    }
    assert_eq!(w.players[slot(&w, A)].seat as usize, k + 1, "still seated");

    // Taken: nobody else sits in it.
    stand_at(&mut w, B, sx - 1.0, sz);
    let before = at(&w, B);
    w.tick(&[Command::Sit {
        id: B,
        seat: k as u8,
    }]);
    assert_eq!(w.players[slot(&w, B)].seat, 0, "the seat is taken");
    assert_eq!(at(&w, B), before, "and the body stays where it was");

    // A step stands the sitter up, and the seat is free again.
    let step = InputFrame {
        seq: 1,
        move_z: 127,
        ..InputFrame::default()
    };
    w.tick(&[Command::Input {
        id: A,
        frame: step,
        favour: 0,
    }]);
    assert_eq!(w.players[slot(&w, A)].seat, 0, "a step stands them up");
    w.tick(&[Command::Sit {
        id: B,
        seat: k as u8,
    }]);
    assert_eq!(w.players[slot(&w, B)].seat as usize, k + 1);

    // And `SEAT_NONE` stands a sitter up where it sits.
    w.tick(&[Command::Sit {
        id: B,
        seat: SEAT_NONE,
    }]);
    assert_eq!(w.players[slot(&w, B)].seat, 0);
}

#[test]
fn a_seat_out_of_reach_refuses() {
    let mut w = world();
    let (sx, sz) = town::seat_world(&w.haven.town, 0).unwrap();
    stand_at(&mut w, A, sx + 3.0, sz);
    let before = at(&w, A);
    w.tick(&[Command::Sit { id: A, seat: 0 }]);
    assert_eq!(w.players[slot(&w, A)].seat, 0);
    assert_eq!(at(&w, A), before);
}
