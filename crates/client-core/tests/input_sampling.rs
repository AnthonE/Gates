//! The render loop samples input; the sim consumes it at a fixed 30 Hz. This
//! gate is about the gap between those two rates.
//!
//! **The defect it exists to catch**: `set_input` overwrites, so before
//! `ClientCore::sticky_buttons` the only render frame that reached the sim was
//! whichever happened to fall last before a tick boundary. Every press that
//! began and ended between two ticks was dropped, and the faster the client
//! rendered the likelier that was — a frame at 400 fps is 2.5 ms and the tick
//! that would carry it is 33 ms away.
//!
//! Measured on the tree that had the defect: a normal ~50 ms click survived at
//! 100 % on every framerate, which is exactly why it went unnoticed for the
//! life of the client. A press lasting one render frame survived at 100 % at
//! 30 fps, 50 % at 60, and above that only by luck of phase.
//!
//! Both directions are gated, because over-delivering is its own bug: a tap
//! that arrived as a two-tick hold would be a second swing nobody asked for.

use client_core::core::ClientCore;
use sim_core::input::{BTN_JUMP, BTN_PRIMARY};

/// Every framerate a player might actually run at, including the ones that
/// broke it. 30 is the floor the cap ladder offers, 400 stands in for
/// "uncapped on a fast box".
const FRAMERATES: [u32; 6] = [30, 60, 120, 144, 240, 400];

/// Drive `fps` render frames a second, holding `btn` down for exactly
/// `press_frames` frames out of every `every_frames`. Returns
/// `(taps issued, taps that reached at least one tick, ticks that carried it)`.
fn tap(fps: u32, press_frames: u32, every_frames: u32, seconds: f64, btn: u8) -> (u32, u32, u32) {
    let mut c = ClientCore::new(0xBEEF, 1, 0);
    let dt = 1000.0 / fps as f64;
    let frames = (fps as f64 * seconds) as u32;
    let (mut taps, mut reached, mut ticks) = (0u32, 0u32, 0u32);
    let mut credited = false;
    // Counted by SEQ rather than by tail length: the tail is capped at
    // `MAX_INPUT_FRAMES` and drops oldest, so a "what is new since last
    // frame" slice stops seeing anything once it saturates. The first
    // version of this probe did that and reported a normal click surviving
    // 15 % of the time — a broken measurement, not a broken game.
    let mut seen: Option<u16> = None;
    for f in 0..frames {
        let phase = f % every_frames;
        if phase == 0 {
            taps += 1;
            credited = false;
        }
        let down = phase < press_frames;
        c.set_input(if down { btn } else { 0 }, 0, 128, 0, 0, 0);
        c.advance(dt);
        for fr in c.predict.tail() {
            let fresh = seen.is_none_or(|s| fr.seq != s && fr.seq.wrapping_sub(s) < 0x8000);
            if fresh && fr.buttons & btn != 0 {
                ticks += 1;
                if !credited {
                    reached += 1;
                    credited = true;
                }
            }
        }
        if let Some(last) = c.predict.tail().last() {
            seen = Some(last.seq);
        }
    }
    (taps, reached, ticks)
}

/// A press lasting a single render frame must reach the sim at every
/// framerate — this is the fix, stated as an assertion.
#[test]
fn a_one_frame_tap_survives_at_every_framerate() {
    for fps in FRAMERATES {
        let (taps, reached, _) = tap(fps, 1, (fps / 4).max(2), 5.0, BTN_JUMP);
        assert!(
            taps > 10,
            "the fixture must issue taps ({taps} at {fps} fps)"
        );
        assert_eq!(
            reached, taps,
            "at {fps} fps only {reached} of {taps} one-frame presses reached a \
             tick — input is being sampled at the tick boundary instead of \
             latched between them, so a fast click vanishes"
        );
    }
}

/// …and must reach it exactly ONCE. A tap delivered as a two-tick hold is a
/// swing the player did not ask for; the cooldown hides it for a gather but
/// nothing hides it for a verb that lands per tick.
#[test]
fn a_tap_is_one_tick_of_the_button_not_two() {
    for fps in FRAMERATES {
        let (taps, _, ticks) = tap(fps, 1, (fps / 4).max(2), 5.0, BTN_PRIMARY);
        assert_eq!(
            ticks, taps,
            "at {fps} fps {taps} one-frame presses became {ticks} ticks of the \
             button — the latch is being replayed rather than consumed"
        );
    }
}

/// A genuinely HELD button must still read as held, at every framerate. The
/// latch supplies a bit that was released before anyone looked; it must not
/// change what a hold means, or sprint would stutter.
#[test]
fn a_held_button_is_still_held() {
    for fps in FRAMERATES {
        let mut c = ClientCore::new(0xBEEF, 1, 0);
        let dt = 1000.0 / fps as f64;
        let mut seen: Option<u16> = None;
        let (mut with, mut total) = (0u32, 0u32);
        for _ in 0..(fps as f64 * 2.0) as u32 {
            c.set_input(BTN_JUMP, 0, 128, 0, 0, 0);
            c.advance(dt);
            for fr in c.predict.tail() {
                if seen.is_none_or(|s| fr.seq != s && fr.seq.wrapping_sub(s) < 0x8000) {
                    total += 1;
                    if fr.buttons & BTN_JUMP != 0 {
                        with += 1;
                    }
                }
            }
            if let Some(last) = c.predict.tail().last() {
                seen = Some(last.seq);
            }
        }
        assert!(total > 30, "ticks ran at {fps} fps ({total})");
        assert_eq!(
            with, total,
            "at {fps} fps a continuously held button reached only {with} of \
             {total} ticks"
        );
    }
}

/// The tick rate is what decides this, not the frame rate — so the number of
/// input frames produced must be the same however fast the client draws.
///
/// It is the other half of "regulated input": the fix above must not have
/// turned the render rate into a *sim* rate, which would put a fast box on a
/// different clock from a slow one and make the cap a gameplay setting.
#[test]
fn the_sim_tick_count_is_independent_of_the_frame_rate() {
    let mut counts = Vec::new();
    for fps in FRAMERATES {
        let mut c = ClientCore::new(0xBEEF, 1, 0);
        let dt = 1000.0 / fps as f64;
        let mut steps = 0u32;
        for _ in 0..(fps as f64 * 3.0) as u32 {
            c.set_input(0, 0, 128, 0, 0, 0);
            steps += c.advance(dt);
        }
        counts.push((fps, steps));
    }
    // Three seconds at 30 Hz. The clock's own rounding gives a frame or two
    // either way; a framerate-coupled sim would be off by multiples.
    for (fps, steps) in &counts {
        assert!(
            (88..=92).contains(steps),
            "at {fps} fps, 3 s produced {steps} sim ticks — expected ~90, so \
             the sim clock is following the frame rate: {counts:?}"
        );
    }
}
