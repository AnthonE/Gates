//! The sample renderer — the audio thread's half of the client's sound.
//!
//! Everything else in this crate decides *what* is heard: the mixer admits a
//! cue, the cadences say when, the director picks a piece. This module is the
//! one place a decision becomes samples, and it runs on the audio thread —
//! natively inside a cpal output callback (`render/audio_out.rs`) at the
//! DEVICE rate, so [`rate`]'s resample is the only one in the chain; in the
//! browser inside an `AudioWorklet` — so it is held to the sim's own
//! discipline rather than the render layer's: **no allocation after the bank
//! is installed, every queue
//! bounded with its overflow counted, no clock, no I/O, no threads, and
//! bit-identical output for a given script.** `tests/engine.rs` and
//! `tests/engine_alloc.rs` are the gates; `render/` only forwards.
//!
//! Why it exists: the previous path made every cue play a `bevy_audio`
//! entity — a rodio decoder plus a sink per play, mixed inside cpal's callback,
//! which in a tab is a `setTimeout` on the main thread
//! (`findings/browser-audio-20260913.md`). One renderer that owns a fixed pool
//! and reads decoded i16 out of a bank is the same work with no allocation
//! on the play path and no per-play object to leak.
//!
//! ## What the game thread sends
//!
//! [`Cmd`] is the whole vocabulary, and it is deliberately dumb: a one-shot
//! with two ear gains and a rate already computed; a held slot that loops (a
//! bed) or plays once (a piece); a gain target; a stop; a cut. Every slow law
//! — a bed's fade, a snapshot crossfade, a music orphan's fade-out — stays on
//! the game thread, which sends the renderer a fresh target each frame and
//! the renderer ramps to it linearly across one block. That keeps the
//! de-clicking here (a step in gain is a click) and every *decision* in the
//! pure model above, where it is already tested.
//!
//! Bank installation is out of band, not a command: a cue's samples arrive
//! through [`Renderer::install`] (natively over [`In::drain_into`]'s own
//! ring, in the worklet by a direct call), and a held loop asked for before
//! its cue has landed **remembers the ask** and starts from sample 0 the
//! block the bank lands — which is what the loading screen relies on, since
//! the beds are asked for before the bank is rendered. A cue is installed
//! once per boot, so a second install of the same cue is a bug upstream:
//! it is **refused, counted** ([`Stats::reinstall_refused`]) **and handed
//! back** rather than replaced, because replacing would free the old box on
//! the audio thread and a free is an allocator call — natively the refused
//! box rides a fourth ring back to the game thread ([`Out::reclaim`]), which
//! is where it is dropped.
//!
//! ## The laws, so the gate can rebuild them
//!
//! `tests/engine.rs` rebuilds every output sample from published parts and
//! compares bits, so the arithmetic is a contract, not an implementation
//! detail:
//!
//! - A voice's cursor is an integer sample position plus a fractional carry
//!   in `[0, 1)`. Not an `f32` cursor: a 10.5 s piece is 463,050 samples,
//!   and an f32 accumulating a rate that many times drifts by thousands of
//!   samples. Advance: `frac += rate; pos += trunc(frac); frac -= trunc`.
//! - A sample is `(a + (b − a) · frac) · (1/32768)` with `a = bank[pos]`,
//!   `b = bank[pos + 1]`; a looping voice's `b` at the last position is
//!   `bank[0]`, because the seam is already cut (`synth::loop_seam`) and the
//!   wrap IS the loop point. A one-shot ends when `pos` reaches `len − 1`,
//!   so `bank[len]` is never read.
//! - A held slot's gain across a block is `g0 + (g1 − g0) · (i + 1) / n`
//!   for sample `i` of `n`, and equals the target on the block's last sample.
//! - The block is summed in a fixed order into a zeroed buffer — held slots
//!   0..[`HELD`], then one-shot slots 0..[`VOICE_SLOTS`] — then hard-clamped
//!   to `[−1, 1]` per sample (rodio's mixer clips the same way).
//!
//! ## The pan law, and the one audible change
//!
//! [`pan`] is equal-power over a quarter turn: `θ = (p + 1)/2 · π/2` for
//! `p = dot(normalize(d_xz), right)`, ears `(cos θ, sin θ)`. A source dead
//! ahead is `1/√2` per ear and a source hard to one side is `0` in the far
//! ear. That is the one thing a player can hear differently from the rodio
//! path, which gave 0.75 per ear at centre and floored the far ear at 0.5 —
//! so a source sweeping past no longer gets louder in the middle, and a
//! hard-panned one is now actually on one side. The right vector comes from
//! `crates/client/src/look.rs::right_dir`: the client owns the yaw convention
//! and shipped it mirrored once, so this crate takes the vector and never a
//! yaw.
//!
//! ## The ledger
//!
//! The mixer needs to know how many one-shots are live and the browser
//! cannot read the audio thread per frame without a per-quantum post — the
//! allocation disease this module exists to cure. So the game thread
//! *predicts* the count in [`Live`]: it started every voice, it knows each
//! cue's length and rate, and the end law above is arithmetic. The
//! prediction is stale by more than a frame, and that is the skew the pool
//! is sized for: the ledger counts a voice down from the game frame that
//! started it, while the `Start` itself crosses the ring and lands at the
//! NEXT device callback — one buffer period, which is one to three frames
//! on an ordinary device and more on a large buffer — so the renderer can
//! hold a voice the ledger has already retired. The pool is [`VOICE_SLOTS`]
//! = `VOICE_CAP + STARTS_PER_FRAME · VOICE_SLACK_FRAMES`: the mixer's full
//! budget for [`VOICE_SLACK_FRAMES`] frames of that latency, so a voice the
//! mixer admitted is not refused here inside it. Past it the renderer
//! refuses and counts ([`Stats::refused`]), and `render/audio.rs::pump`
//! says so once per increment — and reads the audio thread's own last count
//! as a floor under the ledger's, so the room shrinks when the renderer is
//! genuinely fuller than predicted.

