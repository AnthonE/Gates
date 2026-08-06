//! Every sound this client makes, generated at boot from arithmetic.
//!
//! **There are no audio assets in this repo and this module is why.** Three
//! reasons, in the order they bind:
//!
//! 1. **Licence.** `CLAUDE.md` already records the trap: `SeedThree` ships
//!    bird recordings whose `README.txt` cites xeno-canto and *states no
//!    licence per file*, and `Eanpa-Sky`'s four recordings are CC BY-NC-SA —
//!    NC does not survive a sold product. Audio is the single worst-labelled
//!    asset class on the internet, and a game sold through scry's board
//!    cannot carry a sample whose provenance is a forum post.
//! 2. **The IP rail.** `DECISIONS.md` 2026-07-31 puts exactly two things on
//!    it, and one of them is *no asset copied verbatim*. A recording of the
//!    reference game's footsteps would be on the wrong side of that line by
//!    construction.
//! 3. **It is a skeleton-first repo.** `CLAUDE.md`'s first paragraph: the
//!    skeleton is the product. A generated bank makes the *system* real —
//!    buses, budget, falloff, cadence, panning — while the samples stay a
//!    swap away. Nothing in `render/audio.rs` knows or cares that its
//!    `AudioSource` came from arithmetic rather than a file, so replacing
//!    this module with a loader is a change to one function.
//!
//! **What this is not.** It is not a claim that generated audio sounds as good
//! as recorded audio. It does not. `NOW.md` carries the honest version: this
//! is a working bank at programmer-art quality, and the art bar for sound is
//! unwritten (`ART.md` has no audio section at all, which is itself a finding).
//!
//! Determinism is a property, asserted in `tests/sound.rs`: same build → same
//! bytes, because every cue is seeded off its own discriminant and the PRNG is
//! a fixed xorshift. **Not a byte golden**, deliberately — `CLAUDE.md` records
//! that a byte-golden is blind to what a field means, and a golden over a
//! waveform would fail on every tuning pass while proving nothing about
//! whether the sound is a sound. The gates here are structural: a cue has
//! energy, it does not clip, it starts and ends near silence, and the bed's
//! loop seam is continuous.

use super::{Cue, CUE_COUNT, SAMPLE_RATE};

/// Peak every cue is normalized to before the table's gain is applied.
///
/// One peak for the whole bank so that `CueDef::gain` is the ONLY thing that
/// decides relative loudness. A bank whose samples were each at their own
/// natural level would put half the mix in this file and half in the table,
/// which is the split that makes a mix impossible to reason about.
pub const PEAK: f32 = 0.9;

/// The wind bed's length before its loop seam is cut, seconds.
///
/// **Twelve, not six, and the reason is a bug that only looking found.** The
/// first cut was a 6 s bed with gust LFOs at 0.071 and 0.113 Hz — periods of
/// 14 and 8.8 seconds, both *longer than the 5.25 s loop*. Every structural
/// assertion passed (energy, no clipping, a continuous seam) and the bed was a
/// flat hiss with no gusting in it at all, because the loop ended before the
/// first gust finished. It took a waveform plot to see it, which is
/// `CLAUDE.md`'s beige-smear entry with a different file extension.
const BED_SECS: f32 = 12.0;
/// How much of the bed is spent crossfading the tail back into the head.
const BED_FADE_SECS: f32 = 1.5;
/// What the loop actually is once the seam is cut — and the number every gust
/// LFO's rate is derived from, so a gust is continuous across the join.
const BED_LOOP_SECS: f32 = BED_SECS - BED_FADE_SECS;

/// Generate the whole bank, in [`Cue::ALL`] order. Called once, at startup.
///
/// Allocating and not cheap — roughly 1.5 MB of `f32` and a megabyte of WAV.
/// That is a boot cost, not a frame cost, and it is paid on the loading
/// screen where the pipelines are already specializing.
pub fn bank() -> [Vec<u8>; CUE_COUNT] {
    core::array::from_fn(|i| wav(Cue::ALL[i]))
}

