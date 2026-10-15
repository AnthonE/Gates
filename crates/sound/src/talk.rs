//! Players talking (proximity voice, wire v113): decoded speech mixed into
//! the stereo block, one ring per talker.
//!
//! The game thread decodes each talker's Opus frames (`crates/voice`) at the
//! output rate and hands the samples across — natively over an `rtrb` ring
//! of [`TalkMsg`] (`client/src/render/audio_out.rs`), in a browser as a
//! `postMessage` to the worklet ([`crate::worklet::Worklet::talk`]) — with
//! an ear gain per talker that the game thread computes from where they
//! stand. This file is the audio thread's half: it holds a short buffer per
//! talker and adds it into the block.
//!
//! **Jitter, not a clock.** Frames arrive at the network's pace, bunched,
//! and the device pulls at its own. A talker starts playing once
//! [`PREROLL_MS`] is buffered, stops on an empty buffer and waits for the
//! preroll again, and never runs more than [`MAX_LAG_MS`] behind: past it
//! the oldest samples go. So a hiccup costs a gap, never a growing delay.
//!
//! Held to the renderer's discipline: the rings are allocated once in
//! [`Talk::new`], and nothing after that allocates, locks or waits.

/// Talkers mixed at once. Past it the game thread reuses the quietest lane.
pub const TALKERS: usize = 8;
/// One message's samples: a 20 ms frame at up to 48 kHz.
pub const TALK_CHUNK: usize = 960;
/// Buffered before a talker starts (and after a gap), in milliseconds.
pub const PREROLL_MS: u32 = 60;
/// The most a talker may run behind; past it the oldest samples go.
pub const MAX_LAG_MS: u32 = 300;

/// One crossing from the game thread to the audio thread. Fixed size and
/// `Copy`, so it rides a preallocated ring by value: boxing the samples
/// would be an allocation per frame on the way to the audio thread.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy)]
pub enum TalkMsg {
    /// Samples for a talker's lane, at the output rate.
    Pcm {
        lane: u8,
        len: u16,
        pcm: [f32; TALK_CHUNK],
    },
    /// Ear gains for a lane, ramped toward over a few milliseconds.
    Gain { lane: u8, l: f32, r: f32 },
    /// The lane's talker is gone: drop what is buffered.
    Stop { lane: u8 },
}

struct Lane {
    ring: Box<[f32]>,
    head: usize,
    len: usize,
    playing: bool,
    gain: [f32; 2],
    target: [f32; 2],
}

impl Lane {
    fn new(cap: usize) -> Self {
        Self {
            ring: vec![0.0; cap].into_boxed_slice(),
            head: 0,
            len: 0,
            playing: false,
            gain: [0.0; 2],
            target: [0.0; 2],
        }
    }
}

/// The talkers' mix.
pub struct Talk {
    lanes: [Lane; TALKERS],
    preroll: usize,
    max_lag: usize,
    /// Per-sample ramp toward a new gain: ~5 ms to settle.
    ramp: f32,
    /// Samples pushed past the lag cap and dropped.
    pub dropped: u32,
}

impl Talk {
    pub fn new(out_rate: u32) -> Self {
        let max_lag = (out_rate * MAX_LAG_MS / 1000) as usize;
        Self {
            lanes: std::array::from_fn(|_| Lane::new(max_lag + TALK_CHUNK)),
            preroll: (out_rate * PREROLL_MS / 1000) as usize,
            max_lag,
            ramp: 1.0 / (out_rate as f32 * 0.005),
            dropped: 0,
        }
    }

    /// Apply one message from the game thread. An out-of-range lane is
    /// ignored (it arrives from another thread, or from JS).
    pub fn apply(&mut self, msg: &TalkMsg) {
        match *msg {
            TalkMsg::Pcm { lane, len, ref pcm } => {
                self.push(lane as usize, &pcm[..(len as usize).min(TALK_CHUNK)])
            }
            TalkMsg::Gain { lane, l, r } => self.gain(lane as usize, l, r),
            TalkMsg::Stop { lane } => self.stop(lane as usize),
        }
    }

