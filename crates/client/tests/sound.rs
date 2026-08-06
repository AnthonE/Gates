//! The audio model's gate. **Code tier**: no `render` feature, no GPU, no
//! sound card, no clock.
//!
//! This is the whole of what watches `crate::sound`, and it is deliberately
//! the same posture `tests/ui.rs` has: a mixer and a synthesizer are pure
//! arithmetic, so the thing that checks them must not need a window. What it
//! cannot see is whether the bank sounds like anything — `NOW.md` carries that
//! as the honest gap, and `CLAUDE.md`'s beige-smear entry is why it is written
//! down rather than papered over with a statistic.
//!
//! The synthesis assertions are **structural before statistical**, which is
//! the rule the same entry states for frames: a cue is checked for being a
//! sound at all (energy, no clipping, no click at either end, a continuous
//! loop seam) before any number about it is read.

use client::sound::mixer::{Mixer, Request};
use client::sound::steps::{surface_cue, Steps, STRIDE_M};
use client::sound::synth;
use client::sound::{
    Bus, Cue, Mix, CUES, CUE_COUNT, CUE_QUEUE_CAP, MAX_AUDIBLE_M, SAMPLE_RATE, STARTS_PER_FRAME,
    VOICE_CAP,
};

const AT_ORIGIN: [f32; 3] = [0.0, 0.0, 0.0];

// ---------------------------------------------------------------------------
// The table itself.
// ---------------------------------------------------------------------------

/// The one mistake that would silently shift every table in the module by one:
/// a cue added to the enum and forgotten in `Cue::ALL`.
#[test]
fn cue_table_is_dense_and_in_order() {
    assert_eq!(Cue::ALL.len(), CUE_COUNT, "Cue::ALL is not CUE_COUNT long");
    assert_eq!(CUES.len(), CUE_COUNT, "CUES is not CUE_COUNT long");
    for (i, cue) in Cue::ALL.iter().enumerate() {
        assert_eq!(
            cue.idx(),
            i,
            "{cue:?} sits at index {i} of Cue::ALL but its discriminant is {}",
            cue.idx()
        );
    }
}

/// `MAX_AUDIBLE_M` is read by `render/audio.rs` to pick the spatial scale that
/// clamps rodio's own inverse-square law out. If a cue ever carries further
/// than it, that cue falls off the far side of the clamp and gets LOUDER with
/// distance — so the two must not drift.
#[test]
fn no_cue_carries_past_max_audible() {
    for cue in Cue::ALL {
        let d = cue.def();
        assert!(
            d.radius_m <= MAX_AUDIBLE_M,
            "{cue:?} carries {} m, past MAX_AUDIBLE_M = {MAX_AUDIBLE_M}",
            d.radius_m
        );
    }
    // And the scale `render/audio.rs` picks must actually hold, asserted here
    // because float comparison is not const-evaluable at the definition site.
    let spatial_scale = 1.0f32 / 128.0;
    let ear_gap = 0.22f32;
    assert!(spatial_scale * (MAX_AUDIBLE_M + ear_gap) < 1.0);
}

/// Signal cues must not wobble: a symbol that changes pitch every time takes
/// longer to learn than one that does not.
#[test]
fn interface_cues_do_not_vary_in_pitch() {
    for cue in [Cue::Hit, Cue::CraftDone, Cue::Refused, Cue::UiClick] {
        assert_eq!(
            cue.pitch_var(),
            0.0,
            "{cue:?} is a symbol and must not drift"
        );
    }
    for cue in [Cue::StepGrass, Cue::ImpactWood] {
        assert!(cue.pitch_var() > 0.0, "{cue:?} is diegetic and should vary");
    }
}

/// **A cue that cannot vary in pitch and cannot vary in position must not be
/// able to start twice in one frame.** Identical samples summed in a mixer
/// ADD rather than blend — four in-phase copies of the 2 kHz hitmarker is four
/// times the amplitude, straight into the limiter — and neither pitch jitter
/// nor a different emitter position is there to decorrelate them. So a
/// non-positional, non-varying cue needs a non-zero cooldown, and this is the
/// rule rather than four remembered special cases.
///
/// `Death` is the exemption and it states its own reason: you die once.
#[test]
fn stackable_cues_carry_a_cooldown() {
    for cue in Cue::ALL {
        let d = cue.def();
        if d.positional || cue.pitch_var() > 0.0 || cue == Cue::Death || cue == Cue::BedWind {
            continue;
        }
        assert!(
            d.cooldown_ms > 0,
            "{cue:?} is non-positional with no pitch variation and no cooldown - \
             two of them in one frame are the same waveform at twice the amplitude"
        );
    }
}

