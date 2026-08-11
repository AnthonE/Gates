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

use client::sound::birds::Birds;
use client::sound::mixer::{Mixer, Request};
use client::sound::music::{self, Director, Mode, Piece, Tier};
use client::sound::steps::{remote, surface_cue, Steps, STRIDE_M};
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
///
/// **The other exemption is `mixer_started`, and it was written as `is_bed`
/// until the music landed** — which is the same rule stated too narrowly. The
/// hazard here is *two starts in one frame*, and the only thing that can
/// start two of anything in a frame is the mixer; a cue it refuses outright
/// (`Cue::mixer_started` — the beds and the nine music pieces) is owned by a
/// render system that holds exactly one voice, and `music::Director::tick`
/// returns at most one piece per call by construction.
#[test]
fn stackable_cues_carry_a_cooldown() {
    for cue in Cue::ALL {
        let d = cue.def();
        if d.positional || cue.pitch_var() > 0.0 || cue == Cue::Death || !cue.mixer_started() {
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
        music: 0.2,
    };
    assert_eq!(mix.bus_gain(Bus::Game), 0.25);
    assert_eq!(mix.bus_gain(Bus::Ambience), 0.5);
    assert_eq!(mix.bus_gain(Bus::Music), 0.1);
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
// Remote footsteps (`NOW.md` §0x item 6): the local family's positional
// twins. The odometer half is the SAME `Steps` gated above — a remote body
// carries its own instance — so what is new here is the mapping, the table
// row, and the mixer treating the cue as a place in the world.
// ---------------------------------------------------------------------------

/// Every surface has a remote twin, the twin is the same sound byte for
/// byte, and what differs is exactly what should: positional at the body,
/// outranking your own feet, on the step family's own radius.
#[test]
fn a_remote_step_is_the_same_boot_on_the_same_ground() {
    let pairs = [
        (Cue::StepSand, Cue::RemoteStepSand),
        (Cue::StepGrass, Cue::RemoteStepGrass),
        (Cue::StepLitter, Cue::RemoteStepLitter),
        (Cue::StepRock, Cue::RemoteStepRock),
        (Cue::StepWater, Cue::RemoteStepWater),
    ];
    for (local, far) in pairs {
        assert_eq!(remote(local), far, "{local:?} maps to the wrong twin");
        // The ground decides what a step sounds like, not whose boot it is.
        assert_eq!(
            synth::wav(local),
            synth::wav(far),
            "{far:?} is not {local:?}'s own waveform"
        );
        let (l, f) = (local.def(), far.def());
        assert!(f.positional, "{far:?} is an own-fact - it must be a place");
        assert!(!l.positional, "{local:?} grew a position");
        assert_eq!(
            f.radius_m, l.radius_m,
            "the two step families drifted on how far a boot carries"
        );
        assert_eq!(f.gain, l.gain, "the two step families drifted on gain");
        assert!(
            f.priority > l.priority,
            "another player's step is the sound that decides fights - it must \
             outrank your own feet"
        );
        assert!(far.pitch_var() > 0.0, "{far:?} is diegetic and must vary");
    }
    // The mapping is closed over the step family: a non-step cue has no
    // twin and passes through unchanged.
    assert_eq!(remote(Cue::Swing), Cue::Swing);
}

/// The mixer hears a remote step where the body is and not past the radius:
/// the one falloff law is the cull, exactly as it is for the falling tree.
#[test]
fn a_remote_step_is_heard_at_the_body_and_culled_by_distance() {
    let mix = Mix::default();
    // In earshot: one voice, positional, attenuated below the table gain.
    let mut m = Mixer::new();
    m.push(Request::at(Cue::RemoteStepGrass, [8.0, 0.0, 0.0]));
    let starts: Vec<_> = m.tick(16.0, AT_ORIGIN, 0, &mix).to_vec();
    assert_eq!(starts.len(), 1, "a nearby remote step was not heard");
    assert_eq!(
        starts[0].at,
        Some([8.0, 0.0, 0.0]),
        "the step did not play at the body"
    );
    assert!(
        starts[0].gain < Cue::RemoteStepGrass.def().gain,
        "8 m away did not attenuate"
    );
    // Past the radius: no voice at all, not a quiet one — and not counted
    // as starvation, because a cue nobody could hear was never owed a slot.
    let mut m = Mixer::new();
    m.push(Request::at(
        Cue::RemoteStepGrass,
        [Cue::RemoteStepGrass.def().radius_m + 1.0, 0.0, 0.0],
    ));
    assert!(
        m.tick(16.0, AT_ORIGIN, 0, &mix).is_empty(),
        "a step past its radius was heard"
    );
    assert_eq!(m.starved, 0, "a culled step was counted as starvation");
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
        if cue.is_bed() {
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

/// A bed loops forever, so its seam is heard more than any other sample in
/// the game. Continuity across the join is the assertion, and it runs over
/// **every** bed — the wind bed had this gate alone for two lanes, and the
/// surf and submerged beds are generated by different functions that could
/// each get the seam wrong in their own way.
#[test]
fn every_bed_loops_without_a_seam() {
    for cue in Cue::ALL.iter().filter(|c| c.is_bed()) {
        the_bed_loops_without_a_seam(*cue);
    }
}

fn the_bed_loops_without_a_seam(cue: Cue) {
    let s = pcm(&synth::wav(cue));
    let n = s.len();
    assert!(
        n > SAMPLE_RATE as usize * 4,
        "{cue:?} is under four seconds"
    );
    // The join: the last sample flows into the first. A step here is the
    // click that would be heard once every loop, forever.
    let step = (s[0] - s[n - 1]).abs();
    assert!(step < 0.25, "{cue:?}'s loop point steps by {step}");
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
        "{cue:?}'s loop join is {ratio:.2}x its own level - the crossfade is audible"
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
    // Bounded from BOTH sides, and the second bound is the same lesson as the
    // first: the fix for the flat hiss overshot to a 5.4x swing, which does
    // not read as weather — it reads as the ambience cutting out once every
    // ten seconds. Wind gusts; it does not stop.
    swings(Cue::BedWind, 1.35, 4.0);
}

/// Surf is the same finding with a different number. A sea moves **more** than
/// wind does — a break arriving and draining away is most of what surf is, and
/// a bed with a flat level is a hiss with salt in the name. The upper bound is
/// looser than the wind's for the same reason and no looser than that: a surf
/// bed that goes to silence between waves is a bed that switches off.
#[test]
fn the_surf_breaks() {
    swings(Cue::BedSurf, 1.8, 6.0);
}

/// The submerged bed moves least of the three: water over your ears is a
/// pressure, not an event. It still may not be a constant, or it is a test
/// tone.
#[test]
fn the_submerged_bed_is_a_pressure_not_an_event() {
    swings(Cue::BedUnder, 1.15, 3.0);
}

/// Short-term level across a bed's whole loop, in half-second windows.
fn swings(cue: Cue, min: f32, max: f32) {
    let s = pcm(&synth::wav(cue));
    let w = SAMPLE_RATE as usize / 2;
    let levels: Vec<f32> = s.chunks(w).filter(|c| c.len() == w).map(rms).collect();
    assert!(levels.len() >= 8, "{cue:?} is too short to move");
    let lo = levels.iter().cloned().fold(f32::MAX, f32::min);
    let hi = levels.iter().cloned().fold(0.0f32, f32::max);
    let swing = hi / lo;
    assert!(
        swing > min,
        "{cue:?}'s level only moves {swing:.2}x across its length - it is a flat hiss"
    );
    assert!(
        swing < max,
        "{cue:?}'s level moves {swing:.2}x - that is the ambience dropping out"
    );
}

/// **The submerged bed must be measurably darker than the one it replaces.**
/// This is the honest version of "it sounds underwater": a real submerged mix
/// is a steep low-pass and we have no filter node, so what `SNAPSHOTS` can
/// actually promise is that the bed it crossfades to was *generated* dark.
/// Measured as a zero-crossing rate, the same cheap spectral-centroid proxy
/// the five footstep surfaces are separated by.
#[test]
fn the_submerged_bed_is_darker_than_the_air() {
    let zcr = |cue: Cue| {
        let s = pcm(&synth::wav(cue));
        let n = s
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        n as f32 / s.len() as f32
    };
    let under = zcr(Cue::BedUnder);
    let wind = zcr(Cue::BedWind);
    let surf = zcr(Cue::BedSurf);
    assert!(
        under < wind * 0.8,
        "the submerged bed's zero-crossing rate is {under:.4} against the wind's {wind:.4} - \
         it is not darker, it is the same bed"
    );
    assert!(
        under < surf,
        "the submerged bed is brighter than the surf it is heard under"
    );
}

/// A splash is a splash and not a large footstep: it is longer than every
/// footstep in the bank, and it has a droplet tail — energy still arriving
/// well after the burst has decayed.
#[test]
fn a_splash_outlasts_a_footstep() {
    let splash = pcm(&synth::wav(Cue::Splash));
    let step = pcm(&synth::wav(Cue::StepWater));
    assert!(
        splash.len() > step.len() * 2,
        "the splash is {} samples against a water step's {} - it is a loud step",
        splash.len(),
        step.len()
    );
    // The last quarter must still carry something. Without the droplets it is
    // an exponential that reached the noise floor two thirds in.
    let q = splash.len() / 4;
    let head = rms(&splash[..q]);
    let tail = rms(&splash[3 * q..]);
    assert!(
        tail > head * 0.01,
        "the splash's last quarter is {tail:.5} against a head of {head:.5} - no droplets"
    );
    assert!(tail < head, "the splash gets louder as it ends");
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

/// The pig's voice is a snort, not a hiss, and this pins the two things that
/// make it one. **Dark**: the whole cue lives under the 2–5 kHz band the
/// reference carves clear for what matters (`reference/AUDIO.md` §4), so its
/// zero-crossing rate must sit well below a bright transient's — the same
/// proxy the footstep surfaces are separated by. **Double**: a pig snorts in
/// pairs, so the energy must dip between the two exhales and come back —
/// a single decaying burst would be a large footstep with a species name.
#[test]
fn a_snort_is_dark_and_double() {
    let s = pcm(&synth::wav(Cue::Snort));
    let zcr = |s: &[f32]| {
        let n = s
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        n as f32 / s.len() as f32
    };
    let rock = pcm(&synth::wav(Cue::StepRock));
    assert!(
        zcr(&s) < zcr(&rock) * 0.8,
        "the snort ({:.4}) is not darker than a rock step ({:.4}) - it is a hiss, not a breath",
        zcr(&s),
        zcr(&rock)
    );
    // The pair: first exhale, the gap, second exhale (`synth::snort`'s own
    // timeline — bursts at 0.0 s and 0.24 s).
    let at = |t: f32| ((t * SAMPLE_RATE as f32) as usize).min(s.len());
    let first = rms(&s[at(0.02)..at(0.14)]);
    let gap = rms(&s[at(0.18)..at(0.235)]);
    let second = rms(&s[at(0.25)..at(0.38)]);
    assert!(
        first > gap * 1.1,
        "no first exhale: burst {first:.4} against gap {gap:.4}"
    );
    assert!(
        second > gap * 1.1,
        "the snort does not snort twice: second burst {second:.4} against gap {gap:.4}"
    );
    // And the table row is a positional animal call, not an own-fact: a
    // snort with no place would be refused by the mixer (its own gate above).
    let d = Cue::Snort.def();
    assert!(d.positional, "a snort happens at an animal, not to you");
    assert!(
        d.radius_m > 0.0 && d.radius_m < Cue::TreeFall.def().radius_m,
        "the snort carries {} m - past the loudest thing in the forest",
        d.radius_m
    );
    assert!(
        Cue::Snort.pitch_var() > 0.0,
        "a diegetic animal call must vary in pitch"
    );
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

// ---------------------------------------------------------------------------
// Water: snapshots, the surf level, and the waterline.
// ---------------------------------------------------------------------------

/// A bed is never started by the mixer. Asked for as a one-shot it would play
/// its whole 10 s body once, at whatever gain the frame happened to have, on
/// top of the looping entity already playing it — two copies of the same
/// ambience drifting out of phase for the life of the sample.
#[test]
fn the_mixer_refuses_a_bed() {
    let mut m = Mixer::new();
    for cue in Cue::ALL.iter().filter(|c| c.is_bed()) {
        m.push(Request::own(*cue));
    }
    let starts = m.tick(16.0, AT_ORIGIN, 0, &Mix::default());
    assert!(starts.is_empty(), "the mixer started a bed");
    assert_eq!(
        m.dropped, 3,
        "a refused bed was not counted as a caller bug"
    );
}

/// The snapshot crossfade: bounded, monotone toward its target, and it
/// actually arrives — a fade that asymptotes never reaches the state it is
/// fading to, which is how the reference's underwater effect could stay on
/// after a disconnect (`reference/WATER.md` §7).
#[test]
fn a_snapshot_fades_and_arrives() {
    use client::sound::{Snapshot, Snapshots, SNAPSHOT_FADE_S};
    let mut s = Snapshots::default();
    assert_eq!(s.t(), 0.0);
    let dt = SNAPSHOT_FADE_S / 10.0;
    let mut prev = 0.0f32;
    for _ in 0..10 {
        let def = s.tick(Snapshot::Submerged, dt);
        assert!(s.t() >= prev - 1e-6, "the fade went backwards");
        assert!(
            (0.0..=1.0).contains(&s.t()),
            "the fade left [0,1]: {}",
            s.t()
        );
        // Every level it publishes stays inside the two states it lerps.
        assert!(def.game <= 1.0 && def.game >= 0.45 - 1e-6);
        assert!((0.0..=1.0).contains(&def.under));
        prev = s.t();
    }
    assert_eq!(s.t(), 1.0, "the fade did not arrive in SNAPSHOT_FADE_S");
    // And back, in the same time.
    for _ in 0..10 {
        s.tick(Snapshot::Above, dt);
    }
    assert_eq!(s.t(), 0.0, "the fade did not return");
    // A world left resets it outright: a half-submerged mix must not be
    // carried into the next island.
    s.tick(Snapshot::Submerged, dt);
    s.reset();
    assert_eq!(s.t(), 0.0);
}

/// Going under ducks the game bus and swaps the bed, and it does **not** touch
/// the player's own sliders — a snapshot may only scale a mix down.
#[test]
fn the_submerged_snapshot_ducks_without_muting() {
    use client::sound::{Snapshot, Snapshots, SNAPSHOTS};
    let (above, under) = (&SNAPSHOTS[0], &SNAPSHOTS[1]);
    assert_eq!(above.game, 1.0);
    assert!(
        under.game > 0.0 && under.game < above.game,
        "submerged either mutes the game bus or does nothing to it"
    );
    assert_eq!(above.under, 0.0, "the submerged bed is audible above water");
    assert!(under.wind < above.wind, "the wind survives underwater");
    assert!(under.surf < above.surf && under.surf > 0.0);
    assert_eq!(above.bed(Cue::BedWind), above.wind);
    assert_eq!(under.bed(Cue::BedUnder), under.under);
    assert_eq!(above.bed(Cue::Swing), 0.0, "a non-bed has a bed level");

    // The player's sliders survive the snapshot.
    let mix = Mix {
        master: 0.5,
        game: 0.8,
        ambience: 0.7,
        music: 0.2,
    };
    let mut s = Snapshots::default();
    let def = s.tick(Snapshot::Above, 1.0);
    let same = mix.under(&def);
    assert_eq!(same.master, mix.master);
    assert_eq!(same.game, mix.game);
    let ducked = mix.under(under);
    assert!(ducked.game < mix.game && ducked.master == mix.master);
}

/// The surf level reads the world, and it must read it the same way twice —
/// it is a fixed probe rather than a search precisely so the bed does not
/// flicker as a search finds different water.
#[test]
fn the_surf_reads_the_sea_around_it() {
    use client::sound::water::{shore_exposure, surf_gain, SURF_FLOOR};
    use sim_core::terrain::ISLAND_SIZE;
    let seed = 0xA11CEu64;
    let centre = ISLAND_SIZE * 0.5;

    // The middle of the island is the furthest point from any shore there is.
    let inland = shore_exposure(seed, centre, centre);
    assert_eq!(inland, 0.0, "the island's centre hears surf");
    assert_eq!(surf_gain(inland), 0.0, "silence is not silent");

    // Well outside the coast ring, every probe point is sea.
    let at_sea = shore_exposure(seed, centre + 1_200.0, centre);
    assert_eq!(at_sea, 1.0, "the open sea is not all sea");
    assert!((surf_gain(at_sea) - 1.0).abs() < 1e-6);

    // Deterministic: same seed, same place, same answer.
    assert_eq!(at_sea, shore_exposure(seed, centre + 1_200.0, centre));

    // And the floor holds: any sea at all is more than nothing.
    assert!(surf_gain(0.02) >= SURF_FLOOR);
    assert!(surf_gain(1.0) <= 1.0);
    let mut prev = -1.0f32;
    for i in 0..=20 {
        let g = surf_gain(i as f32 / 20.0);
        assert!(g >= prev - 1e-6, "the surf gain fell as exposure rose");
        prev = g;
    }
}

/// Breaking the surface is an **edge**. The failure this rules out is the one
/// a naive "am I under water" check produces: a body bobbing on the waterline
/// asks for a splash every frame.
#[test]
fn a_splash_is_a_crossing_not_a_state() {
    use client::sound::water::Waterline;
    let mut w = Waterline::default();
    // The first sample establishes a side and splashes at nothing — joining a
    // world underwater is not a dive.
    assert!(w.sample(-1.0, 0.016).is_none());
    // Staying under is silent, however long.
    for _ in 0..100 {
        assert!(w.sample(-1.0, 0.016).is_none());
    }
    // Coming up is a crossing.
    assert!(w.sample(0.4, 0.016).is_some());
    assert!(w.sample(0.5, 0.016).is_none());
    // Going back down is another, and a fast one is louder than a slow one.
    let slow = {
        let mut s = Waterline::default();
        s.sample(0.05, 0.016);
        s.sample(-0.01, 0.016).expect("no splash on a slow entry")
    };
    let fast = {
        let mut s = Waterline::default();
        s.sample(2.0, 0.016);
        s.sample(-1.0, 0.016).expect("no splash on a fast entry")
    };
    assert!(
        fast > slow,
        "a dive is not louder than a step: {fast} vs {slow}"
    );
    assert!(slow > 0.0 && fast <= 1.0);
    // A reset forgets the side, so the next world's first sample is not a
    // crossing of the last world's waterline.
    let mut r = Waterline::default();
    r.sample(-1.0, 0.016);
    r.reset();
    assert!(
        r.sample(5.0, 0.016).is_none(),
        "a reset waterline still splashed"
    );
}

// ---------------------------------------------------------------------------
// The two producers landed 2026-08-09: the pig's cadence, and the place
// cue's supply line (broadcast rings, walks stay silent).
// ---------------------------------------------------------------------------

/// The snort cadence: primed silently, bounded, deterministic, and **not a
/// metronome** — a fixed period is the tell that gives away a generated
/// voice, the same way a clock-driven footstep gives away a footstep system.
#[test]
fn the_snort_cadence_is_not_a_metronome() {
    use client::sound::pig::{Snorts, SNORT_JITTER, SNORT_PERIOD_S};
    let lo = SNORT_PERIOD_S * (1.0 - SNORT_JITTER);
    let hi = SNORT_PERIOD_S * (1.0 + SNORT_JITTER);
    let dt = 0.05f32;

    // First sight primes and says nothing — a world join must not be
    // sixty-four pigs clearing their throats at once (the `Steps` /
    // `Waterline` pattern: the first observation establishes state).
    let mut s = Snorts::default();
    assert!(!s.due(0, dt), "a pig snorted the instant it was seen");

    // Ten minutes of frames for one animal: every interval inside the
    // declared band, and the band actually used.
    let mut s = Snorts::default();
    let mut t = 0.0f32;
    let mut fires = Vec::new();
    while t < 600.0 {
        if s.due(0, dt) {
            fires.push(t);
        }
        t += dt;
    }
    assert!(
        fires.len() >= 30,
        "only {} snorts in ten minutes",
        fires.len()
    );
    let intervals: Vec<f32> = fires.windows(2).map(|w| w[1] - w[0]).collect();
    for i in &intervals {
        assert!(
            *i >= lo - dt * 2.0 && *i <= hi + dt * 2.0,
            "interval {i:.2}s left the declared band [{lo}, {hi}]"
        );
    }
    let min = intervals.iter().cloned().fold(f32::MAX, f32::min);
    let max = intervals.iter().cloned().fold(0.0f32, f32::max);
    assert!(
        max - min > 1.0,
        "the pig is a metronome: every interval within {:.2}s of every other",
        max - min
    );

    // Two animals never share a schedule: across eight slots the first
    // calls spread out rather than landing together.
    let mut firsts = Vec::new();
    for slot in 0..8 {
        let mut s = Snorts::default();
        let mut t = 0.0f32;
        loop {
            if s.due(slot, dt) {
                firsts.push(t);
                break;
            }
            t += dt;
            assert!(t < 60.0, "slot {slot} never spoke");
        }
    }
    let fmin = firsts.iter().cloned().fold(f32::MAX, f32::min);
    let fmax = firsts.iter().cloned().fold(0.0f32, f32::max);
    assert!(
        fmax - fmin > 1.0,
        "eight pigs spoke within {:.2}s of each other - a chorus",
        fmax - fmin
    );

    // Deterministic: same slot, same frames, same schedule — the property
    // that keeps the voice out of the OS's random stream.
    let run = |slot: usize| {
        let mut s = Snorts::default();
        let mut t = 0.0f32;
        let mut log = Vec::new();
        while t < 120.0 {
            if s.due(slot, dt) {
                log.push(t.to_bits());
            }
            t += dt;
        }
        log
    };
    assert_eq!(run(5), run(5), "two identical runs spoke differently");

    // A hitch that swallowed three intervals buys ONE snort, not a banked
    // burst — the step odometer's rule, applied to a voice.
    let mut s = Snorts::default();
    s.due(2, dt); // prime
    let mut fired = 0;
    if s.due(2, 100.0) {
        fired += 1;
    }
    for _ in 0..20 {
        if s.due(2, 0.016) {
            fired += 1;
        }
    }
    assert_eq!(fired, 1, "a 100s hitch banked {fired} snorts");

    // A reset forgets the schedule: the next world's herd primes afresh.
    let mut s = Snorts::default();
    s.due(1, dt);
    s.reset();
    assert!(!s.due(1, dt), "a reset herd spoke on first sight");
}

/// **The place cue's supply line: a placement broadcast rings, a sync walk
/// never does.** This is the join-flood gate. The initial world sync (and
/// every resync) restates the whole standing world as `PieceSync` /
/// `DeploySync` walk batches — if those rang, a fresh join into a base
/// would be N place cues at once, and the "fix" would be a timer knob
/// deciding when the flood is over. Instead the core's ring is fed only by
/// the `PiecePlaced` / `DeployPlaced` broadcasts, which the server sends
/// exactly when a placement *happens* — so the guard is the wire's own
/// distinction between an event and a restatement, and no clock is
/// involved. Runs in the code tier against the real decoder: these are the
/// same bytes the shard sends.
#[test]
fn a_placement_broadcast_rings_and_a_sync_walk_does_not() {
    use client_core::core::ClientCore;
    use protocol::{
        encode_event_deploy_placed, encode_event_deploy_sync, encode_event_piece_placed,
        encode_event_piece_sync, MAX_EVENT_MSG_BYTES,
    };
    use sim_core::build::{PieceRec, LOC_PLANE};
    use sim_core::deploy::DeployRec;

    let mut core = ClientCore::new(1, 7, 0);
    let mut buf = [0u8; MAX_EVENT_MSG_BYTES];

    // The join flood: a reset walk restating a standing base, both stores.
    let batch: [PieceRec; 6] = core::array::from_fn(|i| PieceRec {
        cx: 10 + i as u16,
        cz: 20,
        level: 0,
        loc: LOC_PLANE,
        row: 0,
        ..PieceRec::default()
    });
    let len = encode_event_piece_sync(true, &batch, &mut buf).expect("encode");
    core.on_stream(&buf[..len]).expect("decode");
    let dbatch = [DeployRec {
        cx: 11,
        cz: 20,
        level: 0,
        loc: LOC_PLANE,
        row: 0,
        ..DeployRec::default()
    }];
    let len = encode_event_deploy_sync(true, &dbatch, &mut buf).expect("encode");
    core.on_stream(&buf[..len]).expect("decode");
    assert!(
        core.pop_placed().is_none(),
        "a sync walk restating the world rang the place cue - a fresh join would be N cues at once"
    );

    // A live placement broadcast rings exactly once, with its address.
    let rec = PieceRec {
        cx: 40,
        cz: 41,
        level: 1,
        loc: LOC_PLANE,
        row: 0,
        ..PieceRec::default()
    };
    let len = encode_event_piece_placed(&rec, &mut buf).expect("encode");
    core.on_stream(&buf[..len]).expect("decode");
    assert_eq!(
        core.pop_placed(),
        Some((40, 41, 1, LOC_PLANE, false)),
        "a piece placement broadcast did not ring"
    );
    assert!(core.pop_placed().is_none(), "one placement rang twice");

    // The wire then sends the walk's tail batch carrying the SAME record
    // (broadcast first, walk after — `pump_events`' order); the mirror
    // insert is idempotent, so the duplicate delivery stays silent.
    let len = encode_event_piece_sync(false, &[rec], &mut buf).expect("encode");
    core.on_stream(&buf[..len]).expect("decode");
    assert!(
        core.pop_placed().is_none(),
        "the walk's tail batch double-rang a placement the broadcast already rang"
    );

    // A deployable broadcast rings too, carrying the store bit.
    let drec = DeployRec {
        cx: 50,
        cz: 51,
        level: 0,
        loc: LOC_PLANE,
        row: 0,
        ..DeployRec::default()
    };
    let len = encode_event_deploy_placed(&drec, &mut buf).expect("encode");
    core.on_stream(&buf[..len]).expect("decode");
    assert_eq!(
        core.pop_placed(),
        Some((50, 51, 0, LOC_PLANE, true)),
        "a deployable placement broadcast did not ring"
    );
}

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
    const DESTRUCTIVE: [&str; 8] = [
        "pop_hit(",
        "pop_death(",
        "pop_toast(",
        "pop_craft_toast(",
        "pop_craft_refusal(",
        "pop_build_refusal(",
        "pop_deploy_refusal(",
        "pop_placed(",
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

// ---------------------------------------------------------------------------
// The score. `reference/AUDIO.md` §8's design, one rule per test.
// ---------------------------------------------------------------------------

/// Drive a director for `secs` at 60 Hz and return every piece it started,
/// with the second it started at. A whole session's music in a few thousand
/// float adds — the reason `sound::music` is pure.
fn run(d: &mut Director, secs: f32) -> Vec<(f32, Piece)> {
    let dt = 1.0 / 60.0;
    let mut out = Vec::new();
    let mut t = 0.0;
    while t < secs {
        if let Some(p) = d.tick(dt) {
            out.push((t, p));
        }
        t += dt;
    }
    out
}

/// **Music is not continuous, and this is the whole point of the system.**
/// The reference's `music.songgapmin` / `songgapmax` are 240 and 480 seconds;
/// a director that played back to back would be a soundtrack, which is the
/// thing §8 says it deliberately is not.
#[test]
fn songs_are_separated_by_minutes_of_silence() {
    let mut d = Director::new(Mode::World);
    let played = run(&mut d, 3_600.0);
    assert!(
        played.len() > 8,
        "only {} pieces in an hour - the director stalled",
        played.len()
    );
    // Every gap between two consecutive pieces is either a section boundary
    // or a real gap; there is nothing in between.
    let mut gaps = 0;
    for w in played.windows(2) {
        let d_s = w[1].0 - w[0].0;
        if d_s < music::SECTION_S * 1.5 {
            continue;
        }
        gaps += 1;
        // The gap is measured from the END of the last piece's body, so the
        // interval between two STARTS is one section longer.
        let gap = d_s - music::SECTION_S;
        assert!(
            (music::SONG_GAP_MIN_S - 1.0..=music::SONG_GAP_MAX_S + 1.0).contains(&gap),
            "a gap of {gap:.0}s is outside the reference's {}..{}",
            music::SONG_GAP_MIN_S,
            music::SONG_GAP_MAX_S
        );
    }
    assert!(gaps >= 3, "only {gaps} gaps in an hour");
}

/// The first song is the one departure, and it is a knob rather than an
/// accident: at the reference's own gap a player who joins for three minutes
/// hears no music at all, including the operator on the pass that decides
/// whether it is any good.
#[test]
fn the_first_song_does_not_wait_four_minutes() {
    let mut d = Director::new(Mode::World);
    let played = run(&mut d, music::FIRST_GAP_S + 1.0);
    assert_eq!(played.len(), 1, "the first song did not open on time");
    assert!((played[0].0 - music::FIRST_GAP_S).abs() < 0.1);
}

/// A song walks the theme's sections **in order**, wrapping, and a new song
/// starts at the top of the theme. That is what makes several pieces a song
/// rather than several stings — and it is why `SONG_SECTIONS_MIN` is four:
/// A-B-C-A is the shortest walk that has come round.
#[test]
fn a_song_walks_the_theme_in_order() {
    // A WORLD director, because its gaps are what delimit one song from the
    // next — on the menus a song runs straight into the one after it and
    // "where did this song end" has no observable answer.
    let mut d = Director::new(Mode::World);
    let played = run(&mut d, 7_200.0);
    let mut songs: Vec<Vec<usize>> = vec![vec![]];
    let mut last = 0.0f32;
    for (t, p) in &played {
        if !songs.last().unwrap().is_empty() && t - last > music::SECTION_S * 1.5 {
            songs.push(vec![]);
        }
        songs.last_mut().unwrap().push(p.section);
        last = *t;
    }
    assert!(songs.len() >= 8, "only {} songs in two hours", songs.len());
    // The last one is still running when the clock stops.
    songs.pop();
    for song in &songs {
        assert!(
            song.len() >= music::SONG_SECTIONS_MIN as usize
                && song.len() <= music::SONG_SECTIONS_MAX as usize,
            "a song of {} sections: {song:?}",
            song.len()
        );
        for (i, s) in song.iter().enumerate() {
            assert_eq!(
                *s,
                i % music::SECTIONS,
                "the theme is out of order: {song:?}"
            );
        }
    }
}

/// On the menus there is no gap and nothing to be tense about.
#[test]
fn the_menu_plays_continuously_and_calm() {
    let mut d = Director::new(Mode::Menu);
    let played = run(&mut d, 600.0);
    for w in played.windows(2) {
        let gap = w[1].0 - w[0].0;
        assert!(
            (gap - music::SECTION_S).abs() < 0.1,
            "a {gap:.1}s gap on the menu - it is supposed to be continuous"
        );
    }
    // 600 s of menu at one piece per section.
    assert!(played.len() > 70, "only {} pieces", played.len());
}

/// **Taking damage is the biggest bump and a swing is the smallest** — the
/// reference's published order, which is the part of §8 that is a design
/// rather than a number.
#[test]
fn the_bumps_are_ordered_as_the_reference_orders_them() {
    const { assert!(music::BUMP_SWING < music::BUMP_HIT) };
    const { assert!(music::BUMP_HIT < music::BUMP_HURT) };
    // A single swing must not reach a tier: a player chopping wood is not in
    // a fight, and the small bump's whole job is to be small.
    assert_eq!(Tier::of(music::BUMP_SWING), Tier::Calm);
    // Two of the biggest, inside two sections, is the top tier. That is what
    // makes the tier reachable at all — a threshold nothing can cross is a
    // clip that never plays.
    assert_eq!(Tier::of(music::BUMP_HURT * 2.0), Tier::Combat);
}

/// **The intensity is read at a section boundary and nowhere else.** This is
/// the mechanism §8 calls the good one: bumps land whenever they land, and
/// the music changes only where a change can sound like a change.
#[test]
fn intensity_moves_the_music_only_at_a_boundary() {
    let mut d = Director::new(Mode::Menu);
    // One tick to open the first piece, which is calm.
    let first = d.tick(0.001).expect("the menu opens immediately");
    assert_eq!(first.tier, Tier::Calm);

    // Get hurt twice, mid-section. Nothing may start.
    d.tick(1.0);
    d.bump(music::BUMP_HURT);
    d.bump(music::BUMP_HURT);
    assert_eq!(d.tier(), Tier::Combat, "the bumps did not land");
    for _ in 0..100 {
        assert!(
            d.tick(1.0 / 60.0).is_none(),
            "a piece started mid-section - the music lurched"
        );
    }
    // The boundary is where it shows up.
    let next = run(&mut d, music::SECTION_S);
    assert_eq!(next.len(), 1);
    assert_eq!(
        next[0].1.tier,
        Tier::Combat,
        "the boundary did not read the intensity"
    );
}

/// **If nothing happens during a section, the intensity falls at the end of
/// it** — a musical boundary, not a timer. And it decays in the gap too, on
/// the same cadence: a fight during four minutes of silence must not open the
/// next song in combat.
#[test]
fn intensity_decays_at_boundaries_and_in_the_gap() {
    let mut d = Director::new(Mode::Menu);
    d.tick(0.001);
    d.bump(music::BUMP_HURT);
    d.bump(music::BUMP_HURT);
    let hot = d.intensity();
    // **The section the bumps landed in does not decay** — that is the rule,
    // and it is the first boundary this crosses. The second one is quiet, and
    // it must fall by exactly one step rather than by a per-frame fraction of
    // one.
    run(&mut d, music::SECTION_S + 0.1);
    assert_eq!(
        d.intensity(),
        hot,
        "the section the damage landed in decayed anyway"
    );
    run(&mut d, music::SECTION_S);
    let after = d.intensity();
    assert!(
        (hot - after - music::SECTION_DECAY).abs() < 1e-3,
        "one quiet section took {hot} to {after} - not one decay step"
    );
    // Bumping keeps it up: a section in which something happened does not
    // decay, however long it is.
    let held = d.intensity();
    for _ in 0..(60.0 * music::SECTION_S) as u32 {
        d.tick(1.0 / 60.0);
        d.bump(0.0001);
    }
    assert!(
        d.intensity() >= held,
        "a section with events in it decayed anyway"
    );

    // And in the gap, where there is no section playing at all.
    let mut w = Director::new(Mode::World);
    w.bump(music::BUMP_HURT);
    run(&mut w, music::SECTION_S * 3.0);
    assert_eq!(w.intensity(), 0.0, "intensity survived the silence");
}

/// A director in a gap is not playing a song, and one that just started a
/// song is. The flag the render layer reads has to mean what it says.
#[test]
fn the_gap_and_the_song_are_distinguishable() {
    let mut d = Director::new(Mode::World);
    assert!(!d.in_song(), "a fresh world director is mid-song");
    run(&mut d, music::FIRST_GAP_S + 0.1);
    assert!(d.in_song(), "the first song did not start");
}

/// Leaving a world starts the director over, and the mode is what makes the
/// menus continuous. The random stream must NOT reset with it — every world
/// in a session drawing the identical gap sequence is the one thing this
/// determinism is not for.
#[test]
fn a_reset_changes_the_mode_and_keeps_the_stream() {
    let mut a = Director::new(Mode::World);
    let first = run(&mut a, 1_200.0);
    a.reset(Mode::World);
    let second = run(&mut a, 1_200.0);
    let starts = |v: &[(f32, Piece)]| v.iter().map(|(t, _)| *t).collect::<Vec<_>>();
    assert_ne!(
        starts(&first),
        starts(&second),
        "two worlds in one session drew the same gaps"
    );

    // And the mode takes effect immediately.
    let mut m = Director::new(Mode::World);
    m.reset(Mode::Menu);
    assert!(m.tick(0.001).is_some(), "the menu did not open on music");
}

/// At most one piece per call, which is what lets the render layer spawn
/// without a loop and what exempts the music from
/// `stackable_cues_carry_a_cooldown`.
#[test]
fn a_tick_starts_at_most_one_piece() {
    let mut d = Director::new(Mode::Menu);
    // A ten-minute frame. Absurd, and the point: no `dt` may produce two.
    assert!(d.tick(600.0).is_some());
    assert!(d.tick(600.0).is_some());
}

/// The table is the only place the mapping lives, and `piece_of` is the only
/// reader — so the two cannot drift, and `Cue::is_music` cannot be a
/// remembered list.
#[test]
fn every_piece_is_findable_and_only_pieces_are_music() {
    let mut seen = 0;
    for cue in Cue::ALL {
        if !cue.is_music() {
            continue;
        }
        seen += 1;
        let (s, t) = music::piece_of(cue).expect("is_music but not in PIECES");
        assert_eq!(music::PIECES[s][t.idx()], cue);
        assert_eq!(cue.def().bus, Bus::Music, "{cue:?} is not on the music bus");
        assert!(!cue.def().positional, "{cue:?} is somewhere - music is not");
    }
    assert_eq!(
        seen,
        music::SECTIONS * music::TIERS,
        "the enum and the table disagree about how many pieces there are"
    );
    // And nothing else may claim to be music.
    assert!(!Cue::BedWind.is_music());
    assert!(!Cue::Swing.is_music());
}

/// **The mixer never starts a piece.** It would be scored against
/// `STARTS_PER_FRAME` and `VOICE_CAP`, so a busy frame could refuse the score
/// — or the score could refuse an axe.
#[test]
fn the_mixer_refuses_a_piece_of_music() {
    let mut m = Mixer::new();
    for cue in Cue::ALL.iter().filter(|c| c.is_music()) {
        m.push(Request::own(*cue));
    }
    let starts = m.tick(16.0, AT_ORIGIN, 0, &Mix::default());
    assert!(starts.is_empty(), "the mixer started a piece of music");
    assert_eq!(m.dropped, (music::SECTIONS * music::TIERS) as u32);
}

/// **The music bus opens at a fifth, and that is the reference's number.**
/// Their `audio.musicvolume` defaults to 0.2 where master and game default to
/// 1 — a score at parity with footsteps is a score players turn off.
#[test]
fn the_music_bus_opens_where_the_reference_opens_it() {
    let mix = Mix::default();
    assert_eq!(mix.music, 0.2);
    assert_eq!(mix.bus_gain(Bus::Music), 0.2);
    assert!(
        mix.bus_gain(Bus::Music) < mix.bus_gain(Bus::Game),
        "music is not under the game bus"
    );
}

/// **Every piece is `SECTION_S` of body and `TAIL_S` of ring-out**, and the
/// ring-out is not decoration: it is what lets the director cut from any
/// piece to any other without stopping playback and without fading
/// (`reference/AUDIO.md` §8). A piece whose length drifted from the table
/// would have the next one starting over its body.
#[test]
fn a_piece_is_a_body_and_a_tail() {
    let body = (music::SECTION_S * SAMPLE_RATE as f32) as usize;
    for cue in Cue::ALL.iter().filter(|c| c.is_music()) {
        let s = pcm(&synth::wav(*cue));
        let secs = s.len() as f32 / SAMPLE_RATE as f32;
        assert!(
            (secs - music::PIECE_S).abs() < 0.05,
            "{cue:?} is {secs:.2}s, not {:.2}",
            music::PIECE_S
        );
        // The tail must be quieter than the body — it is a decay, not more
        // music — and it must not be silence, or there is nothing covering
        // the join.
        let tail = rms(&s[body..]);
        let mid = rms(&s[body / 4..body]);
        assert!(
            tail < mid * 0.6,
            "{cue:?}'s tail ({tail:.3}) is as loud as its body ({mid:.3}) - it is not a tail"
        );
        assert!(
            tail > 0.002,
            "{cue:?} has no tail ({tail:.4}) - a cut to the next piece would be audible"
        );
    }
}

/// The tiers have to be **audibly** ordered or the whole intensity system is
/// a table nobody can hear.
///
/// Measured on the body rather than the tail, and **through the cue's gain**,
/// because that is what reaches the mixer: `synth::PEAK` normalizes every cue
/// in the bank to one peak, so the waveforms alone say nothing about level and
/// a test that read them would be measuring crest factor — under which a
/// sustained calm pad scores *above* a transient-heavy combat piece. The level
/// is the table's (`M_CALM` / `M_TENSE` / `M_COMBAT`), which is the module's
/// own law, and this is the gate on it.
#[test]
fn the_tiers_get_louder_as_they_climb() {
    let body = (music::SECTION_S * SAMPLE_RATE as f32) as usize;
    for (s, row) in music::PIECES.iter().enumerate() {
        let e: Vec<f32> = row
            .iter()
            .map(|c| rms(&pcm(&synth::wav(*c))[..body]) * c.def().gain)
            .collect();
        assert!(
            e[1] > e[0] * 1.1 && e[2] > e[1] * 1.1,
            "section {s} is not audibly ordered by intensity: {e:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The forest layer.
// ---------------------------------------------------------------------------

/// **A layer is sparse, and a bed is not.** The whole distinction
/// `reference/AUDIO.md` §3 draws between the two is that a layer's timing is
/// its own: in the woods a call every few seconds, on the beach one every
/// couple of minutes, and never twice the same interval.
#[test]
fn the_forest_layer_is_sparser_in_the_open() {
    let count = |cover: f32| {
        let mut b = Birds::default();
        let dt = 1.0 / 60.0;
        let mut n = 0;
        for _ in 0..(60.0 * 600.0) as u32 {
            if b.due(cover, dt) {
                n += 1;
            }
        }
        n
    };
    let wood = count(1.0);
    let beach = count(0.0);
    assert!(wood > 30, "only {wood} calls in ten minutes of forest");
    assert!(beach > 0, "the open ground is silent, not sparse");
    assert!(
        wood > beach * 5,
        "a wood ({wood}) is not meaningfully busier than a beach ({beach})"
    );
}

/// The first frame in a world must be silent: a countdown that started at
/// zero would greet every join with a dawn chorus, which is the rule
/// `Steps`, `Waterline` and `pig::Snorts` all already state.
#[test]
fn the_forest_layer_primes_silently_and_resets() {
    let mut b = Birds::default();
    assert!(!b.due(1.0, 1.0 / 60.0), "a bird called on the first frame");
    // Run it to a call, then reset: the next world must prime again rather
    // than firing on the last one's schedule.
    let mut fired = false;
    for _ in 0..(60.0 * 120.0) as u32 {
        fired |= b.due(1.0, 1.0 / 60.0);
    }
    assert!(fired, "no bird in two minutes of forest");
    b.reset();
    assert!(!b.due(1.0, 1.0 / 60.0), "a reset layer called immediately");
}

/// The perch index has to land inside the candidate set, including the empty
/// set — a call with no perch is a call with no position, and the mixer
/// refuses one of those (`reference/AUDIO.md` §6's origin bug).
#[test]
fn a_perch_is_always_in_range() {
    let mut b = Birds::default();
    for _ in 0..2_000 {
        b.due(1.0, 1.0 / 60.0);
        for n in [0usize, 1, 3, 40] {
            let i = b.perch(n);
            assert!(i < n.max(1), "perch {i} of {n}");
        }
    }
}

/// The bird is ambience, not game: a player who turns the bed down means the
/// birds too. And it carries no further than the surf does.
#[test]
fn the_bird_is_ambience_and_is_positional() {
    let d = Cue::Bird.def();
    assert_eq!(d.bus, Bus::Ambience);
    assert!(d.positional, "a bird has to be somewhere");
    assert!(d.radius_m <= MAX_AUDIBLE_M);
    assert!(
        d.priority <= 1,
        "a bird outranks a footstep - it must never be why an axe was refused"
    );
    // A single sample retriggered at its own pitch for minutes on end is the
    // machine-gun tell, and this is the widest variation in the table.
    assert!(Cue::Bird.pitch_var() > 0.10);
}
