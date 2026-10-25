//! The Bevy half of the client's audio: the bank, the producers, and the
//! frame's command buffer to the renderer.
//!
//! **Bevy plays; it does not decide.** Every level, every cull and every
//! cadence in here came out of `crate::sound`, which is pure and tested
//! headless — and since audio engine v0 so does every SAMPLE: the mixer's
//! choices leave this file as bounded [`Cmd`]s to `sound::engine::Renderer`,
//! which is the audio thread on both targets (natively inside cpal's
//! callback in `render/audio_out.rs`; in the browser an `AudioWorklet`).
//! This file owns three things and no others: installing the generated bank
//! into the engine, the producers that turn what happened into requests, and
//! the [`Engine`] buffer those requests leave through.
//!
//! ## The rules, and the finding they replaced
//!
//! - **The engine pans.** `engine::start_cmd` turns a mixer `Start` into two
//!   ear gains against `look::right_dir`'s vector — the client owns the yaw
//!   convention and the engine takes the vector, never a yaw.
//! - **`Start.gain` is the client's one distance law.** `sound::falloff`
//!   already put the cue gain, the caller's gain, the falloff and the bus
//!   into it, and nothing downstream attenuates again.
//! - **The game thread decides and sends bounded commands; the renderer
//!   owns time.** A bed's fade and a snapshot crossfade are computed here
//!   per frame and sent as gain TARGETS
//!   the renderer ramps to across one block; it runs no law of its own. Why
//!   this shape is `findings/browser-audio-20260913.md`: the previous path
//!   made every play a `bevy_audio` entity — a decoder and a sink per cue,
//!   mixed in a callback that in a tab is a `setTimeout` on the main thread.
//!
//! History, one paragraph: until 2026-09-13 rodio did the panning, and it
//! attenuates spatial audio by `min(1/d², 1)` in world units — everything
//! past ~5 m inaudible in a metre-scale world — so this file carried a
//! `SPATIAL_SCALE = 1/128` that shrank every emitter inside that clamp and an
//! `EAR_GAP_M` for the head it panned with. Both retired with that path
//! (`DECISIONS.md` §open, audio v0); the pan is `engine::pan` now, and there
//! is nothing left to clamp out.

use bevy::prelude::*;

use crate::sound::birds::{self, Birds};
use crate::sound::engine::{self, Cmd, Live, Stats, HELD_BEDS};
use crate::sound::mixer::{Mixer, Request, Start, Takes};
use crate::sound::music::{self, Director, Track};
use crate::sound::steps::{Steps, Strokes};
use crate::sound::talk::TalkMsg;
use crate::sound::voice::Voices;
use crate::sound::water::Waterline;
use crate::sound::{Cue, Mix, Snapshot, SnapshotDef, Snapshots, CUE_COUNT, SAMPLE_RATE, VOICE_CAP};

use super::{Eye, Net};

/// Commands a frame may hand the renderer. Overflow policy: **drop the newest
/// and count it** ([`Engine::dropped`]) — the mixer's rule one queue up, and
/// for its reason: a cue's value is that it happened, so the earlier asks
/// win. A frame's worst case is arithmetic and well under it — three bed
/// gains, `STARTS_PER_FRAME` starts, plus three loops or three stops and a cut on a screen transition
/// (`DECISIONS.md` §open, audio engine v0).
pub const CMD_FRAME_CAP: usize = 32;

/// How fast a bed's gain follows the world, per second. A bed that snapped
/// would click every time the tree count under the camera changed by one.
///
/// **This is the WORLD's half of a bed's level, not the mixer's.** A snapshot
/// moves the same gains far faster (`sound::SNAPSHOT_FADE_S`, 0.3 s against
/// 2 s here), and that asymmetry is deliberate: walking out of a forest is a
/// slow fact and putting your head under water is an instant one.
pub const BED_FADE_PER_S: f32 = 0.5;

/// The same for a bed that is one source in place (the recycler's motor,
/// the furnace's roar), whose level is your distance to it: fast enough to
/// keep up with a sprint across its reach, slow enough not to click.
pub const SOURCE_FADE_PER_S: f32 = 3.0;

/// The looping beds, in the order [`Sound::bed_gain`] indexes them.
pub const BEDS: [Cue; 9] = [
    Cue::BedWind,
    Cue::BedSurf,
    Cue::BedUnder,
    Cue::BedRain,
    Cue::BedRotor,
    Cue::BedTown,
    Cue::BedNight,
    Cue::BedRotorFar,
    Cue::BedFurnace,
];

/// How far a running recycler's motor is heard, metres: full at the machine,
/// gone at this. **(knob)**
pub const RECYCLER_HUM_R_M: f32 = 16.0;

// One held engine slot per bed (`engine::HELD_BEDS`); music follows them.
const _: () = assert!(BEDS.len() == HELD_BEDS);

/// How far a lit furnace's roar is heard, metres: full at the box, gone at
/// this. A campfire's crackle carries a little further (`FIRE_REACH_M`): it
/// is open to the air. **(knob)**
pub const FURNACE_ROAR_R_M: f32 = 12.0;

/// How far to one side the furnace's roar may pan, as a fraction of a
/// one-shot's hard pan: a furnace at your elbow is still heard in both ears.
pub const FURNACE_PAN: f32 = 0.6;

/// The furnace's held slot: its index in [`BEDS`].
const FURNACE_SLOT: u8 = 8;
const _: () = assert!(matches!(BEDS[FURNACE_SLOT as usize], Cue::BedFurnace));

/// How far the heli's rotor carries, metres: `heli::HELI_AOI_SCALE` × the
/// AOI's enter radius. The server sends the heli island-wide (it is on the
/// map), so past this it is drawn but silent, and it fades up from here.
pub const ROTOR_R_M: f32 =
    (sim_core::heli::HELI_AOI_SCALE * sim_core::limits::AOI_ENTER_CM) as f32 / 100.0;

/// Inside this the rotor is all near layer, metres; past [`ROTOR_FAR_M`] it
/// is all far layer, crossfaded between. Near is the close recording's
/// slap, swish and whine; far is a distant one's thump with the top gone.
pub const ROTOR_NEAR_M: f32 = 60.0;
pub const ROTOR_FAR_M: f32 = 250.0;

/// Inside this the rotor is at full level, metres; past it the level
/// falls off like a source does in open air, softened
/// ([`ROTOR_ROLLOFF`]).
pub const ROTOR_FULL_M: f32 = 20.0;
/// The rolloff's exponent: 1.0 is inverse distance (−6 dB a doubling);
/// 0.6 is −3.6 dB a doubling, so a gunship stays a presence across the
/// island rather than vanishing behind the next ridge.
pub const ROTOR_ROLLOFF: f32 = 0.6;

/// The near layer's top end is taken off with distance by the one-shots'
/// own law ([`engine::lp_of`]) over this radius, so it has darkened by the
/// time it hands over to the far layer.
pub const ROTOR_DARK_M: f32 = 300.0;

/// How far to one side the rotor may pan, as a fraction of a one-shot's
/// hard pan: a gunship 30 m off is still loud in the far ear.
pub const ROTOR_PAN: f32 = 0.7;

/// How much of a real Doppler shift the rotor gets: 1.0 is the physics
/// (±6 % at the heli's 22 m/s cruise), 0 none. The reference wrote a
/// Doppler system for its helicopters; a fraction keeps a flyby a fall in
/// pitch rather than a siren. **(knob)**
pub const ROTOR_DOPPLER: f32 = 0.3;
/// The speed of sound, m/s.
const SOUND_MPS: f32 = 343.0;
/// How long the hull's closing speed is smoothed over, seconds, so a frame
/// that lands late is not a wobble in pitch.
pub const ROTOR_DOPPLER_SMOOTH_S: f32 = 0.3;

/// The rotor's playback rate for a hull moving away at `receding_mps`
/// (negative closing in): [`ROTOR_DOPPLER`] of the physical shift.
pub fn doppler(receding_mps: f32) -> f32 {
    if !receding_mps.is_finite() {
        return 1.0;
    }
    (SOUND_MPS / (SOUND_MPS + ROTOR_DOPPLER * receding_mps)).clamp(0.9, 1.1)
}

/// The rotor's held slots: their indices in [`BEDS`].
const ROTOR_SLOT: u8 = 4;
const ROTOR_FAR_SLOT: u8 = 7;
const _: () = assert!(matches!(BEDS[ROTOR_SLOT as usize], Cue::BedRotor));
const _: () = assert!(matches!(BEDS[ROTOR_FAR_SLOT as usize], Cue::BedRotorFar));

/// The rotor's two layers' levels for a hull `dist` metres off, 0..1 each:
/// one loudness law — full inside [`ROTOR_FULL_M`], a softened inverse
/// distance past it, eased to zero at [`ROTOR_R_M`] — split between the
/// near and far recordings by an equal-power crossfade, so the total holds
/// through the handover and the sound changes colour, not level.
pub fn rotor_levels(dist: f32) -> (f32, f32) {
    let d = dist.max(0.0);
    let edge = (d / ROTOR_R_M).min(1.0);
    let loud = (ROTOR_FULL_M / d.max(ROTOR_FULL_M)).powf(ROTOR_ROLLOFF) * (1.0 - edge * edge);
    let t = ((d - ROTOR_NEAR_M) / (ROTOR_FAR_M - ROTOR_NEAR_M)).clamp(0.0, 1.0);
    let far = t * t * (3.0 - 2.0 * t);
    (loud * (1.0 - far).sqrt(), loud * far.sqrt())
}

/// Where the rotor's two layers are heard from, for a hull `d` metres off
/// the ears (hull minus listener) whose right is `right`: [`engine::pan`]'s
/// equal-power law scaled by √2, so dead ahead is the `(1, 1)` every bed
/// plays at and the total is the same all the way round, with the side
/// taken over the full 3-D distance (a hull overhead is centred, not hard
/// to whichever side it leans). The near layer darkens over
/// [`ROTOR_DARK_M`]; the far one was recorded dark. Both shift in pitch as
/// the hull closes or recedes ([`doppler`]).
pub fn rotor_place(d: [f32; 3], right: [f32; 2], receding_mps: f32, out_rate: u32) -> [Cmd; 2] {
    let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let (ear_l, ear_r) = bed_ears(d, right, ROTOR_PAN);
    let rate = doppler(receding_mps);
    [
        Cmd::Place {
            slot: ROTOR_SLOT,
            ear_l,
            ear_r,
            lp: engine::lp_of(n, ROTOR_DARK_M, out_rate),
            rate,
        },
        Cmd::Place {
            slot: ROTOR_FAR_SLOT,
            ear_l,
            ear_r,
            lp: 0,
            rate,
        },
    ]
}

/// A bed's two ear weights for a source `d` metres off the ears whose right
/// is `right`, panned at most `pan` of a one-shot's hard pan: [`engine::pan`]'s
/// equal-power law scaled by √2, so dead ahead is the `(1, 1)` a bed starts
/// at, with the side taken over the full 3-D distance.
fn bed_ears(d: [f32; 3], right: [f32; 2], pan: f32) -> (f32, f32) {
    let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    let p = if n < 1e-3 {
        0.0
    } else {
        ((d[0] * right[0] + d[2] * right[1]) / n).clamp(-1.0, 1.0) * pan
    };
    let theta = (p + 1.0) * 0.5 * std::f32::consts::FRAC_PI_2;
    (
        std::f32::consts::SQRT_2 * theta.cos(),
        std::f32::consts::SQRT_2 * theta.sin(),
    )
}

/// Where the furnace's roar is heard from, for the nearest lit furnace `d`
/// metres off the ears: [`bed_ears`] at [`FURNACE_PAN`], unfiltered — its
/// reach is short enough that the level law is all the distance it needs.
pub fn furnace_place(d: [f32; 3], right: [f32; 2]) -> Cmd {
    let (ear_l, ear_r) = bed_ears(d, right, FURNACE_PAN);
    Cmd::Place {
        slot: FURNACE_SLOT,
        ear_l,
        ear_r,
        lp: 0,
        rate: 1.0,
    }
}

/// What of the generated bank has reached the engine, and how long each cue
/// is — the number the live ledger ([`Engine::live`]) predicts a voice's end
/// from, because the renderer frees a voice by sample count and the game
/// thread never reads it back per frame.
#[derive(Resource)]
pub struct Bank {
    installed: [bool; CUE_COUNT],
    len_s: [f32; CUE_COUNT],
    takes: [u8; CUE_COUNT],
}

impl Default for Bank {
    fn default() -> Self {
        Self {
            installed: [false; CUE_COUNT],
            len_s: [0.0; CUE_COUNT],
            takes: [1; CUE_COUNT],
        }
    }
}

impl Bank {
    /// Has this cue's PCM been handed to the engine?
    pub fn installed(&self, cue: Cue) -> bool {
        self.installed[cue.idx()]
    }

    /// One take's length in seconds at the bank's rate; zero until it
    /// lands.
    pub fn len_s(&self, cue: Cue) -> f32 {
        self.len_s[cue.idx()]
    }

    /// How many takes the cue's samples hold (`sound_bank::pcm`).
    pub fn takes(&self, cue: Cue) -> u8 {
        self.takes[cue.idx()]
    }

    /// How many cues have landed.
    pub fn count(&self) -> usize {
        self.installed.iter().filter(|b| **b).count()
    }

    fn land(&mut self, cue: Cue, samples: usize, takes: u8) {
        let takes = takes.max(1);
        self.installed[cue.idx()] = true;
        self.takes[cue.idx()] = takes;
        self.len_s[cue.idx()] = (samples / takes as usize) as f32 / SAMPLE_RATE as f32;
    }
}

/// The audio thread's side of the seam, as the game thread last saw it.
///
/// Every number here has a reader, which is what makes them counters and
/// not prose. The fault counters — `ring_dropped`, `install_refused`,
/// `unrouted`, `stream_errors`, and `stats.{refused, unbanked, cut,
/// bad_cmd, dropped, reinstall_refused, return_dropped}` — are printed by
/// [`pump`] once per new increment; the gauges — `live_thread`, `alive`,
/// `routed`, `backlog`, `stats.blocks` — are on the F7 report's audio table
/// (`render/report.rs`) and in `web::heap_report`'s line.
#[derive(Clone, Copy, Debug, Default)]
pub struct Diag {
    /// One-shots sounding on the audio thread at its last block.
    pub live_thread: u32,
    /// Has a device callback filled a buffer yet? False for the life of a
    /// run on a box with no device — every capture run, CI.
    pub alive: bool,
    /// Is there an audio thread to drain the rings at all? False on a box
    /// with no device, where the flush drops and counts (`unrouted`)
    /// instead of filling a ring nobody reads.
    pub routed: bool,
    /// Commands waiting in the native ring — how far behind the audio
    /// thread is.
    pub backlog: usize,
    /// Commands the native ring refused (`engine::Out::ring_dropped`).
    pub ring_dropped: u32,
    /// Installs the native install ring refused.
    pub install_refused: u32,
    /// Commands and installs dropped at the flush for want of an output.
    pub unrouted: u32,
    /// Errors the device stream reported since it opened.
    pub stream_errors: u32,
    /// The renderer's own counters, off its one-slot mailbox.
    pub stats: Stats,
}

/// The game thread's per-frame command buffer to the renderer.
///
/// **Bounded** (`CLAUDE.md` wall 4) at [`CMD_FRAME_CAP`], drop-newest and
/// counted. Every push is a `Copy` into a fixed array and nothing here
/// allocates after `Default`. A flush empties it every `PostUpdate` into
/// whichever seam this target has: `render/audio_out.rs::flush` into cpal's
/// ring, `render/audio_web.rs::flush` into the `AudioWorklet`'s port.
#[derive(Resource)]
pub struct Engine {
    frame: [Cmd; CMD_FRAME_CAP],
    n: usize,
    /// Commands dropped at the frame buffer since start.
    pub dropped: u32,
    /// Cues waiting to cross to the audio thread. Allocated ONCE to
    /// `CUE_COUNT` and only ever filled at build (the whole bank) or one a
    /// frame (the spread bank), never per play — a cue is installed once per
    /// boot, so the capacity is the bound, and a push past it is refused and
    /// counted in [`Engine::install_refused`] rather than grown.
    installs: Vec<(Cue, Box<[i16]>)>,
    /// Installs refused at the frame buffer since start.
    pub install_refused: u32,
    /// The mixer's `live`: what the renderer has sounding, predicted here
    /// from what was started (`sound::engine::Live`).
    pub live: Live,
    /// What the audio thread reported last, copied in by the flush.
    pub diag: Diag,
    /// The renderer's rate, Hz — what every `Start`'s rate is computed
    /// against. [`SAMPLE_RATE`] until `audio_out::open` writes the device's
    /// at plugin build; the worklet stage will write the page's. Read each
    /// frame by [`pump`], so a `Start` already in the ring when it moves
    /// plays at the old rate for that one frame, which is accepted.
    pub out_rate: u32,
    /// The frame's ask of the music output, which is not the engine: the
    /// score is whole tracks streamed beside it (`sound::music`). [`music`]
    /// fills it and each target's flush carries it out.
    pub score: Score,
    /// Players' voices this frame (`render/voice.rs`): decoded samples and
    /// ear gains for the audio thread's `sound::talk`. Bounded at
    /// [`TALK_FRAME_CAP`], drop-newest and counted in `talk_dropped`.
    talk: Vec<TalkMsg>,
    pub talk_dropped: u32,
}