// ---------------------------------------------------------------------------
// Falloff and the mix.
// ---------------------------------------------------------------------------

/// The property the whole cull rests on: the law is exactly zero at the
/// radius, so a source crossing it fades rather than clicking.
#[test]
fn falloff_reaches_zero_at_the_radius() {
    use client::sound::falloff;
    assert_eq!(falloff(0.0, 40.0), 1.0);
    assert_eq!(falloff(40.0, 40.0), 0.0);
    assert_eq!(falloff(41.0, 40.0), 0.0);
    // Monotone all the way in.
    let mut prev = 1.1f32;
    for i in 0..=40 {
        let g = falloff(i as f32, 40.0);
        assert!(g < prev, "falloff rose at {i} m");
        prev = g;
    }
    // A zero radius is a cue that carries nowhere, not one that carries
    // everywhere — the difference between a silent bug and a deafening one.
    assert_eq!(falloff(0.0, 0.0), 0.0);
}

#[test]
fn master_scales_every_bus_and_silences_at_zero() {
    let mix = Mix {
        master: 0.5,
        game: 0.5,
        ambience: 1.0,
    };
    assert_eq!(mix.bus_gain(Bus::Game), 0.25);
    assert_eq!(mix.bus_gain(Bus::Ambience), 0.5);
    let silent = Mix {
        master: 0.0,
        ..Mix::default()
    };
    assert_eq!(silent.bus_gain(Bus::Game), 0.0);
    assert_eq!(silent.bus_gain(Bus::Ambience), 0.0);
}

/// A muted bus must not merely be inaudible — it must not consume a voice,
/// because a player who turned the sound down has not agreed to keep paying
/// `VOICE_CAP` for it.
#[test]
fn a_silenced_master_starts_nothing() {
    let mut m = Mixer::new();
    let mix = Mix {
        master: 0.0,
        ..Mix::default()
    };
    for _ in 0..4 {
        m.push(Request::own(Cue::Hit));
    }
    assert!(m.tick(16.0, AT_ORIGIN, 0, &mix).is_empty());
}

// ---------------------------------------------------------------------------
// The mixer's four bounds.
// ---------------------------------------------------------------------------

#[test]
fn frame_budget_is_a_hard_cap() {
    let mut m = Mixer::new();
    let mix = Mix::default();
    // Distinct cues, because one cue would be stopped by its own cooldown and
    // this is testing the budget, not the cooldown.
    for cue in [
        Cue::Hit,
        Cue::Gather,
        Cue::CraftDone,
        Cue::UiClick,
        Cue::Refused,
        Cue::Hurt,
        Cue::Swing,
    ] {
        m.push(Request::own(cue));
    }
    let n = m.tick(16.0, AT_ORIGIN, 0, &mix).len();
    assert_eq!(n, STARTS_PER_FRAME, "the frame budget was not the cap");
}

#[test]
fn the_voice_cap_refuses_rather_than_steals() {
    let mut m = Mixer::new();
    let mix = Mix::default();
    for cue in [Cue::Hit, Cue::Gather, Cue::CraftDone, Cue::UiClick] {
        m.push(Request::own(cue));
    }
    // The pool is full: nothing starts, and nothing already sounding is asked
    // to stop (the mixer cannot stop anything — that is the point).
    assert!(m.tick(16.0, AT_ORIGIN, VOICE_CAP, &mix).is_empty());

    // One slot free: exactly one starts.
    let mut m = Mixer::new();
    for cue in [Cue::Hit, Cue::Gather, Cue::CraftDone, Cue::UiClick] {
        m.push(Request::own(cue));
    }
    assert_eq!(m.tick(16.0, AT_ORIGIN, VOICE_CAP - 1, &mix).len(), 1);
}

