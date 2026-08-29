//! Every sound this client makes, generated at boot from arithmetic.
//!
//! **There are no audio assets in this repo and this module is why.** Three
//! reasons, in the order they bind:
//!
//! 1. **Licence.** `CLAUDE.md` already records the trap: `SeedThree` ships
//!    bird recordings whose `README.txt` cites xeno-canto and *states no
//!    licence per file*, and `Eanpa-Sky`'s four recordings are CC BY-NC-SA —
//!    NC does not survive a sold product. Audio is the single worst-labelled
//!    asset class on the internet, and a game sold through elo's board
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

use super::music;
use super::{Cue, CUE_COUNT, SAMPLE_RATE};

use std::f32::consts::{PI, TAU};

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
/// **Measured 2026-08-11: 11.7 MB of WAV in ~0.8 s release, ~3.9 s debug** —
/// and the score is nearly all of both. Nine pieces at
/// `music::PIECE_S` each is 94 seconds of audio against the ~30 the rest of
/// the bank comes to, and it is dominated by `f32::sin`: roughly 65 million
/// evaluations across the drone, the pad and the plucked line.
///
/// That is a boot cost, not a frame cost, and it is paid on the loading screen
/// where the pipelines are already specializing — but it is no longer
/// negligible, and it is written down rather than implied. **A sine lookup
/// table was tried and measured SLOWER in both profiles** (release 0.90 →
/// 1.00 s), because the per-call `LazyLock` deref cost more than the `sin` it
/// replaced; reverted rather than kept on the theory that it should have
/// helped. The real lever is not an optimization at all: the day recorded
/// pieces replace `score`, this cost leaves with it.
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
        // Breaking the surface: `StepWater`'s gesture at four times the
        // length, with a low displacement body under it. A splash is a step
        // into water that did not stop at the ankle.
        Cue::Splash => splash(&mut r),
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

        // ---- the beds ---------------------------------------------------
        Cue::BedWind => bed(&mut r),
        Cue::BedSurf => surf(&mut r),
        Cue::BedUnder => under(&mut r),

        // ---- the animals ------------------------------------------------
        Cue::Snort => snort(&mut r),
        Cue::Howl => howl(&mut r),
        Cue::Growl => growl(&mut r),

        // ---- remote footsteps -------------------------------------------
        // The same ground under someone else's boot: BYTE-IDENTICAL to the
        // local surface, by delegation rather than by copied parameters, so
        // a tuning pass on a footstep can never fork the two. What makes a
        // remote step remote is its def — positional, at the body, culled by
        // the falloff law — never its waveform. `tests/sound.rs` pins the
        // equality.
        Cue::RemoteStepSand => render(Cue::StepSand),
        Cue::RemoteStepGrass => render(Cue::StepGrass),
        Cue::RemoteStepLitter => render(Cue::StepLitter),
        Cue::RemoteStepRock => render(Cue::StepRock),
        Cue::RemoteStepWater => render(Cue::StepWater),

        // ---- a remote swing ---------------------------------------------
        // The same arm through the same air, by delegation for the remote
        // steps' reason: what makes a swing remote is its def — positional,
        // at the body, culled by the falloff law — never its waveform, and
        // a copied parameter set is a fork waiting to happen.
        Cue::RemoteSwing => render(Cue::Swing),

        // ---- the two shots ------------------------------------------------
        // Both are `impact`, which is the bank's transient-plus-body
        // primitive, because that is exactly what a shot is: a crack and
        // whatever the mechanism rings at underneath it.
        //
        // **A bow.** Almost all transient and almost no body — a string
        // released is a broadband snap with a short woody thump off the
        // limbs, gone in a fifth of a second. The high-pass sits well up so
        // it stays a *tick* rather than a thud, which is what keeps it from
        // reading as a footstep at distance: its def already says it only
        // carries 40 m, and a sound that carries a short way and has energy
        // low down is a sound a player mistakes for their own boots.
        Cue::ShotBow => impact(
            &mut r,
            0.20,
            Tone {
                lo_hz: 240.0,
                lo_amp: 0.30,
                lo_tau: 0.030,
                noise_amp: 0.80,
                noise_tau: 0.022,
                lp_hz: 7_000.0,
                hp_hz: 700.0,
                attack_s: 0.0004,
                crackle: 0.35,
            },
        ),
        // **A gun**, and it is the loudest and longest transient in the
        // bank. Three things separate it from every impact above, and each
        // is doing a job at a hundred metres rather than in front of you:
        // a very low body (85 Hz) with a long tail, which is the half that
        // survives distance and tells you *something serious happened over
        // there*; a wide-open low-pass, because the crack's brightness is
        // what makes it read as a gun and not as a tree falling; and a
        // longer `noise_tau` than any impact, which is the report's slap
        // off the terrain rather than a strike on a surface.
        //
        // No falloff filtering — the mixer's law is amplitude only
        // (`sound::falloff`), so a distant shot here is a quiet shot and
        // not a muffled one. That is a real gap against the reference and
        // it belongs to the mixer rather than to this waveform.
        Cue::ShotGun => impact(
            &mut r,
            0.55,
            Tone {
                lo_hz: 85.0,
                lo_amp: 0.95,
                lo_tau: 0.150,
                noise_amp: 1.00,
                noise_tau: 0.060,
                lp_hz: 11_000.0,
                hp_hz: 120.0,
                attack_s: 0.0002,
                crackle: 0.15,
            },
        ),

        // ---- the forest layer -------------------------------------------
        Cue::Bird => bird(&mut r),

        // ---- the score ---------------------------------------------------
        // Nine pieces, one generator, and the table decides which: the arm
        // is a lookup in `music::PIECES` rather than nine parameter sets, so
        // adding a section or a tier is a change to that table alone.
        //
        // **The one wildcard arm in this match, and it is gated rather than
        // trusted.** A cue added to the enum and forgotten everywhere else
        // lands here and renders as silence — which `tests/sound.rs`'s
        // "every cue has energy" assertion turns into a red gate on the next
        // run. Silence rather than a panic because a bank that refuses to
        // build is a client that refuses to boot over a sound.
        _ => match music::piece_of(cue) {
            Some((section, tier)) => score(&mut r, section, tier),
            None => Vec::new(),
        },
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

