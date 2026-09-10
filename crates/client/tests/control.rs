//! **Which of the six arguments lands in which field, checked against the
//! wire.** Code tier: no `render` feature, no GPU, no socket.
//!
//! `look::control` returns `(buttons, yaw, pitch, move_x, move_z, sel)` and
//! `ClientCore::set_input` takes them in that order. Four of the six are
//! integers, so **a swapped pair type-checks perfectly** — and this repo has
//! already paid for exactly that class twice: `look.rs`'s own header records
//! the mouse and strafe axes shipping inverted on both clients, and
//! `sim-core/tests/event_roles.rs` exists because a positional payload can be
//! byte-golden-green with `a` and `b` the wrong way round.
//!
//! So this is `event_roles.rs`'s shape applied to input: drive a real
//! `ClientCore`, let it mint a real frame, decode the real datagram, and
//! assert each argument arrived where it was addressed. Nothing is restated —
//! the expected values come out of `look`'s own published quantizers, because
//! re-deriving `yaw_u16` here would be a second copy of the thing under test
//! (`CLAUDE.md`'s `lattice.rs` trap).
//!
//! **The gate is the ROLE, not the arithmetic.** `look.rs`'s unit tests own
//! whether `yaw_after` subtracts and whether `move_axes` negates; what no test
//! owned before this one is that the value those produce reaches the field it
//! is named for, through a client that is not the Bevy one.
//!
//! **Four mutants run, four caught — after the first draft let two through.**
//! `move_x <-> move_z`, `yaw <-> pitch`, `buttons <-> sel` and `sel` pinned to
//! a constant. The first draft computed its expectations by calling
//! `look::control` a second time, so both sides of every comparison carried
//! the mutation and the middle two SURVIVED. Rebuilding the expectation from
//! `look`'s published parts is what makes this a gate rather than a tautology;
//! the trap is `CLAUDE.md`'s `lattice.rs` entry, and it is recorded here
//! because it caught a live instance in a file written by somebody who had
//! just read it.

use client::look::{self, Aim, Raw};
use client_core::core::ClientCore;

/// A core with a frame in flight, and the datagram it minted.
///
/// `advance` is what mints a frame, and it mints on the sim's own 30 Hz
/// cadence — so this hands it enough wall time for at least one tick and then
/// reads what `poll_input` actually produced.
fn wire_after(raw: Raw, aim: &mut Aim) -> sim_core::input::InputFrame {
    let mut core = ClientCore::new(20260731, 7, 0);
    let (buttons, yaw, pitch, move_x, move_z, sel) = look::control(aim, raw);
    core.set_input(buttons, yaw, pitch, move_x, move_z, sel);
    core.advance(100.0);
    let mut buf = [0u8; sim_core::limits::DATAGRAM_BUDGET_BYTES];
    let n = core.poll_input(&mut buf);
    assert!(n > 0, "the core minted no input frame to check");
    let dg = protocol::decode_input(&buf[..n]).expect("the client's own datagram decodes");
    *dg.frames().last().expect("at least one frame")
}

/// **Every argument arrives where it was addressed.**
///
/// Each of the six is given a value distinguishable from the other five, so a
/// swap cannot pass by coincidence: the buttons byte, the slot and the two
/// axes are all different numbers, and yaw and pitch are asked for at angles
/// whose quantized forms are not each other.
#[test]
fn each_control_argument_reaches_the_field_it_names() {
    let mut aim = Aim::default();
    let raw = Raw {
        // A yaw move and a pitch move of different magnitudes, so the two
        // cannot be confused even after quantizing.
        dx_px: -40.0,
        dy_px: -12.0,
        invert_pitch: false,
        forward: true,
        back: false,
        left: false,
        right: true,
        buttons: 0b0000_0101,
        sel: 3,
    };
    // **The expectation is REBUILT from the published parts, not obtained by
    // calling `control` again.**
    //
    // The first draft of this test did call it again, and running the mutants
    // is what found that out: `yaw <-> pitch` and `buttons <-> sel` both
    // SURVIVED, because both sides of the comparison carried the mutation.
    // That is `CLAUDE.md`'s `lattice.rs` trap verbatim — a naive rebuild that
    // calls the function under test is a rebuild of nothing — and it is worth
    // recording that it caught a live instance in the gate written by the
    // person who had just read the entry.
    //
    // `yaw_after`, `pitch_after`, `move_axes`, `yaw_u16` and `pitch_u8` are
    // `pub` for exactly this, and each has its own unit tests in `look.rs`.
    let e_buttons = raw.buttons;
    let e_sel = raw.sel;
    let e_yaw = look::yaw_u16(look::yaw_after(0.0, raw.dx_px, look::MOUSE_RAD_PER_PX));
    let e_pitch = look::pitch_u8(look::pitch_after(
        0.0,
        raw.dy_px,
        look::MOUSE_RAD_PER_PX,
        raw.invert_pitch,
        look::PITCH_LIMIT,
    ));
    let (e_move_x, e_move_z) = look::move_axes(
        i32::from(raw.forward) - i32::from(raw.back),
        i32::from(raw.right) - i32::from(raw.left),
    );

    let f = wire_after(raw, &mut aim);
    assert_eq!(
        f.buttons, e_buttons,
        "`buttons` did not reach the wire's buttons"
    );
    assert_eq!(f.yaw, e_yaw, "`yaw` did not reach the wire's yaw");
    assert_eq!(f.pitch, e_pitch, "`pitch` did not reach the wire's pitch");
    assert_eq!(
        f.move_x, e_move_x,
        "`move_x` did not reach the wire's move_x"
    );
    assert_eq!(
        f.move_z, e_move_z,
        "`move_z` did not reach the wire's move_z"
    );
    assert_eq!(f.sel, e_sel, "`sel` did not reach the wire's sel");

    // And the two axes are genuinely distinguishable in this fixture, or the
    // assertions above would pass under a swap by coincidence.
    assert_ne!(
        e_move_x, e_move_z,
        "the fixture cannot see a move_x/move_z swap"
    );
    assert_ne!(
        u32::from(e_yaw),
        u32::from(e_pitch),
        "the fixture cannot see a yaw/pitch swap"
    );
}