/// One cue, as 16-bit mono PCM in a WAV container.
///
/// WAV rather than Ogg because we are the encoder: a container we write in 44
/// bytes beats pulling a Vorbis encoder into the client to compress something
/// that was generated in memory and will be decoded back in memory.
pub fn wav(cue: Cue) -> Vec<u8> {
    let mut s = render(cue);
    normalize(&mut s, PEAK);
    to_wav16(&s)
}

/// The sample buffer for a cue, before normalization.
fn render(cue: Cue) -> Vec<f32> {
    // Seeded off the discriminant, so a cue's noise is its own and adding a
    // cue does not reshuffle every other cue's samples.
    let mut r = Rng::new(0x9E37_79B9 ^ (cue as u32).wrapping_mul(0x85EB_CA6B));
    match cue {
        // ---- footsteps -------------------------------------------------
        // Five surfaces, one gesture: a transient with a body under it. What
        // separates them is spectral tilt and decay, which is what separates
        // them in the world — sand absorbs the top end, rock returns it.
        Cue::StepSand => impact(
            &mut r,
            0.18,
            Tone {
                lo_hz: 90.0,
                lo_amp: 0.35,
                lo_tau: 0.05,
                noise_amp: 0.8,
                noise_tau: 0.055,
                lp_hz: 1_100.0,
                hp_hz: 60.0,
                attack_s: 0.002,
                crackle: 0.0,
            },
        ),
        Cue::StepGrass => impact(
            &mut r,
            0.17,
            Tone {
                lo_hz: 110.0,
                lo_amp: 0.25,
                lo_tau: 0.04,
                noise_amp: 0.9,
                noise_tau: 0.045,
                lp_hz: 4_200.0,
                hp_hz: 500.0,
                attack_s: 0.001,
                crackle: 0.25,
            },
        ),
        // Forest floor: dry litter, so the noise is GATED into crackles
        // rather than smooth. `crackle` is what makes twigs out of hiss.
        Cue::StepLitter => impact(
            &mut r,
            0.22,
            Tone {
                lo_hz: 100.0,
                lo_amp: 0.22,
                lo_tau: 0.04,
                noise_amp: 1.0,
                noise_tau: 0.075,
                lp_hz: 6_500.0,
                hp_hz: 700.0,
                attack_s: 0.001,
                crackle: 0.65,
            },
        ),
        Cue::StepRock => impact(
            &mut r,
            0.14,
            Tone {
                lo_hz: 150.0,
                lo_amp: 0.30,
                lo_tau: 0.03,
                noise_amp: 1.0,
                noise_tau: 0.028,
                lp_hz: 9_000.0,
                hp_hz: 1_400.0,
                attack_s: 0.0005,
                crackle: 0.15,
            },
        ),
        // Water is the one that is not a transient: it swells. Long attack,
        // long tail, band-limited to the splash range.
        Cue::StepWater => impact(
            &mut r,
            0.34,
            Tone {
                lo_hz: 70.0,
                lo_amp: 0.15,
                lo_tau: 0.05,
                noise_amp: 1.0,
                noise_tau: 0.13,
                lp_hz: 3_400.0,
                hp_hz: 600.0,
                attack_s: 0.022,
                crackle: 0.0,
            },
        ),

        // ---- the swing -------------------------------------------------
        // A whoosh is a moving band, not a static one: the filter sweeps up
        // as the head accelerates and the envelope peaks after the start
        // rather than at it. A fixed-band noise burst reads as a hiss.
        Cue::Swing => whoosh(&mut r, 0.26),

        // ---- impacts ---------------------------------------------------
        // Wood: one low mode, fast decay, dry.
        Cue::ImpactWood => impact(
            &mut r,
            0.30,
            Tone {
                lo_hz: 185.0,
                lo_amp: 0.85,
                lo_tau: 0.075,
                noise_amp: 0.55,
                noise_tau: 0.035,
                lp_hz: 3_000.0,
                hp_hz: 200.0,
                attack_s: 0.0008,
                crackle: 0.2,
            },
        ),
        // Stone: lower, shorter, and grittier — most of the energy is the
        // noise, because a rock struck is mostly a crunch.
        Cue::ImpactStone => impact(
            &mut r,
            0.26,
            Tone {
                lo_hz: 95.0,
                lo_amp: 0.55,
                lo_tau: 0.045,
                noise_amp: 0.95,
                noise_tau: 0.05,
                lp_hz: 5_500.0,
                hp_hz: 300.0,
                attack_s: 0.0005,
                crackle: 0.35,
            },
        ),
        // Metal is the only cue with two partials, and they are deliberately
        // INHARMONIC (620 and 1370 Hz, not 620 and 1240): a harmonic pair
        // reads as a musical note, and struck metal is not a note.
        Cue::ImpactMetal => metal(&mut r, 0.55),

        // ---- the interface answering you --------------------------------
        Cue::Gather => impact(
            &mut r,
            0.20,
            Tone {
                lo_hz: 210.0,
                lo_amp: 0.6,
                lo_tau: 0.045,
                noise_amp: 0.7,
                noise_tau: 0.03,
                lp_hz: 4_000.0,
                hp_hz: 250.0,
                attack_s: 0.001,
                crackle: 0.3,
            },
        ),
        // Two soft notes, up. The only cue in the bank that is allowed to be
        // musical, because "the thing you asked for exists now" is the one
        // message that is not diegetic.
        Cue::CraftDone => chime(&[(523.25, 0.0, 0.16), (784.0, 0.10, 0.26)]),
        // Refusal is a short, flat, LOW buzz, and low on purpose: the
        // reference's mix note is that ambience is EQ-carved at 2-5 kHz to
        // leave room for the things that matter, and a refusal is not one of
        // them. It should be noticed and then ignored.
        Cue::Refused => buzz(155.0, 0.14),

        // ---- combat -----------------------------------------------------
        // The hitmarker: a bright, very short tick. Nothing else in the bank
        // lives up here, which is what makes it readable through gunfire.
        Cue::Hit => chime(&[(2_100.0, 0.0, 0.035), (3_150.0, 0.004, 0.028)]),
        // Being hurt: a thud with a downward sweep in it. Down, because every
        // organism on earth reads a falling pitch as damage.
        Cue::Hurt => sweep(&mut r, 0.42, 240.0, 130.0, 0.55),
        Cue::Death => sweep(&mut r, 1.10, 190.0, 55.0, 0.35),

        // ---- the world --------------------------------------------------
        Cue::Place => impact(
            &mut r,
            0.24,
            Tone {
                lo_hz: 140.0,
                lo_amp: 0.7,
                lo_tau: 0.06,
                noise_amp: 0.45,
                noise_tau: 0.03,
                lp_hz: 2_600.0,
                hp_hz: 150.0,
                attack_s: 0.001,
                crackle: 0.1,
            },
        ),
        // A tree coming down: leaves first (a rising noise swell), then the
        // crack, then the ground. The only cue that is a sequence, and the
        // longest thing in the bank at 1.9 s.
        Cue::TreeFall => tree_fall(&mut r),
        Cue::UiClick => chime(&[(1_450.0, 0.0, 0.018)]),

        // ---- the bed ----------------------------------------------------
        Cue::BedWind => bed(&mut r),
    }
}

