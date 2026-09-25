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

/// The honest half: a loss burst starves the server, the burst's frames
/// then arrive together, and every one of them still runs — the starved
/// ticks earned the credit to catch up.
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
