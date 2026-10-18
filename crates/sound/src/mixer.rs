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

use super::{
    falloff, Cue, Mix, CUE_COUNT, CUE_QUEUE_CAP, MAX_SAME_CUE, RESERVE_PRIORITY, STARTS_PER_FRAME,
    VOICE_CAP, VOICE_RESERVE,
};

/// The band a diegetic cue's playback rate may wander in, as the mixer's
/// speed about 1.0 — inclusive at both ends. Below it a voice holds a slot for
/// minutes; above it a cue is a click. The renderer refuses a rate outside
/// this same band scaled to its device (`engine::Renderer::apply`, counted in
/// `bad_cmd`), so the two halves of the seam agree about what a rate may be
/// (`DECISIONS.md` §open, audio engine v0).
pub const SPEED_MIN: f32 = 0.25;
pub const SPEED_MAX: f32 = 4.0;

/// How hard a gunshot at the listener ducks the quiet layer, 0..1, scaled
/// by its own falloff — a shot across the valley ducks nothing.
pub const DUCK_SHOT: f32 = 0.6;
/// A charge going off: the whole of it.
pub const DUCK_BLAST: f32 = 1.0;
/// How long a full duck takes to let go, seconds.
pub const DUCK_RELEASE_S: f32 = 0.8;
/// What a full duck takes off a ducked sound: half.
pub const DUCK_DEPTH: f32 = 0.5;
/// The priority at and under which a cue is ducked: footsteps, birds, the
/// animals' ambience. The beds and the music take [`Mixer::duck`] from
/// `render/audio.rs`, which holds their voices.
pub const DUCK_PRIORITY: u8 = 2;

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
    /// A positional cue's cooldown is its source's: where each of its last
    /// few starts was, and the milliseconds left on each. A second body
    /// [`SAME_SOURCE_M`] away is not held by the first.
    cool_at: [[([f32; 3], f32); SOURCES]; CUE_COUNT],
    /// Voices of each cue sounding as the frame opens, from the ledger
    /// ([`Self::set_live_of`]): what [`MAX_SAME_CUE`] is held against.
    live_of: [u8; CUE_COUNT],
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
    /// How ducked the quiet layer is, 0..1 (Rust's gunshot ducking: a shot
    /// near you drops the ambience and the small sounds for a moment, which
    /// is most of why it sounds loud).
    duck: f32,
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
            cool_at: [[([0.0; 3], 0.0); SOURCES]; CUE_COUNT],
            live_of: [0; CUE_COUNT],
            starts: [NO_START; STARTS_PER_FRAME],
            started: 0,
            dropped: 0,
            starved: 0,
            rng: 0x2545_F491,
            duck: 0.0,
        }
    }

    /// The multiplier the ducked layer is under right now: 1 when nothing
    /// is ducking it, down to `1 − DUCK_DEPTH`. The beds and the music read
    /// this; the mixer applies it to its own quiet cues.
    pub fn duck(&self) -> f32 {
        1.0 - DUCK_DEPTH * self.duck
    }

    /// How many voices of each cue are sounding (`engine::Live::counts`),
    /// for the next [`Self::tick`]. Zeros when never set: no cue is held by
    /// [`MAX_SAME_CUE`] then.
    pub fn set_live_of(&mut self, counts: [u8; CUE_COUNT]) {
        self.live_of = counts;
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
        // **A bed is never started here.** A loop asked for as a one-shot is a voice that plays its whole 10 s
        // body once at whatever the frame's gain happened to be, on top of
        // the looping entity that is already playing it — two copies of the
        // same ambience, drifting out of phase, for the life of the sample. A
        // bed is a voice the render layer holds rather than an event anyone
        // fires (`Cue::mixer_started`). Refused and counted with the other caller
        // bugs (`super::CUE_QUEUE_CAP`), because that is what they are.
        if !req.cue.mixer_started() || self.queued >= CUE_QUEUE_CAP {
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
        for (_, c) in self.cool_at.iter_mut().flatten() {
            *c = (*c - dt_ms).max(0.0);
        }
        self.duck = (self.duck - dt_ms / (DUCK_RELEASE_S * 1000.0)).max(0.0);
        let ducked = self.duck();
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
            let duck = if def.priority <= DUCK_PRIORITY {
                ducked
            } else {
                1.0
            };
            let gain = def.gain * req.gain * fall * mix.bus_gain(def.bus) * duck;
            // Below the cull radius, silenced by the mix, or on cooldown —
            // the clock of the row it is a second voice of (`Cue::lead`),
            // and for a place in the world only that place's.
            let lead = req.cue.lead().idx();
            let cooled = match (def.positional, req.at) {
                (true, Some(at)) => self.cool_at[lead]
                    .iter()
                    .all(|&(was, left)| left <= 0.0 || apart(Some(at), Some(was))),
                _ => self.cool[lead] <= 0.0,
            };
            let ok = fall > 0.0 && gain > 0.0 && cooled;
            *slot = if ok {
                (def.priority, gain, dist)
            } else {
                (0, -1.0, dist)
            };
        }

        // Pass two: take the best, up to the frame's budget and the voice cap.
        // A selection scan rather than a sort — at most 32 × 4 comparisons,
        // no allocation, and the tie-break is explicit rather than whatever a
        // sort happens to be stable about. The cap is [`VOICE_CAP`], and
        // [`VOICE_RESERVE`] past it for the cues that matter most; no one cue
        // past [`MAX_SAME_CUE`] voices.
        let mut taken = [false; CUE_QUEUE_CAP];
        let mut of = self.live_of;
        while self.started < STARTS_PER_FRAME {
            let sounding = live + self.started;
            let mut best: Option<usize> = None;
            for i in 0..self.queued {
                if taken[i] || scored[i].1 < 0.0 {
                    continue;
                }
                let cap = if scored[i].0 >= RESERVE_PRIORITY {
                    VOICE_CAP + VOICE_RESERVE
                } else {
                    VOICE_CAP
                };
                if sounding >= cap || of[self.queue[i].cue.idx()] >= MAX_SAME_CUE {
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
            of[req.cue.idx()] = of[req.cue.idx()].saturating_add(1);
            let def = req.cue.def();
            let lead = req.cue.lead().idx();
            match (def.positional, req.at) {
                (true, Some(at)) => {
                    // Its own source's slot if it has one, else the slot
                    // whose clock is nearest done.
                    let slots = &mut self.cool_at[lead];
                    let k = slots
                        .iter()
                        .position(|&(was, left)| left > 0.0 && !apart(Some(at), Some(was)))
                        .unwrap_or_else(|| {
                            (0..SOURCES)
                                .min_by(|&a, &b| slots[a].1.total_cmp(&slots[b].1))
                                .unwrap_or(0)
                        });
                    slots[k] = (at, def.cooldown_ms as f32);
                }
                _ => self.cool[lead] = def.cooldown_ms as f32,
            }
            // **The cooldown binds WITHIN the frame too**, and it did not
            // until the gate said so: the scoring pass above reads `cool`
            // once, so four footsteps requested in one frame all scored as
            // allowed and all four started together — a 90 ms cooldown that
            // three of them never had to pass. A cue with a cooldown may
            // start once a frame; a cue without one (a tree falling) may
            // start as often as the budget allows, which is what a zero
            // cooldown means. **By row, not by clock:** this is what lets
            // `HurtAgain` start beside `Hurt` in a frame of two blows, while
            // across frames the scoring pass reads the lead's clock. **By
            // source, for a place in the world:** two players stepping on
            // the same ground in one frame are two steps, so a squad can be
            // counted by ear.
            if def.cooldown_ms > 0 {
                for (j, s) in scored.iter_mut().enumerate().take(self.queued) {
                    let other = self.queue[j];
                    if j != i
                        && !taken[j]
                        && other.cue == req.cue
                        && !(def.positional && apart(other.at, req.at))
                    {
                        // Below zero, so it is not counted as starved: it was
                        // refused by its own cooldown, not for want of a voice.
                        s.1 = -1.0;
                    }
                }
            }
            let kick = match req.cue {
                Cue::ShotGun | Cue::ShotHeli => DUCK_SHOT,
                Cue::Blast => DUCK_BLAST,
                _ => 0.0,
            };
            if kick > 0.0 {
                let fall = if def.positional {
                    falloff(scored[i].2, def.radius_m)
                } else {
                    1.0
                };
                self.duck = self.duck.max(kick * fall);
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
                    (1.0 + self.roll() * var).clamp(SPEED_MIN, SPEED_MAX)
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

/// How far apart two starts of one positional cue are two sources, metres:
/// past it each keeps its own cooldown. A body's two feet, a gun's muzzle
/// and its shooter are inside it; the next player over is not.
pub const SAME_SOURCE_M: f32 = 2.0;

/// How many sources of one positional cue hold their own cooldown at once.
/// Past it the clock nearest done is reused: a fifth body inside a cue's
/// cooldown can wait out the shortest of the four.
pub const SOURCES: usize = 4;

/// Two positions of one cue are different sources.
fn apart(a: Option<[f32; 3]>, b: Option<[f32; 3]>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => dist(a, b) > SAME_SOURCE_M,
        _ => false,
    }
}

/// Which take of a cue to play: never the one it played last, so a cue with
/// takes is never heard twice running (Rust records several variations per
/// sound for this; a pitch nudge alone reads as the same sample).
pub struct Takes {
    last: [u8; CUE_COUNT],
    rng: u32,
}

impl Default for Takes {
    fn default() -> Self {
        Self {
            last: [u8::MAX; CUE_COUNT],
            rng: 0x9E37_79B9,
        }
    }
}

impl Takes {
    /// A take of `cue`, below `takes`. With one take it is always 0; with
    /// more, a uniform pick among the ones that did not play last.
    pub fn pick(&mut self, cue: Cue, takes: u8) -> u8 {
        if takes <= 1 {
            return 0;
        }
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        // The lead's memory: a second voice plays its lead's takes, and
        // must not start the one its lead just did beside it.
        let cue = cue.lead();
        let last = self.last[cue.idx()];
        let t = if last < takes {
            let t = (x % (takes as u32 - 1)) as u8;
            if t >= last {
                t + 1
            } else {
                t
            }
        } else {
            (x % takes as u32) as u8
        };
        self.last[cue.idx()] = t;
        t
    }
}

/// Straight-line distance, metres.
fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}