// ---------------------------------------------------------------------------
// Primitives. Five of them, and everything above is a parameterization.
// ---------------------------------------------------------------------------

/// xorshift32. Deterministic, seedable, and the same on every platform —
/// which is what makes "same build → same bytes" an assertion rather than a
/// hope. Not `fastrand`: this must not share a stream with the tree generator.
struct Rng(u32);

impl Rng {
    fn new(seed: u32) -> Self {
        // A zero state is a fixed point for xorshift, so it can never be one.
        Self(if seed == 0 { 0x1234_5678 } else { seed })
    }
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// White noise in [-1, 1).
    fn noise(&mut self) -> f32 {
        (self.next_u32() as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
    /// Uniform in [0, 1).
    fn unit(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }
}

/// A one-pole low-pass. The only filter in this module: a high-pass is
/// `x - lp(x)` and a band-pass is one of each, which is enough shaping for a
/// bank made of noise bursts and decaying sines.
struct Lp {
    y: f32,
    a: f32,
}

impl Lp {
    fn new(hz: f32) -> Self {
        // 1 - e^(-2πf/fs), the standard one-pole coefficient.
        let a = 1.0 - (-std::f32::consts::TAU * hz / SAMPLE_RATE as f32).exp();
        Self {
            y: 0.0,
            a: a.clamp(0.0, 1.0),
        }
    }
    fn run(&mut self, x: f32) -> f32 {
        self.y += self.a * (x - self.y);
        self.y
    }
}

/// Everything that distinguishes one impact from another.
struct Tone {
    /// The body: a decaying sine under the noise.
    lo_hz: f32,
    lo_amp: f32,
    lo_tau: f32,
    /// The transient: filtered noise.
    noise_amp: f32,
    noise_tau: f32,
    lp_hz: f32,
    hp_hz: f32,
    /// How long the envelope takes to reach full, seconds. A splash has one;
    /// a rock does not.
    attack_s: f32,
    /// 0 = smooth hiss, 1 = sparse crackle. What makes dry litter out of
    /// noise: the noise is gated by a second, sparser random process, so the
    /// energy arrives as discrete snaps rather than as a wash.
    crackle: f32,
}

/// A transient with a body under it — the shape of every footstep and every
/// impact in the bank.
fn impact(r: &mut Rng, dur_s: f32, t: Tone) -> Vec<f32> {
    let n = samples(dur_s);
    let mut lp = Lp::new(t.lp_hz);
    let mut hp = Lp::new(t.hp_hz);
    let mut out = Vec::with_capacity(n);
    let sr = SAMPLE_RATE as f32;
    // The crackle gate's own slow envelope, so snaps thin out as the sound
    // decays instead of stopping dead.
    let mut gate = 0.0f32;
    for i in 0..n {
        let time = i as f32 / sr;
        let att = attack(time, t.attack_s);
        let mut x = r.noise();
        if t.crackle > 0.0 {
            // Gate: open on a random draw, then decay. `crackle` sets both how
            // often it opens and how much of the signal it owns.
            if r.unit() < 0.010 + 0.05 * (1.0 - t.crackle) {
                gate = 1.0;
            }
            gate *= 0.9985;
            x *= 1.0 - t.crackle + t.crackle * gate;
        }
        let band = {
            let low = lp.run(x);
            low - hp.run(low)
        };
        let noise = band * t.noise_amp * (-time / t.noise_tau).exp();
        let body =
            (std::f32::consts::TAU * t.lo_hz * time).sin() * t.lo_amp * (-time / t.lo_tau).exp();
        out.push((noise + body) * att * edges(i, n));
    }
    out
}

/// The swing: a band of noise whose centre sweeps up and whose envelope peaks
/// a third of the way in.
fn whoosh(r: &mut Rng, dur_s: f32) -> Vec<f32> {
    let n = samples(dur_s);
    let mut out = Vec::with_capacity(n);
    let sr = SAMPLE_RATE as f32;
    let mut lp_y = 0.0f32;
    let mut hp_y = 0.0f32;
    for i in 0..n {
        let t = i as f32 / n as f32;
        // 400 Hz to 2.6 kHz across the swing, and the high-pass follows it a
        // fixed ratio below — a moving band, not a widening one.
        let centre = 400.0 + 2_200.0 * t;
        let a_lp = 1.0 - (-std::f32::consts::TAU * (centre * 1.7) / sr).exp();
        let a_hp = 1.0 - (-std::f32::consts::TAU * (centre * 0.45) / sr).exp();
        let x = r.noise();
        lp_y += a_lp.clamp(0.0, 1.0) * (x - lp_y);
        hp_y += a_hp.clamp(0.0, 1.0) * (lp_y - hp_y);
        // Peak at t = 0.33: sin(π t^0.6) is fast up, slow down, which is what
        // an arm does.
        let env = (std::f32::consts::PI * t.powf(0.6)).sin().max(0.0);
        out.push((lp_y - hp_y) * env * edges(i, n));
    }
    out
}

/// Struck metal: two inharmonic partials plus a bright transient.
fn metal(r: &mut Rng, dur_s: f32) -> Vec<f32> {
    let n = samples(dur_s);
    let mut out = Vec::with_capacity(n);
    let mut hp = Lp::new(2_000.0);
    let sr = SAMPLE_RATE as f32;
    for i in 0..n {
        let time = i as f32 / sr;
        let p1 = (std::f32::consts::TAU * 620.0 * time).sin() * (-time / 0.20).exp();
        let p2 = (std::f32::consts::TAU * 1_370.0 * time).sin() * (-time / 0.11).exp() * 0.6;
        let x = r.noise();
        let bright = (x - hp.run(x)) * (-time / 0.012).exp() * 0.7;
        out.push((p1 + p2) * 0.55 + bright);
    }
    for (i, v) in out.iter_mut().enumerate() {
        *v *= edges(i, n);
    }
    out
}

/// Soft sines at given (Hz, start seconds, duration seconds). The bank's only
/// musical primitive.
fn chime(notes: &[(f32, f32, f32)]) -> Vec<f32> {
    let total = notes.iter().map(|(_, s, d)| s + d).fold(0.0f32, f32::max);
    let n = samples(total);
    let mut out = vec![0.0f32; n];
    let sr = SAMPLE_RATE as f32;
    for (hz, start, dur) in notes {
        let from = samples(*start);
        let len = samples(*dur);
        for k in 0..len {
            let i = from + k;
            if i >= n {
                break;
            }
            let time = k as f32 / sr;
            // A 4 ms attack, because a sine that starts at full amplitude
            // clicks — and a click on the hitmarker would be the loudest
            // thing in the mix.
            let env = attack(time, 0.004) * (-time / (dur * 0.45)).exp();
            out[i] += (std::f32::consts::TAU * hz * time).sin() * env;
        }
    }
    for (i, v) in out.iter_mut().enumerate() {
        *v *= edges(i, n);
    }
    out
}

/// A flat, detuned low tone. The refusal.
///
/// **Two CLOSELY spaced partials, not a harmonic pair.** The first cut used
/// `hz` and `hz × 1.5`, which is a perfect fifth — a chord, and a pleasant
/// one, which is the opposite of what "your input was refused" should sound
/// like. 1.06× puts the second partial ~9 Hz away, and two tones 9 Hz apart
/// beat against each other nine times a second. That beat is the buzz.
fn buzz(hz: f32, dur_s: f32) -> Vec<f32> {
    let n = samples(dur_s);
    let sr = SAMPLE_RATE as f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let time = i as f32 / sr;
        let a = (std::f32::consts::TAU * hz * time).sin();
        let b = (std::f32::consts::TAU * (hz * 1.06) * time).sin() * 0.9;
        out.push((a + b) * attack(time, 0.006) * edges(i, n));
    }
    out
}

/// A pitch sweep with noise in it — damage and death.
fn sweep(r: &mut Rng, dur_s: f32, from_hz: f32, to_hz: f32, noise_amp: f32) -> Vec<f32> {
    let n = samples(dur_s);
    let sr = SAMPLE_RATE as f32;
    let mut out = Vec::with_capacity(n);
    let mut lp = Lp::new(900.0);
    // Phase is integrated rather than computed from an instantaneous
    // frequency, because `sin(2π f(t) t)` sweeps at twice the rate you asked
    // for and lands an octave low. This is the one arithmetic trap in the
    // file.
    let mut phase = 0.0f32;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let time = i as f32 / sr;
        let hz = from_hz + (to_hz - from_hz) * t * t;
        phase += std::f32::consts::TAU * hz / sr;
        let env = (-time / (dur_s * 0.34)).exp();
        let n_part = lp.run(r.noise()) * noise_amp * (-time / (dur_s * 0.12)).exp();
        out.push((phase.sin() + n_part) * env * edges(i, n));
    }
    out
}