/// A two-pole resonator — one formant.
///
/// The second filter in this module, and it earns its place where `Lp` cannot:
/// a formant is a *peak*, and no arrangement of one-poles makes a peak. Added
/// with the wolf, whose growl is a pulse train that only reads as a throat
/// once four of these are across it (`WOLF_FORMANTS`).
///
/// `y[n] = b0·x[n] + a1·y[n−1] + a2·y[n−2]` with the poles at radius
/// `r = e^(−πB/fs)` and angle `2πF/fs` — the textbook resonator. `r < 1`
/// always, so it is unconditionally stable and cannot ring away on us.
struct Res {
    b0: f32,
    a1: f32,
    a2: f32,
    y1: f32,
    y2: f32,
}

impl Res {
    fn new(hz: f32, bw: f32) -> Self {
        let sr = SAMPLE_RATE as f32;
        let r = (-std::f32::consts::PI * bw / sr).exp();
        let theta = std::f32::consts::TAU * hz / sr;
        let a1 = 2.0 * r * theta.cos();
        let a2 = -r * r;
        // **Normalized to unity gain AT THE PEAK, evaluated rather than
        // approximated**, and the difference is not cosmetic. The obvious
        // `b0 = 1 − r²` looks like a normalization and is not one: a
        // resonator's gain rises as its bandwidth narrows, so with the four
        // bandwidths in `WOLF_FORMANTS` it hands F1 a peak of 20.6 and F4 a
        // peak of 3.0 — and then a −6 dB/oct tilt on top multiplies a spread
        // that is already there, for 48:1 in F1's favour. That is one formant
        // with three inaudible companions, which would have made `growl`'s
        // doc comment a claim about a filter bank it did not have.
        //
        // `|1 − a₁e^{−jθ} − a₂e^{−2jθ}|` is that gain in closed form, so
        // dividing it out leaves every formant peaking at 1 and the source's
        // own spectrum is then the only thing shaping the envelope — which is
        // what the model says it is.
        let (c, s) = (theta.cos(), theta.sin());
        let (c2, s2) = ((2.0 * theta).cos(), (2.0 * theta).sin());
        let re = 1.0 - a1 * c - a2 * c2;
        let im = a1 * s + a2 * s2;
        Self {
            b0: (re * re + im * im).sqrt(),
            a1,
            a2,
            y1: 0.0,
            y2: 0.0,
        }
    }
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.a1 * self.y1 + self.a2 * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
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

/// A periodic surge envelope, locked to the loop like [`bed`]'s gusts are.
///
/// `harmonic` is how many surges fit in one loop, so it is an integer by
/// construction and the envelope is exactly continuous across the join.
/// `sharp` raises `(½ + ½ sin)` to a power: 1 is a sine, 3 is a short peak
/// with a long trough, which is what a wave arriving and draining away is.
fn surge(time: f32, harmonic: f32, phase: f32, sharp: i32) -> f32 {
    let f0 = 1.0 / BED_LOOP_SECS;
    let s = 0.5 + 0.5 * (std::f32::consts::TAU * harmonic * f0 * time + phase).sin();
    let mut v = s;
    for _ in 1..sharp {
        v *= s;
    }
    v
}

/// The surf bed: waves arriving, breaking, and draining back.
///
/// **Three layers and a lag, and the lag is the whole thing.** A break is a
/// low boom; the wash that follows it is broadband hiss; and the hiss arrives
/// *after* the boom and outlasts it. Put them in phase and the result is a
/// tremolo on white noise, which reads as a machine and not as a sea. The
/// offset is ~0.9 rad, about a seventh of a surge.
///
/// The surge rate is **two per loop**, i.e. one every 5.25 s, which is inside
/// a decibel of the 5.8 s period the renderer's longest wave actually runs at
/// (`render/water.rs`'s `omega` on a 52 m swell). Not a coincidence and not
/// enforced by anything — the two would have to be wired together to be
/// enforced, and a bed that had to know the wave set would be a coupling for
/// a fact nobody can hear the phase of.
fn surf(r: &mut Rng) -> Vec<f32> {
    let n = samples(BED_SECS);
    let sr = SAMPLE_RATE as f32;
    let mut boom = Lp::new(190.0);
    let mut wash_lp = Lp::new(4_200.0);
    let mut wash_hp = Lp::new(650.0);
    let mut deep = Lp::new(90.0);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let time = i as f32 / sr;
        // The break: sharp, twice a loop.
        let hit = 0.22 + 0.78 * surge(time, 2.0, 0.0, 3);
        // The wash: broader, lagging, and it never goes away entirely.
        let wash = 0.42 + 0.58 * surge(time, 2.0, -0.9, 2);
        // A second, slower swell so two consecutive breaks are not identical.
        let swell = 0.72 + 0.28 * surge(time, 1.0, 1.4, 1);
        let x = r.noise();
        let low = boom.run(x) * 1.7;
        let air = {
            let l = wash_lp.run(x);
            l - wash_hp.run(l)
        };
        let body = deep.run(x) * 0.9;
        out.push((low * hit + air * wash * 0.62 + body * swell * 0.5) * swell);
    }
    loop_seam(out, samples(BED_FADE_SECS))
}

