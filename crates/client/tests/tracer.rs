//! Gate: the wire's instant-shot reading, and what each reader does with it.
//!
//! **Why this file exists.** `sim-core/tests/gun.rs` used to assert that a
//! firearm raises no `EV_SHOT` at all, and its reason was a fact about *this*
//! crate: the payload is a muzzle speed and a drop, `render/tracer.rs` re-flies
//! exactly those integers, and a zero in both would hang a motionless streak at
//! the shooter's eye for four seconds. Wire v54 spends that spare pattern —
//! `speed == 0` means the shot did not fly — so the sim now raises the event
//! and the refusal moved here, where it always belonged. Deleting a gate
//! because the thing it protected against became reachable would be the exact
//! trade `CLAUDE.md` forbids; this is that gate, one layer out.
//!
//! **Driven as arithmetic, not through a window**, which is `tests/fell.rs`'s
//! stated posture: the two decisions are split out of their systems
//! (`protocol::shot_is_instant`, `render::audio::shot_cue`) precisely so they
//! can be driven at both ends without a socket, a GPU or a shard. What no test
//! here claims is that the systems are *scheduled* — `tests/frame_gates.rs`
//! owns that question for the render app as a whole.
//!
//! Feature-gated like every other test that names `client::render`: the
//! module is behind `--features render` (`crates/client/Cargo.toml` says
//! why), so an ungated file here is red on a plain `cargo test --workspace`.

#![cfg(feature = "render")]

use client::render::audio::shot_cue;
use client::sound::{Cue, CUES, MAX_AUDIBLE_M};
use protocol::shot_is_instant;

/// Zero is the instant reading and **nothing else is**.
///
/// The half worth asserting is the second one. A predicate written as
/// `speed < SOMETHING` or `speed <= 0` would pass the interesting case and
/// silently reclassify slow rounds as beams — and a slow round is exactly
/// what a crossbow bolt or a thrown weapon looks like on this wire.
#[test]
fn only_a_zero_muzzle_speed_is_instant() {
    assert!(shot_is_instant(0), "zero is the instant reading");
    for speed in [1u16, 2, 100, 1_333, 30_000, u16::MAX] {
        assert!(
            !shot_is_instant(speed),
            "{speed} mm/tick is a round in flight, not a beam"
        );
    }
}

/// The mapping from that bit to a sound, both ways.
///
/// No item crosses `EV_SHOT`, so this one bit is the whole of how the mixer
/// tells a rifle from a bow. Getting it backwards is not a silent defect —
/// it puts a hundred-metre report on every arrow — but it is invisible to
/// every other gate in the tree, because both cues exist and both render.
#[test]
fn an_instant_shot_is_a_gun_and_a_flying_one_is_a_bow() {
    assert_eq!(
        shot_cue(0),
        Cue::ShotGun,
        "a shot with no flight came out of a barrel"
    );
    for speed in [1u16, 1_333, u16::MAX] {
        assert_eq!(
            shot_cue(speed),
            Cue::ShotBow,
            "{speed} mm/tick is an arrow, and an arrow does not carry 100 m"
        );
    }
}

/// The disclosure gap is the mechanic, so it is asserted rather than left to
/// two rows of a table nobody diffs.
///
/// Both numbers are the reference's, off one sentence (`CueDef::radius_m`):
/// a silenced weapon there carries "a maximum of 40m instead of the 100m it
/// used to be". What a balance pass may move is the pair; what it may not do
/// is close the gap, because a bow that discloses you as far as a rifle has
/// no reason to exist once a rifle is craftable.
#[test]
fn a_bow_discloses_you_over_a_smaller_circle_than_a_gun() {
    let bow = CUES[Cue::ShotBow as usize];
    let gun = CUES[Cue::ShotGun as usize];
    assert!(
        bow.positional && gun.positional,
        "a report happens where the shooter is - a non-positional one arrives \
         at full gain from nowhere, which is a lie about where a threat is"
    );
    assert_eq!(gun.radius_m, 100.0, "the reference's loud ranged weapon");
    assert_eq!(bow.radius_m, 40.0, "the reference's quiet one");
    assert!(
        gun.radius_m >= bow.radius_m * 2.0,
        "the gun carries {} m and the bow {} m - under 2x the bow stops being \
         the stealth option and the two cues stop meaning anything",
        gun.radius_m,
        bow.radius_m
    );
}

/// The gunshot is what sets `MAX_AUDIBLE_M` now, and the two must not drift.
///
/// `tests/sound.rs` already refuses a cue that carries *past* the ceiling.
/// This is the other direction and it is the one that rots quietly: a ceiling
/// left above every radius is a spatial scale sized for a sound nobody plays,
/// which costs nothing today and misleads the next person to raise a radius.
#[test]
fn the_ceiling_is_the_loudest_cue_and_that_cue_is_the_gun() {
    let loudest = CUES
        .iter()
        .map(|d| d.radius_m)
        .fold(f32::MIN, |a, b| if b > a { b } else { a });
    assert_eq!(
        loudest, MAX_AUDIBLE_M,
        "MAX_AUDIBLE_M is the maximum of the table, not a number beside it"
    );
    assert_eq!(
        CUES[Cue::ShotGun as usize].radius_m,
        MAX_AUDIBLE_M,
        "the gunshot is the cue that sets the ceiling"
    );
}