#[test]
fn the_queue_is_bounded_and_says_so() {
    let mut m = Mixer::new();
    for _ in 0..(CUE_QUEUE_CAP + 9) {
        m.push(Request::own(Cue::Hit));
    }
    assert_eq!(m.queued(), CUE_QUEUE_CAP, "the queue grew past its cap");
    assert_eq!(m.dropped, 9, "dropped requests were not counted");
}

#[test]
fn a_cue_on_cooldown_does_not_retrigger() {
    let mut m = Mixer::new();
    let mix = Mix::default();
    let cool = Cue::StepGrass.def().cooldown_ms as f32;

    m.push(Request::own(Cue::StepGrass));
    assert_eq!(m.tick(16.0, AT_ORIGIN, 0, &mix).len(), 1);
    // Immediately again: refused.
    m.push(Request::own(Cue::StepGrass));
    assert!(m.tick(16.0, AT_ORIGIN, 0, &mix).is_empty());
    // After the cooldown has run down: allowed.
    m.push(Request::own(Cue::StepGrass));
    assert_eq!(m.tick(cool, AT_ORIGIN, 0, &mix).len(), 1);
}

// ---------------------------------------------------------------------------
// The ordering that makes the budget mean something.
// ---------------------------------------------------------------------------

/// The bug this ordering exists to prevent: cull BEFORE budget, or a hundred
/// inaudible cues from across the island spend the frame and silence the axe
/// in your hands.
#[test]
fn distant_cues_do_not_spend_the_budget() {
    let mut m = Mixer::new();
    let mix = Mix::default();
    let far = [10_000.0, 0.0, 0.0];
    for _ in 0..CUE_QUEUE_CAP - 1 {
        m.push(Request::at(Cue::TreeFall, far));
    }
    m.push(Request::own(Cue::Swing));
    let starts = m.tick(16.0, AT_ORIGIN, 0, &mix);
    assert_eq!(starts.len(), 1, "an inaudible crowd took the frame");
    assert_eq!(starts[0].cue, Cue::Swing);
}

/// When the budget does bind, the loud-and-important survive.
#[test]
fn priority_decides_who_is_heard() {
    let mut m = Mixer::new();
    let mix = Mix::default();
    // Six own-facts, four slots. Death (8) and Hurt (7) outrank the rest.
    for cue in [
        Cue::StepRock,
        Cue::Swing,
        Cue::Death,
        Cue::UiClick,
        Cue::Hurt,
        Cue::Gather,
    ] {
        m.push(Request::own(cue));
    }
    let heard: Vec<Cue> = m
        .tick(16.0, AT_ORIGIN, 0, &mix)
        .iter()
        .map(|s| s.cue)
        .collect();
    assert_eq!(heard.len(), STARTS_PER_FRAME);
    assert_eq!(heard[0], Cue::Death, "death did not win the frame");
    assert_eq!(heard[1], Cue::Hurt);
    assert!(
        !heard.contains(&Cue::StepRock),
        "a footstep outranked something that mattered"
    );
}

/// Ties on priority break on distance, so the near tree is the one you hear
/// first — and when the budget bites, the far one is what it drops.
#[test]
fn nearer_wins_a_tie() {
    let mut m = Mixer::new();
    let mix = Mix::default();
    // Pushed far-first on purpose: queue order is the LAST tie-break, so a
    // near source arriving third must still be chosen first.
    for d in [80.0f32, 8.0, 40.0] {
        m.push(Request::at(Cue::TreeFall, [d, 0.0, 0.0]));
    }
    let gains: Vec<f32> = m
        .tick(16.0, AT_ORIGIN, 0, &mix)
        .iter()
        .map(|s| s.gain)
        .collect();
    // `TreeFall` has no cooldown — a forest being cleared is meant to be
    // audible — so all three fit inside the frame budget, in distance order.
    assert_eq!(gains.len(), 3);
    assert!(
        gains[0] > gains[1] && gains[1] > gains[2],
        "the trees were not chosen nearest-first: {gains:?}"
    );
}