/// The submerged bed.
///
/// Dark by construction rather than by a filter on something brighter: the
/// only content above 400 Hz is the bubbles, and there is not much of that.
/// `tests/sound.rs` asserts it is measurably darker than the wind bed, using
/// the same zero-crossing proxy the footstep surfaces are separated by — which
/// is the honest version of "it sounds underwater", given that the real answer
/// is a low-pass we have no node for (`sound::SNAPSHOTS`).
fn under(r: &mut Rng) -> Vec<f32> {
    let n = samples(BED_SECS);
    let sr = SAMPLE_RATE as f32;
    let mut rumble = Lp::new(120.0);
    let mut mid = Lp::new(380.0);
    let mut out = vec![0.0f32; n];
    for (i, v) in out.iter_mut().enumerate() {
        let time = i as f32 / sr;
        let slow = 0.62 + 0.38 * surge(time, 1.0, 0.0, 1);
        let x = r.noise();
        *v = rumble.run(x) * 2.4 * slow + mid.run(x) * 0.35;
    }
    // Bubbles: short rising sines, sparse. They are what stops the rumble
    // being a fan. One every ~1.4 s on average, and the loop crossfade takes
    // care of any that straddle the join.
    let count = (BED_SECS / 1.4) as usize;
    for b in 0..count {
        let at = samples(BED_SECS * (b as f32 + 0.35 * r.unit()) / count as f32);
        let hz = 240.0 + 520.0 * r.unit();
        let dur = 0.045 + 0.05 * r.unit();
        let len = samples(dur);
        let mut phase = 0.0f32;
        for k in 0..len {
            let i = at + k;
            if i >= n {
                break;
            }
            let t = k as f32 / len as f32;
            // Rising, because a bubble shrinks as it rises and its pitch goes
            // up with it.
            phase += std::f32::consts::TAU * (hz * (1.0 + 0.8 * t)) / sr;
            out[i] += phase.sin() * (1.0 - t) * 0.16;
        }
    }
    loop_seam(out, samples(BED_FADE_SECS))
}

/// Breaking the surface.
///
/// Three parts in the order they happen: the low *displacement* of a body
/// entering water, the broadband burst of the cavity collapsing, and the
/// droplets falling back. The droplet tail is what separates a splash from a
/// large footstep — without it this is just `StepWater` turned up.
fn splash(r: &mut Rng) -> Vec<f32> {
    let dur = 0.80f32;
    let n = samples(dur);
    let sr = SAMPLE_RATE as f32;
    let mut out = vec![0.0f32; n];
    let mut lp = Lp::new(3_800.0);
    let mut hp = Lp::new(420.0);
    for (i, v) in out.iter_mut().enumerate() {
        let time = i as f32 / sr;
        // The burst: a 12 ms swell into a 0.16 s decay. Water does not click.
        let env = attack(time, 0.012) * (-time / 0.16).exp();
        let x = r.noise();
        let band = {
            let l = lp.run(x);
            l - hp.run(l)
        };
        // The displacement: a low body that falls in pitch as the cavity
        // closes. Integrated phase, not `sin(2π f(t) t)` — see `sweep`.
        let body = (std::f32::consts::TAU * (135.0 - 60.0 * (time / dur).min(1.0)) * time).sin()
            * (-time / 0.10).exp()
            * 0.55;
        *v = band * env + body;
    }
    // Droplets: sparse high ticks over the second half.
    let from = samples(0.16);
    let mut drop_hp = Lp::new(2_600.0);
    for i in from..n {
        let time = (i - from) as f32 / sr;
        if r.unit() < 0.0016 {
            let len = samples(0.02).min(n - i);
            for k in 0..len {
                let t = k as f32 / sr;
                let x = r.noise();
                out[i + k] += (x - drop_hp.run(x)) * (-t / 0.004).exp() * 0.5;
            }
        }
        // And a thin wash under them, dying with the rest.
        out[i] += r.noise() * 0.03 * (-time / 0.22).exp();
    }
    for (i, v) in out.iter_mut().enumerate() {
        *v *= edges(i, n);
    }
    out
}

