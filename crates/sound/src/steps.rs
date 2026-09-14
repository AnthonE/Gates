//! Footsteps: when one happens, and what it lands on.
//!
//! Two facts, both pure, both testable without a window:
//!
//! - **Cadence is distance, not time.** One step per [`STRIDE_M`] of ground
//!   covered. A timer would make a sprinting player's steps land at the same
//!   rate as a walking player's, which is the tell that gives away a
//!   footstep system built on a clock; distance gets sprint cadence for free
//!   and costs one subtraction.
//! - **The surface is the terrain's, not a guess.** `terrain::splat` already
//!   answers "what is the ground made of here" for the renderer's own
//!   blending, and it is a pure function of the seed and the position. The
//!   footstep reads the same four weights the ground is drawn with, so the
//!   sound cannot disagree with the picture — which is the whole reason the
//!   reference records barefoot footsteps per surface (`reference/AUDIO.md`
//!   §5) and then has to keep the surface tables in sync by hand.
//!
//! The channel order is `terrain::splat`'s own: `[sand, grass, forest floor,
//! rock]`. `TERRAIN.md` owns it; this module reads it and must not restate it
//! as a number.

use super::Cue;

/// Metres of ground covered per footstep (`DECISIONS.md` §open, "audio v0").
///
/// Not measured against the reference — a stride is a knob and this one ships
/// at roughly a walking pace for a 1.6 m eye.
pub const STRIDE_M: f32 = 0.85;

/// Below this speed nothing is heard at all, m/s. Stops a player nudged by a
/// collision resolve from ticking the odometer into a step.
pub const STEP_MIN_SPEED: f32 = 0.4;

/// The speed at which a footstep is at full gain, m/s. Slower is quieter,
/// linearly, floored at [`STEP_MIN_GAIN`] — the reference's mix has footsteps
/// as one of the two things ambience is EQ-carved to make room for
/// (`reference/AUDIO.md` §4), so they are never inaudible, only quieter.
pub const STEP_FULL_SPEED: f32 = 5.5;
/// The quietest a footstep gets.
pub const STEP_MIN_GAIN: f32 = 0.45;

/// Which footstep a splat weight set calls for.
///
/// **Argmax, not a blend.** A blended footstep is two samples at partial gain,
/// which is two voices out of [`super::VOICE_CAP`] for one step and sounds
/// like neither surface. The dominant channel is what the ground mostly is.
pub fn surface_cue(splat: [u8; 4], below_sea: bool) -> Cue {
    if below_sea {
        return Cue::StepWater;
    }
    let mut best = 0usize;
    for (i, w) in splat.iter().enumerate() {
        if *w > splat[best] {
            best = i;
        }
    }
    match best {
        0 => Cue::StepSand,
        1 => Cue::StepGrass,
        2 => Cue::StepLitter,
        _ => Cue::StepRock,
    }
}

/// The positional twin of a local footstep cue — the same surface heard at
/// another body (`render/audio.rs::remote_steps` is the producer). Identity
/// on anything that is not a footstep: only the step family has remote
/// twins, and every caller feeds this [`surface_cue`]'s own output.
pub fn remote(cue: Cue) -> Cue {
    match cue {
        Cue::StepSand => Cue::RemoteStepSand,
        Cue::StepGrass => Cue::RemoteStepGrass,
        Cue::StepLitter => Cue::RemoteStepLitter,
        Cue::StepRock => Cue::RemoteStepRock,
        Cue::StepWater => Cue::RemoteStepWater,
        other => other,
    }
}

/// The odometer. One per body being listened to: the local player's lives in
/// `render/audio.rs::Sound` and reads the predictor; each remote body
/// carries its own (`render/audio.rs::RemoteSteps`) and reads the
/// interpolator, which is the only truth a client has about someone else's
/// feet.
#[derive(Default)]
pub struct Steps {
    /// Ground covered since the last step, metres.
    travelled: f32,
    /// Last sampled position, or `None` before the first sample.
    last: Option<[f32; 3]>,
    /// Last sampled speed, m/s — the gain of the step that fires.
    speed: f32,
}

/// What a sampled frame produced.
pub struct Step {
    /// 0..1, from the speed the step was taken at.
    pub gain: f32,
}

impl Steps {
    /// Sample this frame's position. Returns a step when one lands.
    ///
    /// `grounded` is the predictor's own flag: **airborne travel does not tick
    /// the odometer, and landing does not release a burst of banked steps.**
    /// A jump that covered 4 m would otherwise fire five footsteps at the
    /// moment of touchdown.
    pub fn sample(&mut self, pos: [f32; 3], grounded: bool, dt_s: f32) -> Option<Step> {
        // `?` on the first frame: establish the origin, walk nowhere.
        let last = self.last.replace(pos)?;
        if !grounded {
            // Keep the position current (so the next grounded frame measures
            // from where we landed, not from where we jumped) and bank
            // nothing.
            self.travelled = 0.0;
            return None;
        }
        // Horizontal only. A slope should not cost extra steps for the height
        // it gained, and a lift or a step-up should cost none at all.
        let dx = pos[0] - last[0];
        let dz = pos[2] - last[2];
        let d = (dx * dx + dz * dz).sqrt();
        self.speed = if dt_s > 0.0 { d / dt_s } else { 0.0 };
        if self.speed < STEP_MIN_SPEED {
            return None;
        }
        self.travelled += d;
        if self.travelled < STRIDE_M {
            return None;
        }
        // Subtract rather than zero: a frame that covered two strides has the
        // second one's ground already banked, so cadence stays honest at low
        // frame rates instead of quietly dropping steps. Capped at one stride
        // of banked ground, because a hitch that covered ten metres must not
        // buy a burst of eleven footsteps once the frames come back.
        self.travelled = (self.travelled - STRIDE_M).min(STRIDE_M);
        let t = (self.speed / STEP_FULL_SPEED).clamp(0.0, 1.0);
        Some(Step {
            gain: STEP_MIN_GAIN + (1.0 - STEP_MIN_GAIN) * t,
        })
    }

    /// Forget where we were — a teleport, a respawn, or a new world. Without
    /// this the next sample measures the whole jump as ground covered and
    /// fires a step for every 0.85 m of it.
    pub fn reset(&mut self) {
        self.travelled = 0.0;
        self.last = None;
        self.speed = 0.0;
    }
}