/// A cooldown that only bound between frames would let four footsteps
/// requested in one frame all start together — which is a 90 ms cooldown that
/// three of them never had to pass. This is the assertion that found it.
#[test]
fn a_cooldown_binds_within_one_frame() {
    let mut m = Mixer::new();
    let mix = Mix::default();
    for _ in 0..4 {
        m.push(Request::own(Cue::StepGrass));
    }
    assert_eq!(
        m.tick(16.0, AT_ORIGIN, 0, &mix).len(),
        1,
        "a cue with a cooldown started more than once in one frame"
    );
    // And they were refused by the cooldown, not starved of a voice: the pool
    // was empty and the frame budget was untouched.
    assert_eq!(m.starved, 0, "a cooldown refusal was counted as starvation");
}

/// The reference's own bug, refused rather than reproduced: placement effects
/// that fired at the world origin instead of at the socket
/// (`reference/AUDIO.md` §6). A positional cue with no position is a caller
/// bug, and playing it at (0,0,0) would be a lie about where it happened.
#[test]
fn a_positional_cue_with_no_position_is_refused() {
    let mut m = Mixer::new();
    let mix = Mix::default();
    m.push(Request::own(Cue::TreeFall));
    assert!(m.tick(16.0, AT_ORIGIN, 0, &mix).is_empty());
    assert_eq!(m.dropped, 1, "the caller bug was not counted");
}

/// Two runs over identical frames must pick identical voices at identical
/// rates. Not a nicety: it is what makes every assertion above reproducible,
/// and it is the property the pitch variation could most easily have broken.
#[test]
fn the_mixer_is_deterministic() {
    let run = || {
        let mut m = Mixer::new();
        let mix = Mix::default();
        let mut log = Vec::new();
        for frame in 0..60 {
            m.push(Request::own(Cue::StepGrass));
            m.push(Request::at(Cue::ImpactWood, [frame as f32, 0.0, 3.0]));
            for s in m.tick(16.0, AT_ORIGIN, 0, &mix) {
                log.push((s.cue, s.gain.to_bits(), s.speed.to_bits()));
            }
        }
        log
    };
    let a = run();
    let b = run();
    assert!(!a.is_empty(), "the deterministic run heard nothing");
    assert_eq!(a, b, "two identical runs produced different voices");
}

/// Pitch variation must stay well clear of zero — a rate at or below zero is
/// a voice that never ends and holds a `VOICE_CAP` slot for the process's life.
#[test]
fn playback_rates_stay_sane() {
    let mut m = Mixer::new();
    let mix = Mix::default();
    let mut seen = 0;
    for _ in 0..400 {
        m.push(Request::own(Cue::StepLitter));
        for s in m.tick(200.0, AT_ORIGIN, 0, &mix) {
            assert!(
                s.speed > 0.25 && s.speed < 4.0,
                "playback rate {} is out of range",
                s.speed
            );
            assert!(
                s.gain > 0.0 && s.gain <= 1.0,
                "gain {} out of range",
                s.gain
            );
            seen += 1;
        }
    }
    assert!(seen > 100, "only {seen} voices started over 400 frames");
}

// ---------------------------------------------------------------------------
// Footsteps.
// ---------------------------------------------------------------------------

#[test]
fn the_surface_comes_from_the_dominant_splat_channel() {
    // `terrain::splat`'s channel order: [sand, grass, forest floor, rock].
    assert_eq!(surface_cue([200, 30, 20, 5], false), Cue::StepSand);
    assert_eq!(surface_cue([30, 200, 20, 5], false), Cue::StepGrass);
    assert_eq!(surface_cue([30, 20, 200, 5], false), Cue::StepLitter);
    assert_eq!(surface_cue([30, 20, 5, 200], false), Cue::StepRock);
    // Below the waterline the ground does not get a vote.
    assert_eq!(surface_cue([30, 20, 5, 200], true), Cue::StepWater);
}

#[test]
fn cadence_is_distance_not_time() {
    let mut s = Steps::default();
    // The first sample only establishes the origin.
    assert!(s.sample([0.0, 0.0, 0.0], true, 0.016).is_none());
    // Walk one stride in one-tenth-stride hops at a walking pace.
    let hop = STRIDE_M / 10.0;
    let dt = hop / 3.0; // 3 m/s, comfortably over STEP_MIN_SPEED
    let mut steps = 0;
    for i in 1..=40 {
        if s.sample([hop * i as f32, 0.0, 0.0], true, dt).is_some() {
            steps += 1;
        }
    }
    assert_eq!(steps, 4, "four strides of ground did not make four steps");
}