/// The pig: two nasal exhales with a grunt under them.
///
/// **The flutter is what makes it a snort.** A snort is breath forced
/// through a nostril, and the nostril flaps — so the noise band is
/// amplitude-modulated at ~26 Hz, which is the one thing that separates it
/// from a puff of band-passed static. Two bursts rather than one because a
/// pig snorts in pairs (the second shorter and softer, like an echo of
/// intent), and it is the double that reads as an animal rather than as a
/// pneumatic valve. The band is low (230–950 Hz) on purpose: the cue lives
/// in the ambience register and must not fight the 2–5 kHz carve the
/// reference keeps clear for what matters (`reference/AUDIO.md` §4).
///
/// Under both runs the grunt — a low sine falling from 105 Hz as the breath
/// runs out, phase integrated rather than computed from an instantaneous
/// frequency (see `sweep` — the one arithmetic trap in this file).
fn snort(r: &mut Rng) -> Vec<f32> {
    let dur = 0.55f32;
    let n = samples(dur);
    let sr = SAMPLE_RATE as f32;
    let mut out = vec![0.0f32; n];
    let mut lp = Lp::new(950.0);
    let mut hp = Lp::new(230.0);
    // (start s, length s, level) — the pair. The second at full level: the
    // first cut had it at 0.8 under a grunt with a 0.18 s tail, and the gap
    // between the exhales measured LOUDER than the second one — a pair the
    // gate could not hear as a pair (`a_snort_is_dark_and_double` found it).
    for (start, len, amp) in [(0.0f32, 0.16f32, 1.0f32), (0.24, 0.20, 1.0)] {
        let from = samples(start);
        for k in 0..samples(len) {
            let i = from + k;
            if i >= n {
                break;
            }
            let t = k as f32 / sr;
            let env = attack(t, 0.010) * (-t / (len * 0.38)).exp();
            let flutter = 0.55 + 0.45 * (std::f32::consts::TAU * 26.0 * t).sin();
            let x = r.noise();
            let band = {
                let l = lp.run(x);
                l - hp.run(l)
            };
            out[i] += band * env * flutter * amp;
        }
    }
    // The grunt decays fast enough (0.10 s tau) to be out of the way before
    // the second exhale — a longer tail fills the gap between the pair and
    // the snort stops snorting twice.
    let mut phase = 0.0f32;
    for (i, v) in out.iter_mut().enumerate() {
        let time = i as f32 / sr;
        let hz = 105.0 - 30.0 * (time / dur).min(1.0);
        phase += std::f32::consts::TAU * hz / sr;
        *v += phase.sin() * 0.32 * (-time / 0.10).exp();
    }
    for (i, v) in out.iter_mut().enumerate() {
        *v *= edges(i, n);
    }
    out
}

/// The wolf's vocal tract, as four formants: `(centre Hz, bandwidth Hz)`.
///
/// **Derived rather than picked.** Measured formant *dispersion* in large-dog
/// growls is 671 ± 253 Hz (Faragó et al. 2010); for a uniform tube
/// `dF = c / 2L`, so 671 Hz is a 26 cm tract — the 50 kg end of Riede &
/// Fitch's radiographed range, which is where a wolf sits. Taking
/// `dF = 680 Hz`, the formants are the odd multiples of `dF/2`: 340, 1020,
/// 1700, 2380. The fifth (3060) is dropped — it is above where the energy is
/// and it costs a filter.
///
/// The bandwidths are the one engineering choice here (no canid measurements
/// are published) at Q ≈ 6–10.
///
/// Two consequences worth stating, because both are easy to get wrong:
///
/// - **Formants are fixed per animal and do not glide with F0.** They are a
///   property of the throat, not of the note. `howl` moves its fundamental
///   through a contour and leaves this table alone.
/// - **This is what `Cue::pitch_var` is really varying.** A playback rate
///   moves F0 *and* the formants together, which is exactly a differently
///   sized animal — dogs judge body size off formant dispersion, so the
///   ±12 % on a howl is not a detune, it is a bigger or smaller wolf.
const WOLF_FORMANTS: [(f32, f32); 4] = [
    (340.0, 60.0),
    (1_020.0, 110.0),
    (1_700.0, 180.0),
    (2_380.0, 250.0),
];

/// How many harmonics a howl is built from. A wolf's howl carries harmonics
/// up to the eighteenth, but the energy sits in the first eight once
/// [`WOLF_FORMANTS`] has shaped it — past that the terms are below the breath
/// noise and cost samples for nothing.
const HOWL_HARMONICS: usize = 8;

/// The vocal tract's response at one frequency, as an amplitude weight.
///
/// The source–filter model, evaluated analytically instead of run as a filter:
/// a howl is built as a sum of harmonics at known frequencies, so the tract
/// can be applied as a gain per harmonic rather than as four resonators over
/// the whole buffer. Same model, no state, no chance of a filter ringing.
///
/// Each formant is a Lorentzian peak; the `1/f` term is the net spectral tilt
/// of the textbook chain — −12 dB/oct at the glottis, +6 dB/oct from radiation
/// at the mouth, so −6 dB/oct out.
fn formant_gain(f: f32) -> f32 {
    let mut g = 0.0;
    for (centre, bw) in WOLF_FORMANTS {
        let d = (f - centre) / bw;
        g += 1.0 / (1.0 + d * d);
    }
    g * (WOLF_FORMANTS[0].0 / f.max(60.0))
}

