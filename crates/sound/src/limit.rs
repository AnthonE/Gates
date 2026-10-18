//! The last stage before the device: a peak limiter over everything summed.
//!
//! The engine clamps its own block to ±1 ([`crate::engine`]), but the score
//! and the players' voices are added AFTER that, so a shout at arm's length
//! over a gunshot summed past full scale and hard-clipped at the device. This
//! holds the sum under [`CEILING`]: the gain drops at once to what the
//! loudest sample needs and comes back over [`RELEASE_S`], and a final clamp
//! catches whatever the gain was still on its way to.
//!
//! Below the ceiling it is exactly nothing — the gain sits at 1.0 and every
//! sample passes through bit for bit — so a quiet mix renders the same with
//! it as without (`tests/worklet.rs` compares the two paths sample for
//! sample). Allocation-free; one `f32` of state.

/// The highest a sample leaves at, linear (about −0.4 dBFS).
pub const CEILING: f32 = 0.95;

/// How long the gain takes to come back once the peak has passed, seconds.
/// Long enough not to pump on the cycles of a voice, short enough that a
/// gunshot does not hold the next sentence down.
pub const RELEASE_S: f32 = 0.2;

/// The limiter's state. See the module docs.
pub struct Limiter {
    gain: f32,
    release: f32,
}

impl Limiter {
    /// A limiter for output at `out_rate`, at unity.
    pub fn new(out_rate: u32) -> Self {
        let n = RELEASE_S * out_rate.max(1) as f32;
        Self {
            gain: 1.0,
            release: 1.0 - (-1.0 / n).exp(),
        }
    }

    /// Limit interleaved stereo in place.
    pub fn run(&mut self, lr: &mut [f32]) {
        for fr in lr.chunks_exact_mut(2) {
            let peak = fr[0].abs().max(fr[1].abs());
            let want = if peak > CEILING { CEILING / peak } else { 1.0 };
            if want < self.gain {
                self.gain = want;
            } else if self.gain < 1.0 {
                self.gain += (want - self.gain) * self.release;
                // Back at unity exactly, so a quiet mix is bit-identical again.
                if self.gain > 0.999 {
                    self.gain = 1.0;
                }
            }
            if self.gain < 1.0 {
                fr[0] = (fr[0] * self.gain).clamp(-CEILING, CEILING);
                fr[1] = (fr[1] * self.gain).clamp(-CEILING, CEILING);
            }
        }
    }

    /// The gain it is holding, 0..1.
    pub fn gain(&self) -> f32 {
        self.gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_passes_untouched_and_loud_is_held_under_the_ceiling() {
        let mut l = Limiter::new(48_000);
        let quiet: Vec<f32> = (0..256)
            .map(|i| 0.9 * ((i % 7) as f32 / 7.0 - 0.5))
            .collect();
        let mut q = quiet.clone();
        l.run(&mut q);
        assert_eq!(q, quiet);

        // A voice at 1.6 over a mix that already sat at full scale.
        let mut loud = vec![1.6f32; 256];
        l.run(&mut loud);
        assert!(loud.iter().all(|s| s.abs() <= CEILING));
        assert!(l.gain() < 1.0);

        // And back to unity once it has passed: bit-identical again.
        let mut silence = vec![0.0f32; 4 * 48_000];
        l.run(&mut silence);
        assert_eq!(l.gain(), 1.0);
        let mut q = quiet.clone();
        l.run(&mut q);
        assert_eq!(q, quiet);
    }
}
