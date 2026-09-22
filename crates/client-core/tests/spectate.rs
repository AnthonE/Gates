//! **A watcher's core drives nothing and draws the watched body** (spectators
//! v0, wire v73; `NETCODE.md` §2.3). Wire in, core state out, no socket — the
//! shape of `tests/wire.rs`, for the one mode that file never needed.
//!
//! Four claims, each a property the renderer relies on and must not decide:
//!
//! 1. input is never built — `set_input` is ignored and every datagram the
//!    core emits carries acks and no frame, however long it runs;
//! 2. the watched body is interpolated, so the camera has a sample to follow
//!    (`spectate_view`) and it is the wire's position and look angles;
//! 3. `render_position` readers see the watched body, not the world origin;
//! 4. a watcher who arrives after a death learns it from the first snapshot,
//!    and a later stale snapshot cannot raise a death that is over.

use client_core::core::ClientCore;
use protocol::{
    decode_input, encode_event_respawn, encode_snapshot, EntityState, Nudge, SnapshotHeader,
    MAX_EVENT_MSG_BYTES,
};
use sim_core::limits::DATAGRAM_BUDGET_BYTES;
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};

/// The watched body. Distinct from every other number below.
const TARGET: u32 = 0x0207;

fn body(qx: i32, qz: i32, yaw: u16, pitch: u8, dead: bool) -> EntityState {
    EntityState {
        id: TARGET,
        qx,
        qy: 3_000,
        qz,
        qvy: 0,
        grounded: true,
        sleeping: false,
        dead,
        wounded: false,
        yaw,
        pitch,
        held: None,
        lit: false,
    }
}

/// An absolute (zero-baseline) snapshot at `tick` holding just the body.
fn snapshot(tick: u32, e: &EntityState) -> Vec<u8> {
    let mut buf = [0u8; DATAGRAM_BUDGET_BYTES];
    let n = encode_snapshot(
        &SnapshotHeader {
            tick,
            baseline_age: 0,
            last_executed_seq: 0,
            nudge: Nudge::Ok,
            buffered_depth: 0,
            repeat_count: 0,
        },
        &[],
        std::slice::from_ref(e),
        &[],
        &mut buf,
    )
    .expect("encodes");
    buf[..n].to_vec()
}

#[test]
fn a_watcher_never_builds_an_input_frame() {
    let mut c = ClientCore::spectator(1, TARGET, 100);
    assert!(c.spectating());
    let mut out = [0u8; DATAGRAM_BUDGET_BYTES];
    let mut datagrams = 0;
    for i in 0..90u32 {
        // Every input a player could make, every frame: forward, jump, swing.
        c.set_input(0xFF, 0x4000, 200, 127, 127, 3);
        c.on_datagram(&snapshot(
            100 + i,
            &body(1_000 + i as i32, 2_000, 0, 128, false),
        ));
        c.advance(1000.0 / 30.0);
        let n = c.poll_input(&mut out);
        if n > 0 {
            datagrams += 1;
            let dg = decode_input(&out[..n]).expect("the ack datagram decodes");
            assert!(
                dg.frames().is_empty(),
                "a watcher sent a frame: {:?}",
                dg.frames()
            );
        }
    }
    // Acks still flow — they are how the shard keeps delta-coding this seat —
    // and they flow at the tick rate, not once.
    assert!(datagrams > 60, "only {datagrams} ack datagrams in 3 s");
    assert_eq!(c.buttons(), 0, "set_input reached the core");
}

/// The contrast that makes the test above mean something: the same calls on
/// a player's core DO build frames.
#[test]
fn a_player_core_on_the_same_calls_builds_frames() {
    let mut c = ClientCore::new(1, TARGET, 100);
    assert!(!c.spectating());
    let mut out = [0u8; DATAGRAM_BUDGET_BYTES];
    let mut frames = 0;
    for i in 0..30u32 {
        c.set_input(0, 0x4000, 128, 0, 127, 0);
        c.on_datagram(&snapshot(100 + i, &body(1_000, 2_000, 0, 128, false)));
        c.advance(1000.0 / 30.0);
        let n = c.poll_input(&mut out);
        if n > 0 {
            frames += decode_input(&out[..n]).unwrap().frames().len();
        }
    }
    assert!(frames > 0);
}

#[test]
fn the_camera_follows_the_watched_body_and_its_look() {
    let mut c = ClientCore::spectator(1, TARGET, 100);
    assert!(c.spectate_view().is_none(), "no sample yet, no view");
    // Two seconds of a body walking +x at a fixed look.
    for i in 0..60u32 {
        c.on_datagram(&snapshot(
            100 + i,
            &body(1_000 + 10 * i as i32, 2_000, 0x2000, 150, false),
        ));
        c.advance(1000.0 / 30.0);
    }
    let v = c.spectate_view().expect("the watched body is sampled");
    // Somewhere on the walked segment, at the wire's height.
    let x0 = 1_000.0 * POS_XZ_Q;
    let x1 = (1_000.0 + 590.0) * POS_XZ_Q;
    assert!(v.x >= x0 && v.x <= x1, "x {} outside the walk", v.x);
    assert!((v.z - 2_000.0 * POS_XZ_Q).abs() < 1e-3);
    assert!((v.y - 3_000.0 * POS_Y_Q).abs() < 1e-3);
    assert!((v.yaw - 0x2000 as f32).abs() < 0.5, "yaw {}", v.yaw);
    assert!((v.pitch - 150.0).abs() < 0.5, "pitch {}", v.pitch);
    // The eye is the view, not the (never-stepped) predictor's.
    let eye = c.eye_position();
    assert_eq!(eye, [v.x, v.y, v.z]);
    // And `render_position` readers see the body, not the origin.
    let p = c.predict.render_position();
    assert!(
        (p[0] - (1_000.0 + 590.0) * POS_XZ_Q).abs() < 1e-3,
        "the predictor was not told where the body is: {p:?}"
    );
    assert_eq!(
        c.predict.error_magnitude(),
        0.0,
        "a watcher smooths nothing"
    );
}

#[test]
fn a_late_watcher_learns_a_death_from_its_first_snapshot_only() {
    let mut c = ClientCore::spectator(1, TARGET, 100);
    c.on_datagram(&snapshot(100, &body(1_000, 2_000, 0, 128, true)));
    assert!(
        c.dead,
        "arrived after the death: the first snapshot says so"
    );
    // The respawn arrives on the event lane…
    let mut ev = [0u8; MAX_EVENT_MSG_BYTES];
    let n = encode_event_respawn(false, &mut ev).expect("encodes");
    c.on_stream(&ev[..n]).expect("decodes");
    assert!(!c.dead);
    // …and a snapshot from before it, delayed behind it, must not re-raise the
    // screen. (Newer ticks only: the view refuses an older one outright, so
    // this is the newer-but-still-dead snapshot the race can produce.)
    c.on_datagram(&snapshot(101, &body(1_000, 2_000, 0, 128, true)));
    assert!(!c.dead, "a stale snapshot re-raised a death that is over");
}

#[test]
fn a_player_core_is_not_told_its_own_death_by_a_snapshot() {
    // The contrast: a player learns its death from the event it was present
    // for, so the seeding above must be a spectator's alone.
    let mut c = ClientCore::new(1, TARGET, 100);
    c.on_datagram(&snapshot(100, &body(1_000, 2_000, 0, 128, true)));
    assert!(!c.dead);
}