/// The wolf's far voice: a swell that rises fast, wavers on a plateau, and
/// falls away under where it started.
///
/// **The only sustained pitched cue in the bank**, and that is what makes it
/// hard rather than easy. `bird` already found the trap one register up — a
/// swept sine with no wobble reads as a kettle, not an animal — and a howl
/// holds its note for three seconds where a chirp is gone in eighty
/// milliseconds, so it has three seconds to sound like a test tone.
///
/// Every shape below is measured rather than chosen (Iberian-wolf and
/// Indian-wolf howl corpora; Tooze/Harrington/Fentress 1990):
///
/// - **The contour is asymmetric, and that is the finding.** Maximum F0 falls
///   in the first quarter of 79 % of howls and minimum F0 in the last quarter
///   of 78 %, so it is a fast rise onto the note and a long terminal fall —
///   not an arc. Roughly 260 Hz → 470 Hz by 18 % → drift → ~280 Hz.
/// - **The waver is slow.** 1–15 inflexion points across a call of several
///   seconds is 0.5–3 Hz, an order of magnitude under a singer's vibrato, at
///   ±2–4 % of F0. A 5 Hz wobble is a theremin.
/// - **The breaks are the highest-value detail.** Real howls carry 1–8
///   *frequency discontinuities* — instantaneous steps of ±8–15 %, part of the
///   nonlinear phenomena documented in canid howling. They are what stop the
///   pitch reading as an envelope generator, because no LFO does that.
/// - **Amplitude peaks early** (first half in 83 % of calls) and decays.
///
/// The harmonics are phase-integrated off the fundamental (see `sweep` — the
/// one arithmetic trap in this file) rather than computed from an
/// instantaneous frequency, so a moving pitch does not tear, and each one is
/// weighted by [`formant_gain`] rather than by a `1/h` rule: the throat
/// decides which harmonics are loud, and it is the same throat as the growl's.
fn howl(r: &mut Rng) -> Vec<f32> {
    let dur = 3.0f32;
    let n = samples(dur);
    let sr = SAMPLE_RATE as f32;
    let mut out = vec![0.0f32; n];
    // The plateau pitch. 400–470 Hz brackets the Indian-wolf mean of
    // 422 ± 126 Hz. Each wolf is its own animal before `Cue::pitch_var` ever
    // gets to it: the bank holds one howl, so the spread that stops a chorus
    // being a unison starts here and the playback rate widens it.
    let root = 400.0 + r.unit() * 70.0;
    // Three breaks, at drawn points in the plateau, each a step that persists
    // until the next one — a discontinuity, not a bump.
    let breaks: [(f32, f32); 3] = [
        (0.30 + r.unit() * 0.08, 1.0 + (r.unit() - 0.5) * 0.24),
        (0.46 + r.unit() * 0.08, 1.0 + (r.unit() - 0.5) * 0.24),
        (0.60 + r.unit() * 0.06, 1.0 + (r.unit() - 0.5) * 0.24),
    ];
    let mut phase = [0.0f32; HOWL_HARMONICS];
    let mut breath_lp = Lp::new(2_600.0);
    let mut breath_hp = Lp::new(900.0);
    for (i, v) in out.iter_mut().enumerate() {
        let t = i as f32 / sr;
        let u = t / dur;
        // Rise over the first 18 %, hold to 72 %, then the terminal fall to
        // 0.60 of the plateau — which ends *below* where the rise began, the
        // shape that reads as a call ending rather than a note switched off.
        let (shape, held) = if u < 0.18 {
            (0.55 + 0.45 * smooth(u / 0.18), 0.0)
        } else if u < 0.72 {
            (1.0, 1.0)
        } else {
            (1.0 - 0.40 * smooth((u - 0.72) / 0.28), 0.0)
        };
        let waver = 1.0 + 0.03 * held * (std::f32::consts::TAU * 1.6 * t).sin();
        let mut step = 1.0f32;
        for (at, mult) in breaks {
            if u >= at {
                step = mult;
            }
        }
        let f0 = root * shape * waver * step;
        // Peaks at ~28 % and decays from there — never symmetric.
        let env = attack(t, 0.22) * (1.0 - 0.75 * smooth(((u - 0.28) / 0.72).clamp(0.0, 1.0)));
        let mut tone = 0.0f32;
        for (h, ph) in phase.iter_mut().enumerate() {
            let mult = h as f32 + 1.0;
            let f = f0 * mult;
            *ph += std::f32::consts::TAU * f / sr;
            tone += ph.sin() * formant_gain(f);
        }
        // Breath under the tone: inaudible as noise, and the difference
        // between an oscillator and a mouth. Under 5 %, per the corpus.
        let x = r.noise();
        let breath = {
            let l = breath_lp.run(x);
            l - breath_hp.run(l)
        };
        *v = (tone * 0.55 + breath * 0.045) * env * edges(i, n);
    }
    out
}

