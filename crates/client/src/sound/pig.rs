//! When a pig speaks: the snort cadence, pure and headless.
//!
//! The reference identifies a boar by its snorting before you see it
//! (`NOW.md` §0m item 3), so the voice's job is presence, not reaction —
//! a call every handful of seconds from wherever the animal is. Two rules
//! shape everything here:
//!
//! - **Not a metronome.** A fixed period is the tell that gives away a
//!   generated voice the same way a clock-driven footstep gives away a
//!   footstep system (`steps.rs`). Every interval is drawn fresh from a
//!   hash of the roster slot and the cycle number, so one animal never
//!   repeats its own rhythm and two animals never share one.
//! - **Deterministic, from the world's own identities.** The variation is
//!   hashed from the mob's roster slot — never from an OS random source
//!   (the house style: `synth`'s bank and the mixer's pitch stream are both
//!   seeded xorshift). This is client presentation; the sim never sees it,
//!   and two clients hearing the same pig hear the same cadence.
//!
//! Dormancy is respected by construction rather than by a check: the render
//! half reads the *drawn* herd, which exists only for mobs inside AOI
//! (`AOI_EXIT_CM` = 208 m), and every mob a client can see is awake —
//! `MOB_WAKE_CM` (240 m) deliberately encloses AOI (`limits.rs`). A pig this
//! module voices is therefore always a pig the sim is simming, and the cue's
//! own 40 m radius culls most of those besides.

use sim_core::limits::MAX_MOBS;

/// Mean seconds between one animal's snorts (`DECISIONS.md` §open,
/// "pig voice v0"). Not measured against anything — an opening value in
/// `CONTENT.md`'s sense, like every number in `sound/mod.rs`.
pub const SNORT_PERIOD_S: f32 = 9.0;

/// How far an interval may wander from the mean, as a fraction: each draw
/// lands in `PERIOD × [1−J, 1+J]`, so 4.5–13.5 s at the defaults.
pub const SNORT_JITTER: f32 = 0.5;

/// Per-slot countdowns. One state for the whole roster because the wire's
/// mob ids name fixed roster slots (`mob::slot_of_id`), so a fixed array is
/// exact — no map, no allocation, wall 4's shape for free.
pub struct Snorts {
    /// Seconds until the slot's next snort; negative = never seen, primed
    /// silently on first sight (the `Steps`/`Waterline` pattern — the first
    /// observation establishes state and sounds like nothing, so a world
    /// join is not sixty-four pigs clearing their throats at once).
    next: [f32; MAX_MOBS],
    /// Which interval draw the slot is on — the hash input that makes each
    /// interval its own.
    cycle: [u32; MAX_MOBS],
}

impl Default for Snorts {
    fn default() -> Self {
        Self {
            next: [-1.0; MAX_MOBS],
            cycle: [0; MAX_MOBS],
        }
    }
}

impl Snorts {
    /// Advance one drawn animal's clock by `dt_s`. Returns `true` on the
    /// frame its snort lands.
    ///
    /// **Assign on fire, never accumulate**: a frame hitch that swallowed
    /// two intervals produces one snort and a fresh countdown, not a banked
    /// burst — the same rule the step odometer states for a ten-metre
    /// hitch.
    pub fn due(&mut self, slot: usize, dt_s: f32) -> bool {
        if slot >= MAX_MOBS {
            return false;
        }
        if self.next[slot] < 0.0 {
            // First sight: prime silently, phase-offset by the slot's own
            // first draw so a herd walked into does not speak in unison.
            self.next[slot] = interval(slot, 0);
            self.cycle[slot] = 1;
            return false;
        }
        self.next[slot] -= dt_s;
        if self.next[slot] > 0.0 {
            return false;
        }
        self.next[slot] = interval(slot, self.cycle[slot]);
        self.cycle[slot] = self.cycle[slot].wrapping_add(1);
        true
    }

    /// Forget every clock — leaving a world. A stale countdown carried into
    /// the next island would voice its pigs on the last island's schedule.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The slot's `cycle`-th interval, seconds: `SNORT_PERIOD_S` scaled into
/// `[1−J, 1+J]` by a hash of (slot, cycle).
fn interval(slot: usize, cycle: u32) -> f32 {
    let h = hash01(slot as u32, cycle);
    SNORT_PERIOD_S * (1.0 - SNORT_JITTER + 2.0 * SNORT_JITTER * h)
}

/// A uniform draw in [0, 1) from two integers. The same integer-mixing
/// register as `synth`'s seeding (splitmix-style multiplies), self-contained
/// so the sound model stays free of anyone else's stream.
///
/// Public because it is now the house convention for per-animal variation,
/// not only the voice's: the gait's phase offset (`render/mobs.rs::Gait`)
/// draws from the same well, so one roster slot's walk and snort are both
/// its own and neither needs a second hash to forget to match this one.
pub fn hash01(slot: u32, cycle: u32) -> f32 {
    let mut x = slot
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add(cycle.wrapping_mul(0x85EB_CA6B));
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    // Top 24 bits: exact in f32, uniform, and never 1.0.
    (x >> 8) as f32 / (1u32 << 24) as f32
}