#[test]
fn standing_still_and_falling_are_both_silent() {
    let mut s = Steps::default();
    s.sample([0.0, 0.0, 0.0], true, 0.016);
    // Standing: under the minimum speed.
    for _ in 0..120 {
        assert!(s.sample([0.001, 0.0, 0.0], true, 0.016).is_none());
    }
    // Airborne across four metres, then landing: no banked burst.
    let mut s = Steps::default();
    s.sample([0.0, 0.0, 0.0], false, 0.016);
    for i in 1..=40 {
        assert!(
            s.sample([0.1 * i as f32, 5.0, 0.0], false, 0.016).is_none(),
            "a jump made a footstep"
        );
    }
    assert!(
        s.sample([4.05, 0.0, 0.0], true, 0.016).is_none(),
        "landing released a banked step"
    );
}

/// A hitch that covered ten metres must not buy eleven footsteps once the
/// frames come back.
#[test]
fn a_frame_hitch_does_not_buy_a_burst() {
    let mut s = Steps::default();
    s.sample([0.0, 0.0, 0.0], true, 0.016);
    assert!(s.sample([10.0, 0.0, 0.0], true, 0.5).is_some());
    // At most one stride is banked, so a stationary frame after it produces
    // nothing (it is under the speed floor) and one more stride produces one.
    let mut extra = 0;
    for i in 1..=4 {
        if s.sample([10.0 + 0.2 * i as f32, 0.0, 0.0], true, 0.05)
            .is_some()
        {
            extra += 1;
        }
    }
    assert!(extra <= 1, "a hitch banked {extra} extra steps");
}

/// A new world, a respawn or a teleport must not be measured as ground
/// covered.
#[test]
fn reset_forgets_the_previous_world() {
    let mut s = Steps::default();
    s.sample([0.0, 0.0, 0.0], true, 0.016);
    s.reset();
    assert!(
        s.sample([900.0, 0.0, 900.0], true, 0.016).is_none(),
        "a teleport was heard as a footstep"
    );
}

// ---------------------------------------------------------------------------
// The generated bank. Structural first, statistical second.
// ---------------------------------------------------------------------------

/// Decode our own WAV back to samples. A parser this small is worth having in
/// the test rather than a dependency: it also asserts the header we wrote is
/// the header we meant.
fn pcm(wav: &[u8]) -> Vec<f32> {
    assert_eq!(&wav[0..4], b"RIFF", "not a RIFF file");
    assert_eq!(&wav[8..12], b"WAVE", "not a WAVE file");
    assert_eq!(&wav[12..16], b"fmt ", "no fmt chunk");
    assert_eq!(u16::from_le_bytes([wav[20], wav[21]]), 1, "not PCM");
    assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1, "not mono");
    let sr = u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]);
    assert_eq!(sr, SAMPLE_RATE, "sample rate is not the bank's");
    assert_eq!(u16::from_le_bytes([wav[34], wav[35]]), 16, "not 16-bit");
    assert_eq!(&wav[36..40], b"data", "no data chunk");
    let n = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as usize;
    assert_eq!(
        wav.len(),
        44 + n,
        "the data chunk length lies about the file"
    );
    wav[44..]
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / i16::MAX as f32)
        .collect()
}

fn rms(s: &[f32]) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
}

/// Every cue must be a sound: it exists, it has energy, it does not clip, and
/// it neither begins nor ends on a step discontinuity — which is what a click
/// is.
#[test]
fn every_cue_is_a_sound() {
    for cue in Cue::ALL {
        let wav = synth::wav(cue);
        let s = pcm(&wav);
        assert!(
            s.len() > SAMPLE_RATE as usize / 100,
            "{cue:?} is {} samples - under 10 ms",
            s.len()
        );
        let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.5, "{cue:?} peaks at {peak} - it is near silence");
        assert!(peak <= 1.0, "{cue:?} peaks at {peak} - it clips");
        assert!(rms(&s) > 0.01, "{cue:?} has no sustained energy");
        // The head and the tail must be near zero, or the sample clicks at
        // both ends of every single playback. **Except the bed**, whose first
        // sample must be continuous with its LAST rather than with silence —
        // `the_bed_loops_without_a_seam` is its version of this check.
        if cue == Cue::BedWind {
            continue;
        }
        assert!(
            s[0].abs() < 0.05,
            "{cue:?} starts at {} - that is a click",
            s[0]
        );
        assert!(
            s[s.len() - 1].abs() < 0.05,
            "{cue:?} ends at {} - that is a click",
            s[s.len() - 1]
        );
    }
}

