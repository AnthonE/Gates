//! The mixer: which of this frame's cues actually get a voice.
//!
//! Pure, allocation-free and deterministic. Every buffer is a fixed array —
//! `CLAUDE.md`'s client rule is **no per-frame allocations**, and a mixer runs
//! every frame by definition, so a `Vec` here would be a heap touch per frame
//! for the life of the process.
//!
//! The order of operations is the whole design, and it is deliberately the
//! reference's ordering rather than the obvious one:
//!
//! 1. **Cull by distance first** — a cue outside its radius is not a quiet
//!    voice, it is no voice, and it must not consume budget deciding that.
//! 2. **Then the cooldown** — the reference's cheapest defence against a
//!    retrigger storm, and the one that costs a single comparison.
//! 3. **Then priority against the frame budget** — [`super::STARTS_PER_FRAME`].
//! 4. **Then the voice cap** — [`super::VOICE_CAP`], refusing the new voice
//!    rather than stealing an old one.
//!
//! Doing (3) before (1) is the bug this ordering exists to prevent: a hundred
//! inaudible cues from across the island would fill the frame's budget and
//! silence the axe in your hands.

use super::{falloff, Cue, Mix, CUE_COUNT, CUE_QUEUE_CAP, STARTS_PER_FRAME, VOICE_CAP};

/// A cue somebody wants heard this frame.
#[derive(Clone, Copy, Debug)]
pub struct Request {
    pub cue: Cue,
    /// Where it happened, world metres. `None` for an own-fact — see
    /// [`super::CueDef::positional`].
    pub at: Option<[f32; 3]>,
    /// A caller's multiplier on the cue's own gain, 0..1. A footstep taken at
    /// a walk is not a footstep taken at a sprint, and that is the caller's
    /// fact, not the table's.
    pub gain: f32,
}

impl Request {
    /// An own-fact at full gain — the common case.
    pub fn own(cue: Cue) -> Self {
        Self {
            cue,
            at: None,
            gain: 1.0,
        }
    }

    /// A cue at a place, full gain.
    pub fn at(cue: Cue, at: [f32; 3]) -> Self {
        Self {
            cue,
            at: Some(at),
            gain: 1.0,
        }
    }

    /// A cue at a place with the caller's own scaling.
    pub fn with_gain(mut self, gain: f32) -> Self {
        self.gain = gain.clamp(0.0, 1.0);
        self
    }
}

/// A voice the render layer should start this frame.
#[derive(Clone, Copy, Debug)]
pub struct Start {
    pub cue: Cue,
    /// The final linear gain: cue gain × caller gain × distance falloff × bus
    /// × master. **The render layer applies this verbatim and computes no
    /// falloff of its own** — see `render/audio.rs`'s note on rodio, which
    /// has an inverse-square law of its own that we deliberately clamp out.
    pub gain: f32,
    /// Where to put the emitter, world metres. `None` means non-positional:
    /// no panning, no distance, straight to both ears.
    pub at: Option<[f32; 3]>,
    /// Playback rate, 1.0 being the bank's own. Off-unity for the diegetic
    /// cues so the same sample is not heard twice — see [`Cue::pitch_var`].
    pub speed: f32,
}

/// The mixer's frame state.
pub struct Mixer {
    queue: [Request; CUE_QUEUE_CAP],
    queued: usize,
    /// Milliseconds left before each cue may start again.
    cool: [f32; CUE_COUNT],
    starts: [Start; STARTS_PER_FRAME],
    started: usize,
    /// Requests refused because the queue was full since the last reset.
    /// Non-zero is a caller bug — see [`super::CUE_QUEUE_CAP`].
    pub dropped: u32,
    /// Audible, off cooldown, and still not started — refused for want of a
    /// voice ([`VOICE_CAP`]) **or** of frame budget
    /// ([`STARTS_PER_FRAME`]). Load, not a bug, and deliberately one counter
    /// for both: from the caller's side "the mixer was full" is one condition,
    /// and splitting it would imply the two have different remedies.
    ///
    /// A cue refused by its own cooldown is **not** counted here — that is the
    /// system working, not the system saturated.
    pub starved: u32,
    /// The pitch-variation stream. **Deterministic and owned here** rather
    /// than pulled from the OS, so a mixer replayed against the same frames
    /// produces the same voices at the same rates — which is what makes the
    /// whole module testable.
    rng: u32,
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

impl Mixer {
    pub fn new() -> Self {
        const EMPTY: Request = Request {
            cue: Cue::UiClick,
            at: None,
            gain: 0.0,
        };
        const NO_START: Start = Start {
            cue: Cue::UiClick,
            gain: 0.0,
            at: None,
            speed: 1.0,
        };
        Self {
            queue: [EMPTY; CUE_QUEUE_CAP],
            queued: 0,
            cool: [0.0; CUE_COUNT],
            starts: [NO_START; STARTS_PER_FRAME],
            started: 0,
            dropped: 0,
            starved: 0,
            rng: 0x2545_F491,
        }
    }