/// Voice messages one frame may carry across: ~8 talkers, a frame or two
/// each, and their gains.
pub const TALK_FRAME_CAP: usize = 48;

/// What the score asks of the music output: the native stream
/// (`audio_out::Music`) or the page's `<audio>` element (`audio_web`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Score {
    /// A track to start from its beginning this frame, replacing whatever the
    /// output was playing.
    pub start: Option<Track>,
    /// End what is playing this frame. Applied before `start`.
    pub stop: bool,
    /// The level to hold the music at, linear. A target: each output ramps to
    /// it rather than stepping.
    pub gain: f32,
    /// Something else already started a track — the page starts the intro
    /// inside the PLAY press, before the game exists — and how far into it,
    /// seconds. Written by the output's open, taken by [`music`] once.
    pub adopt: Option<(Track, f32)>,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            frame: [Cmd::CutVoices; CMD_FRAME_CAP],
            n: 0,
            dropped: 0,
            installs: Vec::with_capacity(CUE_COUNT),
            install_refused: 0,
            live: Live::new(),
            diag: Diag::default(),
            out_rate: SAMPLE_RATE,
            score: Score::default(),
            talk: Vec::with_capacity(TALK_FRAME_CAP),
            talk_dropped: 0,
        }
    }
}

impl Engine {
    /// Queue a voice message for this frame's flush.
    pub fn push_talk(&mut self, msg: TalkMsg) {
        if self.talk.len() >= TALK_FRAME_CAP {
            self.talk_dropped = self.talk_dropped.saturating_add(1);
            return;
        }
        self.talk.push(msg);
    }

    /// The frame's voice messages, in order; empty afterwards.
    pub fn take_talk(&mut self) -> impl Iterator<Item = TalkMsg> + '_ {
        self.talk.drain(..)
    }

    /// Queue a command for this frame's flush.
    pub fn push(&mut self, cmd: Cmd) {
        if self.n >= CMD_FRAME_CAP {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.frame[self.n] = cmd;
        self.n += 1;
    }

    /// Queue a cue's samples for this frame's flush.
    pub fn install(&mut self, cue: Cue, pcm: Box<[i16]>) {
        if self.installs.len() >= CUE_COUNT {
            self.install_refused = self.install_refused.saturating_add(1);
            return;
        }
        self.installs.push((cue, pcm));
    }

    /// The frame's commands, in the order they were pushed; the buffer is
    /// empty afterwards. What the flush reads, and what the tests read.
    pub fn take(&mut self) -> impl Iterator<Item = Cmd> + '_ {
        let n = core::mem::take(&mut self.n);
        self.frame[..n].iter().copied()
    }

    /// The cues waiting to cross, in install order; empty afterwards. A
    /// drain, so the vector keeps its one allocation.
    pub fn take_installs(&mut self) -> impl Iterator<Item = (Cue, Box<[i16]>)> + '_ {
        self.installs.drain(..)
    }

    /// Commands waiting for the flush.
    pub fn pending(&self) -> usize {
        self.n
    }
}

/// What anything in the client asks for a sound through.
///
/// A resource rather than a Bevy `Event` on purpose: the queue is **bounded**
/// (`crate::sound::CUE_QUEUE_CAP`) with a stated overflow policy, which is
/// `CLAUDE.md` wall 4, and Bevy's event buffers are unbounded within a frame.
/// Every caller here is ultimately a packet or a keystroke.
#[derive(Resource, Default)]
pub struct Sound {
    pub mixer: Mixer,
    /// Which take each cue plays next (`sound::mixer::Takes`).
    pub takes: Takes,
    pub steps: Steps,
    /// The local body's swim strokes.
    pub strokes: Strokes,
    /// The waterline, as a thing the local body crosses.
    pub waterline: Waterline,
    /// Each roster slot's voice clock (`sound::voice` — pure; [`voices`] is
    /// the producer that reads it against the drawn herd).
    pub voices: Voices,
    /// When a song plays and which one (`sound::music`). Lives on the
    /// resource rather than on a world entity because it runs on the menus
    /// too, where there is no world.
    pub music: Director,
    /// The forest layer's clock (`sound::birds`), driven by [`bed`], which
    /// already has the cover score and the props to perch on.
    pub birds: Birds,
    /// The Verdigris counter's clock (`sound::geiger`), driven by [`geiger`].
    pub geiger: crate::sound::geiger::Geiger,
    /// Each bed's current gain, moving toward its target at
    /// [`BED_FADE_PER_S`]. Held rather than recomputed so the crossfade is
    /// state, not a function of a frame.
    bed_gain: [f32; BEDS.len()],
    bed_target: [f32; BEDS.len()],
    /// The level each bed was last SENT. A standing player sends nothing:
    /// the renderer holds a target until it is given another, so a gain
    /// equal to the last one is a command with no content.
    bed_sent: [f32; BEDS.len()],
    /// The rotor layers' places, as last sent ([`rotor_place`]); `None`
    /// after a fresh `Loop`, which recentres the slot.
    rotor_sent: [Option<Cmd>; 2],
    /// The hull's distance last frame and its smoothed receding speed, for
    /// the rotor's Doppler; `None` with no hull drawn.
    rotor_dist: Option<f32>,
    rotor_receding: f32,
    /// The furnace roar's place, as last sent ([`furnace_place`]).
    furnace_sent: Option<Cmd>,
    /// The snapshot crossfade, and this frame's resolved mixer state.
    snapshots: Snapshots,
    snap: SnapshotDef,
    /// The track the output is playing, as this side last asked for it.
    sounding: Sounding,
    /// A film is rendering (`film::Film`): the camera is not the recorder's
    /// body, so that body's own sounds are not the camera's ([`film_hears`]).
    pub film: bool,
}

impl Sound {
    /// Ask for a cue. The one door — see [`Sound`].
    pub fn play(&mut self, req: Request) {
        if self.film && !film_hears(req.cue) {
            return;
        }
        self.mixer.push(req);
    }
}

/// Whether a film's camera hears this cue. Everything positional, the beds
/// and the thunder; never the recorder's own sounds — its steps, its hurt
/// and its death, the interface, and the Verdigris counter, whose clicks in
/// a trailer's wolf shot read as the wolf's paws because the recorder stood
/// in the green.
pub fn film_hears(cue: Cue) -> bool {
    cue.def().positional
        || matches!(
            cue,
            Cue::BedWind
                | Cue::BedSurf
                | Cue::BedUnder
                | Cue::BedRain
                | Cue::BedRotor
                | Cue::BedRotorFar
                | Cue::BedTown
                | Cue::BedNight
                | Cue::BedFurnace
                | Cue::Thunder
        )
}

/// The track the music output is playing, as the game thread knows it — the
/// output is asked, never read back, so the countdown here is what says a
/// track has run out.
#[derive(Clone, Copy, Debug, Default)]
struct Sounding {
    track: Option<Track>,
    /// Seconds of it left.
    left_s: f32,
    /// Its own fade, 1 while it plays and falling once [`Self::next`] is set.
    fade: f32,
    /// The track the director asked for while this one was still audible:
    /// it starts when this one has faded out.
    next: Option<Track>,
}

/// How long a track takes to fade out when the menu replaces it, seconds.
/// The only fade the score has: songs otherwise begin and end as recorded.
pub const MUSIC_FADE_S: f32 = 1.2;

/// The fade's level at `x` of the way through it (1 → 0), on a dB curve: the
/// reference made its volume curves logarithmic because linear fades sounded
/// abrupt (devblog 80), and a linear gain is most of its travel in the last
/// few dB. −60 dB at the end, then silence.
pub fn fade_curve(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if x >= 1.0 {
        0.0
    } else {
        10f32.powf(-3.0 * x)
    }
}

/// Build the bank, at plugin-build time rather than in a schedule.
///
/// **`Startup` is too late, and finding out cost a capture run.** Bevy
/// schedules the first state transition with
/// `insert_startup_before(PreStartup, StateTransition)` — so on a start that
/// opens directly on `Screen::Loading` (which is every `--capture` and every
/// `--server` launch) **`OnEnter(Loading)` runs BEFORE `Startup`**. The bank
/// was a `Startup` system and [`setup`] took `Res<Bank>`; the first probe run
/// after the audio slice died on *"Parameter failed validation: Resource does
/// not exist"* with the system name compiled out. `setup` reads [`Engine`]
/// now and [`pump`] reads [`Bank`], and the argument is unchanged.
///
/// Building it here removes the ordering question rather than answering it:
/// the resource exists before any schedule runs at all. The cost is the same
/// ~12 MB of arithmetic, paid a few milliseconds earlier.
///
/// The hazard is general and this file is not the only place it can bite —
/// anything hung on `OnEnter(Loading)` that reads a `Startup`-inserted
/// resource has the same bug. `textures::load` is a `Startup` system today and
/// gets away with it only because nothing reads `Textures` until `Update`.
pub fn build_bank(app: &mut App) {
    build_bank_with(app, SPREAD_BANK);
}

/// Whether the bank is synthesized over frames rather than inside
/// `Plugin::build` (audio spread v0 — `DECISIONS.md` §open).
///
/// **A browser tab is one thread and `build` runs on it.** The ~12 MB of
/// PCM above is ~0.8 s of arithmetic in a native release build and several
/// times that in wasm, with no SIMD and `opt-level = "s"` — paid before the
/// first frame, while the page shows a canvas that has not painted, which
/// reads as a hung tab (`NOW.md` §0web item 8). Natively the same cost lands
/// on the loading screen next to the pipeline warm-up and has never been
/// the thing anyone waited on, so the desktop keeps the whole bank at build:
/// `music.rs`'s ordering argument (`OnEnter(Loading)` before `Startup`) is
/// about the RESOURCE existing, and the resource exists either way — what
/// spreads is when each cue's samples reach the engine.
pub const SPREAD_BANK: bool = cfg!(target_arch = "wasm32");

/// Build the bank whole into the engine, or hand it over one cue a frame
/// through [`synthesize`]. Both arms are reachable natively so
/// `tests/bank.rs` can drive the spread one.
///
/// A spread bank's beds are asked for before they exist: [`setup`] sends
/// its three `Loop`s on the loading screen's first frame, and the renderer
/// remembers a loop whose cue has not landed and starts it from sample 0
/// the block it does (`engine::Cmd::Loop`) — so a bed on a page starts a
/// few frames late rather than never, with nothing here to remember.
pub fn build_bank_with(app: &mut App, spread: bool) {
    let mut bank = Bank::default();
    if !spread {
        let mut engine = app.world_mut().resource_mut::<Engine>();
        for cue in Cue::ALL {
            let (pcm, takes) = crate::sound_bank::pcm(cue);
            bank.land(cue, pcm.len(), takes);
            engine.install(cue, pcm);
        }
    }
    app.insert_resource(bank);
    if spread {
        app.insert_resource(Synth { next: 0 });
        // A page fetches the recordings (`sound_bank::Rec`), in the order
        // `synthesize` wants them, all asked for at once.
        #[cfg(target_arch = "wasm32")]
        for file in crate::sound_bank::all_files(&synth_order()) {
            wasm_bindgen_futures::spawn_local(async move {
                let got = super::asset_fetch::bytes(format!("assets/{file}")).await;
                crate::sound_bank::landed(file, got);
            });
        }
    }
}

/// The cues still to synthesize. Present only while the bank is being
/// spread, which is what gates [`synthesize`] (`resource_exists`).
#[derive(Resource, Debug)]
pub struct Synth {
    /// Index into [`synth_order`] of the next cue to render.
    next: usize,
}

impl Synth {
    /// How many cues are still owed. Zero is a whole bank.
    pub fn remaining(&self) -> usize {
        CUE_COUNT.saturating_sub(self.next)
    }
}

/// The order a spread bank fills in: the three beds first, because they are
/// the cues playing from the first frame of the loading screen, then the
/// rest in `Cue::ALL` order. Every cue exactly once — `tests/bank.rs` holds
/// it to the enum.
pub fn synth_order() -> [Cue; CUE_COUNT] {
    let mut order = [Cue::ALL[0]; CUE_COUNT];
    let mut n = 0;
    for cue in BEDS {
        order[n] = cue;
        n += 1;
    }
    for cue in Cue::ALL {
        if !BEDS.contains(&cue) {
            order[n] = cue;
            n += 1;
        }
    }
    debug_assert_eq!(n, CUE_COUNT);
    order
}

/// Render one cue a frame into the engine until the bank is whole.
///
/// One cue, not a byte budget: the score's pieces are the expensive ones
/// (`synth::pcm_bank`'s own measurement — nine pieces are most of the 0.8 s)
/// and splitting a piece across frames would mean a partial cue nobody can
/// play. A frame that renders a piece is a long frame on the loading screen,
/// which is where every one of them lands; a frame that renders a footstep
/// is not.
pub fn synthesize(mut synth: ResMut<Synth>, mut bank: ResMut<Bank>, mut engine: ResMut<Engine>) {
    if synth.next >= CUE_COUNT {
        return;
    }
    let cue = synth_order()[synth.next];
    // In a browser its recordings may still be on the way; the cues behind
    // it wait with it, as they were fetched in this order.
    if !crate::sound_bank::ready(cue) {
        return;
    }
    let (pcm, takes) = crate::sound_bank::pcm(cue);
    bank.land(cue, pcm.len(), takes);
    engine.install(cue, pcm);
    synth.next += 1;
}

/// Start the beds.
///
/// Runs on entering `Screen::Loading`. The ears need no camera any more —
/// the pan is computed per start from `Eye` in [`pump`] — so nothing here
/// waits on the rig.
pub fn setup(mut sound: ResMut<Sound>, mut engine: ResMut<Engine>) {
    // Every bed starts SILENT and fades in. Entering a world at full ambience
    // on the first frame of the loading screen is the audio version of the
    // world popping in, and the fade is already the mechanism.
    //
    // **All three run from the first frame, at zero.** A bed started on
    // demand would begin its 10.5 s loop wherever the player happened to
    // break the surface, so the submerged bed would open on a bubble or on
    // nothing; and a voice started under an already-open snapshot has no
    // silence to fade up from. Slots 0..HELD_BEDS by convention (the
    // renderer enforces only `slot < HELD`); the music slots follow.
    for (i, cue) in BEDS.iter().enumerate() {
        engine.push(Cmd::Loop {
            slot: i as u8,
            cue: *cue,
            gain: 0.0,
        });
    }
    sound.bed_sent = [0.0; BEDS.len()];
    sound.rotor_sent = [None; 2];
    sound.furnace_sent = None;
}

/// Reset what the world owned: stop the beds, cut every one-shot, and reset
/// the odometer — which is in a resource that outlives the world, and a
/// stale one measures the distance between two worlds as ground covered and
/// fires a burst of footsteps on the first frame of the next.
///
/// Music is not touched: the output is not the engine, and [`music_mode`]
/// decides what the next screen hears.
pub fn teardown(mut sound: ResMut<Sound>, mut engine: ResMut<Engine>, mut last_hp: ResMut<LastHp>) {
    for i in 0..BEDS.len() {
        engine.push(Cmd::Stop { slot: i as u8 });
    }
    engine.push(Cmd::CutVoices);
    engine.live.cut();
    sound.steps.reset();
    // The herd's clocks too: a countdown carried into the next island would
    // voice its animals on this island's schedule. The forest layer's clock
    // is the same rule and the same reason.
    sound.voices.reset();
    sound.birds.reset();
    // And the counter's: a dose carried over would read the next island's
    // first reading as a bite and burst on the join.
    sound.geiger.reset();
    sound.bed_gain = [0.0; BEDS.len()];
    sound.bed_target = [0.0; BEDS.len()];
    // The waterline and the snapshot go with it, and the second one is the
    // reference's own shipped bug: their underwater sound effect stayed on
    // after disconnecting from a server (`reference/WATER.md` §7). A mix state
    // that outlives its cause.
    sound.waterline.reset();
    sound.snapshots.reset();
    sound.snap = SnapshotDef::default();
    // The same rule for health: a stale `LastHp` would read the next world's
    // first health message as a fall from the last world's and play a hurt
    // sound to a player who just joined.
    last_hp.0 = 0;
}