/// The wolf's near voice: low, rough and continuous.
///
/// Where the howl is a tone, this is a **texture** — but not a noise, and
/// that is the correction the research forced. A growl is tonal-with-noise:
/// dogs read body size off the *formant spacing* in a growl, which is
/// impossible unless the source is periodic enough to excite resolvable
/// formants (Faragó et al. 2010). So it is a pulse train through the same
/// [`WOLF_FORMANTS`] the howl uses, with noise as a 15–30 % aperiodic layer
/// over it — not filtered hiss with a hum underneath.
///
/// Three source facts, all measured:
///
/// - **F0 70–110 Hz.** Slow enough that the ear resolves individual glottal
///   pulses, which is what "rough" is here. The rattle *is* the fundamental;
///   there is no separate tremolo doing that job.
/// - **Period doubling is the character.** Real growls drop intermittently
///   into a regime where alternate pulses differ in amplitude, putting a
///   subharmonic at F0/2 (35–55 Hz). It runs across the middle of the call
///   and stops, because a growl that does it throughout is a synthesizer
///   patch.
/// - **The envelope is near-square**, with a shallow 4–8 Hz breath ripple.
///   A growl does not swell like a howl; it is already happening when you
///   hear it.
///
/// It must also be tellable from the howl with your back turned, which the
/// shared formant bank does not do on its own — what separates them is that
/// this source is an octave and a half lower and never moves.
fn growl(r: &mut Rng) -> Vec<f32> {
    let dur = 1.15f32;
    let n = samples(dur);
    let sr = SAMPLE_RATE as f32;
    let mut out = vec![0.0f32; n];
    // The tract, as four parallel resonators at equal weight — `Res::new`
    // normalizes each to unity peak, so equal here means equal.
    //
    // **The spectral tilt lives on the SOURCE for this voice, not on the
    // filter**, which is the one place the two halves of the shared model are
    // expressed differently. `formant_gain` folds the −6 dB/oct into a
    // per-harmonic weight because the howl is a harmonic sum with a known
    // list of frequencies; a pulse train has no such list, so the same tilt
    // has to be a filter on the source — and it has to be somewhere. A
    // hard-edged pulse has infinite bandwidth and drives F2, F3 and F4 as
    // hard as F1: the first cut of this function did exactly that and came
    // out BRIGHTER than the howl, which
    // `a_growl_is_the_darkest_voice_and_a_held_one` caught.
    let mut tract: Vec<Res> = WOLF_FORMANTS
        .iter()
        .map(|&(centre, bw)| Res::new(centre, bw))
        .collect();
    // Two one-poles in series: −12 dB/oct above 260 Hz, the textbook glottal
    // source spectrum. A real fold does not emit a sawtooth.
    // The glottal pulse. `ph` walks in seconds and the period is redrawn each
    // time it wraps, so no two cycles are the same length — 0.3–0.7 % jitter
    // is what a real fold does and a fixed period is what a sawtooth does.
    let mut glottis = [Lp::new(260.0), Lp::new(260.0)];
    let mut ph = 0.0f32;
    let mut period = 1.0 / 92.0;
    let mut alternate = false;
    for (i, v) in out.iter_mut().enumerate() {
        let t = i as f32 / sr;
        let u = t / dur;
        // The doubled regime, across the middle 45 % of the call.
        let doubling = (0.28..0.73).contains(&u);
        ph += 1.0 / sr;
        if ph >= period {
            ph -= period;
            period = 1.0 / (74.0 + r.unit() * 36.0);
            alternate = !alternate;
        }
        // A narrow falling pulse rather than a full-width saw — 60 % duty,
        // which is the buzzier source a growl needs. Amplitude alternates
        // inside the doubled regime and does not outside it.
        let frac = ph / period;
        let pulse = if frac < 0.6 {
            1.0 - frac / 0.6 * 2.0
        } else {
            -1.0
        };
        let level = if doubling && alternate { 0.6 } else { 1.0 };
        let mut src = pulse * level;
        for lp in glottis.iter_mut() {
            src = lp.run(src);
        }
        // **The aperiodic layer is summed at the SOURCE, not over the
        // output**, which is the textbook chain and was the second half of
        // the same mistake: breath is made at the glottis, so the tract
        // shapes it exactly as it shapes the pulse. Mixed in after the
        // filters it is raw wideband hiss, and it then dominates the
        // brightness of a sound whose whole character is that it is dark.
        // Not low-passed with the pulse, because that rolloff describes the
        // *pulse shape* and aspiration has none.
        src += r.noise() * 0.20;
        let mut voiced = 0.0f32;
        for res in tract.iter_mut() {
            voiced += res.run(src);
        }
        // ±3 dB at 6 Hz — breath across a held sound, not a tremolo.
        let ripple = 0.85 + 0.15 * (std::f32::consts::TAU * 6.0 * t).sin();
        // Near-square: 40 ms on, 80 ms off, sustained in between.
        let env = attack(t, 0.040) * (1.0 - smooth(((u - 0.93) / 0.07).clamp(0.0, 1.0)));
        *v = voiced * ripple * env * edges(i, n);
    }
    out
}

// ---------------------------------------------------------------------------
// Shaping and containers.
// ---------------------------------------------------------------------------
// The forest layer.
// ---------------------------------------------------------------------------

/// A bird call: three whistled chirps with a gap between them.
///
/// **Whistles, not noise bursts**, which is what separates this from every
/// other cue in the bank: a footstep is filtered noise with an envelope and a
/// bird is a near-pure tone that MOVES. The tell is the vibrato — a swept
/// sine with no wobble in it reads as a kettle or a test tone, and 3 % at
/// 38 Hz is enough to stop it.
fn bird(r: &mut Rng) -> Vec<f32> {
    let n = samples(0.62);
    let mut out = vec![0.0f32; n];
    // A rising call, a shorter answer, and a falling tail note. The spacing
    // is jittered off the cue's own stream so the phrase is not three evenly
    // spaced beeps — `Cue::pitch_var` then varies the whole call, but a
    // rhythm inside it cannot come from a playback rate.
    let base = 2_600.0 + r.unit() * 500.0;
    chirp(&mut out, 0.00, 0.085, base * 0.82, base * 1.06, 1.0);
    chirp(
        &mut out,
        0.15 + r.unit() * 0.03,
        0.070,
        base * 1.02,
        base * 1.18,
        0.85,
    );
    chirp(
        &mut out,
        0.30 + r.unit() * 0.04,
        0.120,
        base * 1.12,
        base * 0.74,
        0.7,
    );
    for (i, v) in out.iter_mut().enumerate() {
        *v *= edges(i, n);
    }
    out
}

/// One whistled note: a frequency sweep with vibrato under a bell envelope.
fn chirp(out: &mut [f32], start_s: f32, dur_s: f32, from_hz: f32, to_hz: f32, amp: f32) {
    let sr = SAMPLE_RATE as f32;
    let from = samples(start_s);
    let n = samples(dur_s);
    // Phase INTEGRATED, never `sin(2π f(t) t)` — `sweep`'s trap, one function
    // up: that form sweeps at twice the rate asked for and lands an octave
    // low.
    let mut phase = 0.0f32;
    for k in 0..n {
        let Some(slot) = out.get_mut(from + k) else {
            break;
        };
        let t = k as f32 / sr;
        let u = k as f32 / n as f32;
        let vib = 1.0 + 0.03 * (TAU * 38.0 * t).sin();
        phase += TAU * (from_hz + (to_hz - from_hz) * u) * vib / sr;
        // A bell, because a whistle has neither an attack transient nor a
        // tail: it fades up and back down inside its own length.
        *slot += (phase.sin() + 0.25 * (phase * 2.0).sin()) * (u * PI).sin() * amp;
    }
}

// ---------------------------------------------------------------------------
// The score. `sound::music` decides WHEN; this decides what it sounds like.
// ---------------------------------------------------------------------------