    /// Buffer samples for `lane`, dropping the oldest past the lag cap.
    pub fn push(&mut self, lane: usize, pcm: &[f32]) {
        let max_lag = self.max_lag;
        let Some(l) = self.lanes.get_mut(lane) else {
            return;
        };
        let cap = l.ring.len();
        for &s in pcm {
            if l.len >= max_lag {
                l.head = (l.head + 1) % cap;
                l.len -= 1;
                self.dropped = self.dropped.saturating_add(1);
            }
            let at = (l.head + l.len) % cap;
            l.ring[at] = if s.is_finite() { s } else { 0.0 };
            l.len += 1;
        }
    }

    /// Set `lane`'s ear gains (clamped to 0..=2).
    pub fn gain(&mut self, lane: usize, left: f32, right: f32) {
        if let Some(l) = self.lanes.get_mut(lane) {
            let clamp = |g: f32| {
                if g.is_finite() {
                    g.clamp(0.0, 2.0)
                } else {
                    0.0
                }
            };
            l.target = [clamp(left), clamp(right)];
        }
    }

    /// Drop everything `lane` has buffered.
    pub fn stop(&mut self, lane: usize) {
        if let Some(l) = self.lanes.get_mut(lane) {
            l.len = 0;
            l.playing = false;
        }
    }

    /// Whether `lane` is sounding right now.
    pub fn playing(&self, lane: usize) -> bool {
        self.lanes.get(lane).is_some_and(|l| l.playing)
    }

    /// Add every talker into `lr`, interleaved stereo.
    pub fn mix(&mut self, lr: &mut [f32]) {
        let ramp = self.ramp;
        for l in &mut self.lanes {
            if !l.playing {
                if l.len < self.preroll || l.len == 0 {
                    continue;
                }
                l.playing = true;
            }
            let cap = l.ring.len();
            for frame in lr.chunks_exact_mut(2) {
                if l.len == 0 {
                    l.playing = false;
                    break;
                }
                let s = l.ring[l.head];
                l.head = (l.head + 1) % cap;
                l.len -= 1;
                for ((out, g), t) in frame.iter_mut().zip(&mut l.gain).zip(l.target) {
                    *g += (t - *g).clamp(-ramp, ramp);
                    *out += s * *g;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_talker_waits_for_the_preroll_then_plays_and_pans() {
        let mut t = Talk::new(48_000);
        t.gain(0, 1.0, 0.0);
        let mut lr = [0.0f32; 2 * 128];
        // 20 ms is under the 60 ms preroll: silence.
        t.push(0, &[0.5; 960]);
        t.mix(&mut lr);
        assert!(lr.iter().all(|&s| s == 0.0));
        t.push(0, &[0.5; 960 * 2]);
        // Settle the ramp, then the left ear has it and the right has none.
        for _ in 0..4 {
            lr.fill(0.0);
            t.mix(&mut lr);
        }
        assert!(t.playing(0));
        assert!((lr[2 * 127] - 0.5).abs() < 1e-3, "left {}", lr[2 * 127]);
        assert_eq!(lr[2 * 127 + 1], 0.0);
        // Run it dry: it stops, and waits for a preroll again.
        for _ in 0..40 {
            lr.fill(0.0);
            t.mix(&mut lr);
        }
        assert!(!t.playing(0));
        t.push(0, &[0.5; 100]);
        lr.fill(0.0);
        t.mix(&mut lr);
        assert!(!t.playing(0));
    }

    #[test]
    fn the_lag_is_capped_and_bad_input_is_harmless() {
        let mut t = Talk::new(48_000);
        t.push(0, &[0.1; 48_000]);
        assert_eq!(
            t.dropped as usize,
            48_000 - 48_000 * MAX_LAG_MS as usize / 1000
        );
        t.push(TALKERS, &[1.0; 10]);
        t.gain(TALKERS, 1.0, 1.0);
        t.gain(1, f32::NAN, f32::INFINITY);
        t.push(1, &[f32::NAN; 4_000]);
        let mut lr = [0.0f32; 256];
        t.mix(&mut lr);
        assert!(lr.iter().all(|s| s.is_finite()));
        t.stop(0);
        assert!(!t.playing(0));
    }
}