/// Footsteps, from the predictor.
///
/// Runs where the world runs rather than only `InWorld`: a player reading the
/// settings pane is standing still, so this produces nothing, and gating it on
/// the screen would only mean the odometer misses the frames a panel was open
/// and then fires for them all at once when it closes.
pub fn steps(
    net: NonSend<Net>,
    world: Res<super::WorldId>,
    mut sound: ResMut<Sound>,
    time: Res<Time>,
    fx: Option<ResMut<super::fx::Fx>>,
    blight: Option<Res<super::blight::Blight>>,
    mut air: Local<Air>,
) {
    let mut fx = fx;
    let body = &net.session.core.predict.body;
    let pos = net.session.core.predict.render_position();
    // The landing: the body comes back to the ground faster than a step
    // down would bring it. Before the stride's early-out, which a landing
    // is not part of.
    let vy = body.qvy as f32 * sim_core::movement::VEL_Q;
    if !body.grounded {
        air.airborne = true;
        air.vy = vy;
    } else if air.airborne {
        air.airborne = false;
        let v = -air.vy;
        if v > LAND_SOUND_MPS && pos[1] >= sim_core::terrain::SEA_LEVEL {
            let gain = ((v - LAND_SOUND_MPS) / 7.0).clamp(0.45, 1.0);
            sound.play(Request::own(Cue::Land).with_gain(gain));
            if let Some(fx) = fx.as_deref_mut() {
                // A floor kicks nothing up (`body_landing` knows no floor).
                let ground = floor_step(&net.session.core, &world, pos).unwrap_or_else(|| {
                    let splat = sim_core::terrain::splat(world.seed, pos[0], pos[2]);
                    crate::sound::steps::surface_cue(splat, false)
                });
                super::fx::world::body_landing(fx, ground, Vec3::from(pos));
            }
        }
    }
    // Chest deep the player is swimming, and a swimmer takes no steps: the
    // rule a remote's drawn swim and its silence use (NOW §0chr 1). Banked
    // ground is dropped like an airborne stretch's.
    air.swimming = client_core::interp::swimming(pos[1], air.swimming);
    // A swimmer strokes instead: the wading recording, once a stroke.
    if let Some(gain) = sound.strokes.sample(pos, air.swimming, time.delta_secs()) {
        sound.play(Request::own(Cue::StepWater).with_gain(gain));
    }
    let walking = body.grounded && !air.swimming;
    let Some(mut step) = sound.steps.sample(pos, walking, time.delta_secs()) else {
        return;
    };
    if net.session.core.crouched() {
        step.gain *= CROUCH_STEP_GAIN;
    }
    // A built floor's own material, else the same `splat` the ground under
    // the player is DRAWN with, so the sound cannot disagree with the
    // picture — see `sound::steps`.
    let core = &net.session.core;
    let cue = floor_step(core, &world, pos).unwrap_or_else(|| {
        let splat = sim_core::terrain::splat(world.seed, pos[0], pos[2]);
        let below_sea = pos[1] < sim_core::terrain::SEA_LEVEL;
        crate::sound::steps::surface_cue(splat, below_sea)
    });
    sound.play(Request::own(cue).with_gain(step.gain));
    let now = blight.as_deref().and_then(|b| b.now.as_ref());
    if let Some(depth) = in_leaves_at(&world, pos, |key| plant_size(core, key), now) {
        sound.play(Request::own(Cue::Brush).with_gain(step.gain * brush_gain(depth)));
    }
    if let Some(mut fx) = fx {
        super::fx::world::footstep(&mut fx, cue, Vec3::from(pos), step.gain);
    }
}

/// The step a body with its feet at `pos` takes on a built floor: the
/// floor's material, off the piece record under it
/// (`sim_core::collide::floor_under`). `None` off the pieces, where the
/// ground's splat decides. Twig and wood are boards, stone is a slab, metal
/// is sheet.
pub fn floor_step(
    core: &client_core::core::ClientCore,
    world: &super::WorldId,
    pos: [f32; 3],
) -> Option<Cue> {
    use sim_core::build::{
        MAT_METAL, MAT_STONE, SHAPE_DOORWAY, SHAPE_FRAME, SHAPE_HALF_WALL, SHAPE_LOW_WALL,
        SHAPE_WALL, SHAPE_WINDOW,
    };
    let cols = core.pieces.cols();
    let (cx, cz, level) =
        sim_core::collide::floor_under(world.seed, &world.haven, cols, pos[0], pos[2], pos[1])?;
    let defs = &core.piece_defs;
    let def = core
        .pieces
        .entries()
        .iter()
        .filter(|r| r.cx == cx && r.cz == cz && r.level == level)
        .filter(|r| u16::from(r.row) < core.piece_defs_have.min(defs.piece_count))
        .map(|r| defs.pieces[r.row as usize])
        .find(|d| {
            !matches!(
                d.shape,
                SHAPE_WALL
                    | SHAPE_DOORWAY
                    | SHAPE_WINDOW
                    | SHAPE_FRAME
                    | SHAPE_HALF_WALL
                    | SHAPE_LOW_WALL
            )
        })?;
    Some(match def.material {
        MAT_STONE => Cue::StepConcrete,
        MAT_METAL => Cue::StepMetal,
        _ => Cue::StepWood,
    })
}

/// A body's half-width for brushing a plant, metres: its legs and shoulders
/// are in the leaves before its axis is.
pub const BRUSH_BODY_M: f32 = 0.25;

/// A stride's brush gain at `depth` into a plant: a graze at its edge, the
/// whole plant dragged along you at its stem.
fn brush_gain(depth: f32) -> f32 {
    0.55 + 0.45 * depth
}

/// How deep in a plant's leaves a body standing at `pos` is: 0 at their edge,
/// 1 at the stem, `None` in the open. Off the scatter, cold, like
/// [`scatter_occupant`] (a stride is a rare event), over the 3×3 cells round
/// `pos`, since a plant near a cell's edge reaches into its neighbour.
/// `size` is a plant's drawn size by cell key: 0 picked, 1 grown.
pub fn in_leaves(w: &super::WorldId, pos: [f32; 3], size: impl Fn(u32) -> f32) -> Option<f32> {
    in_leaves_at(w, pos, size, None)
}

/// [`in_leaves`] in the spread as it stands (`render::blight`): a shrub the
/// green has killed is dead scrub, drawn and heard alike.
pub fn in_leaves_at(
    w: &super::WorldId,
    pos: [f32; 3],
    size: impl Fn(u32) -> f32,
    now: Option<&sim_core::blight::Now>,
) -> Option<f32> {
    use sim_core::terrain::{scatter, CELLS_PER_SIDE, CELL_SIZE};
    let cell = |v: f32| (v / CELL_SIZE).floor() as i32;
    let (cx, cz) = (cell(pos[0]), cell(pos[2]));
    let mut deepest: Option<f32> = None;
    for x in cx - 1..=cx + 1 {
        for z in cz - 1..=cz + 1 {
            if !(0..CELLS_PER_SIDE).contains(&x) || !(0..CELLS_PER_SIDE).contains(&z) {
                continue;
            }
            let slot = scatter(w.seed, &w.table, &w.haven, x, z);
            let key = sim_core::gather::cell_key(x as u16, z as u16);
            if !slot.occupant.is_plant() {
                continue;
            }
            let fel = match now {
                Some(n) if !sim_core::blight::within_reach(n, slot.x, slot.z) => 0.0,
                _ => super::blight::live_fel(
                    now,
                    slot.x,
                    slot.z,
                    slot.y,
                    sim_core::terrain::fel_wander(w.seed, slot.x, slot.z),
                ),
            };
            let scenery = super::plants::scenery_with(w.seed, &slot, key, fel);
            let Some(r) = super::plants::leaf_reach(slot.occupant, key, scenery) else {
                continue;
            };
            let grown = size(key);
            if grown <= 0.0 {
                continue;
            }
            let reach = r * slot.scale * grown + BRUSH_BODY_M;
            let d = Vec2::new(pos[0] - slot.x, pos[2] - slot.z).length();
            if d < reach {
                let depth = 1.0 - d / reach;
                deepest = Some(deepest.map_or(depth, |was: f32| was.max(depth)));
            }
        }
    }
    deepest
}

/// A plant's drawn size by cell key, as `props::grow` draws it: 0 while it is
/// picked, growing back toward 1.
fn plant_size(core: &client_core::core::ClientCore, key: u32) -> f32 {
    if core.harvested.contains(key) {
        return 0.0;
    }
    core.harvested.growth(key).map_or(1.0, |at| {
        let now = core.clock.server_est.max(0.0) as u64;
        let span = sim_core::gather::PLANT_GROW_TICKS;
        sim_core::gather::grow_pm_over(at as u64, now, span) as f32 * 0.001
    })
}

/// A crouched step's gain against a standing one at the same speed (v83):
/// the crouch is the quiet walk, heard by players as well as animals.
pub const CROUCH_STEP_GAIN: f32 = 0.5;

/// A fall faster than this, m/s, lands with a thud (a jump lands at ~7).
pub const LAND_SOUND_MPS: f32 = 3.5;

/// The body's last airborne state, for [`steps`]'s landing, and whether it
/// was swimming, for the hysteresis that silences its steps.
#[derive(Default)]
pub struct Air {
    airborne: bool,
    vy: f32,
    swimming: bool,
}

/// How far a lit fire is heard crackling, metres (the cue's own radius).
pub const FIRE_REACH_M: f32 = 14.0;

/// Lit fires crackle: the nearest lit campfire in reach, a short recorded
/// crackle every half second to a second. The fire's light is lit exactly
/// when the sim says the fire is (`structures::fire_lights`), so the light is
/// the authority here too. THE GATE's braziers (`town_fx`) are always lit,
/// and crackle the same way. A furnace does not crackle: its fire is shut in
/// an iron box, and it roars instead (`Cue::BedFurnace`, [`bed`]).
///
/// A fire switched on or off is heard at it: a furnace's flame catching
/// behind its door or cut off, a campfire's in the open
/// (`Feed::oven_flips`, which a join's restatement never rings).
#[allow(clippy::type_complexity)]
pub fn fires(
    time: Res<Time>,
    eye: Res<Eye>,
    feed: Res<super::feed::Feed>,
    lights: Query<
        (
            &PointLight,
            &GlobalTransform,
            Option<&super::structures::FireLight>,
        ),
        Or<(
            With<super::structures::FireLight>,
            With<super::town_fx::Brazier>,
        )>,
    >,
    recyclers: Query<(&GlobalTransform, &super::structures::RecyclerHum)>,
    mut sound: ResMut<Sound>,
    mut next: Local<(f32, u32)>,
) {
    for &(cx, cz, level, loc, lit) in feed.oven_flips() {
        let Some((fire, at)) = lights.iter().find_map(|(_, g, f)| {
            f.filter(|f| (f.cx, f.cz, f.level, f.loc) == (cx, cz, level, loc))
                .map(|f| (f, g.translation()))
        }) else {
            // Not a fire: a recycler's engine, at the machine.
            if let Some((g, _)) = recyclers
                .iter()
                .find(|(_, r)| (r.cx, r.cz, r.level, r.loc) == (cx, cz, level, loc))
            {
                let cue = if lit {
                    Cue::RecyclerStart
                } else {
                    Cue::RecyclerStop
                };
                let at = g.translation() + Vec3::Y * 0.6;
                sound.play(Request::at(cue, at.to_array()));
            }
            continue;
        };
        let cue = match (fire.furnace, lit) {
            (true, true) => Cue::FurnaceLight,
            (true, false) => Cue::FurnaceOut,
            (false, true) => Cue::Kindle,
            (false, false) => Cue::Snuff,
        };
        sound.play(Request::at(cue, at.to_array()));
    }
    next.0 -= time.delta_secs();
    if next.0 > 0.0 {
        return;
    }
    let nearest = lights
        .iter()
        .filter(|(l, _, f)| l.intensity > 0.0 && !f.is_some_and(|f| f.furnace))
        .map(|(_, g, _)| g.translation())
        .filter(|p| p.distance(eye.pos) < FIRE_REACH_M)
        .min_by(|a, b| a.distance(eye.pos).total_cmp(&b.distance(eye.pos)));
    // xorshift, for the gap: a fire that crackles on a metronome is a loop.
    let mut x = next.1.max(1);
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    next.1 = x;
    let r = x as f32 / u32::MAX as f32;
    let Some(at) = nearest else {
        next.0 = 0.5;
        return;
    };
    sound.play(Request::at(Cue::FireCrackle, at.to_array()));
    next.0 = 0.45 + 0.75 * r;
}

/// The Verdigris counter in your ear: clicks at the rate of the green where
/// you stand (`sound::geiger`), and a burst when your dose jumps. Quiet on
/// the beach, out of the green, and when you are dead.
pub fn geiger(
    net: NonSend<Net>,
    world: Res<super::WorldId>,
    feed: Res<super::feed::Feed>,
    blight: Option<Res<super::blight::Blight>>,
    time: Res<Time>,
    mut sound: ResMut<Sound>,
) {
    let core = &net.session.core;
    if core.dead {
        sound.geiger.reset();
        return;
    }
    // A hit this frame, then the meter's reading: a jump beside a hit is a
    // bite heard.
    if !feed.hurt_from().is_empty() {
        sound.geiger.hurt();
    }
    if feed.applied2 & client_core::core::APPLIED2_EXPOSURE != 0 {
        sound.geiger.dose(core.fel_pct);
    }
    let [x, feet, z] = core.predict.render_position();
    let depth = blight
        .as_deref()
        .and_then(|b| b.now.as_ref())
        .map_or(0.0, |now| {
            crate::sound::geiger::field(now, world.seed, &world.haven, x, z, feet)
        });
    if sound.geiger.due(depth, time.delta_secs()) {
        sound.play(Request::own(Cue::Geiger));
    }
}

/// One remote body's step odometer — the same `sound::steps::Steps` the
/// local player runs, one per drawn body, carried as a component so it dies
/// with the entity and a body that leaves AOI and returns starts fresh (no
/// map to sweep, no reset to remember). `bodies::stream` inserts it at
/// spawn.
#[derive(Component, Default)]
pub struct RemoteSteps(pub Steps, pub Strokes);

/// Another player's footsteps — the sound that decides fights, and until
/// this system nothing produced it: only the local body has a predictor,
/// so a remote's cadence comes off the same distance-integrated odometer
/// fed the INTERPOLATED transform `bodies::stream` just wrote (this runs
/// after `Stream`). The surface is `terrain::splat` at THEIR position, the
/// cue is the local family's positional twin (`steps::remote`), and "only
/// nearby" is the mixer's own falloff at the cue's radius — the falling
/// tree's pattern: push with a position, let the one distance law cull.
///
/// One honest gap, the wire's: a teleport (death, respawn) reads as ground
/// covered, which the odometer's own hitch cap bounds at ONE step. A
/// sleeper stands still and the speed floor keeps it silent for free.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn remote_steps(
    world: Res<super::WorldId>,
    net: Option<NonSend<Net>>,
    time: Res<Time>,
    rig: Res<super::anim::Rig>,
    mut bodies: Query<
        (&Transform, &mut RemoteSteps, &super::anim::BodyAnim),
        Or<(With<super::bodies::Body>, With<super::guards::Guard>)>,
    >,
    mut sound: ResMut<Sound>,
    eye: Res<Eye>,
    mut fx: Option<ResMut<super::fx::Fx>>,
    blight: Option<Res<super::blight::Blight>>,
) {
    let dt = time.delta_secs();
    let now = blight.as_deref().and_then(|b| b.now.as_ref());
    for (t, mut steps, anim) in bodies.iter_mut() {
        let pos = [t.translation.x, t.translation.y, t.translation.z];
        // Spaced as the drawn feet are spaced (`Rig::step_m`), so a remote's
        // steps land at the cadence its legs are moving; and nothing in the
        // air, off the wire's airborne bit, or from a body drawn swimming
        // (whose clip has no stride, so it would fall back to `stride_m`).
        let walking = !anim.airborne && !anim.swimming;
        // A swimmer is heard stroking, at the body.
        if let Some(gain) = steps.1.sample(pos, anim.swimming, dt) {
            sound.play(Request::at(Cue::RemoteStepWater, pos).with_gain(gain));
        }
        let Some(mut step) = steps.0.sample_with(pos, walking, dt, rig.step_m(anim)) else {
            continue;
        };
        if anim.crouched {
            step.gain *= CROUCH_STEP_GAIN;
        }
        let floor = net
            .as_deref()
            .and_then(|n| floor_step(&n.session.core, &world, pos));
        let cue = crate::sound::steps::remote(floor.unwrap_or_else(|| {
            let splat = sim_core::terrain::splat(world.seed, pos[0], pos[2]);
            let below_sea = pos[1] < sim_core::terrain::SEA_LEVEL;
            crate::sound::steps::surface_cue(splat, below_sea)
        }));
        sound.play(Request::at(cue, pos).with_gain(step.gain));
        // Only within earshot: the leaves are nine scatter lookups, and a
        // body across the valley would be culled by the mixer anyway.
        if t.translation.distance(eye.pos) <= Cue::RemoteBrush.def().radius_m {
            let size = |key| {
                net.as_deref()
                    .map_or(1.0, |n| plant_size(&n.session.core, key))
            };
            if let Some(depth) = in_leaves_at(&world, pos, size, now) {
                let gain = step.gain * brush_gain(depth);
                sound.play(Request::at(Cue::RemoteBrush, pos).with_gain(gain));
            }
        }
        if let Some(fx) = fx.as_deref_mut() {
            if t.translation.distance(eye.pos) <= super::fx::world::STEP_FX_M {
                super::fx::world::footstep(fx, cue, t.translation, step.gain);
            }
        }
    }
}