use crate::mixer::{Start, SPEED_MAX, SPEED_MIN};
use crate::{Cue, CUE_COUNT, SAMPLE_RATE, STARTS_PER_FRAME, VOICE_CAP};

/// Frames per render call — the `AudioWorklet`'s render quantum. Native uses
/// the same so there is one code path (`DECISIONS.md` §open, audio engine v0).
pub const BLOCK: usize = 128;

/// Frames of overshoot the pool absorbs between the mixer's stale count and
/// the device callback that lands a `Start`.
///
/// A `Start` crosses the ring and lands at the next callback — one buffer
/// period, one to three frames on an ordinary device — while the ledger
/// counts the voice down from the game frame that sent it, so the renderer
/// can be fuller than the ledger says by a few frames' budget. Four frames
/// covers a 2,048-frame buffer at 44.1 kHz at 60 fps; past that
/// [`Stats::refused`] counts and `audio::pump` says so
/// (`DECISIONS.md` §open, audio engine v0).
pub const VOICE_SLACK_FRAMES: usize = 4;

/// One-shot voice slots. Pinned below to
/// `VOICE_CAP + STARTS_PER_FRAME · VOICE_SLACK_FRAMES`: the mixer's live
/// count lags the renderer by the callback latency, so the pool holds that
/// many frames' budget of overshoot and a voice the mixer admitted is not
/// refused here inside [`VOICE_SLACK_FRAMES`] frames of it. Past that the
/// policy is **refuse, never steal** — a stolen voice is an audible cut and
/// the refusal is counted in [`Renderer::refused`].
pub const VOICE_SLOTS: usize = 40;
const _: () = assert!(VOICE_SLOTS == VOICE_CAP + STARTS_PER_FRAME * VOICE_SLACK_FRAMES);

/// Held slots for the beds: wind, surf, submerged, one each.
pub const HELD_BEDS: usize = 3;
/// Held slots for music: consecutive pieces overlap by
/// `PIECE_S − SECTION_S` (2.5 s) → 2; a menu transition adds an orphan fading
/// over the client's `MUSIC_FADE_S` → 3; one spare.
pub const HELD_MUSIC: usize = 4;
/// All held slots. A `slot` byte in a [`Cmd`] is an index below this; which
/// slots are beds and which are music is the game thread's convention
/// ([`HELD_BEDS`] first), not a rule the renderer enforces.
pub const HELD: usize = 7;
const _: () = assert!(HELD == HELD_BEDS + HELD_MUSIC);

/// The renderer's own command queue, filled by [`Renderer::push`] between
/// blocks. Overflow policy: **drop the newest and count it**
/// ([`Renderer::dropped`]) — a cue's value is that it happened, so the
/// earlier asks win, the mixer's own rule one queue down.
pub const CMD_RING_CAP: usize = 64;

/// The native ring from the game thread to the audio thread. Forty-odd
/// frames of backlog at four starts a frame before a stalled audio thread
/// starts dropping ([`Out::ring_dropped`]).
pub const CMD_RING_CAP_NATIVE: usize = 512;

/// A [`Cmd`] on the wire: tag, cue, slot, a pad byte, three little-endian
/// f32. The browser transport carries commands as these records; native does
/// not encode at all.
pub const CMD_BYTES: usize = 16;