/// The key. A2, and everything in a piece is a semitone offset from it.
const ROOT_HZ: f32 = 110.0;

/// Beats per minute. Chosen so a section is a whole number of beats:
/// `music::SECTION_S` is 8 s, which is exactly 12 beats at 90 — a piece is a
/// phrase that ends where it should rather than a clip that stops.
const BPM: f32 = 90.0;
const BEAT_S: f32 = 60.0 / BPM;

/// Each section's chord, as semitones from [`ROOT_HZ`].
///
/// A minor plagal turn — i, ♭VI, ♭VII — which is the most-used progression in
/// survival and exploration scoring for a reason: it moves without resolving,
/// so a song can be cut short at any section boundary (which is exactly what
/// `music::Director` does) and never sound interrupted. **That property is
/// the whole reason the progression is this one**: a cadence that wanted to
/// land on the tonic would be a cadence the director keeps stepping on.
const CHORDS: [[f32; 3]; music::SECTIONS] = [
    [0.0, 3.0, 7.0],  // i   — A C E
    [-4.0, 0.0, 3.0], // ♭VI — F A C
    [-2.0, 2.0, 5.0], // ♭VII— G B D
];

/// The melody's note set: A minor pentatonic, two octaves up from the root.
const PENT: [f32; 5] = [12.0, 15.0, 17.0, 19.0, 22.0];

/// One piece: drone, pad, melody, and — at the top tier — a pulse, all under
/// a reverb whose tail is what makes a cut from any piece to any other sound
/// like a join.
///
/// The buffer is `music::PIECE_S` long and **no note starts after
/// `music::SECTION_S`**. That is the arithmetic behind the reference's
/// "pieces end with reverb tails and delays rather than looping cleanly": the
/// last `music::TAIL_S` holds only what the reverb is still ringing out, so
/// the next piece can start at the body's end and play over it.
fn score(r: &mut Rng, section: usize, tier: music::Tier) -> Vec<f32> {
    let n = samples(music::PIECE_S);
    let body_s = music::SECTION_S;
    let mut out = vec![0.0f32; n];
    let sr = SAMPLE_RATE as f32;
    let chord = CHORDS[section];

    // How the tiers differ, in one table rather than in three branches. Every
    // column is a knob (`DECISIONS.md` §open, "music v0") and none is
    // measured — the ORDER is what is designed: each step up is denser,
    // lower, and shorter-decayed than the one below it.
    let (drone_amp, pad_amp, note_amp, decay_s, pulse_amp, subdiv) = match tier {
        music::Tier::Calm => (0.30, 0.34, 0.30, 1.60, 0.00, 3),
        music::Tier::Tense => (0.42, 0.28, 0.34, 0.90, 0.10, 2),
        music::Tier::Combat => (0.55, 0.20, 0.38, 0.55, 0.34, 1),
    };

    // 1. The drone: root and fifth, detuned against each other so they beat
    //    slowly. It is the only layer that is present in every sample of the
    //    body, and it is what makes the nine pieces one theme.
    for (i, v) in out.iter_mut().enumerate() {
        let t = i as f32 / sr;
        let env = attack(t, 1.2) * ((body_s - t) / 2.0).clamp(0.0, 1.0);
        if env <= 0.0 {
            continue;
        }
        let a = (TAU * ROOT_HZ * t).sin();
        let b = (TAU * ROOT_HZ * 1.5 * 1.001 * t).sin() * 0.6;
        // A third partial an octave down gives it a floor without adding a
        // note: at 55 Hz it is felt more than heard.
        let c = (TAU * ROOT_HZ * 0.5 * t).sin() * 0.5;
        *v += (a + b + c) * env * drone_amp * 0.33;
    }

    // 2. The pad: the section's chord, slow in and slow out. This is the
    //    layer that says WHICH section you are hearing.
    for semis in chord {
        pad(&mut out, hz(semis), pad_amp * 0.33, body_s, 0.0015);
    }

    // 3. The melody, on the beat grid. `subdiv` is how many beats apart the
    //    slots are, so the tiers differ in density without differing in
    //    tempo — a piece that changed tempo could not be cut against its
    //    neighbours.
    let beats = (body_s / BEAT_S).floor() as usize;
    let mut prev = 2usize;
    for b in (0..beats).step_by(subdiv) {
        // Not every slot sounds: a rest is what stops a melody being a scale.
        if r.unit() < 0.18 {
            continue;
        }
        // Step to a neighbouring degree more often than leaping, which is
        // the cheapest thing that makes a note sequence read as a line
        // rather than as a draw.
        let step = (r.next_u32() % 3) as i32 - 1;
        prev = (prev as i32 + step).clamp(0, PENT.len() as i32 - 1) as usize;
        let octave = if tier == music::Tier::Combat && r.unit() < 0.4 {
            -12.0
        } else {
            0.0
        };
        pluck(
            &mut out,
            samples(b as f32 * BEAT_S),
            hz(PENT[prev] + octave),
            decay_s,
            note_amp,
        );
        // The top tier answers itself a fifth up on the off-beat: one more
        // voice, no new material.
        if tier == music::Tier::Combat {
            pluck(
                &mut out,
                samples((b as f32 + 0.5) * BEAT_S),
                hz(PENT[prev] + 7.0),
                decay_s * 0.6,
                note_amp * 0.5,
            );
        }
    }

    // 4. The pulse: a low thump on every third beat, and only when there is
    //    something to be tense about. It is the layer a player notices
    //    arriving, which is the whole point of having tiers at all.
    if pulse_amp > 0.0 {
        for b in (0..beats).step_by(3) {
            thump(&mut out, samples(b as f32 * BEAT_S), pulse_amp);
        }
    }

    // 5. The tail. Everything above is dry; this is what rings past the body.
    let mut out = reverb(&out, 0.38);
    for (i, v) in out.iter_mut().enumerate() {
        *v *= edges(i, n);
    }
    out
}