/// Leaves, then the crack, then the ground.
fn tree_fall(r: &mut Rng) -> Vec<f32> {
    let dur = 1.9f32;
    let n = samples(dur);
    let sr = SAMPLE_RATE as f32;
    let mut out = vec![0.0f32; n];
    // 1. The swell: band-limited noise rising over the first 1.1 s.
    let mut lp = Lp::new(2_800.0);
    let mut hp = Lp::new(400.0);
    for (i, v) in out.iter_mut().enumerate() {
        let time = i as f32 / sr;
        let rise = (time / 1.1).clamp(0.0, 1.0);
        let x = r.noise();
        let band = {
            let low = lp.run(x);
            low - hp.run(low)
        };
        *v += band * rise * rise * 0.75;
    }
    // 2. The crack at 1.05 s: a short, hard transient.
    let crack = samples(1.05);
    let mut chp = Lp::new(900.0);
    for k in 0..samples(0.12) {
        let i = crack + k;
        if i >= n {
            break;
        }
        let time = k as f32 / sr;
        let x = r.noise();
        out[i] += (x - chp.run(x)) * (-time / 0.020).exp() * 1.1;
    }
    // 3. The ground at 1.25 s: a low body that outlives everything.
    let thud = samples(1.25);
    for k in 0..(n - thud) {
        let i = thud + k;
        let time = k as f32 / sr;
        out[i] += (std::f32::consts::TAU * 52.0 * time).sin() * (-time / 0.20).exp() * 0.9;
        out[i] += (std::f32::consts::TAU * 88.0 * time).sin() * (-time / 0.11).exp() * 0.4;
    }
    for (i, v) in out.iter_mut().enumerate() {
        *v *= edges(i, n);
    }
    out
}