/// The matter a blow met, heard at the point it met it.
///
/// **Three cues with no producer, from audio v0 to 2026-09-13.**
/// `Cue::ImpactWood`, `ImpactStone` and `ImpactMetal` were in the bank, in
/// the table and in `assets/sound/WANTED.md` as *"producer owed"*, and a
/// landed hatchet blow was voiced by `Cue::Gather` alone — the interface's
/// tick for a payout, non-positional, the same click for a tree and a rock.
/// So hitting wood did not sound like wood, and a swing that connected but
/// paid nothing (a refused tool, a barrel) made no sound at all past the
/// whoosh. `tests/sound.rs`'s own words: *a cue with no producer is a table
/// row that ships silence*.
///
/// Off `impact::Contacts` rather than off the feed directly, and that is the
/// point of the list: `impact::contacts` has already resolved *where* and
/// *on what* for the chips, the sparks and the dust, so the thock cannot
/// land on a different blow from the debris — and the one de-duplication
/// rule (`impact::same_blow`) is applied once, there, instead of a second
/// time here. Which cue is [`contact_cue`]'s: a blow, a round, a body and a
/// charge each sound like themselves.
///
/// Positional even for the swinger's own blow, `shots`' argument: an
/// impact is a thing that happens at a place, and the place is at arm's
/// length. Everyone else's swings and every arrow's stop are the same cue
/// at their own points, which is the disclosure the reference relies on —
/// a chop in the next clearing is heard as a chop.
pub fn impacts(
    contacts: Res<super::impact::Contacts>,
    mut sound: ResMut<Sound>,
    mut roll: Local<u32>,
) {
    for c in contacts.iter() {
        // A cheap stream for the ricochet's odds: cosmetic, and never read
        // by anything that decides.
        *roll = roll.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let glance = (*roll >> 16) % 100 < RICOCHET_PCT;
        if let Some(cue) = contact_cue(c.weapon, c.matter, glance) {
            sound.play(Request::at(cue, [c.at.x, c.at.y, c.at.z]));
        }
    }
}

/// Triple T's tung: a blow with the bat knocks like a hollow log, laid over
/// whatever [`impacts`] says it struck, at the same point. Your own blows off
/// your own hand; anyone else's off the swinger standing at it with a bat in
/// theirs (a melee impact carries no striker, and the swing and its impact
/// land on the same tick).
pub fn tungs(
    contacts: Res<super::impact::Contacts>,
    net: Option<NonSend<Net>>,
    feed: Res<super::feed::Feed>,
    eye: Res<Eye>,
    mut sound: ResMut<Sound>,
) {
    use super::impact::{ContactKind, Weapon};
    if contacts.is_empty() {
        return;
    }
    let Some(n) = net else { return };
    let core = &n.session.core;
    let bat =
        |row: Option<usize>| row.is_some_and(|r| crate::ui::hold::HELD_MODELS[r].key == "bat");
    let mine = bat(crate::ui::hold::held_model_in_hand(
        &core.catalog,
        &core.inv,
        n.sel,
    ));
    let at_tick = core.render_tick();
    let mut rs = client_core::interp::RemoteState::default();
    for c in contacts.iter() {
        if c.weapon != Weapon::Melee {
            continue;
        }
        let tung = match c.kind {
            ContactKind::Swing | ContactKind::Flesh => mine,
            // Yours too, when the sim's mark and the pick did not agree.
            ContactKind::Impact => feed.swings().iter().any(|&id| {
                if id == core.player_id {
                    return mine && eye.pos.distance(c.at) < TUNG_REACH_M;
                }
                core.interp.sample(id, at_tick, &mut rs)
                    && Vec3::new(rs.x, rs.y, rs.z).distance(c.at) < TUNG_REACH_M
                    && bat(crate::ui::hold::held_model_of(&core.catalog, rs.held))
            }),
        };
        if tung {
            sound.play(Request::at(Cue::Tung, [c.at.x, c.at.y, c.at.z]));
        }
    }
}

/// How far from a swinger's feet its blow can land, metres: arm and bat,
/// with room for the contact's lift up a trunk.
const TUNG_REACH_M: f32 = 4.0;

/// How often a round on stone or metal glances off whining, percent.
pub const RICOCHET_PCT: u32 = 30;

/// What a contact sounds like, by what struck it and what it struck: a
/// blow is the impact family, a round or an arrow the bullet family (a
/// `glance` on the hard two is a ricochet), a body is a body whatever hit
/// it, and a charge is a blast. A round into water splashes.
pub fn contact_cue(
    weapon: super::impact::Weapon,
    matter: super::impact::Matter,
    glance: bool,
) -> Option<Cue> {
    use super::impact::{Matter, Weapon};
    match (weapon, matter) {
        (Weapon::Blast, _) => Some(Cue::Blast),
        (_, Matter::Flesh) => Some(Cue::FleshHit),
        (Weapon::Melee, m) => super::impact::impact_cue(m),
        (_, Matter::Water) => Some(Cue::RemoteStepWater),
        (_, Matter::Metal) if glance => Some(Cue::Ricochet),
        (_, Matter::Stone) if glance => Some(Cue::Ricochet),
        (_, Matter::Metal) => Some(Cue::BulletMetal),
        (_, Matter::Stone) => Some(Cue::BulletStone),
        (_, Matter::Wood) => Some(Cue::BulletWood),
        (_, Matter::Dirt | Matter::Sand | Matter::Grass | Matter::Plant) => Some(Cue::BulletSoil),
    }
}

/// Another player's swing — the second sound that decides fights, and until
/// this system nothing produced it: a remote's arm moved in silence.
///
/// **Off the same `feed.swings()` slice `bodies::stream` animates from**, so
/// the sound and the arc cannot disagree about who swung, and off the
/// TRANSFORM that system just wrote, so they cannot disagree about where. It
/// runs after `Stream` for that reason, like [`remote_steps`] beside it.
///
/// Two properties fall out of querying the drawn bodies rather than the feed
/// alone, and both are load-bearing:
///
/// - **Your own swing cannot reach here.** `bodies::stream` skips
///   `core.player_id`, so no entity carries it — the local arm stays
///   [`Cue::Swing`], non-positional, exactly once (`viewmodel::animate`, at
///   the sim's own cadence — it was `render::input`'s press until
///   2026-09-13, and a press is not a swing). Without
///   that you would hear your own swing twice, once at each ear and once at
///   your feet.
/// - **A swinger outside AOI cannot either.** No body, no transform, no
///   sound — which is the honest cull, because a position we do not have is
///   not a position we may guess at.
///
/// "Only nearby" is then the mixer's own falloff at the cue's radius: the
/// falling tree's pattern, and the remote footsteps' — push with a position
/// and let the one distance law decide.
pub fn remote_swings(
    feed: Res<super::feed::Feed>,
    bodies: Query<(&super::bodies::Body, &Transform)>,
    mut sound: ResMut<Sound>,
) {
    if feed.swings().is_empty() {
        return;
    }
    for (body, t) in bodies.iter() {
        if !feed.swings().contains(&body.0) {
            continue;
        }
        sound.play(Request::at(
            Cue::RemoteSwing,
            [t.translation.x, t.translation.y, t.translation.z],
        ));
    }
}

/// Is `item` a dressing rather than a meal: it heals and feeds nothing.
fn dressing(catalog: &protocol::ItemCatalog, item: u16) -> bool {
    let row = catalog.row(item as usize);
    row.health > 0 && row.food == 0 && row.water == 0
}

/// Other bodies' hands, heard at the body (wire v93, `SUB_HEARD`): a
/// magazine seated, a meal or a dressing, a drink, a keypad taking a code, a
/// box's lid or a backpack's leather, a bow coming back. The server sends only what is in
/// earshot; the mixer's falloff does the rest.
pub fn remote_heard(
    net: Option<NonSend<Net>>,
    feed: Res<super::feed::Feed>,
    bodies: Query<(&super::bodies::Body, &Transform)>,
    mut sound: ResMut<Sound>,
) {
    if feed.heard().is_empty() {
        return;
    }
    let Some(n) = net.as_deref() else {
        return;
    };
    let core = &n.session.core;
    let mut rs = client_core::interp::RemoteState::default();
    for &(id, deed, item) in feed.heard() {
        let Some((_, t)) = bodies.iter().find(|(b, _)| b.0 == id) else {
            continue;
        };
        let cue = match deed {
            // The deed names no item; the gun is the one in their hands.
            protocol::DEED_RELOAD => {
                let held = core
                    .interp
                    .sample(id, core.render_tick(), &mut rs)
                    .then_some(rs.held)
                    .flatten();
                reload_cue(&core.catalog, held, true)
            }
            protocol::DEED_MEAL if dressing(&n.session.core.catalog, item) => Cue::RemoteBandage,
            protocol::DEED_MEAL => Cue::RemoteEat,
            protocol::DEED_DRINK => Cue::RemoteDrink,
            protocol::DEED_KEYPAD => Cue::Unlock,
            protocol::DEED_OPEN_BOX => Cue::RemoteContainerOpen,
            protocol::DEED_OPEN_BAG => Cue::RemoteEquip,
            protocol::DEED_DRAW => Cue::RemoteBowDraw,
            _ => continue,
        };
        let p = t.translation;
        sound.play(Request::at(cue, [p.x, p.y + REMOTE_HANDS_H_M, p.z]));
    }
}

/// Gunshots and blasts from past the interest set (wire v113, `SUB_REPORT`):
/// the shard sends no body for them, only what went off and where, out to
/// `GUN_REPORT_CM` and `BLAST_REPORT_CM`. A gun is its far layer at the
/// place, a blast is a blast; the falloff and the distance low-pass make
/// them the dull far crack and rumble they should be.
pub fn far_reports(feed: Res<super::feed::Feed>, mut sound: ResMut<Sound>) {
    for &(kind, x, y, z) in feed.reports() {
        let cue = match kind {
            protocol::REPORT_GUN => Cue::ShotGunFar,
            protocol::REPORT_BLAST => Cue::Blast,
            _ => continue,
        };
        let at = [x as f32, y as f32 + REMOTE_HANDS_H_M, z as f32];
        sound.play(Request::at(cue, at));
    }
}

/// Where another body's hands are heard from, above its feet.
const REMOTE_HANDS_H_M: f32 = 1.1;
/// Time in the air before coming down is heard: a jump is ~0.7 s, and a
/// step off a 0.6 m ledge is ~0.25 s, which is a step and not a landing.
pub const REMOTE_LAND_AIR_S: f32 = 0.3;

/// One remote body as the ear last knew it: what it held (`None` until
/// first sampled) and how long it has been off the ground. Inserted at
/// spawn beside [`RemoteSteps`], so a body that re-enters interest starts
/// fresh rather than "drawing" whatever it already had in hand.
#[derive(Component, Default)]
pub struct RemoteHands {
    held: Option<Option<u16>>,
    air_s: f32,
}

/// What other bodies' hands and feet do that the snapshot already says: a
/// new item drawn (`RemoteState::held` changing to something), and a
/// landing after a real jump or fall (`airborne` clearing) — a splash if it
/// came down in the sea. No wire of its own: the facts were there.
pub fn remote_hands(
    net: Option<NonSend<Net>>,
    world: Res<super::WorldId>,
    time: Res<Time>,
    mut bodies: Query<(
        &super::bodies::Body,
        &Transform,
        &super::anim::BodyAnim,
        &mut RemoteHands,
    )>,
    mut sound: ResMut<Sound>,
    eye: Res<Eye>,
    mut fx: Option<ResMut<super::fx::Fx>>,
) {
    let Some(n) = net.as_deref() else {
        return;
    };
    let core = &n.session.core;
    let tick = core.render_tick();
    let dt = time.delta_secs();
    let mut rs = client_core::interp::RemoteState::default();
    for (body, t, anim, mut hands) in bodies.iter_mut() {
        let at = t.translation;
        // The feet, for the splash: the drawn root is lifted and rides the
        // swell while it floats (`bodies::stream`).
        let mut feet_y = at.y;
        if core.interp.sample(body.0, tick, &mut rs) {
            feet_y = rs.y;
            let now = (!rs.dead && !rs.wounded).then_some(rs.held).flatten();
            if let Some(was) = hands.held {
                if now.is_some() && now != was && !rs.sleeping {
                    sound.play(Request::at(
                        Cue::RemoteEquip,
                        [at.x, at.y + REMOTE_HANDS_H_M, at.z],
                    ));
                }
            }
            hands.held = Some(now);
        }
        if anim.airborne {
            // Floating is not falling: a swim banks no air time, or hauling
            // out (or touching the bottom) would land like a jump.
            if anim.swimming {
                hands.air_s = 0.0;
            } else {
                hands.air_s += dt;
            }
            continue;
        }
        if hands.air_s >= REMOTE_LAND_AIR_S {
            let wet = feet_y < sim_core::terrain::SEA_LEVEL;
            let cue = if wet {
                Cue::RemoteSplash
            } else {
                Cue::RemoteLand
            };
            sound.play(Request::at(cue, [at.x, at.y, at.z]));
            if let Some(fx) = fx.as_deref_mut() {
                if at.distance(eye.pos) <= super::fx::world::STEP_FX_M {
                    if wet {
                        super::fx::world::splash(fx, at, 0.7);
                    } else {
                        let splat = sim_core::terrain::splat(world.seed, at.x, at.z);
                        let ground = crate::sound::steps::surface_cue(splat, false);
                        super::fx::world::body_landing(fx, ground, at);
                    }
                }
            }
        }
        hands.air_s = 0.0;
    }
}

/// Which report a shot makes, from the one bit that separates the two
/// weapons.
///
/// Split out of [`shots`] for `tests/fell.rs`'s stated reason — a decision
/// inside a system needs a window and a socket to drive, and this one is
/// arithmetic. `protocol::shot_is_instant` is the law itself; this is the
/// mapping from it to a sound, and the mapping is the half a gate can
/// actually be wrong about.
///
/// No item crosses the wire on `EV_SHOT` and none is wanted: the two cues
/// exist because the two *disclosure radii* differ, and the radius is what
/// the wire's one bit already tells you.
pub fn shot_cue(speed_mmpt: u16, low: u16) -> Cue {
    // A silenced gun is heard as the crossbow's thunk, at its 40 m.
    if protocol::shot_is_quiet(speed_mmpt, low) {
        Cue::ShotCrossbow
    } else if protocol::shot_is_gun(speed_mmpt, low) {
        Cue::ShotGun
    } else {
        Cue::ShotBow
    }
}

