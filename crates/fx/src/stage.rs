//! The fx stage's contract: every game stages an effect the same way, so one
//! shooter (`fxstrip.mjs`) and one agent loop (the `fx` skill) serve all of
//! them.
//!
//! - **One named effect, fired twice at once**: in front of a dark card (the
//!   left half of the frame; light reads here) and a light card (the right
//!   half; smoke and dust read here). Both are unlit, so the plate (the
//!   frame before the effect fires) measures the game's tonemap and nothing
//!   else.
//! - **A fixed camera** ([`Framing`]) sized to the effect's radius: no shake,
//!   no TAA, no bloom unless asked, no server.
//! - **A clock that steps** ([`Clock`]), never the wall clock: 60 steps a
//!   second, warmed up for [`WARM`] frames, step 0 the plate, the effect
//!   fired on step 1, and a shooter may hold it at any step to take a frame.
//!   A page shows where it stands in `window.__fx = {name, step, ready, held}`
//!   and is driven by `window.__fxStep`.

use crate::v3::{self, V3};

/// Steps a second.
pub const HZ: u32 = 60;
/// One step, seconds.
pub const DT: f32 = 1.0 / HZ as f32;
/// Frames drawn before the plate, so pipelines, shadows and exposure settle.
pub const WARM: u32 = 12;

/// The cards' colours, sRGB: about 25 and 185 of 255 on screen with a
/// neutral tonemap. A game whose tonemap moves them tunes its cards until
/// the strip's plate reads `ok` (`fxstrip.mjs`).
pub const DARK_SRGB: f32 = 0.10;
pub const LIGHT_SRGB: f32 = 0.73;

/// A card: a rectangle facing the camera, `half` its half-width and
/// half-height, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Card {
    pub centre: V3,
    pub half: [f32; 2],
    pub normal: V3,
    /// Its colour, sRGB grey.
    pub srgb: f32,
}

/// Where the stage puts its camera, its cards and its two effects, in the
/// stage's own frame (+Y up, the camera on +Z looking down −Z at the cards).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Framing {
    pub eye: V3,
    pub look: V3,
    /// Vertical field of view, radians.
    pub fov_y: f32,
    /// Where each effect fires, on its card's face: `[dark, light]`.
    pub anchors: [V3; 2],
    /// The way an effect is thrown: out of the card, toward the camera and a
    /// little up, so sparks spray, fall and bounce off the card they hit.
    pub dir: V3,
    /// The cards behind them: `[dark, light]`.
    pub cards: [Card; 2],
    /// A floor under both, for effects that fall (grit, drops, a ring).
    pub floor_y: f32,
}

impl Framing {
    /// The framing for an effect about `radius` metres across from where it
    /// fires to where most of it ends, on a 16:9 frame.
    pub fn new(radius: f32) -> Self {
        let r = radius.max(0.01);
        let fov_y = 40f32.to_radians();
        // Each half of the frame is 2.2r wide; 4.4r across a 16:9 frame is
        // 2.475r tall, seen whole from this far.
        let dist = (4.4 * r * 9.0 / 16.0) * 0.5 / (fov_y * 0.5).tan();
        let card = |x: f32, srgb: f32| Card {
            // Wider and taller than the frame, so the frame is all card.
            centre: [x, 0.0, 0.0],
            half: [1.3 * r, 1.6 * r],
            normal: v3::Z,
            srgb,
        };
        Self {
            eye: [0.0, 0.0, dist],
            look: v3::ZERO,
            fov_y,
            anchors: [[-1.1 * r, 0.0, 0.0], [1.1 * r, 0.0, 0.0]],
            dir: v3::norm_or([0.0, 0.35, 1.0], v3::Z),
            cards: [card(-1.3 * r, DARK_SRGB), card(1.3 * r, LIGHT_SRGB)],
            floor_y: -1.2 * r,
        }
    }
}

/// One frame of the stage's clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tick {
    /// Seconds to advance the effects by this frame: [`DT`] or zero.
    pub dt: f32,
    /// The step the clock stands at after this frame.
    pub step: u32,
    /// Fire the effect this frame (the clock just reached step 1).
    pub fire: bool,
    /// Clear what is left of the last run (a looping clock went back to 0).
    pub clear: bool,
}

/// The stage's clock: one call to [`Clock::frame`] per frame drawn.
#[derive(Clone, Copy, Debug, Default)]
pub struct Clock {
    frames: u32,
    step: u32,
    held: u32,
    /// Free-running, go back to the plate after this many steps (zero:
    /// never). A shooter's request always wins over looping.
    pub loop_steps: u32,
}

impl Clock {
    pub fn new() -> Self {
        Self::default()
    }

    /// A clock that replays the effect every `steps` when nobody drives it —
    /// a person watching the stage in a browser.
    pub fn looping(steps: u32) -> Self {
        Self {
            loop_steps: steps,
            ..Self::default()
        }
    }

    /// Advance one frame. `want` is the step a shooter asked for (the page's
    /// `__fxStep`), or `None` to run free: the clock steps once a frame until
    /// it stands at `want`, then holds.
    pub fn frame(&mut self, want: Option<u32>) -> Tick {
        self.frames = self.frames.saturating_add(1);
        let mut tick = Tick {
            dt: 0.0,
            step: self.step,
            fire: false,
            clear: false,
        };
        if self.frames <= WARM {
            return tick;
        }
        let go = match want {
            Some(w) => self.step < w,
            None => true,
        };
        if !go {
            self.held = self.held.saturating_add(1);
            return tick;
        }
        if want.is_none() && self.loop_steps > 0 && self.step >= self.loop_steps {
            self.step = 0;
            self.held = 0;
            tick.step = 0;
            tick.clear = true;
            return tick;
        }
        self.step += 1;
        self.held = 0;
        tick.dt = DT;
        tick.step = self.step;
        tick.fire = self.step == 1;
        tick
    }

    /// Whether the warm-up is over and the plate is on screen.
    pub fn ready(&self) -> bool {
        self.frames >= WARM
    }

    pub fn step(&self) -> u32 {
        self.step
    }

    /// Frames drawn since the clock last moved: a shooter takes its frame
    /// once this is 2 or more, when the step it asked for is on screen.
    pub fn held(&self) -> u32 {
        self.held
    }

    /// Seconds since the effect fired at the step the clock stands at.
    pub fn t(&self) -> f32 {
        self.step.saturating_sub(1) as f32 * DT
    }
}

/// The steps a strip shoots for an effect that lasts `span_s` seconds, `n`
/// frames of it: step 0 (the plate), then `n` steps from 1 to the end, packed
/// toward the start, where an impact happens. `fxstrip.mjs` makes the same
/// list for a page it drives.
pub fn steps(span_s: f32, n: usize) -> Vec<u32> {
    let last = ((span_s.max(DT) * HZ as f32).round() as u32).max(1);
    let mut out = vec![0];
    let n = n.max(1);
    for i in 0..n {
        let f = if n == 1 {
            1.0
        } else {
            i as f32 / (n - 1) as f32
        };
        let s = 1 + (f * f * (last - 1) as f32).round() as u32;
        if out.last() != Some(&s) {
            out.push(s);
        }
    }
    out
}
