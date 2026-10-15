//! A client may not move faster than the tick rate by sending frames ahead
//! of it. The consume throttle takes two frames a tick only against the
//! catch-up credit (`limits::INPUT_CATCHUP_CREDIT_CAP`), never on depth
//! alone — found 2026-09-25, when depth alone gave a 60 Hz client 2× speed.

use server::client::ClientNetState;
use sim_core::input::InputFrame;
use sim_core::limits::{INPUT_CATCHUP_CREDIT_CAP, INPUT_DRIFT_CREDIT_TICKS};

fn frame(seq: u16) -> InputFrame {
    InputFrame {
        seq,
        ..InputFrame::default()
    }
}

fn executed(c: &mut ClientNetState) -> u32 {
    match c.consume_input() {
        Some(got) => 1 + got.prev.is_some() as u32,
        None => 0,
    }
}

/// Strides one frame walks the body: `movement::step` scales each axis by
/// 1/127 and clamps the wish vector to unit length, so a full diagonal is
/// one stride and its 2/3 decay (84, 84) is 0.94 of one.
fn stride(f: &InputFrame) -> f32 {
    let x = f32::from(f.move_x) / 127.0;
    let z = f32::from(f.move_z) / 127.0;
    (x * x + z * z).sqrt().min(1.0)
}

/// What one tick walked: each consumed frame's stride, or on a starved tick
/// the decayed stand-in `core.rs` feeds the sim. Past the ramp the sim
/// re-runs that last stand-in, which does not move.
fn walked(c: &mut ClientNetState) -> f32 {
    match c.consume_input() {
        Some(got) => stride(&got.frame) + got.prev.as_ref().map_or(0.0, stride),
        None => c.ghost_frame().as_ref().map_or(0.0, stride),
    }
}

fn walk(seq: u16) -> InputFrame {
    InputFrame {
        seq,
        move_x: 127,
        move_z: 127,
        ..InputFrame::default()
    }
}

/// Two frames minted per tick for ten seconds: the server runs at most one
/// a tick plus the join allowance and the drift trickle.
#[test]
fn a_client_running_ahead_gains_only_the_allowance() {
    let mut c = ClientNetState::new();
    c.reset(1);
    let ticks = 300u32;
    let mut seq = 0u16;
    let mut ran = 0u32;
    for _ in 0..ticks {
        for _ in 0..2 {
            c.push_frame(frame(seq), None);
            seq = seq.wrapping_add(1);
        }
        ran += executed(&mut c);
    }
    let allowance = INPUT_CATCHUP_CREDIT_CAP as u32 + ticks / INPUT_DRIFT_CREDIT_TICKS as u32;
    assert!(
        ran <= ticks + allowance,
        "{ran} frames ran in {ticks} ticks — a speedhack (allowance {allowance})"
    );
}

/// A lag switch: hold the uplink for `held` ticks of every `period`, then
/// let the backlog through at once. An honest sender walks the same frames
/// one per tick. The switch must end no further than the honest sender plus
/// the allowance any client can already spend by running ahead. Diagonal on
/// purpose: that is where the ghost walks 1.4 strides.
#[test]
fn a_lag_switch_gains_only_the_allowance() {
    let ticks = 900u32;
    let allowance =
        (INPUT_CATCHUP_CREDIT_CAP as u32 + ticks / INPUT_DRIFT_CREDIT_TICKS as u32) as f32;
    for (period, held) in [(9u32, 7u32), (20, 7)] {
        let mut honest = ClientNetState::new();
        honest.reset(1);
        let mut cheat = ClientNetState::new();
        cheat.reset(2);
        let (mut h, mut x) = (0.0f32, 0.0f32);
        let mut sent = 0u16;
        for t in 0..ticks {
            let seq = t as u16;
            honest.push_frame(walk(seq), None);
            h += walked(&mut honest);
            if t % period >= held {
                while sent <= seq {
                    cheat.push_frame(walk(sent), None);
                    sent += 1;
                }
            }
            x += walked(&mut cheat);
        }
        assert!(
            x <= h + allowance,
            "a {held}-of-{period} lag switch walked {x:.1} strides against an honest {h:.1} \
             in {ticks} ticks (allowance {allowance})"
        );
    }
}

/// The honest half: a loss burst starves the server, the burst's frames
/// then arrive together, and every one of them still runs — the bank, and
/// the starved ticks that stood still, pay for the catch-up.
#[test]
fn a_loss_burst_still_catches_up_in_full() {
    let mut c = ClientNetState::new();
    c.reset(1);
    let mut seq = 0u16;
    let mut ran = 0u32;
    // Steady play long enough to spend the join allowance on nothing.
    for _ in 0..600u32 {
        c.push_frame(frame(seq), None);
        seq = seq.wrapping_add(1);
        ran += executed(&mut c);
    }
    // Ten ticks of nothing arriving, then all ten frames in one datagram.
    for _ in 0..10u32 {
        ran += executed(&mut c);
    }
    let sent_before = seq;
    for _ in 0..10u32 {
        c.push_frame(frame(seq), None);
        seq = seq.wrapping_add(1);
    }
    // Steady again; the backlog drains two a tick.
    for _ in 0..30u32 {
        c.push_frame(frame(seq), None);
        seq = seq.wrapping_add(1);
        ran += executed(&mut c);
    }
    // The throttle drains to its threshold; the dilation nudge slows the
    // client for the rest, which here is a few ticks with nothing sent.
    for _ in 0..10u32 {
        ran += executed(&mut c);
    }
    assert!(sent_before > 0);
    assert_eq!(
        ran, seq as u32,
        "a frame an honest client sent was not run (sent {seq}, ran {ran})"
    );
}