/// What the game thread tells the renderer. `Copy`, fixed-size, and complete.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
    /// A one-shot at two ear gains and a playback rate
    /// ([`rate`]). Refused, counted and never stolen when the pool is full.
    Start {
        cue: Cue,
        gain_l: f32,
        gain_r: f32,
        rate: f32,
    },
    /// A held looping voice — a bed. If the cue is not installed yet the slot
    /// remembers the ask and starts from sample 0 the block the bank lands.
    Loop { slot: u8, cue: Cue, gain: f32 },
    /// A held one-shot — a music piece. Frees the slot when it ends; on an
    /// uninstalled cue it is silence, counted in [`Renderer::unbanked`].
    Play { slot: u8, cue: Cue, gain: f32 },
    /// A gain target the renderer ramps to linearly across the next block.
    /// Ignored on a free slot: a fade sent to a piece that just ended is a
    /// race, not a bug.
    Gain { slot: u8, gain: f32 },
    /// Release a held slot.
    Stop { slot: u8 },
    /// End every one-shot — leaving a world.
    CutVoices,
}

/// The tag byte of a [`Cmd`] record. An enum rather than six numeric
/// constants so the two halves of the codec cannot disagree about a value.
#[derive(Clone, Copy)]
#[repr(u8)]
enum Tag {
    Start = 0,
    Loop = 1,
    Play = 2,
    Gain = 3,
    Stop = 4,
    CutVoices = 5,
}

impl Tag {
    fn of(byte: u8) -> Option<Tag> {
        Some(match byte {
            0 => Tag::Start,
            1 => Tag::Loop,
            2 => Tag::Play,
            3 => Tag::Gain,
            4 => Tag::Stop,
            5 => Tag::CutVoices,
            _ => return None,
        })
    }
}