    /// xorshift32 — see [`Self::rng`].
    fn roll(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Ask for a cue. Bounded: past [`super::CUE_QUEUE_CAP`] the request is
    /// refused and counted, never pushed. This is `CLAUDE.md` wall 4 on a
    /// client-driven path — every one of these callers is ultimately a packet
    /// or a keystroke.
    pub fn push(&mut self, req: Request) {
        // **A bed is never started here.** A loop asked for as a one-shot is a
        // voice that plays its whole 10 s body once at whatever the frame's
        // gain happened to be, on top of the looping entity that is already
        // playing it — two copies of the same ambience, drifting out of phase,
        // for the life of the sample. Refused and counted with the other
        // caller bugs (`super::CUE_QUEUE_CAP`), because that is what it is.
        if req.cue.is_bed() || self.queued >= CUE_QUEUE_CAP {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.queue[self.queued] = req;
        self.queued += 1;
    }

    /// How many requests are waiting. For tests and for the render layer's
    /// diagnostics; the mixer drains itself in [`Self::tick`].
    pub fn queued(&self) -> usize {
        self.queued
    }

    /// Resolve the frame.
    ///
    /// `live` is how many voices are already sounding — the render layer
    /// counts its own entities, because the mixer cannot know when a
    /// one-shot ended. `dt_ms` advances the cooldowns.
    ///
    /// Returns the slice of voices to start, which is at most
    /// [`super::STARTS_PER_FRAME`] long and borrows the mixer's own buffer.
    pub fn tick(&mut self, dt_ms: f32, listener: [f32; 3], live: usize, mix: &Mix) -> &[Start] {
        for c in self.cool.iter_mut() {
            *c = (*c - dt_ms).max(0.0);
        }
        self.started = 0;

        // Pass one: score every queued request, in place. A refused request
        // scores below zero and is never looked at again this frame.
        //
        // The score is not the gain. Gain decides how loud; `priority` decides
        // who gets to be loud at all, and a distant tree falling must outrank
        // a nearby footstep even though it will be quieter.
        let mut scored = [(0u8, 0.0f32, 0.0f32); CUE_QUEUE_CAP];
        for (i, slot) in scored.iter_mut().enumerate().take(self.queued) {
            let req = self.queue[i];
            let def = req.cue.def();
            let (dist, fall) = match (def.positional, req.at) {
                (true, Some(p)) => {
                    let d = dist(p, listener);
                    (d, falloff(d, def.radius_m))
                }
                // A positional cue with no position is a caller bug of exactly
                // the class `reference/AUDIO.md` §6 records in the reference —
                // placement effects that fired at the world origin instead of
                // at the socket. Refuse it rather than play it at the wrong
                // place, and count it where a caller bug is already counted.
                (true, None) => {
                    self.dropped = self.dropped.saturating_add(1);
                    (0.0, 0.0)
                }
                (false, _) => (0.0, 1.0),
            };
            let gain = def.gain * req.gain * fall * mix.bus_gain(def.bus);
            // Below the cull radius, silenced by the mix, or on cooldown.
            let ok = fall > 0.0 && gain > 0.0 && self.cool[req.cue.idx()] <= 0.0;
            *slot = if ok {
                (def.priority, gain, dist)
            } else {
                (0, -1.0, dist)
            };
        }

        // Pass two: take the best, up to the frame's budget and the voice cap.
        // A selection scan rather than a sort — at most 32 × 4 comparisons,
        // no allocation, and the tie-break is explicit rather than whatever a
        // sort happens to be stable about.
        let room = VOICE_CAP.saturating_sub(live);
        let budget = STARTS_PER_FRAME.min(room);
        let mut taken = [false; CUE_QUEUE_CAP];
        while self.started < budget {
            let mut best: Option<usize> = None;
            for i in 0..self.queued {
                if taken[i] || scored[i].1 < 0.0 {
                    continue;
                }
                let Some(b) = best else {
                    best = Some(i);
                    continue;
                };
                // Priority, then nearer, then earlier in the queue. Every term
                // is a total order over this frame's own data, so two runs on
                // the same frame pick the same voices.
                let (bp, _, bd) = scored[b];
                let (ip, _, id) = scored[i];
                if ip > bp || (ip == bp && id < bd) {
                    best = Some(i);
                }
            }
            let Some(i) = best else { break };
            taken[i] = true;
            let req = self.queue[i];
            let def = req.cue.def();
            self.cool[req.cue.idx()] = def.cooldown_ms as f32;
            // **The cooldown binds WITHIN the frame too**, and it did not
            // until the gate said so: the scoring pass above reads `cool`
            // once, so four footsteps requested in one frame all scored as
            // allowed and all four started together — a 90 ms cooldown that
            // three of them never had to pass. A cue with a cooldown may
            // start once a frame; a cue without one (a tree falling) may
            // start as often as the budget allows, which is what a zero
            // cooldown means.
            if def.cooldown_ms > 0 {
                for (j, s) in scored.iter_mut().enumerate().take(self.queued) {
                    if j != i && !taken[j] && self.queue[j].cue == req.cue {
                        // Below zero, so it is not counted as starved: it was
                        // refused by its own cooldown, not for want of a voice.
                        s.1 = -1.0;
                    }
                }
            }
            let var = req.cue.pitch_var();
            self.starts[self.started] = Start {
                cue: req.cue,
                gain: scored[i].1.clamp(0.0, 1.0),
                at: if def.positional { req.at } else { None },
                // Clamped well clear of zero: a rate at or below zero is a
                // voice that never ends, which would hold a slot in
                // `VOICE_CAP` for the life of the process.
                speed: if var > 0.0 {
                    (1.0 + self.roll() * var).clamp(0.25, 4.0)
                } else {
                    1.0
                },
            };
            self.started += 1;
        }

        // Anything still wanted but unstarted was refused for want of a voice
        // or of budget — the honest half of a cap, counted rather than hidden.
        for i in 0..self.queued {
            if !taken[i] && scored[i].1 >= 0.0 {
                self.starved = self.starved.saturating_add(1);
            }
        }

        self.queued = 0;
        &self.starts[..self.started]
    }
}

/// Straight-line distance, metres.
fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}