/// Every shot in earshot, as a report at the shooter (wire v54).
///
/// **This is the fact the game has been broadcasting and nobody was
/// listening to.** `EV_SHOT` has crossed the wire since ranged v0 and had
/// exactly one reader — `render/tracer.rs`, which draws a streak — so a bow
/// made no sound, and a firearm raised no event at all. A gunfight was a
/// private event between the two people in it: the only evidence a shot had
/// been fired was the damage, which is the wrong end of the fight to learn
/// it at. Sound is the reference's primary disclosure channel
/// (`reference/AUDIO.md` §9) and the radius is where the mechanic lives —
/// 40 m for a bow against 100 m for a gun.
///
/// **The shooter's own shot is positional too, and that is deliberate**
/// against `feed`'s own-fact rule below. A report is not an interface sound
/// happening *to* you like a hitmarker; it is a thing that happens at a
/// place, and the place is where you are standing. Emitting it at the body
/// costs one lookup and keeps one rule — `Request::at` for a world event,
/// `Request::own` for an answer from the interface — instead of splitting
/// one cue across both constructors. The falloff at zero distance is 1.0,
/// so it is full gain with no pan, which is what `own` would have given it
/// anyway.
///
/// A shooter with no body is dropped rather than played from the origin,
/// `tracer::launch`'s rule for `tracer::launch`'s reason: the local player
/// is not in `bodies`, so the predictor answers for them.
pub fn shots(
    net: NonSend<Net>,
    feed: Res<super::feed::Feed>,
    eye: Res<Eye>,
    bodies: Query<(&super::bodies::Body, &Transform)>,
    mut sound: ResMut<Sound>,
    gf: Res<super::gunfeel::GunFeel>,
    mv: Option<Res<super::mods::ModView>>,
) {
    let core = &net.session.core;
    // Your own gun, heard the frame its trigger broke (`gunfeel`), as the
    // kind of gun it is — or a silencer's thunk.
    if gf.fired > 0 && !core.spectating() {
        let p = core.predict.position();
        let cue = if mv.as_deref().is_some_and(|v| v.quiet) {
            Cue::ShotCrossbow
        } else {
            gun_cue(&core.catalog, Some(gf.item)).unwrap_or(Cue::ShotGun)
        };
        sound.play(Request::at(cue, [p[0], p[1], p[2]]));
    }
    if feed.shots().is_empty() {
        return;
    }
    let mut rs = client_core::interp::RemoteState::default();
    for &(shooter, _yaw, _pitch, speed_mmpt, low, _fine) in feed.shots() {
        // The one bit that separates the two weapons, and it is the same
        // bit the tracer reads: a projectile cannot leave a muzzle at rest,
        // so zero means the shot was instantaneous and instantaneous means
        // a firearm. Which projectile weapon is the hand's: a crossbow is
        // heard as one (`projectile_cue`).
        let mut cue = shot_cue(speed_mmpt, low);
        let at = if shooter == core.player_id {
            // Your gunshot was heard when it broke; the shard's word on it
            // is not a second report.
            if protocol::shot_is_gun(speed_mmpt, low) && !core.spectating() {
                continue;
            }
            let s = core.inv[(net.sel as usize).min(core.inv.len() - 1)];
            cue = projectile_cue(&core.catalog, cue, (s.count > 0).then_some(s.item));
            // A spectator predicts nothing: the watched body is where it is.
            let p = if core.spectating() {
                core.eye_position()
            } else {
                core.predict.position()
            };
            [p[0], p[1], p[2]]
        } else if let Some(t) = bodies
            .iter()
            .find(|(b, _)| b.0 == shooter)
            .map(|(_, t)| t.translation)
        {
            if core.interp.sample(shooter, core.render_tick(), &mut rs) {
                cue = projectile_cue(&core.catalog, cue, rs.held);
                if protocol::shot_is_gun(speed_mmpt, low)
                    && !protocol::shot_is_quiet(speed_mmpt, low)
                {
                    cue = gun_cue(&core.catalog, rs.held).unwrap_or(cue);
                }
            }
            [t.x, t.y, t.z]
        } else if (super::heli::is_heli(shooter) || super::sentry::is_sentry(shooter))
            && core.interp.sample(shooter, core.render_tick(), &mut rs)
        {
            // The attack helicopter has no drawn `Body`: its gun is heard
            // where the interpolator has it, as its own gun at any range —
            // the far layer's echo would smear a burst the same way. A
            // sentry's gun is a rifle on a mount: the rifle's crack, where
            // the revolver's ring smeared its burst.
            if super::heli::is_heli(shooter) {
                cue = heli_shot_cue(eye.pos.distance(Vec3::new(rs.x, rs.y + 1.6, rs.z)));
            } else if protocol::shot_is_gun(speed_mmpt, low) {
                cue = Cue::ShotRifle;
            }
            [rs.x, rs.y + 1.6, rs.z]
        } else {
            continue;
        };
        let d = eye.pos.distance(Vec3::from(at));
        sound.play(Request::at(far_layer(cue, d), at));
    }
}

/// A flying shot's cue by what loosed it: a crossbow in the hand is a
/// crossbow, a rocket launcher a rocket's launch, anything else that flies
/// is a bow. A gunshot is left alone.
pub fn projectile_cue(catalog: &protocol::ItemCatalog, cue: Cue, held: Option<u16>) -> Cue {
    match held {
        Some(item) if cue == Cue::ShotBow && crate::ui::hold::is_crossbow(catalog, item) => {
            Cue::ShotCrossbow
        }
        Some(item)
            if cue == Cue::ShotBow && gun_cue(catalog, Some(item)) == Some(Cue::ShotRocket) =>
        {
            Cue::ShotRocket
        }
        _ => cue,
    }
}

/// A gun's own report (guns v1), by the kind of gun in the hand: the
/// rifles' crack, the SMGs' chatter, a pistol's snap, the pump's boom, the
/// bolt's long report and a rocket's launch. `None` for anything else — the
/// revolver and the Python keep the revolver's ring.
pub fn gun_cue(catalog: &protocol::ItemCatalog, held: Option<u16>) -> Option<Cue> {
    let name = core::str::from_utf8(catalog.name(held? as usize)).ok()?;
    Some(match crate::ui::icons::stem(name).as_str() {
        "assault_rifle" | "semi_automatic_rifle" => Cue::ShotRifle,
        "thompson" | "custom_smg" => Cue::ShotSmg,
        "semi_automatic_pistol" => Cue::ShotPistol,
        "pump_shotgun" => Cue::ShotShotgun,
        "bolt_action_rifle" => Cue::ShotBolt,
        "rocket_launcher" => Cue::ShotRocket,
        _ => return None,
    })
}

/// A reload's cue by the gun in the hand, `remote` at another body: the
/// families [`gun_cue`] tells apart, and the revolver's cylinder for the
/// rest.
pub fn reload_cue(catalog: &protocol::ItemCatalog, held: Option<u16>, remote: bool) -> Cue {
    let (own, theirs) = match gun_cue(catalog, held) {
        Some(Cue::ShotRifle | Cue::ShotSmg) => (Cue::ReloadRifle, Cue::RemoteReloadRifle),
        Some(Cue::ShotPistol) => (Cue::ReloadPistol, Cue::RemoteReloadPistol),
        Some(Cue::ShotShotgun) => (Cue::ReloadShotgun, Cue::RemoteReloadShotgun),
        Some(Cue::ShotBolt) => (Cue::ReloadBolt, Cue::RemoteReloadBolt),
        _ => (Cue::Reload, Cue::RemoteReload),
    };
    if remote {
        theirs
    } else {
        own
    }
}

/// Past this far the heli's gun is its far layer, metres: where the near
/// round's falloff and the far one's meet at the same loudness (the two
/// rows' gains), so the switch is a change of colour and not a step — the
/// reference's gunshots "fade to the distant sound at the same distance".
pub const HELI_SHOT_FAR_M: f32 = 60.0;

/// The heli's round heard from `d` metres: near, or past
/// [`HELI_SHOT_FAR_M`] the distant blast and echo.
pub fn heli_shot_cue(d: f32) -> Cue {
    if d > HELI_SHOT_FAR_M {
        Cue::ShotHeliFar
    } else {
        Cue::ShotHeli
    }
}

/// Past this far a gunshot is its far layer, metres: where the near
/// report's falloff and the far one's meet at about the same level
/// (`CUES`' two rows), so the switch is a change of colour and not a step.
pub const SHOT_FAR_M: f32 = 45.0;

/// The layer of a report heard from `d` metres. Only the gun has a far
/// layer; a bow does not carry far enough to need one.
pub fn far_layer(cue: Cue, d: f32) -> Cue {
    if d <= SHOT_FAR_M {
        return cue;
    }
    match cue {
        Cue::ShotGun => Cue::ShotGunFar,
        Cue::ShotRifle => Cue::ShotRifleFar,
        Cue::ShotSmg => Cue::ShotSmgFar,
        Cue::ShotPistol => Cue::ShotPistolFar,
        Cue::ShotShotgun => Cue::ShotShotgunFar,
        Cue::ShotBolt => Cue::ShotBoltFar,
        c => c,
    }
}

/// This frame's own-facts, as cues.
///
/// **Reads [`super::feed::Feed`]; pops nothing.** It used to pop the core's
/// rings directly, and so did `hud::feedback` — two systems written on two
/// branches, each correct alone, that git merged without a conflict into a
/// client whose HUD ate every event before the mixer saw one. `feed.rs`'s
/// header is the whole account; the rule that came out of it is that
/// `feed::drain` is the only `pop_*` call site in the client.
pub fn feed(
    net: NonSend<Net>,
    feed: Res<super::feed::Feed>,
    world: Option<Res<super::WorldId>>,
    mut sound: ResMut<Sound>,
    mut cont: Local<(u8, u32)>,
) {
    // A container opened under your hands: a box's or a crate's latch and
    // lid, a backpack's leather. Off the open view naming a new container,
    // which is the server answering the open, not the key.
    let open = (net.session.core.cont_kind, net.session.core.cont_handle);
    if open != *cont && open.0 != sim_core::inventory::CONT_SELF {
        let core = &net.session.core;
        let furnace = crate::ui::oven::converter_open(
            open.0,
            open.1,
            core.deploys.entries(),
            &core.deploy_defs,
            core.deploy_defs_have,
        ) == Some(sim_core::deploy::ARCH_FURNACE);
        let cue = if open.0 == sim_core::inventory::CONT_BAG
            || open.0 == sim_core::inventory::CONT_BODY
        {
            Cue::Equip
        } else if furnace {
            Cue::FurnaceOpen
        } else {
            Cue::ContainerOpen
        };
        sound.play(Request::own(cue));
    }
    *cont = open;
    // One marker per frame however many landed — `Cue::Hit`'s own cooldown
    // would refuse the rest anyway, and asking for four identical clicks so
    // three can be thrown away is work the queue does not need to do.
    //
    // WHICH marker is `sound::hit`'s judgement and not this system's, the
    // shape `sound::hurt` established: the rung arrives already merged in
    // `feed.hit_part` and the choice of cue is a pure function a headless
    // test can drive.
    if let Some(req) = crate::sound::hit::request(feed.hits, feed.hit_part) {
        sound.play(req);
        // The middle bump. One per frame however many landed, for the same
        // reason the marker is: the director reads its tier once a section,
        // so four bumps in one frame and one bump in one frame are the same
        // musical fact. Rung-blind on purpose — a headshot is not a
        // different musical event, it is the same fight going better.
        sound.music.bump(music::BUMP_HIT);
    }
    for &(victim, _killer) in feed.deaths() {
        // Someone else dying is not your own fact and has no position on this
        // wire, so only your own death makes a sound. A death rattle for a
        // player across the island would be a lie about where they are.
        if victim == net.session.core.player_id {
            sound.play(Request::own(Cue::Death));
        }
    }
    // Going down is the same sound as dying (wounded v0), and on purpose:
    // the reference shipped ONE wounded sound (Devblog 57) and the blow that
    // put you on the ground would have been the death a minute ago. A voice
    // of its own is a `synth.rs` row and a bank entry; `NOW.md` §0wnd.
    if feed.wounded.is_some() {
        sound.play(Request::own(Cue::Death));
    }
    for _ in feed.gathered() {
        sound.play(Request::own(Cue::Gather));
    }
    for _ in feed.crafted() {
        sound.play(Request::own(Cue::CraftDone));
    }
    // A blueprint learned: it was a toast and no sound.
    if !feed.learned().is_empty() {
        sound.play(Request::own(Cue::Learn));
    }
    // A kiosk trade went through (THE GATE): the coins.
    for _ in feed.traded() {
        sound.play(Request::own(Cue::Trade));
    }
    // A town sentry locking on: the beep, at the gun, for everyone in
    // earshot — Rust's turret tells the whole town before it fires.
    {
        let core = &net.session.core;
        let mut rs = client_core::interp::RemoteState::default();
        for &(gun, _) in feed.sentry_locks() {
            if core.interp.sample(gun, core.render_tick(), &mut rs) {
                sound.play(Request::at(Cue::SentryLock, [rs.x, rs.y + 1.6, rs.z]));
            }
        }
    }
    // A mouthful landed (the refused half buzzes below, as `Refused::Consume`).
    // Read off the drained list, never a second pop. A heal with nothing to
    // eat in it — a bandage, a medkit — is wrapped rather than chewed: the
    // HUD's EAT/USE line draws the same distinction off the same row.
    for &(item, _slot) in feed.consumed() {
        let cue = if dressing(&net.session.core.catalog, item) {
            Cue::Bandage
        } else {
            Cue::Eat
        };
        sound.play(Request::own(cue));
    }
    // A magazine seated: your own hands, so an own-fact. (It borrowed the
    // positional `Cue::Place` with no position, which the mixer refuses —
    // a reload was silent.) As the kind of gun it is.
    if feed.reloaded > 0 {
        let core = &net.session.core;
        let s = core.inv[(net.sel as usize).min(core.inv.len() - 1)];
        let held = (s.count > 0).then_some(s.item);
        sound.play(Request::own(reload_cue(&core.catalog, held, false)));
    }
    // A drink landed: it was silent.
    if feed.applied & client_core::core::APPLIED_DRANK != 0 {
        sound.play(Request::own(Cue::Drink));
    }
    // A knock, at the door knocked on — broadcast, so it is often somebody
    // else's hand, and where the door is is the news.
    if let Some(world) = world {
        let core = &net.session.core;
        for &(cx, cz, level, loc, _who) in feed.knocks() {
            let (x, z) = sim_core::build::anchor(cx, cz, loc);
            let plate = core.pieces.cols().plate(cx, cz).unwrap_or(0);
            let y = super::structures::level_base_y(world.seed, &world.haven, cx, cz, level, plate);
            // A sheet-metal or garage door rings under the fist.
            let metal = core
                .deploys
                .entries()
                .iter()
                .find(|r| (r.cx, r.cz, r.level, r.loc) == (cx, cz, level, loc))
                .filter(|r| u16::from(r.row) < core.deploy_defs_have)
                .is_some_and(|r| {
                    core.deploy_defs.defs[r.row as usize].matter == sim_core::deploy::MATTER_METAL
                });
            let cue = if metal { Cue::KnockMetal } else { Cue::Knock };
            sound.play(Request::at(cue, [x, y + 1.3, z]));
        }
        // Something mended, at it: a repair latches the readout at its
        // whole hp (`APPLIED_STRUCT_HIT`'s doc), where a hit leaves it short.
        let (cx, cz, level, loc, left, max) = core.struct_hit;
        if feed.applied & client_core::core::APPLIED_STRUCT_HIT != 0 && max > 0 && left == max {
            let (x, z) = sim_core::build::anchor(cx, cz, loc);
            let plate = core.pieces.cols().plate(cx, cz).unwrap_or(0);
            let y = super::structures::level_base_y(world.seed, &world.haven, cx, cz, level, plate);
            sound.play(Request::at(Cue::Repair, [x, y + 1.2, z]));
        }
        // A lock or a hearth took the code: the keypad's accept, at it.
        for &(cx, cz, level, loc, grant) in feed.auths() {
            if grant == sim_core::lock::GRANT_NONE {
                continue;
            }
            let (x, z) = sim_core::build::anchor(cx, cz, loc);
            let plate = core.pieces.cols().plate(cx, cz).unwrap_or(0);
            let y = super::structures::level_base_y(world.seed, &world.haven, cx, cz, level, plate);
            sound.play(Request::at(Cue::Unlock, [x, y + 1.2, z]));
        }
        // A charge going live: its fuse sizzles where it was stuck, for
        // anyone near enough to run.
        if feed.applied2 & client_core::core::APPLIED2_CHARGE != 0 {
            let (cx, cz, level, loc, _row, fuse) = core.charge_placed;
            if fuse > 0 {
                let plate = core.pieces.cols().plate(cx, cz).unwrap_or(0);
                // The piece it is stuck to: a spelled id names one the grid
                // cannot (freeform W2), and the mirror says where it is.
                let at = super::structures::addr_root(
                    world.seed,
                    &world.haven,
                    core.pieces.store(),
                    (cx, cz, level, loc),
                    plate,
                )
                .translation;
                sound.play(Request::at(Cue::Fuse, [at.x, at.y + 1.0, at.z]));
            }
        }
    }
    // Every refusal kind, one sound, but one: a trigger pulled on an empty
    // magazine is the gun's dry click, because the hand is on a weapon and
    // not on a menu. A player does not need to hear the difference between
    // a refused craft and a refused placement — the toast already says
    // which — they need to hear that the button did nothing.
    //
    // Deliberately not a count: this line said "three" while `Refused` held
    // four, and then five when the consume verbs joined (2026-08-15). The
    // predicate below is variant-agnostic, so a new refusal kind gets this
    // cue for free the moment `feed.rs` pushes it — which is the property
    // worth writing down, and a number here only ever goes stale against it.
    let dry = |&(kind, code, _): &(super::feed::Refused, u8, u16)| {
        kind == super::feed::Refused::Reload && u32::from(code) == sim_core::ranged::REFUSE_RL_EMPTY
    };
    if feed.refusals().any(|r| dry(&r)) {
        sound.play(Request::own(Cue::DryFire));
    }
    if feed.refusals().any(|r| !dry(&r)) {
        sound.play(Request::own(Cue::Refused));
    }
    // A spill borrows that cue rather than minting one, on the same
    // argument one step out: the player is looking at the TREE, not at the
    // toast line, so what they need from the ear is "that did not go the
    // way you expected" — and the toast says which. It is not literally a
    // refusal (the swing paid; the payment is in a bag at your feet), so a
    // voice of its own is a fair later change; it would cost a `synth.rs`
    // row and a bank entry, which is more than this fact is worth today.
    //
    // Not folded into the `if` above: these are separate arrays and a
    // frame can hold both, but one buzz per frame per class is the point.
    if !feed.spills().is_empty() {
        sound.play(Request::own(Cue::Refused));
    }
}

