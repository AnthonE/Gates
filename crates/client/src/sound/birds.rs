//! The forest layer: a bird calls somewhere in the trees.
//!
//! **This is the one part of the reference's ambience we did not have.**
//! `reference/AUDIO.md` §3 describes two things sitting on top of the beds,
//! and ours had only the first: a *bed* whose level answers to the world
//! (`render/audio.rs::bed` reads drawn prop density, `sound::water` reads the
//! shoreline), and *layers* — "the timing of layers like birds and crickets is
//! tuned separately from the beds themselves". A layer is not a bed turned
//! down: it is sparse, it is somewhere rather than everywhere, and its
//! cadence is its own.
//!
//! Two rules, both inherited rather than invented:
//!
//! - **Cadence like `pig.rs`, position like the falling tree.** The interval
//!   is drawn from a hash of the cycle number so the layer is not a metronome
//!   and needs no OS randomness; the call is pushed with a world position and
//!   the mixer's one falloff law culls it. Nothing here knows what "nearby"
//!   means.
//! - **Cover is the cause, because cover is a cause we have.** Birds are in
//!   trees, and `render/audio.rs::bed` already walks the drawn props to score
//!   how much cover is around the ear — so the layer costs no query of its
//!   own. It is deliberately NOT keyed to time of day: crickets at night is
//!   the obvious companion layer and the client has no day/night cycle, so
//!   dusk would be a cause we invented (`CLAUDE.md`: knobs are spoken, never
//!   invented).
//!
//! What this is not: the reference's localized-ambience *system*, which is a
//! set of placed emitters culled and crossfaded, and whose two published bugs
//! were both properties of having many of them (the cull ran too often; the
//! emitter set leaked). `reference/AUDIO.md` §9.3 is explicit that emitters
//! are a later slice which arrives with a cull budget. This is one voice with
//! one clock and nothing to leak.

use super::pig::hash01;

/// Mean seconds between calls in full cover (`DECISIONS.md` §open,
/// "ambience layers v0"). Not measured against anything.
pub const CALL_PERIOD_S: f32 = 11.0;

/// How far an interval may wander from the mean, as a fraction — the same
/// shape and the same default as `pig::SNORT_JITTER`, because it is the same
/// problem: a fixed period is the tell that gives away a generated voice.
pub const CALL_JITTER: f32 = 0.6;

/// How much longer the wait gets in the open. At zero cover the interval is
/// [`CALL_PERIOD_S`] × this, so a beach hears a bird every couple of minutes
/// and a pine wood hears one every few seconds.
///
/// A stretch rather than a gate: a level that reached "never" would make the
/// layer switch off as a player walked a treeline, which is the failure the
/// surf bed's [`super::water::SURF_FLOOR`] exists to avoid one bed over.
pub const OPEN_STRETCH: f32 = 12.0;

/// How high above the emitting prop's origin the call comes from, metres.
/// A bird is in the canopy, not at the roots.
pub const PERCH_H_M: f32 = 5.0;

/// The forest layer's clock.
///
/// One clock, not one per tree: the layer's unit is *the forest around the
/// ear*, and which trunk a given call comes from is picked by the render half
/// from the props it already walked. That is the difference between a layer
/// and an emitter set — an emitter set has state per emitter and can leak it.
pub struct Birds {
    /// Seconds until the next call; negative = never sampled, primed
    /// silently on the first frame so entering a world is not a dawn chorus.
    next: f32,
    cycle: u32,
}

impl Default for Birds {
    fn default() -> Self {
        Self {
            next: -1.0,
            cycle: 0,
        }
    }
}

impl Birds {
    /// Advance by `dt_s` under `cover` (0..1, the same score the wind bed
    /// reads). Returns `true` on the frame a call lands.
    ///
    /// **Assign on fire, never accumulate** — `pig::Snorts`'s rule: a frame
    /// hitch that swallowed two intervals produces one call and a fresh
    /// countdown, not a banked burst.
    pub fn due(&mut self, cover: f32, dt_s: f32) -> bool {
        if self.next < 0.0 {
            self.next = interval(self.cycle, cover);
            self.cycle = self.cycle.wrapping_add(1);
            return false;
        }
        self.next -= dt_s;
        if self.next > 0.0 {
            return false;
        }
        self.next = interval(self.cycle, cover);
        self.cycle = self.cycle.wrapping_add(1);
        true
    }

    /// Which of `n` candidate perches this call comes from.
    ///
    /// Hashed off the cycle rather than drawn, for the module's stated
    /// reason: no OS randomness, and the same forest walked twice sounds the
    /// same. Returns 0 for an empty set, which the caller must not call it
    /// with — a call with no perch is a call with no position, and the mixer
    /// refuses one of those (`reference/AUDIO.md` §6's origin bug).
    pub fn perch(&self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (hash01(self.cycle, 0x5EED) * n as f32) as usize % n
    }

    /// Forget the clock — leaving a world, for `pig::Snorts`'s reason
    /// exactly: a countdown carried into the next island would voice it on
    /// the last one's schedule.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The `cycle`-th interval under `cover`, seconds.
fn interval(cycle: u32, cover: f32) -> f32 {
    let c = cover.clamp(0.0, 1.0);
    // Linear between the two ends rather than a curve: there is no
    // measurement to justify a shape, and a straight line says so.
    let mean = CALL_PERIOD_S * (OPEN_STRETCH + (1.0 - OPEN_STRETCH) * c);
    let h = hash01(cycle, 0xB1_2D5);
    mean * (1.0 - CALL_JITTER + 2.0 * CALL_JITTER * h)
}