/// The wind bed: two layers of heavily low-passed noise under slow gust LFOs,
/// crossfaded into a seamless loop.
///
/// Two layers rather than one because a single filtered noise band is a hiss
/// at a fixed brightness, and wind is a low body with a gust riding on it.
///
/// **The gust rates are locked to the loop, and that is the whole trick.**
/// Each is an exact integer multiple of `1 / BED_LOOP_SECS`, so at the loop
/// point every LFO is back at the phase it started on — the tail's gust
/// envelope matches the head's *exactly*, and the crossfade is left blending
/// two noise realizations at the same level rather than papering over an
/// amplitude step. An "incommensurate rates so the pattern never repeats"
/// design sounds more sophisticated and is strictly worse: it guarantees the
/// join is the one place in the loop where the envelope jumps.
///
/// One gust per loop (10.5 s) and one at twice that (5.25 s), which is roughly
/// the register real wind gusts at and, more importantly, is at least one full
/// cycle of each inside the loop.
fn bed(r: &mut Rng) -> Vec<f32> {
    let n = samples(BED_SECS);
    let sr = SAMPLE_RATE as f32;
    let mut low = Lp::new(220.0);
    let mut mid_lp = Lp::new(1_600.0);
    let mut mid_hp = Lp::new(500.0);
    let mut out = Vec::with_capacity(n);
    let f0 = 1.0 / BED_LOOP_SECS;
    for i in 0..n {
        let time = i as f32 / sr;
        // The depths are floors, not swings: 0.70 ± 0.30 bottoms at 0.40 of
        // peak, about 8 dB, which is roughly what wind does. The first cut at
        // 0.55 ± 0.45 measured a 5.4x swing across the loop and read as the
        // ambience CUTTING OUT once every ten seconds rather than as weather —
        // `the_bed_gusts` now bounds it from both sides for that reason.
        let gust = 0.70 + 0.30 * (std::f32::consts::TAU * f0 * time).sin();
        let gust2 = 0.72 + 0.28 * (std::f32::consts::TAU * 2.0 * f0 * time + 1.7).sin();
        let x = r.noise();
        let body = low.run(x) * 1.6;
        let air = {
            let l = mid_lp.run(x);
            l - mid_hp.run(l)
        };
        out.push(body * gust + air * gust2 * 0.55);
    }
    loop_seam(out, samples(BED_FADE_SECS))
}

