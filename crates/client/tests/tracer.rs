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
//! stated posture: the decisions are split out of their systems
//! (`protocol::shot_is_instant`, `render::audio::shot_cue`,
//! `render::tracer::Tracers::claim`) precisely so they can be driven at both
//! ends without a socket, a GPU or a shard. What no test here claims is that
//! the systems are *scheduled* — `tests/frame_gates.rs` owns that question for
//! the render app as a whole.
//!
//! ⚠ **The refusal itself went ungated for one commit, which is the whole
//! lesson of the file.** Everything below the predicate tests was written
//! first and all of it passed with `render/tracer.rs`'s
//! `if protocol::shot_is_instant(speed) { continue; }` deleted — because the
//! refusal was a line inside `launch`, a Bevy system taking `NonSend<Net>`
//! and a live session, and no test could reach it. Gating the predicate a
//! call site happens to call is gating the wrong thing. The law moved into
//! `Tracers::claim`, which takes a pool and five integers, and the last three
//! tests here drive it.
//!
//! Feature-gated like every other test that names `client::render`: the
//! module is behind `--features render` (`crates/client/Cargo.toml` says
//! why), so an ungated file here is red on a plain `cargo test --workspace`.

#![cfg(feature = "render")]

use client::render::audio::shot_cue;
use client::render::tracer::{Tracers, TRACERS};
use client::sound::{Cue, CUES, MAX_AUDIBLE_M};
use protocol::shot_is_instant;

/// A shooter's feet. Where a tracer starts is not what any test here is
/// about, so every one of them looses from the origin.
const FEET: [f32; 3] = [0.0, 0.0, 0.0];
/// An arrow's muzzle speed and drop in the wire's units — `content/`'s bow
/// rounded, and any non-zero pair would do.
const ARROW: (u16, u16) = (1_333, 8);

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

/// **The refusal, at the only layer that can be driven.**
///
/// This is the gate the module header is about. `sim-core/tests/gun.rs` used
/// to hold the same property by forbidding the event outright; wire v54 makes
/// the event legal and this is where the property landed. The failure it
/// prevents is not subtle — a slot claimed at `speed == 0` integrates
/// `v.y -= drop; q += v` with `v` all zeroes for `MAX_ARROW_LIFE_TICKS`, so a
/// rifle leaves a bright motionless streak hanging at the shooter's eye for
/// four seconds and one of sixteen slots is gone until it expires.
///
/// The assertion is on `live()`, not on the returned bool: a refusal is a
/// slot that was not taken, and a test that reads only the return value is
/// checking the branch it just read rather than its effect.
///
/// Mutant: delete `claim`'s `if protocol::shot_is_instant(speed_mmpt)` →
/// `live()` is 1 → red.
#[test]
fn an_instant_shot_claims_no_tracer_slot() {
    let mut pool = Tracers::default();
    assert_eq!(pool.live(), 0, "a fresh pool draws nothing");
    assert!(
        !pool.claim(FEET, 0, 0, 0, 0),
        "a shot with no flight has nothing to draw, so nothing was claimed"
    );
    assert_eq!(
        pool.live(),
        0,
        "a beam took one of {TRACERS} tracer slots and will hold it for \
         MAX_ARROW_LIFE_TICKS, drawing a motionless streak at the muzzle - \
         this is the exact failure `gun.rs` used to prevent by refusing the \
         event, and wire v54 moved the refusal here"
    );
}

/// The control, and it is not decoration: every assertion above is satisfied
/// by a `claim` that refuses everything.
///
/// Mutant: `return false` at the top of `claim` → the instant-shot test above
/// stays green and this one goes red.
#[test]
fn an_arrow_claims_one_and_flies_it() {
    let mut pool = Tracers::default();
    let (speed, drop) = ARROW;
    assert!(
        pool.claim(FEET, 0, 0, speed, drop),
        "an arrow in flight is what the pool is for"
    );
    assert_eq!(pool.live(), 1, "the arrow claimed no slot");
    assert!(
        pool.claim(FEET, 0x4000, 32, speed, drop),
        "a second archer is not a refusal"
    );
    assert_eq!(pool.live(), 2, "two arrows, two slots");
}

/// **The overflow policy is refuse-the-newest, and it had no gate at all.**
///
/// `TRACERS`' own doc states it — *"the overflow policy is to refuse the
/// newest tracer — never to steal a live one, because a streak that vanishes
/// mid-flight reads as a bug where a missing one reads as nothing at all"* —
/// and until this test that was a comment. Wall 4 asks for a cap with a
/// stated overflow policy; a stated policy nothing checks is the mood the
/// walls list warns about.
///
/// Mutant: make `free()` fall back to slot 0 when full → the 17th volley
/// steals a live flight, `live()` stays at `TRACERS` and the returned bool
/// says it was claimed → red on the bool.
#[test]
fn a_full_pool_refuses_the_newest_rather_than_stealing() {
    let mut pool = Tracers::default();
    let (speed, drop) = ARROW;
    for n in 0..TRACERS {
        assert!(
            pool.claim(FEET, 0, 0, speed, drop),
            "slot {n} of {TRACERS} was refused while the pool still had room"
        );
    }
    assert_eq!(pool.live(), TRACERS, "the pool did not fill");
    assert!(
        !pool.claim(FEET, 0, 0, speed, drop),
        "the {}th arrow was accepted into a pool of {TRACERS} - something was \
         stolen, and a streak that vanishes mid-flight reads as a bug",
        TRACERS + 1
    );
    assert_eq!(
        pool.live(),
        TRACERS,
        "the pool grew past its own cap, or a live flight was overwritten"
    );
}