impl Cmd {
    /// Encode. Layout: `[tag, cue, slot, 0, f0 LE, f1 LE, f2 LE]`, where a
    /// variant that carries no cue, slot or float leaves that field zero.
    pub fn to_bytes(self) -> [u8; CMD_BYTES] {
        let (tag, cue, slot, f) = match self {
            Cmd::Start {
                cue,
                gain_l,
                gain_r,
                rate,
            } => (Tag::Start, cue as u8, 0, [gain_l, gain_r, rate]),
            Cmd::Loop { slot, cue, gain } => (Tag::Loop, cue as u8, slot, [gain, 0.0, 0.0]),
            Cmd::Play { slot, cue, gain } => (Tag::Play, cue as u8, slot, [gain, 0.0, 0.0]),
            Cmd::Gain { slot, gain } => (Tag::Gain, 0, slot, [gain, 0.0, 0.0]),
            Cmd::Stop { slot } => (Tag::Stop, 0, slot, [0.0; 3]),
            Cmd::CutVoices => (Tag::CutVoices, 0, 0, [0.0; 3]),
        };
        let mut b = [0u8; CMD_BYTES];
        b[0] = tag as u8;
        b[1] = cue;
        b[2] = slot;
        for (i, v) in f.iter().enumerate() {
            b[4 + i * 4..8 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    /// Decode. `None` for an unknown tag, a cue byte off [`Cue::ALL`] on a
    /// variant that carries one, or a slot byte at or past [`HELD`] on a
    /// variant that carries one. A bad record is dropped by the reader, never
    /// coerced into a nearby valid one.
    pub fn from_bytes(b: &[u8; CMD_BYTES]) -> Option<Cmd> {
        let tag = Tag::of(b[0])?;
        let cue = || Cue::ALL.get(b[1] as usize).copied();
        let slot = || {
            if (b[2] as usize) < HELD {
                Some(b[2])
            } else {
                None
            }
        };
        let f =
            |i: usize| f32::from_le_bytes([b[4 + i * 4], b[5 + i * 4], b[6 + i * 4], b[7 + i * 4]]);
        Some(match tag {
            Tag::Start => Cmd::Start {
                cue: cue()?,
                gain_l: f(0),
                gain_r: f(1),
                rate: f(2),
            },
            Tag::Loop => Cmd::Loop {
                slot: slot()?,
                cue: cue()?,
                gain: f(0),
            },
            Tag::Play => Cmd::Play {
                slot: slot()?,
                cue: cue()?,
                gain: f(0),
            },
            Tag::Gain => Cmd::Gain {
                slot: slot()?,
                gain: f(0),
            },
            Tag::Stop => Cmd::Stop { slot: slot()? },
            Tag::CutVoices => Cmd::CutVoices,
        })
    }
}

/// Playback rate in bank samples per output sample: `speed` is the mixer's
/// pitch variation about 1.0, and the ratio resamples the 44.1 kHz bank to
/// whatever the device runs at.
pub fn rate(speed: f32, out_rate: u32) -> f32 {
    speed * SAMPLE_RATE as f32 / out_rate as f32
}

/// The equal-power pan for a source at `d` (emitter minus listener, world
/// metres) heard by a listener whose right vector in the XZ plane is `right`
/// — `look::right_dir`'s `(x, z)`, never a yaw. Returns `(gain_l, gain_r)`.
///
/// `p = dot(normalize(d_xz), right)` clamped to `[−1, 1]`;
/// `θ = (p + 1)/2 · π/2`; ears `(cos θ, sin θ)`, so `gl² + gr² = 1` over
/// the whole arc, centre is `1/√2` per ear and the far ear of a hard-panned
/// source is `0`. A source within a millimetre of the listener's axis has no
/// side and is centred.
pub fn pan(d: [f32; 3], right: [f32; 2]) -> (f32, f32) {
    let (x, z) = (d[0], d[2]);
    let n = (x * x + z * z).sqrt();
    let p = if n < 1e-3 {
        0.0
    } else {
        ((x * right[0] + z * right[1]) / n).clamp(-1.0, 1.0)
    };
    let theta = (p + 1.0) * 0.5 * core::f32::consts::FRAC_PI_2;
    (theta.cos(), theta.sin())
}

/// The [`Cmd::Start`] for a voice the mixer admitted.
///
/// Positional: [`pan`] times `start.gain` per ear, so a source at the pan's
/// centre is `gain/√2` per ear and one sweeping past does not get louder in
/// the middle. Non-positional (an own-fact): both ears at `start.gain`
/// exactly — heard at the table's gain as authored, with no pan and no
/// falloff, which is what `CueDef::positional == false` means.
pub fn start_cmd(start: &Start, listener: [f32; 3], right: [f32; 2], out_rate: u32) -> Cmd {
    let (l, r) = match start.at {
        Some(p) => pan(
            [p[0] - listener[0], p[1] - listener[1], p[2] - listener[2]],
            right,
        ),
        None => (1.0, 1.0),
    };
    Cmd::Start {
        cue: start.cue,
        gain_l: l * start.gain,
        gain_r: r * start.gain,
        rate: rate(start.speed, out_rate),
    }
}

// ---------------------------------------------------------------------------
// The renderer.
// ---------------------------------------------------------------------------

/// A one-shot voice.
#[derive(Clone, Copy)]
struct Voice {
    live: bool,
    cue: Cue,
    pos: u32,
    frac: f32,
    rate: f32,
    gain_l: f32,
    gain_r: f32,
}

const NO_VOICE: Voice = Voice {
    live: false,
    cue: Cue::UiClick,
    pos: 0,
    frac: 0.0,
    rate: 1.0,
    gain_l: 0.0,
    gain_r: 0.0,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum HeldState {
    Free,
    /// A loop asked for before its cue landed. Starts the block it does.
    Pending,
    Playing,
}

/// A held slot — a bed or a piece.
#[derive(Clone, Copy)]
struct Held {
    state: HeldState,
    looping: bool,
    cue: Cue,
    pos: u32,
    frac: f32,
    /// The gain the last block ended on.
    gain: f32,
    /// The gain the next block ends on.
    target: f32,
}

const NO_HELD: Held = Held {
    state: HeldState::Free,
    looping: false,
    cue: Cue::UiClick,
    pos: 0,
    frac: 0.0,
    gain: 0.0,
    target: 0.0,
};

/// The audio-side counters, as one `Copy` record the audio thread can post
/// back ([`In::report`]) so that every counter has a reader on the game
/// thread — a counter nobody can read is prose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub refused: u32,
    pub unbanked: u32,
    pub cut: u32,
    pub dropped: u32,
    pub bad_cmd: u32,
    /// Installs of a cue that was already installed — refused and handed
    /// back ([`Renderer::install`]).
    pub reinstall_refused: u32,
    /// Refused installs the native seam could not hand back because the
    /// return ring was full, and so freed on the audio thread — the seam's
    /// counter, not the renderer's: [`Renderer::stats`] leaves it zero and
    /// [`In::report`] fills it in.
    pub return_dropped: u32,
    pub blocks: u64,
    pub live: u32,
    pub installed: u32,
}

/// The sample renderer. Fixed arrays only; the bank's samples are the only
/// heap it owns, and they arrive already boxed.
pub struct Renderer {
    voices: [Voice; VOICE_SLOTS],
    held: [Held; HELD],
    bank: [Option<Box<[i16]>>; CUE_COUNT],
    queue: [Cmd; CMD_RING_CAP],
    queued: usize,
    out_rate: u32,
    /// The rate a held voice plays at: the bank's rate over the device's.
    held_rate: f32,
    /// The band a one-shot's rate may lie in, inclusive: the mixer's
    /// `SPEED_MIN..=SPEED_MAX` through [`rate`] at this device's rate, so a
    /// `Start` built by [`start_cmd`] from a clamped speed always passes and
    /// nothing outside the mixer's own band does.
    rate_min: f32,
    rate_max: f32,
    /// Starts refused for want of a free voice. Never stolen.
    pub refused: u32,
    /// Starts and plays of a cue that was not installed — silence, counted.
    pub unbanked: u32,
    /// Plays and loops onto a busy held slot: what was there ended.
    pub cut: u32,
    /// Commands dropped by [`Renderer::push`] because the queue was full.
    pub dropped: u32,
    /// Commands refused as malformed: a slot at or past [`HELD`], a gain
    /// that is not finite, a rate that is not finite or is outside
    /// `rate(SPEED_MIN..=SPEED_MAX, out_rate)` — a rate of zero is a voice
    /// that never ends and holds its slot forever, and a rate past the band
    /// is a click the mixer would never have asked for.
    pub bad_cmd: u32,
    /// Installs of a cue already installed, refused and handed back.
    pub reinstall_refused: u32,
    /// Blocks rendered.
    pub blocks: u64,
}

impl Renderer {
    /// A renderer for a device running at `out_rate` Hz. Nothing is
    /// installed yet; every cue is silence until [`Renderer::install`].
    pub fn new(out_rate: u32) -> Self {
        const NONE: Option<Box<[i16]>> = None;
        let out_rate = out_rate.max(1);
        Self {
            voices: [NO_VOICE; VOICE_SLOTS],
            held: [NO_HELD; HELD],
            bank: [NONE; CUE_COUNT],
            queue: [Cmd::CutVoices; CMD_RING_CAP],
            queued: 0,
            out_rate,
            held_rate: rate(1.0, out_rate),
            rate_min: rate(SPEED_MIN, out_rate),
            rate_max: rate(SPEED_MAX, out_rate),
            refused: 0,
            unbanked: 0,
            cut: 0,
            dropped: 0,
            bad_cmd: 0,
            reinstall_refused: 0,
            blocks: 0,
        }
    }

    /// The device rate this renderer was built for.
    pub fn out_rate(&self) -> u32 {
        self.out_rate
    }

    /// Install a cue's samples. A cue is installed once per boot, so a
    /// second install of the same cue is **refused, counted**
    /// ([`Renderer::reinstall_refused`]) **and handed back** as the `Err`:
    /// nothing is replaced, so nothing is freed on the audio thread — the
    /// caller owns the box again and drops it where it may.
    pub fn install(&mut self, cue: Cue, pcm: Box<[i16]>) -> Result<(), Box<[i16]>> {
        let slot = &mut self.bank[cue.idx()];
        if slot.is_some() {
            self.reinstall_refused = self.reinstall_refused.saturating_add(1);
            return Err(pcm);
        }
        *slot = Some(pcm);
        Ok(())
    }

    /// How many cues are installed.
    pub fn installed(&self) -> usize {
        self.bank.iter().filter(|b| b.is_some()).count()
    }

    /// Queue a command for the next block. Never allocates; past
    /// [`CMD_RING_CAP`] the newest is dropped and counted.
    pub fn push(&mut self, cmd: Cmd) {
        if self.queued >= CMD_RING_CAP {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.queue[self.queued] = cmd;
        self.queued += 1;
    }

    /// Apply a command now. [`Renderer::render`] applies the queue through
    /// this before it mixes; the native transport applies straight from its
    /// ring.
    pub fn apply(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Start {
                cue,
                gain_l,
                gain_r,
                rate,
            } => {
                // The band is the mixer's own clamp through `rate` — inclusive,
                // so a `Start` built from a speed at either rail passes.
                if !(gain_l.is_finite()
                    && gain_r.is_finite()
                    && rate.is_finite()
                    && rate >= self.rate_min
                    && rate <= self.rate_max)
                {
                    self.bad_cmd = self.bad_cmd.saturating_add(1);
                    return;
                }
                if self.bank[cue.idx()].is_none() {
                    self.unbanked = self.unbanked.saturating_add(1);
                    return;
                }
                let Some(v) = self.voices.iter_mut().find(|v| !v.live) else {
                    self.refused = self.refused.saturating_add(1);
                    return;
                };
                *v = Voice {
                    live: true,
                    cue,
                    pos: 0,
                    frac: 0.0,
                    rate,
                    gain_l,
                    gain_r,
                };
            }
            Cmd::Loop { slot, cue, gain } | Cmd::Play { slot, cue, gain } => {
                let looping = matches!(cmd, Cmd::Loop { .. });
                if !gain.is_finite() || slot as usize >= HELD {
                    self.bad_cmd = self.bad_cmd.saturating_add(1);
                    return;
                }
                let installed = self.bank[cue.idx()].is_some();
                let h = &mut self.held[slot as usize];
                if h.state != HeldState::Free {
                    self.cut = self.cut.saturating_add(1);
                }
                let state = match (installed, looping) {
                    (true, _) => HeldState::Playing,
                    (false, true) => HeldState::Pending,
                    (false, false) => {
                        self.unbanked = self.unbanked.saturating_add(1);
                        HeldState::Free
                    }
                };
                *h = Held {
                    state,
                    looping,
                    cue,
                    pos: 0,
                    frac: 0.0,
                    gain,
                    target: gain,
                };
            }
            Cmd::Gain { slot, gain } => {
                if !gain.is_finite() || slot as usize >= HELD {
                    self.bad_cmd = self.bad_cmd.saturating_add(1);
                    return;
                }
                let h = &mut self.held[slot as usize];
                if h.state != HeldState::Free {
                    h.target = gain;
                }
            }
            Cmd::Stop { slot } => {
                if slot as usize >= HELD {
                    self.bad_cmd = self.bad_cmd.saturating_add(1);
                    return;
                }
                self.held[slot as usize] = NO_HELD;
            }
            Cmd::CutVoices => {
                for v in self.voices.iter_mut() {
                    v.live = false;
                }
            }
        }
    }

    /// Render one block of interleaved stereo into `out` — `2 · BLOCK`
    /// floats, left then right per frame. Applies the queued commands first,
    /// then sums held slots and voices in slot order into a zeroed buffer,
    /// then clamps. A shorter buffer renders fewer frames and the gain ramps
    /// span whatever was rendered.
    pub fn render(&mut self, out: &mut [f32]) {
        for i in 0..self.queued {
            let cmd = self.queue[i];
            self.apply(cmd);
        }
        self.queued = 0;

        for s in out.iter_mut() {
            *s = 0.0;
        }

        let bank = &self.bank;
        let held_rate = self.held_rate;
        for h in self.held.iter_mut() {
            if h.state == HeldState::Pending && bank[h.cue.idx()].is_some() {
                h.state = HeldState::Playing;
                h.pos = 0;
                h.frac = 0.0;
            }
            if h.state == HeldState::Playing {
                match bank[h.cue.idx()].as_deref() {
                    Some(b) if b.len() >= 2 => mix_held(h, b, held_rate, out),
                    _ => h.state = HeldState::Free,
                }
            }
            // The ramp reaches its target on the block's last sample, whether
            // or not anything was audible on the way.
            h.gain = h.target;
        }
        for v in self.voices.iter_mut().filter(|v| v.live) {
            match bank[v.cue.idx()].as_deref() {
                Some(b) if b.len() >= 2 => mix_voice(v, b, out),
                _ => v.live = false,
            }
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
        self.blocks = self.blocks.saturating_add(1);
    }

    /// How many one-shot voices are sounding.
    pub fn live(&self) -> usize {
        self.voices.iter().filter(|v| v.live).count()
    }

    /// Is a held slot busy — playing, or a loop waiting for its cue?
    pub fn slot_busy(&self, slot: u8) -> bool {
        self.held
            .get(slot as usize)
            .is_some_and(|h| h.state != HeldState::Free)
    }

    /// The counters, as one record. `return_dropped` is the native seam's
    /// and is zero here; [`In::report`] fills it in.
    pub fn stats(&self) -> Stats {
        Stats {
            refused: self.refused,
            unbanked: self.unbanked,
            cut: self.cut,
            dropped: self.dropped,
            bad_cmd: self.bad_cmd,
            reinstall_refused: self.reinstall_refused,
            return_dropped: 0,
            blocks: self.blocks,
            live: self.live() as u32,
            installed: self.installed() as u32,
        }
    }
}

/// One bank sample at an integer position and a fractional carry: linear
/// between `a` and `b`, scaled from i16 full scale.
#[inline]
fn tap(a: i16, b: i16, frac: f32) -> f32 {
    let a = a as f32;
    let b = b as f32;
    (a + (b - a) * frac) * (1.0 / 32768.0)
}

/// Advance a cursor by `rate` samples. `frac` stays in `[0, 1)` exactly:
/// the subtraction is Sterbenz-exact because `trunc(frac) ≥ frac / 2`.
#[inline]
fn advance(pos: &mut u32, frac: &mut f32, rate: f32) {
    *frac += rate;
    let whole = *frac as u32;
    *pos = pos.saturating_add(whole);
    *frac -= whole as f32;
}

/// Mix a held slot into `out`. Loops wrap at the bank's length with the
/// neighbour of the last sample being sample 0; a piece ends at `len − 1`
/// and frees the slot.
fn mix_held(h: &mut Held, b: &[i16], rate: f32, out: &mut [f32]) {
    let len = b.len() as u32;
    let frames = out.len() / 2;
    let (g0, g1) = (h.gain, h.target);
    for (i, fr) in out.chunks_exact_mut(2).enumerate() {
        if h.looping {
            if h.pos >= len {
                h.pos %= len;
            }
        } else if h.pos >= len - 1 {
            h.state = HeldState::Free;
            return;
        }
        let t = (i + 1) as f32 / frames as f32;
        let g = g0 + (g1 - g0) * t;
        let next = if h.pos + 1 >= len { 0 } else { h.pos + 1 };
        let s = tap(b[h.pos as usize], b[next as usize], h.frac);
        let x = s * g;
        fr[0] += x;
        fr[1] += x;
        advance(&mut h.pos, &mut h.frac, rate);
    }
}

/// Mix a one-shot into `out`. Ends when `pos` reaches `len − 1`, so the
/// neighbour read is always inside the bank.
fn mix_voice(v: &mut Voice, b: &[i16], out: &mut [f32]) {
    let len = b.len() as u32;
    for fr in out.chunks_exact_mut(2) {
        if v.pos >= len - 1 {
            v.live = false;
            return;
        }
        let s = tap(b[v.pos as usize], b[v.pos as usize + 1], v.frac);
        fr[0] += s * v.gain_l;
        fr[1] += s * v.gain_r;
        advance(&mut v.pos, &mut v.frac, v.rate);
    }
}

// ---------------------------------------------------------------------------
// The live ledger.
// ---------------------------------------------------------------------------

/// The game thread's prediction of [`Renderer::live`] — what
/// `Mixer::tick` is handed as `live` on both targets.
///
/// One entry per voice slot holding the seconds left before that voice's
/// cursor reaches `len − 1`, which is the renderer's end law: a cue of
/// `len_s` seconds at `speed` is live for `(len_s − 1/SAMPLE_RATE) / speed`,
/// and the one-sample trim is what makes the two counts agree at a frame
/// boundary rather than one of them lagging by a sample (`tests/engine.rs`
/// holds them equal frame for frame).
#[derive(Clone, Copy, Debug)]
pub struct Live {
    left: [f32; VOICE_SLOTS],
}

// By hand: `std` derives `Default` for arrays only up to 32 elements, and the
// pool is past that since the callback slack widened it.
impl Default for Live {
    fn default() -> Self {
        Self {
            left: [0.0; VOICE_SLOTS],
        }
    }
}

impl Live {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a start. `len_s` is the cue's length in seconds at the bank's
    /// rate, `speed` the mixer's rate about 1.0. Returns `false` if every
    /// slot is live — the renderer would have refused the same start.
    pub fn start(&mut self, len_s: f32, speed: f32) -> bool {
        let Some(slot) = self.left.iter_mut().find(|l| **l <= 0.0) else {
            return false;
        };
        *slot = (len_s - 1.0 / SAMPLE_RATE as f32) / speed;
        true
    }

    /// A frame passed.
    pub fn tick(&mut self, dt_s: f32) {
        for l in self.left.iter_mut() {
            if *l > 0.0 {
                *l -= dt_s;
            }
        }
    }

    /// How many voices the renderer has live.
    pub fn count(&self) -> usize {
        self.left.iter().filter(|l| **l > 0.0).count()
    }

    /// Every voice ended — the ledger's [`Cmd::CutVoices`].
    pub fn cut(&mut self) {
        self.left = [0.0; VOICE_SLOTS];
    }
}

// ---------------------------------------------------------------------------
// The native transport.
// ---------------------------------------------------------------------------

/// The game-thread end of the native seam: two SPSC rings into the audio
/// thread and two back. The worklet does not use this — it calls
/// [`Renderer::push`] and [`Renderer::install`] directly from its port.
pub struct Out {
    cmds: rtrb::Producer<Cmd>,
    installs: rtrb::Producer<(Cue, Box<[i16]>)>,
    stats: rtrb::Consumer<Stats>,
    /// Refused installs coming back to be dropped here rather than freed on
    /// the audio thread ([`Out::reclaim`]).
    returns: rtrb::Consumer<Box<[i16]>>,
    /// Commands dropped because the ring was full: the audio thread has
    /// fallen [`CMD_RING_CAP_NATIVE`] commands behind. Drop-newest.
    pub ring_dropped: u32,
}

/// The audio-thread end.
pub struct In {
    cmds: rtrb::Consumer<Cmd>,
    installs: rtrb::Consumer<(Cue, Box<[i16]>)>,
    stats: rtrb::Producer<Stats>,
    returns: rtrb::Producer<Box<[i16]>>,
    /// Refused installs that could not be handed back because the return
    /// ring was full, and were freed here instead. It cannot happen while
    /// the game thread reclaims every frame — the install ring caps a
    /// frame's installs at `CUE_COUNT`, which is the return ring's whole
    /// capacity — and it is counted rather than assumed: posted as
    /// [`Stats::return_dropped`].
    pub return_dropped: u32,
}

/// Build the seam. Allocates once, here, and never again on either end.
pub fn channel() -> (Out, In) {
    let (cmd_tx, cmd_rx) = rtrb::RingBuffer::<Cmd>::new(CMD_RING_CAP_NATIVE);
    let (inst_tx, inst_rx) = rtrb::RingBuffer::<(Cue, Box<[i16]>)>::new(CUE_COUNT);
    // A mailbox, not a queue: one slot. A push while the previous report is
    // UNREAD fails and the slot keeps the OLDEST unread report — that
    // block's report is skipped, and the next callback after a read fills
    // the slot again. Nothing is lost by it: every counter is cumulative,
    // so the next report that lands carries everything the skipped ones
    // would have. (Not "latest wins" — the game thread reads what it last
    // failed to read, not the newest.)
    let (stat_tx, stat_rx) = rtrb::RingBuffer::<Stats>::new(1);
    // Refused installs, back to the game thread to be dropped there. One
    // slot per cue, which is the most the install ring can carry.
    let (ret_tx, ret_rx) = rtrb::RingBuffer::<Box<[i16]>>::new(CUE_COUNT);
    (
        Out {
            cmds: cmd_tx,
            installs: inst_tx,
            stats: stat_rx,
            returns: ret_rx,
            ring_dropped: 0,
        },
        In {
            cmds: cmd_rx,
            installs: inst_rx,
            stats: stat_tx,
            returns: ret_tx,
            return_dropped: 0,
        },
    )
}

impl Out {
    /// Try to send. `false` — and counted — when the ring is full.
    pub fn send(&mut self, cmd: Cmd) -> bool {
        match self.cmds.push(cmd) {
            Ok(()) => true,
            Err(_) => {
                self.ring_dropped = self.ring_dropped.saturating_add(1);
                false
            }
        }
    }

    /// Hand a cue's samples to the audio thread. `false` if the install ring
    /// is full, in which case the samples come back to the caller by being
    /// dropped here — install the bank a cue at a time and check.
    pub fn install(&mut self, cue: Cue, pcm: Box<[i16]>) -> bool {
        self.installs.push((cue, pcm)).is_ok()
    }

    /// The counters the audio thread has posted since the last read, if
    /// any — the oldest unread report, which with a one-slot mailbox is the
    /// only one there is.
    pub fn stats(&mut self) -> Option<Stats> {
        let mut latest = None;
        while let Ok(s) = self.stats.pop() {
            latest = Some(s);
        }
        latest
    }

    /// Commands waiting in the ring — how far behind the audio thread is.
    pub fn backlog(&self) -> usize {
        CMD_RING_CAP_NATIVE - self.cmds.slots()
    }

    /// Drop, here on the game thread, every refused install the audio
    /// thread handed back. Returns how many. Call once a frame; the flush
    /// does.
    pub fn reclaim(&mut self) -> usize {
        let mut n = 0;
        while self.returns.pop().is_ok() {
            n += 1;
        }
        n
    }
}

impl In {
    /// Move everything the game thread sent into the renderer: installs
    /// first (so a command in the same drain can play what just landed),
    /// then at most [`CMD_RING_CAP`] commands. Call once per block, before
    /// [`Renderer::render`]. A refused install is handed back over the
    /// return ring rather than freed here; only a full return ring frees,
    /// and that is counted ([`In::return_dropped`]).
    pub fn drain_into(&mut self, r: &mut Renderer) {
        for _ in 0..CUE_COUNT {
            match self.installs.pop() {
                Ok((cue, pcm)) => {
                    if let Err(pcm) = r.install(cue, pcm) {
                        if self.returns.push(pcm).is_err() {
                            self.return_dropped = self.return_dropped.saturating_add(1);
                        }
                    }
                }
                Err(_) => break,
            }
        }
        for _ in 0..CMD_RING_CAP {
            match self.cmds.pop() {
                Ok(cmd) => r.apply(cmd),
                Err(_) => break,
            }
        }
    }

    /// Post the renderer's counters back, with this end's own
    /// [`Stats::return_dropped`] filled in. `false` if the mailbox still
    /// holds an unread report: this block's report is then skipped and the
    /// slot keeps the older one, which is not a loss — every counter is
    /// cumulative, so the next report that lands carries this one's facts.
    pub fn report(&mut self, r: &Renderer) -> bool {
        let mut s = r.stats();
        s.return_dropped = self.return_dropped;
        self.stats.push(s).is_ok()
    }
}