// ---------------------------------------------------------------------------
// Shaping and containers.
// ---------------------------------------------------------------------------

fn samples(secs: f32) -> usize {
    (secs * SAMPLE_RATE as f32).round().max(1.0) as usize
}

/// Linear rise to full over `attack_s`. Zero-length attacks are allowed and
/// return 1 immediately.
fn attack(time: f32, attack_s: f32) -> f32 {
    if attack_s <= 0.0 {
        1.0
    } else {
        (time / attack_s).clamp(0.0, 1.0)
    }
}

/// Short fades at both ends of a buffer: 0.5 ms in, 4 ms out.
///
/// **Both halves were found by the gate rather than reasoned out**, and the
/// head is the interesting one. Every cue ends on a decaying exponential,
/// which never actually reaches zero, so a tail fade is obviously owed — but
/// `metal()` also *started* at −0.28, because its noise term begins at full
/// amplitude on sample zero, and a buffer that starts on a step clicks at the
/// front of every single playback. A per-generator attack would have fixed
/// that one cue and left the next one to rediscover it; shaping both edges in
/// the one place every generator already passes through cannot be forgotten.
///
/// The head fade is 0.5 ms — 22 samples — deliberately: long enough to remove
/// the discontinuity, short enough that a rock footstep keeps its attack.
///
/// **Not applied to the bed**, which is the exception that proves it: a loop's
/// first sample must be continuous with its *last*, not with silence, and a
/// head fade there would carve a dip into the loop point once every six
/// seconds forever. `loop_seam` owns the bed's edges instead.
fn edges(i: usize, n: usize) -> f32 {
    let out = samples(0.004).min(n / 4).max(1);
    let inn = samples(0.0005).min(n / 4).max(1);
    let head = if i < inn { i as f32 / inn as f32 } else { 1.0 };
    let tail = if i + out >= n {
        (n - i) as f32 / out as f32
    } else {
        1.0
    };
    head * tail
}