/// A scatter slot going away — the loudest positional cue in the client.
///
/// **Read off change detection rather than off `slot_changes()`**, which is
/// the same call `props.rs` makes and for the same reason: `Session::pump`
/// drains every queued message before the renderer looks, so a frame that
/// received two event messages sees only the second one's change feed. The
/// mesh swap is authoritative and this listens to the swap.
///
/// `Ref` rather than a `Changed<T>` filter because `Changed` also fires on the
/// frame a component is ADDED, and props are added by the streamer every time
/// the player walks a chunk into the ring. Without the `is_added` guard, every
/// already-felled stump in the forest would crash to the ground again each
/// time it streamed in.
///
/// **A bush is picked, not felled** (`E`, `gather::pick`): it plays
/// [`Cue::BushPick`] and throws a dust poof and a few leaves, once per slot — a bush is
/// two `Vanish` entities (the blob and its leaf cards), so the key is
/// remembered for the frame. Which `Vanish` node is a bush is read off the
/// scatter by its key, on the transition frame only.
pub fn fell(
    q: Query<(Ref<super::props::Fellable>, &GlobalTransform)>,
    world: Option<Res<super::WorldId>>,
    eye: Res<Eye>,
    mut fx: ResMut<super::fx::Fx>,
    mut sound: ResMut<Sound>,
) {
    let mut picked = [u32::MAX; 8];
    let mut n_picked = 0;
    for (f, t) in q.iter() {
        if !f.is_changed() || f.is_added() || !f.felled {
            // A slot respawning is silent: a tree that comes back after 20-45
            // minutes (`TERRAIN.md` §2) does not do so audibly.
            continue;
        }
        // **One cue per SLOT, not one per entity — and this was already wrong
        // before the tree could topple.** A tree has always been more than one
        // `Fellable`: the canopy is a sibling carrying its own, so a chop fired
        // `TreeFall` for the trunk *and* `ImpactStone` for the needles, at the
        // same position on the same frame. Nothing caught it because both cues
        // are real cues and the mix is the only place the pair is audible.
        // Felling v0 would have made it three, which is what made it visible.
        //
        // `FellPart` names the parts, so the rule can be stated instead of
        // inferred: the trunk speaks for the tree, a `Vanish` node speaks for
        // itself, and the canopy and the stump are silent because they are
        // parts of something that already made a sound.
        let occupant = if matches!(f.part, super::props::FellPart::Vanish) {
            scatter_occupant(world.as_deref(), f.key)
        } else {
            None
        };
        let plant = occupant.filter(|&o| sim_core::gather::pickable(o));
        let cue = match f.part {
            super::props::FellPart::Trunk => Cue::TreeFall,
            super::props::FellPart::Vanish if plant.is_some() => {
                if picked[..n_picked].contains(&f.key) {
                    continue;
                }
                if n_picked < picked.len() {
                    picked[n_picked] = f.key;
                    n_picked += 1;
                }
                let high = super::impact::strike_height(plant.map_or(0, |o| o as u8));
                let base = t.translation();
                super::fx::world::picked(&mut fx, base, high, eye.pos);
                let at = base + Vec3::Y * high;
                sound.play(Request::at(Cue::BushPick, [at.x, at.y, at.z]));
                continue;
            }
            // An ore node's own break is `ore::shatter`'s crash; a barrel is
            // metal, not a pick on rock.
            super::props::FellPart::Vanish => match occupant {
                Some(
                    sim_core::terrain::Occupant::StoneNode
                    | sim_core::terrain::Occupant::MetalNode
                    | sim_core::terrain::Occupant::SulfurNode,
                ) => continue,
                Some(sim_core::terrain::Occupant::BarrelSlot) => Cue::ImpactMetal,
                _ => Cue::ImpactStone,
            },
            // …and `Far` with them: the hull is the trunk at a distance, so
            // a chop that felled both would play `TreeFall` twice on one
            // frame at one position — the exact defect this match was
            // written to end.
            super::props::FellPart::Canopy
            | super::props::FellPart::Stump
            | super::props::FellPart::Far
            | super::props::FellPart::Emptied => continue,
        };
        let p = t.translation();
        sound.play(Request::at(cue, [p.x, p.y, p.z]));
    }
}

/// What stands at cell key `key`, the scatter's answer, cold.
fn scatter_occupant(
    world: Option<&super::WorldId>,
    key: u32,
) -> Option<sim_core::terrain::Occupant> {
    let w = world?;
    let (cx, cz) = ((key >> 16) as i32, (key & 0xFFFF) as i32);
    Some(sim_core::terrain::scatter(w.seed, &w.table, &w.haven, cx, cz).occupant)
}

/// Something new in your hand (`Cue::Equip`): the hotbar moved to a slot
/// holding an item, or the item in the held slot was swapped for another.
/// Not an emptied hand, and not an item arriving in an empty one — that is a
/// join's or a respawn's inventory landing, not a draw. A deployable is drawn
/// as a blueprint unrolling (`sheet`), so it rustles (`Cue::MapPaper`).
pub fn hands(
    net: Option<NonSend<Net>>,
    mut sound: ResMut<Sound>,
    mut last: Local<Option<(u8, u16)>>,
) {
    let Some(n) = net.as_deref() else {
        return;
    };
    let core = &n.session.core;
    let held = if core.holstered() {
        sim_core::gather::NO_ITEM
    } else {
        core.inv
            .get(usize::from(n.sel))
            .filter(|s| s.count > 0)
            .map_or(sim_core::gather::NO_ITEM, |s| s.item)
    };
    if let Some((sel, was)) = *last {
        let drawn = held != sim_core::gather::NO_ITEM
            && (n.sel != sel || (held != was && was != sim_core::gather::NO_ITEM));
        if drawn {
            let cue = if super::sheet::up(n) {
                Cue::MapPaper
            } else {
                Cue::Equip
            };
            sound.play(Request::own(cue));
        }
    }
    *last = Some((n.sel, held));
}

/// The map unfolded or put away (`Cue::MapPaper`): on entering and on
/// leaving `Screen::Map`.
pub fn map_paper(sound: Option<ResMut<Sound>>) {
    if let Some(mut sound) = sound {
        sound.play(Request::own(Cue::MapPaper));
    }
}

/// The interface's click (`Cue::UiClick`), on any button the pointer
/// presses. One system over every screen's buttons rather than a line in
/// each screen's handler: a screen added later clicks without knowing this
/// exists. Runs where the mixer does (`world_running`), so the in-world
/// panels and the Esc menu click; the desktop's title menu does not yet.
pub fn ui_click(buttons: Query<&Interaction, Clickable>, mut sound: ResMut<Sound>) {
    if buttons.iter().any(|i| *i == Interaction::Pressed) {
        sound.play(Request::own(Cue::UiClick));
    }
}

/// The buttons [`ui_click`] hears: any, but a [`Quiet`] one.
type Clickable = (Changed<Interaction>, With<Button>, Without<Quiet>);
/// The buttons [`ui_hover`] hears: any, but a [`NoHover`] one.
type Hoverable = (Changed<Interaction>, With<Button>, Without<NoHover>);

/// A button whose press has a cue of its own (the CRAFT button's
/// [`Cue::CraftStart`]), so [`ui_click`] leaves it alone rather than
/// stacking a click under it.
#[derive(Component)]
pub struct Quiet;

/// A button the pointer sweeps across without a tick: the inventory's slots
/// and the recipe grid, where a tick per cell would be a rattle. Rust's
/// slots are silent under the pointer too; its buttons are not.
#[derive(Component)]
pub struct NoHover;

/// The pointer arriving on a button (`Cue::UiHover`): the faintest tick.
///
/// **Only a pointer that MOVED this frame.** Every panel is rebuilt when
/// what it draws changes (`panels::rebuild`), and a rebuild respawns the
/// button under a still pointer, which then turns `Hovered` without anybody
/// having moved — a tick there would sound like a hover the player never
/// made. And not the button just pressed: letting go turns it back to
/// `Hovered`, which is the end of a click, not the start of a hover.
pub fn ui_hover(
    buttons: Query<(Entity, &Interaction), Hoverable>,
    mut moved: MessageReader<bevy::window::CursorMoved>,
    mut pressed: Local<Option<Entity>>,
    mut sound: ResMut<Sound>,
) {
    let moved = moved.read().count() > 0;
    let mut hover = false;
    for (e, i) in &buttons {
        match i {
            Interaction::Pressed => *pressed = Some(e),
            Interaction::Hovered if *pressed == Some(e) => *pressed = None,
            Interaction::Hovered => hover = true,
            Interaction::None => {}
        }
    }
    if hover && moved {
        sound.play(Request::own(Cue::UiHover));
    }
}

/// The panels' own sounds: the pack opening and closing as the inventory
/// pages come and go, and whatever a panel asked for on its last press
/// (`panels::Ui::sound` — an item set down, an item dropped, the craft
/// started). One slot rather than a queue: a panel does one thing a press.
///
/// A box opening is the box's sound ([`feed`]'s `ContainerOpen`), so the
/// pack's rustle stays out from under it.
pub fn panel_sounds(
    net: NonSend<Net>,
    ui: Option<ResMut<super::panels::Ui>>,
    mut last: Local<super::panels::Panel>,
    mut sound: ResMut<Sound>,
) {
    use super::panels::Panel;
    let Some(mut ui) = ui else {
        return;
    };
    let pack = |p: Panel| {
        matches!(
            p,
            Panel::Inventory | Panel::Craft | Panel::Vendor | Panel::Tech
        )
    };
    let now = ui.panel;
    if now != *last {
        let boxed = net.session.core.cont_kind != sim_core::inventory::CONT_SELF;
        if pack(now) && !pack(*last) && !boxed {
            sound.play(Request::own(Cue::InvOpen));
        } else if now == Panel::None && pack(*last) {
            sound.play(Request::own(Cue::InvClose));
        }
        *last = now;
    }
    if let Some(cue) = ui.sound.take() {
        sound.play(Request::own(cue));
    }
}

/// A placement landing — the second positional cue, at the cell it landed.
///
/// **Reads [`super::feed::Feed`]; pops nothing** (`feed::drain` is the one
/// pop site). The join-flood guard lives in the CORE, not here: the ring
/// behind `Feed::placed` is fed only by the `PiecePlaced`/`DeployPlaced`
/// broadcasts — a placement *happening* — and never by the sync walks, which
/// merely restate the world. So a fresh join streaming every standing piece,
/// or a resync restating them, produces zero entries here by construction,
/// with no timer deciding when the flood is over. Distance does the rest the
/// way it does for the falling tree: the request carries the position and
/// `sound::falloff` culls it at the cue's own radius.
///
/// The position is [`super::structures::base_transform`]'s — the same
/// anchor the mesh stands at, edge canonicalisation included, so the sound
/// cannot come from a different place than the wall appears in.
///
/// **The plate is read out of the mirror, not off the event** (build plate
/// v1): `feed.placed()` carries an address and the placement broadcast that
/// raised it also inserted the record, so the column is in the piece mirror by
/// the time this runs. A hammer blow is a point source at head height either
/// way — but the column's floor is the one number here that can be a whole
/// storey out, and a sound a storey off is a sound in the wrong room.
pub fn place(
    feed: Res<super::feed::Feed>,
    world: Res<super::WorldId>,
    net: NonSend<super::Net>,
    mut sound: ResMut<Sound>,
) {
    let core = &net.session.core;
    for &(cx, cz, level, loc, deploy) in feed.placed() {
        let plate = core.pieces.cols().plate(cx, cz).unwrap_or(0);
        // A deployable sounds where it stands (free placement).
        let rec = if deploy {
            core.deploys
                .entries()
                .iter()
                .find(|r| (r.cx, r.cz, r.level, r.loc) == (cx, cz, level, loc))
        } else {
            None
        };
        let p = match rec {
            Some(r) => super::structures::deploy_fx_feet(
                world.seed,
                &world.haven,
                core.pieces.cols(),
                &core.deploy_defs,
                core.deploy_defs_have,
                r,
            ),
            None => super::structures::addr_root(
                world.seed,
                &world.haven,
                core.pieces.store(),
                (cx, cz, level, loc),
                plate,
            ),
        }
        .translation;
        // A piece sounds like what it is made of: an upgrade to stone is a
        // block seating, to metal a sheet going on — the reference's grades
        // each have their own. A deployable, a twig or a wooden piece is
        // the plank going down.
        let material = (!deploy)
            .then(|| {
                core.pieces
                    .find(cx, cz, level, loc)
                    .filter(|r| {
                        (r.row as u16) < core.piece_defs_have.min(core.piece_defs.piece_count)
                    })
                    .map(|r| core.piece_defs.pieces[r.row as usize].material)
            })
            .flatten();
        let cue = match material {
            Some(sim_core::build::MAT_STONE) => Cue::PlaceStone,
            Some(sim_core::build::MAT_METAL) => Cue::PlaceMetal,
            _ => Cue::Place,
        };
        sound.play(Request::at(cue, [p.x, p.y, p.z]));
    }
}

/// The herd's voices, off the drawn animals' interpolated positions.
///
/// Dormancy-respecting by construction: an `Animal` entity exists only for a
/// mob inside AOI (208 m), and every mob a client can see is awake —
/// `MOB_WAKE_CM` (240 m) deliberately encloses AOI (`limits.rs`), so a
/// voiced animal is always a simmed animal. "Only nearby" is then the mixer's
/// own falloff at the cue's radius — the falling-tree pattern: push with a
/// position, let the one distance law cull.
///
/// The cadence is `sound::voice`'s — hashed per roster slot and cycle, so it
/// is deterministic (no OS randomness) and not a metronome. The head height
/// puts the emitter at the snout rather than under the feet.
///
/// **This system does not know what a pig or a wolf is, and that is the fix
/// it carries.** Until 2026-08-14 it was `pigs`, it played [`Cue::Snort`]
/// unconditionally, and so every wolf on the island snorted. It now asks
/// `voice::Voices::due` — which reads the species off the roster slot itself
/// (`mob::kind_of`) — and plays whatever cue it is handed. Adding a third
/// species is then a change to `sound::voice` and to nothing here.
///
/// The one thing this system decides is the **register**, because it is the
/// only half that knows where the listener is: `near` is measured to the
/// emitter point, the same point the mixer will then cull against, so the
/// distance the register turns on and the distance the cue is audible to
/// are the same arithmetic on the same two positions.
pub fn voices(
    herd: Query<(&super::mobs::Animal, &Transform, &super::mobs::Gait)>,
    eye: Res<Eye>,
    time: Res<Time>,
    feed: Res<super::feed::Feed>,
    mut sound: ResMut<Sound>,
    mut seen: Local<HerdSeen>,
) {
    let dt = time.delta_secs();
    let switch = crate::sound::voice::switch_m();
    seen.now.clear();
    for (animal, t, gait) in herd.iter() {
        let Some(slot) = sim_core::mob::slot_of_id(animal.0) else {
            continue;
        };
        let p = t.translation;
        let at = [p.x, p.y + super::mobs::voice_h_of(slot), p.z];
        seen.now.push((animal.0, Vec3::from_array(at)));
        // A blow landed on it (`EV_HIT` is the striker's fact, so this is
        // your own): its pain, from it.
        if feed.hit_victims().contains(&animal.0) {
            sound.voices.called(slot);
            sound.play(Request::at(crate::sound::voice::hurt_of(slot), at));
            continue;
        }
        // A pack call the sim raised (wire v76): the howl goes up now, from
        // this animal, and its own clock restarts so the ambient cadence
        // does not answer it a second later. `Feed` is read, never drained
        // — `feed::drain` is the one reader of the core's rings.
        if feed.howls().contains(&animal.0) {
            sound.voices.called(slot);
            sound.play(Request::at(Cue::Howl, at));
            continue;
        }
        // A bite (the animal's `EV_SWING`): its near voice with the lunge,
        // so a fight between an animal and someone else is heard too.
        if feed.swings().contains(&animal.0) {
            let cue = crate::sound::voice::cue_of(slot, true);
            sound.voices.called(slot);
            sound.play(Request::at(cue, at));
            continue;
        }
        // A sleeping animal says nothing; its clock waits.
        if gait.asleep {
            continue;
        }
        // The hunt starting (v99): its near voice at once — a pig's charge
        // snort, a wolf's growl — rather than whenever the clock comes round.
        if sound.voices.rouse(slot, gait.hostile) {
            let cue = crate::sound::voice::cue_of(slot, true);
            sound.voices.called(slot);
            sound.play(Request::at(cue, at));
            continue;
        }
        let d = [at[0] - eye.pos.x, at[1] - eye.pos.y, at[2] - eye.pos.z];
        let close = d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= switch * switch;
        // The register is the brain's now, not the distance alone: a wolf
        // growls when it is hunting and close enough to hear, and howls
        // otherwise; a pig snorts fast while it charges.
        let near =
            gait.hostile && (close || sim_core::mob::kind_of(slot) != sim_core::mob::MOB_WOLF);
        let Some(cue) = sound.voices.due(slot, near, dt) else {
            continue;
        };
        sound.play(Request::at(cue, at));
    }
    for (id, at) in seen.deaths(eye.pos) {
        if let Some(slot) = sim_core::mob::slot_of_id(id) {
            let cue = crate::sound::voice::death_of(slot);
            sound.play(Request::at(cue, at.to_array()));
        }
    }
}

