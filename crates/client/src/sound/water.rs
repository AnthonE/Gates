//! What water sounds like from where you are standing: how much surf is
//! within earshot, whether your head is under it, and when you broke the
//! surface.
//!
//! Pure, allocation-free, and driven by `terrain::height` — the same function
//! `render/water.rs` draws the sea from and `sound::steps` picks a footstep
//! off. **The sound cannot disagree with the picture** because both read the
//! one worldgen; that is the rule `steps.rs` already runs on, applied to the
//! bed instead of to the step.
//!
//! `reference/WATER.md` §7 is the research. Two of its findings bind here:
//!
//! - **Ambience near water is localized ambience.** The reference's earliest
//!   published pass is "seagull and tide noises when coming near bodies of
//!   water", and river sound was later folded into the localized ambience
//!   system precisely so it would fade in and out smoothly rather than
//!   switching. So the surf is a *level*, computed from the world, never a
//!   trigger.
//! - **Their underwater sound effect once stayed on after disconnecting.** A
//!   mix state that outlived its cause. [`submerged`] is a pure function of a
//!   height against `SEA_LEVEL` recomputed every frame, so there is no state
//!   here that *can* outlive anything.

use sim_core::terrain::{self, SEA_LEVEL};

/// How far inland the surf still carries, metres.
///
/// A knob, and a modest one on purpose (`DECISIONS.md` §open, "water audio
/// v0"). The reference's own scale datum for a loud sound is that a silenced
/// weapon carries 40 m where it used to carry 100 (`reference/AUDIO.md` §1),
/// so a shoreline heard from a couple of hundred metres inland would be the
/// loudest thing on the island.
pub const SURF_R_M: f32 = 96.0;

/// Bearings the exposure probe walks, as unit vectors.
///
/// Authored rather than computed for the same reason `render/water.rs`'s wave
/// directions are: a trig call that can be paid offline is paid offline. Eight
/// is enough to tell a cove from a headland and cheap enough to run every
/// frame — the whole probe is 24 `height` taps, against the ~21 k the ground
/// streamer spends on one chunk.
const D: f32 = std::f32::consts::FRAC_1_SQRT_2;
const BEARINGS: [[f32; 2]; 8] = [
    [1.0, 0.0],
    [D, D],
    [0.0, 1.0],
    [-D, D],
    [-1.0, 0.0],
    [-D, -D],
    [0.0, -1.0],
    [D, -D],
];

/// Rings the probe samples, as fractions of [`SURF_R_M`], with the weight each
/// carries. Nearer water is louder water; the weights are the falloff law
/// sampled at three radii rather than a second curve that could disagree with
/// it (`sound::falloff` is `(1 − d/R)²`, which is 0.5625, 0.25 and 0.0625
/// here).
const RINGS: [(f32, f32); 3] = [(0.25, 0.5625), (0.5, 0.25), (0.75, 0.0625)];

/// How much sea is within earshot, 0..1.
///
/// A weighted fraction of a fixed 24-point probe that is below sea level. Not
/// a distance to the nearest water: a player standing in the middle of a bay
/// hears surf from three sides and one on a spit hears it from two, and a
/// nearest-point measure cannot tell those apart. It is also why the probe is
/// a fixed pattern rather than a search — it has to be the same 24 taps every
/// frame or the level flickers as the search finds different water.
pub fn shore_exposure(seed: u64, x: f32, z: f32) -> f32 {
    let mut hit = 0.0f32;
    let mut total = 0.0f32;
    for (frac, weight) in RINGS {
        let r = SURF_R_M * frac;
        for b in BEARINGS {
            total += weight;
            if terrain::height(seed, x + b[0] * r, z + b[1] * r) < SEA_LEVEL {
                hit += weight;
            }
        }
    }
    if total <= 0.0 {
        return 0.0;
    }
    (hit / total).clamp(0.0, 1.0)
}

/// The quietest the surf bed gets when there is any sea within earshot at all.
///
/// A floor rather than a ramp from zero: one probe point in the sea is a
/// distant shoreline, not silence, and a level that reached zero would make
/// the bed switch on and off as a player walked a contour.
pub const SURF_FLOOR: f32 = 0.18;

/// The surf bed's target gain from an exposure.
pub fn surf_gain(exposure: f32) -> f32 {
    if exposure <= 0.0 {
        return 0.0;
    }
    let e = exposure.clamp(0.0, 1.0);
    (SURF_FLOOR + (1.0 - SURF_FLOOR) * e).min(1.0)
}

/// Is the listener's head under the water?
///
/// Takes the *ear* height, not the feet: the mix changes when your head goes
/// under, and a player wading chest-deep is still hearing the world above.
pub fn submerged(ear_y: f32) -> bool {
    ear_y < SEA_LEVEL
}

/// The speed at which breaking the surface is at full gain, m/s.
pub const SPLASH_FULL_SPEED: f32 = 4.0;
/// The quietest a splash gets — stepping in slowly still makes a sound.
pub const SPLASH_MIN_GAIN: f32 = 0.35;

/// The waterline, as a thing you cross.
///
/// Entering and leaving are one cue and one crossing: `sound::steps` already
/// plays `StepWater` for every stride taken in the shallows, so this is the
/// *edge*, not the wading.
#[derive(Default)]
pub struct Waterline {
    /// Which side of the waterline we were on, and at what height. `None`
    /// before the first sample — the frame a world is joined must not be read
    /// as a crossing.
    last: Option<(bool, f32)>,
}

impl Waterline {
    /// Sample this frame's foot height. Returns a splash gain when the surface
    /// was broken in either direction.
    ///
    /// The vertical speed is measured here rather than passed in, because the
    /// alternative is the same fact tracked in two places: the renderer would
    /// have to keep a previous height beside a struct whose entire job is to
    /// remember the previous height. It makes a dive louder than a step, which
    /// is the construction `steps::sample` uses for a sprint against a walk.
    pub fn sample(&mut self, feet_y: f32, dt_s: f32) -> Option<f32> {
        let below = feet_y < SEA_LEVEL;
        let (was, was_y) = self.last.replace((below, feet_y))?;
        if was == below {
            return None;
        }
        let speed = if dt_s > 0.0 {
            (feet_y - was_y).abs() / dt_s
        } else {
            0.0
        };
        let t = (speed / SPLASH_FULL_SPEED).clamp(0.0, 1.0);
        Some(SPLASH_MIN_GAIN + (1.0 - SPLASH_MIN_GAIN) * t)
    }

    /// Forget where we were — a respawn, a teleport, or a new world. Without
    /// it the first sample in the next world is measured against the last
    /// one's waterline and splashes on dry land.
    pub fn reset(&mut self) {
        self.last = None;
    }
}