/// Same build, same bytes. The property that makes the bank a fixed thing to
/// reason about — and NOT a byte golden, which would fail on every tuning pass
/// while proving nothing about whether the sound is a sound (`CLAUDE.md`).
#[test]
fn the_bank_is_deterministic() {
    for cue in Cue::ALL {
        assert_eq!(
            synth::wav(cue),
            synth::wav(cue),
            "{cue:?} generated differently on two runs"
        );
    }
    // And one cue's noise must be its own: a shared PRNG stream would make
    // every cue shift when a new one was added ahead of it.
    assert_ne!(
        synth::wav(Cue::StepGrass),
        synth::wav(Cue::StepLitter),
        "two cues generated identical samples"
    );
}

/// The bed loops forever, so its seam is heard more than any other sample in
/// the game. Continuity across the join is the assertion.
#[test]
fn the_bed_loops_without_a_seam() {
    let s = pcm(&synth::wav(Cue::BedWind));
    let n = s.len();
    assert!(
        n > SAMPLE_RATE as usize * 4,
        "the bed is under four seconds"
    );
    // The join: the last sample flows into the first. A step here is the
    // click that would be heard once every loop, forever.
    let step = (s[0] - s[n - 1]).abs();
    assert!(step < 0.25, "the bed's loop point steps by {step}");
    // And the energy must not dip across the join, which is what a LINEAR
    // crossfade of two uncorrelated noise signals does (~3 dB) and what the
    // equal-power pair in `loop_seam` exists to prevent. Compare the window
    // spanning the join against the bed's own middle.
    let w = SAMPLE_RATE as usize / 10;
    let mut join: Vec<f32> = s[n - w..].to_vec();
    join.extend_from_slice(&s[..w]);
    let middle = rms(&s[n / 2 - w..n / 2 + w]);
    let ratio = rms(&join) / middle;
    assert!(
        ratio > 0.6 && ratio < 1.7,
        "the loop join is {ratio:.2}x the bed's own level - the crossfade is audible"
    );
}

/// The bed must actually gust, and this assertion exists because the first one
/// did not.
///
/// Its LFOs were at 0.071 and 0.113 Hz — periods of 14 and 8.8 seconds, both
/// longer than the 5.25 s loop — so the loop ended before the first gust
/// finished and the bed was a flat hiss. **Every other assertion in this file
/// passed on it**: it had energy, it did not clip, and its seam was continuous.
/// Only a waveform plot showed it, which is `CLAUDE.md`'s beige-smear entry
/// exactly: a statistic cannot see whether the thing is a picture of anything.
/// This is that finding turned back into a gate.
#[test]
fn the_bed_gusts() {
    let s = pcm(&synth::wav(Cue::BedWind));
    // Short-term level, in half-second windows across the whole loop.
    let w = SAMPLE_RATE as usize / 2;
    let levels: Vec<f32> = s.chunks(w).filter(|c| c.len() == w).map(rms).collect();
    assert!(levels.len() >= 8, "the bed is too short to gust");
    let lo = levels.iter().cloned().fold(f32::MAX, f32::min);
    let hi = levels.iter().cloned().fold(0.0f32, f32::max);
    // Bounded from BOTH sides, and the second bound is the same lesson as the
    // first: the fix for the flat hiss overshot to a 5.4x swing, which does
    // not read as weather — it reads as the ambience cutting out once every
    // ten seconds. Wind gusts; it does not stop.
    let swing = hi / lo;
    assert!(
        swing > 1.35,
        "the bed's level only moves {swing:.2}x across its length - it is a flat hiss, not wind"
    );
    assert!(
        swing < 4.0,
        "the bed's level moves {swing:.2}x - that is the ambience dropping out, not a gust"
    );
}