/// An animal gone from the herd this near has died: `mobs::stream` drops it
/// for that or for leaving the interest set, and the interest set ends far
/// past this. Metres.
pub const DEATH_HEARD_M: f32 = 60.0;

/// The herd as [`voices`] last saw it, for the death cry. An animal must be
/// missing two frames running (a model swap respawns one across a frame),
/// and a frame that loses more than two at once is a teleport or a resync,
/// not a slaughter.
#[derive(Default)]
pub struct HerdSeen {
    now: Vec<(u32, Vec3)>,
    last: Vec<(u32, Vec3)>,
    missing: Vec<(u32, Vec3, u8)>,
    died: Vec<(u32, Vec3)>,
}

impl HerdSeen {
    /// This frame's deaths in earshot of `ear`, after `now` was filled.
    fn deaths(&mut self, ear: Vec3) -> std::vec::Drain<'_, (u32, Vec3)> {
        let HerdSeen {
            now,
            last,
            missing,
            died,
        } = self;
        died.clear();
        for &(id, at) in last.iter() {
            if !now.iter().any(|&(n, _)| n == id) {
                missing.push((id, at, 0));
            }
        }
        missing.retain_mut(|(id, at, frames)| {
            if now.iter().any(|&(n, _)| n == *id) {
                return false;
            }
            *frames += 1;
            if *frames < 2 {
                return true;
            }
            if at.distance(ear) < DEATH_HEARD_M {
                died.push((*id, *at));
            }
            false
        });
        if died.len() > 2 {
            died.clear();
        }
        std::mem::swap(now, last);
        died.drain(..)
    }
}

/// Health, as a change rather than as an event.
///
/// `EV_HEALTH` is absolute and own-fact (`sim_core::world`), so a *fall* in
/// `core.hp` is what every damage route in the sim has in common — the four
/// that announce and the three `damage_routes.rs` marks silent alike. Tracked
/// here rather than in the core because it is a presentation fact: the core is
/// right to publish the value and wrong to publish a delta nobody but the HUD
/// and this file wants.
#[derive(Resource, Default)]
pub struct LastHp(pub u16);

/// Being hurt, as a **blow** rather than as a health bar.
///
/// **No pain grunt, as Rust has none** (operator, 2026-10-17: *"get rid of any
/// sound we make when we take damage rust doesnt do that"*): an announced
/// blow is the flesh hit, one that crossed your head is Rust's headshot
/// crunch, and taking damage
/// still tenses the score. [`crate::sound::hurt::request`] is the decision that it did,
/// the fall and the announced blows alike, for the reason that module gives.
/// `Feed` is taken immutably, which is the shape `CLAUDE.md`'s two-drains
/// trap requires of every reader that is not `feed::drain` itself;
/// scheduling puts this after it.
pub fn hurt(
    net: NonSend<Net>,
    feed: Res<super::feed::Feed>,
    mut last: ResMut<LastHp>,
    mut sound: ResMut<Sound>,
) {
    let core = &net.session.core;
    // A rise (heal, respawn) and the first message of all are both silent.
    // `last.0 == 0` is "we have never seen one", which a fresh world is.
    let fall = if last.0 > 0 {
        last.0.saturating_sub(core.hp)
    } else {
        0
    };
    last.0 = core.hp;
    if feed.hurt_head {
        sound.play(Request::own(Cue::Headshot));
    }
    if crate::sound::hurt::request(fall, feed.hurt_damage, feed.hurts, core.hp_max).is_some() {
        // A blow that landed is a wet hit on your own body, as Rust's is
        // (operator, 2026-10-19: a cactus or a spike trap should sound like
        // one): the flesh hit, not a grunt. Announced blows only, so hunger
        // and thirst stay quiet, and not under the headshot's crunch.
        if feed.hurts > 0 && !feed.hurt_head {
            let g = crate::sound::hurt::weight(feed.hurt_damage, core.hp_max);
            sound.play(Request::own(Cue::FleshHit).with_gain(g));
        }
        // **The biggest bump, and theirs is too** (`reference/AUDIO.md` §8's
        // published order: a weapon in play < a bullet past your head <
        // taking damage). Two of these inside two sections is what puts the
        // score in its top tier. It rides the same decision as the cue, so a
        // blow armor ate whole now tenses the music too — it is the shooting
        // that matters to the score, not the bookkeeping.
        sound.music.bump(music::BUMP_HURT);
    }
}

/// The ambience bed's level: quiet in the open, louder under trees.
///
/// This is the reference's **localized ambience** at the smallest size it can
/// honestly be (`reference/AUDIO.md` §3): there, a set of emitters is culled
/// against the listener and crossfaded by distance, and the performance note
/// in its own devblog is that the culling was updating too often. Ours is one
/// looping voice whose gain reads how many scatter props are drawn nearby —
/// so the bed already answers to the world rather than being a constant, and
/// it costs one query length per frame instead of an emitter set.
/// The forest layer rides along: [`bed`] already walks the props and already
/// scores the cover, so a bird costs a countdown and an index. See
/// `sound::birds` for why a layer is not a bed turned down.
// The mix state to write, where the ears are, which island this is, the
// cover query, the clock, the player's sliders, the tick, the day pin, the
// weather, the engine the levels leave through, the heli, the recyclers
// with the sim's word on which are running, and the furnaces.
#[allow(clippy::too_many_arguments)]
pub fn bed(
    mut sound: ResMut<Sound>,
    eye: Res<Eye>,
    world: Res<super::WorldId>,
    props: Query<(&GlobalTransform, &super::props::Fellable)>,
    time: Res<Time>,
    settings: Res<super::Settings>,
    feed: Res<super::feed::Feed>,
    pin: Res<super::rig::DayPin>,
    mut weather: ResMut<super::weather::WeatherNow>,
    mut engine: ResMut<Engine>,
    helis: Query<&GlobalTransform, With<super::heli::HeliBody>>,
    net: NonSend<Net>,
    recyclers: Query<(&GlobalTransform, &super::structures::RecyclerHum)>,
    fires: Query<(&GlobalTransform, &super::structures::FireLight)>,
) {
    // How much cover is within earshot, 0..1. `COVER_FULL` scatter slots
    // inside the radius is "in the woods"; none is "on the beach".
    //
    // **It counts every gatherable slot, not only trees** — a boulder field
    // reads as cover here and a pine wood reads the same. That is a
    // simplification and not a claim: a bed that told rock from canopy is the
    // localized-emitter slice (`reference/AUDIO.md` §9.3), not this one.
    //
    // **What it must count is SLOTS, and a slot is more than one entity.** A
    // tree is four `Fellable`s (trunk, canopy, stump, far hull) and every
    // other slot is one, so counting entities would score a pine wood 4× a
    // boulder field of the same density — and `COVER_FULL` was calibrated
    // when a tree was two.
    // Counting the parts that are one-per-slot fixes the units. This is the
    // second thing felling v0 found by adding a part: `Fellable` is not a
    // slot, it is a piece of one.
    //
    // **So `COVER_FULL` moves 14 → 7, and that is a unit conversion rather
    // than a retune.** It was tuned by ear in a pine wood, where every slot
    // counted twice, so 14 entities WAS 7 trees. Leaving it at 14 in the
    // corrected unit would silently double how much forest the bed needs and
    // make the woods quieter — a tuning change nobody chose, arriving as a
    // side effect of a bug fix, which is the worst way for one to arrive.
    const FOREST_R2: f32 = 22.0 * 22.0;
    const COVER_FULL: f32 = 7.0;
    // The two halves of "is this a tree near me", as closures, because the
    // bird layer below walks the same set for a perch and an index into a
    // set that was filtered differently is an index into nothing.
    let is_perch = |f: &super::props::Fellable| {
        matches!(
            f.part,
            super::props::FellPart::Trunk | super::props::FellPart::Vanish
        )
    };
    let near_eye = |p: Vec3| p.distance_squared(eye.pos) < FOREST_R2;
    let near = props
        .iter()
        .filter(|(t, f)| is_perch(f) && near_eye(t.translation()))
        .count();
    let cover = (near as f32 / COVER_FULL).clamp(0.0, 1.0);

    // The forest layer. A second walk of the same query, and it runs on the
    // one frame in a few hundred that a call actually lands — a buffer of
    // candidate perches would cost every frame to save that one, and would
    // cap how much forest the layer can see for nothing.
    //
    // Daylight only, now that a day exists (day/night v0): birds roost at
    // night, and the cause is the server's own clock rather than one this
    // layer invented — the refusal `birds.rs`' header recorded is repaid.
    // Crickets are the night companion: a bed (`Cue::BedNight`), below.
    //
    // Through `world::is_night` rather than the open-coded comparison this
    // used to carry: the sim reads the same boundary now (a predator's
    // notice radius is the hour's), and two hand-written thresholds against
    // one constant is how the birds and the wolves come to disagree about
    // when dusk was.
    //
    // The tick comes through `DayPin` for the same reason one level up: on a
    // `--capture` run the hour is pinned, and reading the raw estimate here
    // would roost the birds at the box's hour while the sun stood at noon.
    let is_day = !sim_core::world::is_night(pin.day_tick(feed.server_tick_est, &feed.env));
    // …and not in the rain (weather v0): birds shelter.
    let singing = is_day && weather.rain < 0.15;
    if singing && sound.birds.due(cover, time.delta_secs()) && near > 0 {
        let want = sound.birds.perch(near);
        if let Some(p) = props
            .iter()
            .filter(|(t, f)| is_perch(f) && near_eye(t.translation()))
            .map(|(t, _)| t.translation())
            .nth(want)
        {
            sound.play(Request::at(Cue::Bird, [p.x, p.y + birds::PERCH_H_M, p.z]));
        }
    }
    // Open ground is windier than the inside of a forest, but a forest is not
    // silent — it is the same wind in the canopy. So the bed never drops
    // below half, and cover moves it rather than gating it.
    // The weather's wind rides on top (weather v0): a breeze at clear, a
    // gale in a storm.
    // Under a roof the gale is outside: its share of the bed drops.
    let gale = weather.wind * if weather.sheltered { 0.35 } else { 1.0 };
    sound.bed_target[0] = (1.0 - 0.45 * cover) * (0.6 + 0.8 * gale);
    // The surf reads how much sea is within earshot, from the same
    // `terrain::height` the water is drawn from — 24 taps, a fixed pattern, so
    // the level cannot flicker as a search finds different water. A storm's
    // sea is louder than a breeze's, by the roughness the drawn swell is
    // raised by (`water::sea_state`).
    sound.bed_target[1] = crate::sound::water::surf_gain(crate::sound::water::shore_exposure(
        world.seed, eye.pos.x, eye.pos.z,
    )) * (1.0 + 0.6 * super::water::sea_state(weather.wind));
    // The submerged bed has no world level of its own: it is entirely the
    // snapshot's, which is the point of a snapshot.
    sound.bed_target[2] = 1.0;
    // The rain (weather v0): as hard as it falls, duller under a roof. The
    // snapshot takes it away underwater. Snow falls near silent, so the bed
    // fades as the rain turns to it (snow v0).
    sound.bed_target[3] = weather.falling_rain() * if weather.sheltered { 0.35 } else { 1.0 };
    // The heli's rotor, by distance to the drawn hull — none drawn, none
    // heard. Its side and its distance's muffling are its place, sent
    // below ([`rotor_place`]).
    let (near, far) = helis.single().map_or((0.0, 0.0), |gt| {
        rotor_levels(gt.translation().distance(eye.pos))
    });
    sound.bed_target[ROTOR_SLOT as usize] = near;
    sound.bed_target[ROTOR_FAR_SLOT as usize] = far;
    // The motor loop (`Cue::BedTown`) at the nearest RUNNING recycler, by
    // distance, and silent when none runs. It was THE GATE's yard bed, and
    // a yard that always droned read as recyclers nobody was using.
    sound.bed_target[5] = {
        let ovens = net.session.core.ovens();
        let d = recyclers
            .iter()
            .filter(|(_, r)| ovens.is_lit(r.cx, r.cz, r.level, r.loc))
            .map(|(g, _)| g.translation().distance(eye.pos))
            .fold(f32::INFINITY, f32::min);
        let near = (1.0 - d / RECYCLER_HUM_R_M).clamp(0.0, 1.0);
        near * near
    };
    // The furnace's roar at the nearest LIT furnace, by distance, and from
    // its side (placed below). Silent when none burns near.
    let furnace = {
        let ovens = net.session.core.ovens();
        fires
            .iter()
            .filter(|(_, f)| f.furnace && ovens.is_lit(f.cx, f.cz, f.level, f.loc))
            .map(|(g, _)| g.translation())
            .min_by(|a, b| {
                a.distance_squared(eye.pos)
                    .total_cmp(&b.distance_squared(eye.pos))
            })
    };
    sound.bed_target[FURNACE_SLOT as usize] = furnace.map_or(0.0, |p| {
        let near = (1.0 - p.distance(eye.pos) / FURNACE_ROAR_R_M).clamp(0.0, 1.0);
        near * near
    });
    // The night's crickets, the birds' other half: up as the light goes,
    // quiet in the rain, duller under a roof.
    sound.bed_target[6] = weather.night
        * (1.0 - 3.0 * weather.rain).max(0.0)
        * if weather.sheltered { 0.5 } else { 1.0 };

    // Thunder: each bolt's clap once its sound has crossed the distance
    // (`weather::update` queued it at the bolt's own time plus d / 343).
    let now_s = feed.server_tick_est / sim_core::limits::TICK_HZ as f64;
    while let Some(gain) = weather.take_thunder(now_s) {
        sound.play(Request::own(Cue::Thunder).with_gain(gain));
    }

    for i in 0..BEDS.len() {
        // A machine's bed is a source you walk up to, so its level keeps up
        // with your feet; the world's beds drift.
        let rate = if i == 5 || i == FURNACE_SLOT as usize {
            SOURCE_FADE_PER_S
        } else {
            BED_FADE_PER_S
        };
        let d = rate * time.delta_secs();
        let (g, t) = (sound.bed_gain[i], sound.bed_target[i]);
        sound.bed_gain[i] = if g < t {
            (g + d).min(t)
        } else {
            (g - d).max(t)
        };
    }

    // The level is a TARGET the renderer ramps to across its next block;
    // the fade above is the law and it stays here. Sent only when it moved:
    // a standing player sends nothing, and the buffer's cap is sized on
    // that.
    let snap = sound.snap;
    let mix = mix_of(&settings).under(&snap);
    for (i, cue) in BEDS.iter().enumerate() {
        let def = cue.def();
        let level = sound.bed_gain[i]
            * snap.bed(*cue)
            * def.gain
            * mix.bus_gain(def.bus)
            * sound.mixer.duck();
        if level != sound.bed_sent[i] {
            sound.bed_sent[i] = level;
            engine.push(Cmd::Gain {
                slot: i as u8,
                gain: level,
            });
        }
    }

    // The roar from the furnace's side. With none lit in earshot, its level
    // is fading to nothing and its last place holds.
    if let Some(p) = furnace.filter(|p| p.distance(eye.pos) < FURNACE_ROAR_R_M) {
        let d = p - eye.pos;
        let right = crate::look::right_dir(crate::look::yaw_u16(eye.yaw));
        let place = furnace_place([d.x, d.y, d.z], [right.0, right.1]);
        if sound.furnace_sent != Some(place) {
            sound.furnace_sent = Some(place);
            engine.push(place);
        }
    }

    // The rotor from where the hull is. With no hull drawn its level is
    // fading to nothing and its last place holds.
    if let Ok(gt) = helis.single() {
        let d = gt.translation() - eye.pos;
        let dist = d.length();
        let dt = time.delta_secs();
        if let (Some(last), true) = (sound.rotor_dist, dt > 0.0) {
            let k = (dt / ROTOR_DOPPLER_SMOOTH_S).min(1.0);
            sound.rotor_receding += ((dist - last) / dt - sound.rotor_receding) * k;
        }
        sound.rotor_dist = Some(dist);
        let (x, z) = crate::look::right_dir(crate::look::yaw_u16(eye.yaw));
        let places = rotor_place(
            [d.x, d.y, d.z],
            [x, z],
            sound.rotor_receding,
            engine.out_rate,
        );
        for (sent, place) in sound.rotor_sent.iter_mut().zip(places) {
            if *sent != Some(place) {
                *sent = Some(place);
                engine.push(place);
            }
        }
    } else {
        sound.rotor_dist = None;
        sound.rotor_receding = 0.0;
    }
}