/// A semitone offset from [`ROOT_HZ`], in Hz.
fn hz(semis: f32) -> f32 {
    ROOT_HZ * (semis / 12.0).exp2()
}

/// A struck string: four harmonics, each decaying faster than the one below
/// it. The cheapest thing that is not a sine and does not read as a beep.
fn pluck(out: &mut [f32], start: usize, hz: f32, decay_s: f32, amp: f32) {
    let sr = SAMPLE_RATE as f32;
    // Rendered until it is inaudible rather than for a fixed length, so a
    // note struck near the end of the body decays INTO the tail instead of
    // being cut at the body's edge.
    let n = samples(decay_s * 3.0);
    for k in 0..n {
        let Some(slot) = out.get_mut(start + k) else {
            break;
        };
        let t = k as f32 / sr;
        let mut v = 0.0;
        // **One `exp` per sample, not four.** The h-th harmonic's envelope is
        // `exp(-t·h/τ)`, which is `exp(-t/τ)` raised to h — so the four
        // decays are successive multiplications of the first. Exactly the same
        // arithmetic, a quarter of the transcendentals, and `exp` is as
        // expensive as `sin` in the loop that dominates the whole bank's
        // generation time.
        let fall = (-t / decay_s).exp();
        let mut decay = fall;
        for h in 1..=4u32 {
            let hf = h as f32;
            // 1/h² amplitudes and a decay that scales with h: a bright
            // attack that darkens as it falls, which is what a struck thing
            // does and a sine does not.
            v += (TAU * hz * hf * t).sin() * (1.0 / (hf * hf)) * decay;
            decay *= fall;
        }
        *slot += v * attack(t, 0.005) * amp;
    }
}

/// A slow chord voice: two detuned sines swelling in and back out over the
/// body.
fn pad(out: &mut [f32], hz: f32, amp: f32, body_s: f32, detune: f32) {
    let sr = SAMPLE_RATE as f32;
    for (i, v) in out.iter_mut().enumerate() {
        let t = i as f32 / sr;
        let env = attack(t, 2.0) * ((body_s - t) / 1.5).clamp(0.0, 1.0);
        if env <= 0.0 {
            continue;
        }
        let a = (TAU * hz * t).sin();
        let b = (TAU * hz * (1.0 + detune) * t).sin();
        *v += (a + b) * 0.5 * env * amp;
    }
}

/// The pulse: a pitch-dropping sine with a noise transient on it.
fn thump(out: &mut [f32], start: usize, amp: f32) {
    let sr = SAMPLE_RATE as f32;
    let dur = 0.34;
    let n = samples(dur);
    let mut phase = 0.0f32;
    for k in 0..n {
        let Some(slot) = out.get_mut(start + k) else {
            break;
        };
        let t = k as f32 / sr;
        // 92 Hz falling to 45 over the first tenth of a second. Integrated,
        // for `chirp`'s reason.
        let f = 45.0 + 47.0 * (-t / 0.045).exp();
        phase += TAU * f / sr;
        *slot += phase.sin() * (-t / 0.10).exp() * attack(t, 0.003) * amp;
    }
}

/// A Schroeder reverb — four parallel combs into two series allpasses.
///
/// **The tail is not decoration; it is the mechanism.** `reference/AUDIO.md`
/// §8: pieces end with reverb tails rather than looping cleanly, and the
/// stated reason is that it lets the system cut from any piece to any other
/// in any order without stopping playback and without fading. Everything
/// `music::Director` does about transitions rests on there being ~2.5 s of
/// ring-out after the last note.
///
/// The delays are mutually prime-ish in samples so the combs do not reinforce
/// into a ringing pitch; the feedback is sized for a ~2.3 s RT60 at this
/// sample rate, which is what fills `music::TAIL_S`.
fn reverb(dry: &[f32], wet: f32) -> Vec<f32> {
    const COMBS: [(usize, f32); 4] = [(1557, 0.90), (1617, 0.89), (1491, 0.90), (1422, 0.89)];
    const ALLPASS: [(usize, f32); 2] = [(225, 0.5), (556, 0.5)];
    let mut acc = vec![0.0f32; dry.len()];
    for (len, fb) in COMBS {
        let mut buf = vec![0.0f32; len];
        let mut i = 0usize;
        for (k, x) in dry.iter().enumerate() {
            let y = buf[i];
            buf[i] = x + y * fb;
            i = (i + 1) % len;
            acc[k] += y * 0.25;
        }
    }
    for (len, g) in ALLPASS {
        let mut buf = vec![0.0f32; len];
        let mut i = 0usize;
        for v in acc.iter_mut() {
            let d = buf[i];
            let y = d - g * *v;
            buf[i] = *v + g * d;
            i = (i + 1) % len;
            *v = y;
        }
    }
    dry.iter()
        .zip(acc.iter())
        .map(|(d, w)| d * (1.0 - wet) + w * wet)
        .collect()
}

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

/// Smoothstep on [0, 1] — `3x² − 2x³`, flat at both ends.
///
/// Used where a shape has to *arrive* rather than change direction: a howl's
/// rise onto its note and its fall off it are both this, and a linear ramp
/// there is audible as a corner in the pitch. `attack` is deliberately linear
/// and stays so — an amplitude ramp of a few milliseconds has no corner to
/// hear, and this is for the ones that last a third of a second.
fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
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