/// A footstep is a transient: most of its energy is in the first third. A
/// sample whose energy is flat across its length is a hiss, not a step, and
/// that is the failure mode a "does it have energy" check cannot see.
#[test]
fn footsteps_are_transients() {
    for cue in [
        Cue::StepSand,
        Cue::StepGrass,
        Cue::StepLitter,
        Cue::StepRock,
    ] {
        let s = pcm(&synth::wav(cue));
        let third = s.len() / 3;
        let head = rms(&s[..third]);
        let tail = rms(&s[third * 2..]);
        assert!(
            head > tail * 2.0,
            "{cue:?} has a flat envelope (head {head:.4}, tail {tail:.4}) - it is a hiss"
        );
    }
}

/// The surfaces must actually differ in brightness, or five footstep cues are
/// one footstep cue with five names. Rock returns the top end; sand absorbs
/// it. Measured as a zero-crossing rate, which is the cheapest honest proxy
/// for spectral centroid and needs no FFT.
#[test]
fn the_surfaces_differ_in_timbre() {
    let zcr = |cue: Cue| {
        let s = pcm(&synth::wav(cue));
        let n = s
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        n as f32 / s.len() as f32
    };
    let sand = zcr(Cue::StepSand);
    let rock = zcr(Cue::StepRock);
    assert!(
        rock > sand * 1.5,
        "rock ({rock:.4}) is not brighter than sand ({sand:.4}) - the surfaces are the same sound"
    );
}

// ---------------------------------------------------------------------------
// The one-drain rule, as a gate. Source-scanning, because the thing being
// protected is a *call site*, not a value.
// ---------------------------------------------------------------------------

/// **`render::feed::drain` must be the only caller of `ClientCore::pop_*` in
/// the client**, and this test exists because the alternative already
/// happened.
///
/// `audio::feed` and `hud::feedback` were written on two branches, each
/// popping the core's own-fact rings, each correct alone. The rings are
/// DESTRUCTIVE — every fact is handed over exactly once — so the merge, which
/// had no textual conflict and broke no test, produced a client whose HUD
/// drained every ring before the mixer saw one and a game that made no sound
/// for a hit, a gather, a craft or a refusal.
///
/// A grep is the right instrument here: the defect is not a wrong value, it is
/// a second call site, and no amount of unit testing either half can see it.
/// This runs in the code tier and scans `src/render/` as text, so it does not
/// need the `render` feature to hold the rule that lives behind it.
#[test]
fn only_the_feed_drain_pops_the_core() {
    use std::path::Path;

    fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
        for e in std::fs::read_dir(dir).expect("src/render must exist") {
            let p = e.expect("readable entry").path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&p).expect("readable source");
                out.push((p.display().to_string(), text));
            }
        }
    }

    let mut files = Vec::new();
    walk(Path::new("src/render"), &mut files);
    assert!(
        files.len() > 10,
        "only found {} render sources",
        files.len()
    );

    // The destructive verbs. `pop_chat` is deliberately NOT here: chat is a
    // single-reader surface by nature (one composer) and `render/chat.rs` owns
    // it — if a second reader ever wants it, it joins the feed and joins this
    // list in the same commit.
    const DESTRUCTIVE: [&str; 7] = [
        "pop_hit(",
        "pop_death(",
        "pop_toast(",
        "pop_craft_toast(",
        "pop_craft_refusal(",
        "pop_build_refusal(",
        "pop_deploy_refusal(",
    ];

    let mut offenders = Vec::new();
    for (path, text) in &files {
        if path.ends_with("feed.rs") {
            continue;
        }
        for (n, line) in text.lines().enumerate() {
            // Doc comments and ordinary comments may name these freely — the
            // rule is about calls, and every one of them mentions the rule.
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for verb in DESTRUCTIVE {
                if code.contains(verb) {
                    offenders.push(format!("{path}:{}: {}", n + 1, code.trim()));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the core's own-fact rings are destructive and `render::feed::drain` must be their \
         only reader - these call sites would silently split the events with it:\n  {}",
        offenders.join("\n  ")
    );

    // And the drain must actually still be there, or this test passes by
    // asserting that nobody reads the events at all.
    let drain = files
        .iter()
        .find(|(p, _)| p.ends_with("feed.rs"))
        .map(|(_, t)| t)
        .expect("render/feed.rs must exist");
    for verb in DESTRUCTIVE {
        assert!(
            drain.contains(verb),
            "render/feed.rs no longer calls {verb} - the drain has stopped draining"
        );
    }
}