/// The waterline: which snapshot the mix is in, and the splash on crossing it.
///
/// **Runs before [`bed`] and [`pump`]**, because both read the snapshot this
/// resolves and a mix state one frame stale is a mix that comes up as your
/// head goes back under.
pub fn water(
    net: NonSend<Net>,
    eye: Res<Eye>,
    time: Res<Time>,
    mut sound: ResMut<Sound>,
    fx: Option<ResMut<super::fx::Fx>>,
) {
    let dt = time.delta_secs();
    // The EARS, not the feet: the mix changes when your head goes under, and a
    // player wading chest-deep is still hearing the world above.
    let want = if crate::sound::water::submerged(eye.pos.y) {
        Snapshot::Submerged
    } else {
        Snapshot::Above
    };
    sound.snap = sound.snapshots.tick(want, dt);

    // The feet, for the splash: breaking the surface is your body entering the
    // water, which happens well before your head does.
    let at = net.session.core.predict.render_position();
    if let Some(gain) = sound.waterline.sample(at[1], dt) {
        sound.play(Request::own(Cue::Splash).with_gain(gain));
        if let Some(mut fx) = fx {
            super::fx::world::splash(&mut fx, Vec3::from(at), gain);
        }
    }
}

/// The score: tick the director, start the track it asks for, and hold the
/// music output at the player's music level.
///
/// **Runs everywhere, unlike every other system in this file.** There is no
/// `world_running`, no `Net` and no `Eye` in its arguments, because music
/// plays on the menus too — `sound::music::Mode::Menu` is what makes that a
/// different behaviour rather than a different code path.
///
/// One track sounds at a time. A track asked for while another is still
/// audible (the menu opening over a world song) waits for that one to fade
/// out over [`MUSIC_FADE_S`]; otherwise it starts the frame it is asked for.
pub fn music(
    mut sound: ResMut<Sound>,
    mut engine: ResMut<Engine>,
    time: Res<Time>,
    settings: Res<super::Settings>,
) {
    let dt = time.delta_secs();
    let sound = &mut *sound;
    let score = &mut engine.score;
    score.start = None;
    score.stop = false;
    if let Some((track, at)) = score.adopt.take() {
        sound.music.adopt(track, at);
        sound.sounding = Sounding {
            track: Some(track),
            left_s: (track.len_s() - at).max(0.0),
            fade: 1.0,
            next: None,
        };
    }

    let now = &mut sound.sounding;
    if now.track.is_some() {
        now.left_s -= dt;
        if now.left_s <= 0.0 && now.next.is_none() {
            now.track = None;
        }
    }
    if let Some(track) = sound.music.tick(dt) {
        if now.track.is_some() {
            now.next = Some(track);
        } else {
            *now = Sounding {
                track: Some(track),
                left_s: track.len_s(),
                fade: 1.0,
                next: None,
            };
            score.start = Some(track);
        }
    }
    if let Some(next) = now.next {
        now.fade -= if MUSIC_FADE_S > 0.0 {
            dt / MUSIC_FADE_S
        } else {
            1.0
        };
        if now.fade <= 0.0 {
            score.stop = true;
            *now = Sounding {
                track: Some(next),
                left_s: next.len_s(),
                fade: 1.0,
                next: None,
            };
            score.start = Some(next);
        }
    }

    // The music sits under the same duck the beds do (`Mixer::duck`): a
    // gunshot or a blast pulls it down with them.
    let fade = if now.next.is_some() {
        fade_curve(1.0 - now.fade)
    } else {
        1.0
    };
    score.gain = mix_of(&settings).bus_gain(crate::sound::Bus::Music) * sound.mixer.duck() * fade;
}

/// Put the director on a new screen's schedule.
///
/// Called on entering the menu and on entering a world, which are the only
/// two transitions music has. The intro carries across both: it loops on the
/// menu and plays on into a world joined from it; a world song is replaced
/// by the intro when the menu opens (`Director::reset`, then [`music`]'s
/// fade).
pub fn music_mode(mode: music::Mode) -> impl Fn(ResMut<Sound>) {
    move |mut sound: ResMut<Sound>| {
        sound.music.reset(mode);
    }
}

/// Resolve the frame and start what the mixer chose.
///
/// The voice count is the ledger's (`engine::Live`) first: the game thread
/// started every voice, it knows each cue's length and rate, and the
/// renderer's end law is arithmetic. It is **stale by design** — ticked
/// before the mixer sees it, the mixer's own starts landing after, and each
/// `Start` reaching the renderer only at the next device callback, one
/// buffer period later — so the renderer can hold a voice the ledger has
/// already retired. The pool tolerates `engine::VOICE_SLACK_FRAMES` frames
/// of that (`engine::VOICE_SLOTS`), and the audio thread's own last count
/// (`Diag::live_thread`) is taken as a FLOOR under the ledger's, so the
/// mixer's room shrinks when the renderer is genuinely fuller than
/// predicted. Past the slack the renderer refuses rather than steals, and
/// the refusal is printed below.
// Seven: the mixer, the engine, the bank's lengths, the listener, the clock,
// the mix, and the counters' last report. Every one is a distinct source
// this frame reads.
#[allow(clippy::too_many_arguments)]
pub fn pump(
    mut sound: ResMut<Sound>,
    mut engine: ResMut<Engine>,
    bank: Res<Bank>,
    eye: Res<Eye>,
    time: Res<Time>,
    settings: Res<super::Settings>,
    mut reported: Local<Reported>,
    net: Option<NonSend<Net>>,
    world: Option<Res<super::WorldId>>,
) {
    let mix = mix_of(&settings).under(&sound.snap);
    let dt = time.delta_secs();
    engine.live.tick(dt);
    let live = engine.live.count().max(engine.diag.live_thread as usize);
    // The renderer's rate: the device's once `audio_out::open` has written
    // it, the bank's until then.
    let out_rate = engine.out_rate;
    let listener = [eye.pos.x, eye.pos.y, eye.pos.z];
    // The ears' right, in world XZ, off the client's one yaw convention
    // (`look::right_dir`, gated against Bevy's own `Transform::right()` in
    // `tests/look.rs`) — the engine takes the vector and never a yaw.
    let right = {
        let (x, z) = crate::look::right_dir(crate::look::yaw_u16(eye.yaw));
        [x, z]
    };
    let dt_ms = dt * 1000.0;
    // Copied out because `starts` borrows the mixer and the ledger is on
    // another resource — at most `STARTS_PER_FRAME` of them, on the stack,
    // no allocation.
    let mut chosen = [None::<Start>; crate::sound::STARTS_PER_FRAME];
    // Ears under water hear every one-shot through the water's low-pass.
    let under = engine::lp_under(sound.snapshots.t(), out_rate);
    sound.mixer.set_live_of(engine.live.counts());
    {
        let starts = sound.mixer.tick(dt_ms, listener, live, &mix);
        for (slot, s) in chosen.iter_mut().zip(starts.iter()) {
            *slot = Some(*s);
        }
    }
    // Walls between the ears and a source (`ui::occlusion`): the built
    // pieces this client already collides against, walked from the ear.
    let walls = match (settings.occlusion, net.as_deref(), world.as_deref()) {
        (true, Some(n), Some(w)) => Some((w.seed, &w.haven, n.session.core.pieces.cols())),
        _ => None,
    };
    for mut start in chosen.into_iter().flatten() {
        let mut walled = 0;
        if let (Some((seed, haven, cols)), Some(at)) = (walls, start.at) {
            let n = crate::ui::occlusion::walls_between(seed, haven, cols, listener, at);
            if let Some((gain, hz)) = crate::ui::occlusion::muffle(n) {
                start.gain *= gain;
                walled = engine::lp_hz(hz, out_rate);
            }
        }
        // The rate is the mixer's speed resampled to the renderer's rate —
        // `engine::rate` is the one resampler in the chain (`audio_out.rs`).
        // The take is the bank's to count and never the last one played.
        let takes = bank.takes(start.cue);
        let take = sound.takes.pick(start.cue, takes);
        let rear = start
            .at
            .map_or(0, |at| rear_lp(listener, at, right, out_rate));
        engine.push(
            engine::start_cmd(&start, listener, right, out_rate)
                .with_take(take, takes)
                .muffled(under)
                .muffled(walled)
                .muffled(rear),
        );
        // The ledger counts what the renderer will actually sound: a cue
        // that has not landed (a spread bank's first frames) is silence
        // there, counted as `unbanked`, and must not hold a ledger slot for
        // its length. The ledger's own refusal (`false`) is the renderer's
        // refusal one frame early and needs no second count.
        if bank.installed(start.cue) {
            engine
                .live
                .start_cue(start.cue, bank.len_s(start.cue), start.speed);
        }
    }
    // A dropped request is a caller over `CUE_QUEUE_CAP`, which is a bug in
    // the caller rather than a load condition (`sound::CUE_QUEUE_CAP`). Said
    // once per new drop and never asserted: the counter is cumulative, so a
    // `debug_assert` would turn one legitimately busy frame — a mass respawn
    // felling thirty slots at once — into a panic on every frame after it.
    if sound.mixer.dropped > reported.mixer {
        reported.mixer = sound.mixer.dropped;
        warn!(
            "sound: {} cue requests dropped since start - a caller is over CUE_QUEUE_CAP",
            sound.mixer.dropped
        );
    }
    // The seam's counters, the same way — once per new increment — and
    // **gated on the audio thread being alive.** On a box with no device
    // (every capture run, CI) there is no audio thread: the flush drops and
    // counts, and one line below says so rather than a warning a frame.
    let diag = engine.diag;
    if diag.alive {
        if engine.dropped > reported.frame {
            reported.frame = engine.dropped;
            warn!(
                "audio: {} commands dropped at the frame buffer since start - a frame is over CMD_FRAME_CAP",
                engine.dropped
            );
        }
        if diag.ring_dropped > reported.ring {
            reported.ring = diag.ring_dropped;
            warn!(
                "audio: {} commands dropped at the native ring since start - the audio thread is behind",
                diag.ring_dropped
            );
        }
        if diag.stats.refused > reported.refused {
            reported.refused = diag.stats.refused;
            warn!(
                "audio: {} starts refused by the renderer since start - the pool is fuller than \
                 the ledger and its VOICE_SLACK_FRAMES of callback latency allow for",
                diag.stats.refused
            );
        }
        if engine.install_refused + diag.install_refused > reported.install {
            reported.install = engine.install_refused + diag.install_refused;
            warn!(
                "audio: {} installs refused at the rings since start - a frame handed over more \
                 than a bank",
                reported.install
            );
        }
        if diag.stats.reinstall_refused > reported.reinstall {
            reported.reinstall = diag.stats.reinstall_refused;
            warn!(
                "audio: {} installs refused by the renderer since start - a cue was handed over \
                 twice, and the second copy came back to be dropped here",
                diag.stats.reinstall_refused
            );
        }
        if diag.stats.return_dropped > reported.returns {
            reported.returns = diag.stats.return_dropped;
            warn!(
                "audio: {} refused installs freed on the audio thread since start - the return \
                 ring was full, so the flush is not reclaiming",
                diag.stats.return_dropped
            );
        }
        if diag.stats.cut > reported.cut {
            reported.cut = diag.stats.cut;
            warn!(
                "audio: {} plays or loops landed on a busy held slot since start - the game \
                 thread's slot round-robin is wrong",
                diag.stats.cut
            );
        }
        if diag.stats.unbanked > reported.unbanked {
            reported.unbanked = diag.stats.unbanked;
            warn!(
                "audio: {} starts or plays landed before their cue's samples since start - \
                 silence, counted (a spread bank's first frames, or a start that raced its \
                 install across the seam)",
                diag.stats.unbanked
            );
        }
        if diag.stats.bad_cmd > reported.bad_cmd {
            reported.bad_cmd = diag.stats.bad_cmd;
            warn!(
                "audio: {} commands refused by the renderer as malformed since start - a gain \
                 or a rate outside its band, or a slot past HELD",
                diag.stats.bad_cmd
            );
        }
    } else if !diag.routed && !reported.said_none && diag.unrouted > 0 {
        reported.said_none = true;
        info!(
            "audio output: none - {} commands dropped at the flush since start",
            diag.unrouted
        );
    }
    // Not gated on `alive`: a device that went away is exactly the case
    // where the callback has stopped.
    if diag.stream_errors > reported.stream_errors {
        reported.stream_errors = diag.stream_errors;
        warn!(
            "audio: {} errors on the device stream since start - the device went away or the \
             backend faulted",
            diag.stream_errors
        );
    }
}

/// [`pump`]'s memory of what it has already said, so each counter's new
/// increment is printed once.
#[derive(Default)]
pub struct Reported {
    mixer: u32,
    frame: u32,
    ring: u32,
    refused: u32,
    install: u32,
    reinstall: u32,
    returns: u32,
    cut: u32,
    unbanked: u32,
    bad_cmd: u32,
    stream_errors: u32,
    said_none: bool,
}

/// The settings screen's sliders as a [`Mix`].
fn mix_of(s: &super::Settings) -> Mix {
    Mix {
        master: s.master(),
        game: s.vol_game,
        ambience: s.vol_ambience,
        music: s.vol_music,
        steps: s.vol_steps,
        interface: s.vol_interface,
    }
}

/// The cutoff a sound straight behind the head is heard through, Hz. Stereo
/// pans left and right and cannot say front from back; the top end can, as
/// the ear's own shadow does, so a step behind you is a duller step.
pub const REAR_HZ: f32 = 5_000.0;
/// Where the rear shadow starts, Hz: a sound beside you is unfiltered.
pub const REAR_OPEN_HZ: f32 = 18_000.0;

/// The low-pass byte for a source at `at` heard by ears at `listener` whose
/// right is `right` (world XZ): 0 in front and beside, darker the further
/// behind, [`REAR_HZ`] dead behind. Nothing for a source at the ears.
pub fn rear_lp(listener: [f32; 3], at: [f32; 3], right: [f32; 2], out_rate: u32) -> u8 {
    let (dx, dz) = (at[0] - listener[0], at[2] - listener[2]);
    let len = (dx * dx + dz * dz).sqrt();
    if len < 1.0 {
        return 0;
    }
    // Forward is the right turned a quarter (`look::right_dir`'s inverse).
    let (fx, fz) = (right[1], -right[0]);
    let ahead = (dx * fx + dz * fz) / len;
    // Beside the head is open: a rounding hair behind it is not a shadow.
    if ahead >= -0.05 {
        return 0;
    }
    engine::lp_hz(
        REAR_OPEN_HZ * (REAR_HZ / REAR_OPEN_HZ).powf(-ahead),
        out_rate,
    )
}

/// The frame budget must not exceed the pool it draws from.
const _: () = assert!(crate::sound::STARTS_PER_FRAME <= VOICE_CAP);

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(seen: &mut HerdSeen, herd: &[(u32, f32)]) -> Vec<u32> {
        seen.now.clear();
        seen.now
            .extend(herd.iter().map(|&(id, x)| (id, Vec3::new(x, 0.0, 0.0))));
        seen.deaths(Vec3::ZERO).map(|(id, _)| id).collect()
    }

    #[test]
    fn an_animal_gone_near_has_died_and_one_gone_far_has_left() {
        let mut seen = HerdSeen::default();
        assert!(frame(&mut seen, &[(1, 10.0), (2, 150.0), (3, 20.0)]).is_empty());
        // Gone one frame is not yet a death: a model swap respawns one.
        assert!(frame(&mut seen, &[]).is_empty());
        // 3 is back, so it never died; 1 is still gone and near, 2 far.
        assert_eq!(frame(&mut seen, &[(3, 20.0)]), vec![1]);
        assert!(
            frame(&mut seen, &[(3, 20.0)]).is_empty(),
            "a death is cried once"
        );
    }

    #[test]
    fn a_herd_lost_at_once_is_a_teleport_not_a_slaughter() {
        let mut seen = HerdSeen::default();
        frame(&mut seen, &[(1, 5.0), (2, 6.0), (3, 7.0)]);
        frame(&mut seen, &[]);
        assert!(frame(&mut seen, &[]).is_empty());
    }
}