/// Crossfade the tail of a buffer back into its head so it loops without a
/// seam, and return the loop.
///
/// The output is `buf.len() - fade` samples long. Sample `i` of the head is
/// blended with sample `len - fade + i` of the tail under an equal-power
/// (sin/cos) pair, so the loop point carries the tail's energy into the
/// head's, and the total power stays flat across the join — a linear
/// crossfade of two uncorrelated noise signals dips ~3 dB in the middle,
/// which is audible as a pulse once a second forever.
fn loop_seam(mut buf: Vec<f32>, fade: usize) -> Vec<f32> {
    let n = buf.len();
    if fade == 0 || fade * 2 >= n {
        return buf;
    }
    let loop_len = n - fade;
    for i in 0..fade {
        let t = i as f32 / fade as f32;
        let w_head = (t * std::f32::consts::FRAC_PI_2).sin();
        let w_tail = (t * std::f32::consts::FRAC_PI_2).cos();
        buf[i] = buf[i] * w_head + buf[loop_len + i] * w_tail;
    }
    buf.truncate(loop_len);
    buf
}

/// Scale to a target peak. A buffer with no energy is left alone rather than
/// divided by zero — and `tests/sound.rs` asserts no cue is one.
fn normalize(buf: &mut [f32], peak: f32) {
    let max = buf.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if max <= 1e-6 {
        return;
    }
    let k = peak / max;
    for v in buf.iter_mut() {
        *v *= k;
    }
}

/// 16-bit mono PCM in a WAV container.
///
/// Written by hand because it is 44 bytes and the alternative is a crate. The
/// layout is the canonical one: `RIFF` size `WAVE`, a 16-byte `fmt ` chunk
/// declaring PCM/mono/[`SAMPLE_RATE`], then `data`.
fn to_wav16(samples: &[f32]) -> Vec<u8> {
    let bytes = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + bytes);
    let sr = SAMPLE_RATE;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + bytes) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // format: PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // channels: mono
    out.extend_from_slice(&sr.to_le_bytes());
    out.extend_from_slice(&(sr * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(bytes as u32).to_le_bytes());
    for s in samples {
        // Clamped, not wrapped: a sample past full scale must be a flat top,
        // not a sign flip, which is the difference between a loud sound and a
        // catastrophic one. `normalize` should have made this unreachable and
        // the clamp is what makes "should" not matter.
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}