/// **Walking forward strafes nowhere, and strafing right goes right.**
///
/// The axis identity `look.rs`'s header is entirely about, stated at the level
/// a second client actually uses — `control`, not `move_axes` — because that
/// is the level where the negation can be lost. `move_x` points along the
/// body's LEFT on the wire, so a player asking to strafe right sends a
/// NEGATIVE `move_x`; that sign is the bug the header records, and this is the
/// only test that holds it through `control`.
#[test]
fn the_strafe_axis_keeps_the_sign_the_sim_expects() {
    let key = |forward, back, left, right| {
        let mut aim = Aim::default();
        let f = wire_after(
            Raw {
                forward,
                back,
                left,
                right,
                ..Raw::default()
            },
            &mut aim,
        );
        (f.move_x, f.move_z)
    };
    assert_eq!(
        key(true, false, false, false),
        (0, 127),
        "W is forward only"
    );
    assert_eq!(
        key(false, true, false, false),
        (0, -127),
        "S is backward only"
    );
    assert_eq!(
        key(false, false, false, true),
        (-127, 0),
        "D strafes right, which is a NEGATIVE move_x — the sim's axis points left \
         (`look.rs`'s header). A positive one here is the inversion that shipped."
    );
    assert_eq!(key(false, false, true, false), (127, 0), "A strafes left");
    assert_eq!(
        key(false, false, false, false),
        (0, 0),
        "no key, no movement"
    );
    assert_eq!(
        key(true, true, true, true),
        (0, 0),
        "opposed keys cancel rather than overrunning the axis"
    );
}

/// The mouse turns the way the mouse moved, through `control` rather than
/// through `yaw_after` — same reason as above: the composition is where a sign
/// gets lost.
#[test]
fn the_mouse_turns_the_way_it_was_pushed() {
    let mut aim = Aim::default();
    look::control(
        &mut aim,
        Raw {
            dx_px: 50.0,
            ..Raw::default()
        },
    );
    assert!(
        aim.yaw < 0.0,
        "a mouse pushed RIGHT must turn the view right, which is yaw decreasing \
         (`look.rs`'s header). Got {}",
        aim.yaw
    );

    let mut up = Aim::default();
    look::control(
        &mut up,
        Raw {
            dy_px: -30.0,
            ..Raw::default()
        },
    );
    assert!(up.pitch > 0.0, "a mouse pushed UP must raise the view");

    let mut inverted = Aim::default();
    look::control(
        &mut inverted,
        Raw {
            dy_px: -30.0,
            invert_pitch: true,
            ..Raw::default()
        },
    );
    assert!(
        inverted.pitch < 0.0,
        "the invert setting must reverse pitch and nothing else"
    );
    assert_eq!(
        inverted.yaw, up.yaw,
        "the invert setting reversed yaw as well as pitch"
    );
}

/// Pitch is clamped, so a player who keeps dragging up does not roll the
/// camera over the top — and the clamp is `look::PITCH_LIMIT`, read rather
/// than retyped.
#[test]
fn pitch_stops_at_the_limit_however_hard_it_is_pushed() {
    let mut aim = Aim::default();
    for _ in 0..500 {
        look::control(
            &mut aim,
            Raw {
                dy_px: -100.0,
                ..Raw::default()
            },
        );
    }
    assert!(
        (aim.pitch - look::PITCH_LIMIT).abs() < 1e-4,
        "pitch ran past the limit: {} vs {}",
        aim.pitch,
        look::PITCH_LIMIT
    );
}
