//! The Verdigris counter: when it clicks.
//!
//! A real counter reads the field, not your dose, so the rate follows how
//! deep the green is where you stand (`sim_core::blight`), read as the meter
//! reads it: nothing below the beach line (`World::exposure_inputs`), and the
//! depth at the ground under the feet. A dose that jumps — a turned animal's
//! bite — bursts a few clicks on top, so the bite that got through is heard.
//!
//! Pure: no clock and no Bevy. The caller hands in the frame's `dt` and plays
//! [`crate::Cue::Geiger`] whenever [`Geiger::due`] says so.

use sim_core::blight::{self, Now};
use sim_core::terrain::{self, Haven, BEACH_MAX_H};

/// Clicks a second at full depth.
pub const FULL_HZ: f32 = 14.0;
/// Below this depth the counter is quiet: the edge of the green, not noise.
pub const FLOOR: f32 = 0.01;
/// A dose that rises by this many percent between two readings bursts.
pub const DOSE_STEP_PCT: u8 = 3;
/// How many clicks a burst is, and roughly how far apart, seconds.
pub const BURST_CLICKS: u8 = 6;
pub const BURST_GAP_S: f32 = 0.04;
/// The longest frame the counter integrates: a hitch does not buy a volley.
pub const MAX_DT_S: f32 = 0.1;

/// How deep the green is for the counter at (`x`, `z`), the body's feet at
/// `feet`: zero on the beach, else the spread at the ground under it.
pub fn field(now: &Now, seed: u64, haven: &Haven, x: f32, z: f32, feet: f32) -> f32 {
    if feet < BEACH_MAX_H || !blight::within_reach(now, x, z) {
        return 0.0;
    }
    let h = terrain::ground(seed, haven, x, z);
    blight::depth(now, seed, x, z, h).any()
}

/// Clicks a second at `depth`.
pub fn rate_hz(depth: f32) -> f32 {
    if depth < FLOOR {
        0.0
    } else {
        FULL_HZ * depth.min(1.0)
    }
}

/// The counter's clock: a Poisson stream at the field's rate, plus a burst
/// when the dose jumps.
#[derive(Clone, Copy, Debug)]
pub struct Geiger {
    /// Hazard accumulated since the last click, in expected clicks.
    acc: f32,
    /// The hazard the next click waits for, an exponential draw.
    due: f32,
    rng: u32,
    burst: u8,
    burst_t: f32,
    last_pct: Option<u8>,
}

impl Default for Geiger {
    fn default() -> Self {
        let mut g = Geiger {
            acc: 0.0,
            due: 1.0,
            rng: 0x6E1C_4A2D,
            burst: 0,
            burst_t: 0.0,
            last_pct: None,
        };
        g.due = g.draw();
        g
    }
}

impl Geiger {
    fn unit(&mut self) -> f32 {
        // xorshift32: the counter's own, so its cadence is no one else's.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// An exponential with mean 1: the hazard to the next click.
    fn draw(&mut self) -> f32 {
        -(1.0 - self.unit()).max(1e-6).ln()
    }

    /// The meter's reading arrived (`fel_pct`). The first after a reset only
    /// primes, so a join carrying an old dose is not heard as a bite.
    pub fn dose(&mut self, fel_pct: u8) {
        if let Some(l) = self.last_pct {
            if fel_pct >= l.saturating_add(DOSE_STEP_PCT) {
                self.burst = BURST_CLICKS;
                self.burst_t = 0.0;
            }
        }
        self.last_pct = Some(fel_pct);
    }

    /// One frame of `dt_s` at `depth`: whether to click now. At most one a
    /// call, which is also all the mixer would start a frame.
    pub fn due(&mut self, depth: f32, dt_s: f32) -> bool {
        let dt = dt_s.clamp(0.0, MAX_DT_S);
        let mut click = false;
        // The hazard form: a change in depth is felt at once, where a gap
        // drawn at the old rate would leave you deaf for seconds after
        // stepping into the heart.
        self.acc += rate_hz(depth) * dt;
        if self.acc >= self.due {
            self.acc -= self.due;
            self.due = self.draw();
            self.acc = self.acc.min(self.due);
            click = true;
        }
        if self.burst > 0 {
            self.burst_t -= dt;
            if self.burst_t <= 0.0 {
                self.burst -= 1;
                self.burst_t = BURST_GAP_S * (0.7 + 0.6 * self.unit());
                click = true;
            }
        }
        click
    }

    /// Off a world, dead, or between sessions: no burst pending and no dose
    /// to compare the next reading with.
    pub fn reset(&mut self) {
        self.acc = 0.0;
        self.burst = 0;
        self.burst_t = 0.0;
        self.last_pct = None;
        self.due = self.draw();
    }
}
