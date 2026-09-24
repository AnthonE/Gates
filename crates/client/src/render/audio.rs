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
//!   owns time.** A bed's fade, a snapshot crossfade and a music orphan's
//!   [`MUSIC_FADE_S`] are computed here per frame and sent as gain TARGETS
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
use crate::sound::engine::{self, Cmd, Live, Stats, HELD_BEDS, HELD_MUSIC};
use crate::sound::mixer::{Mixer, Request, Start, Takes};
use crate::sound::music::{self, Director};
use crate::sound::steps::Steps;
use crate::sound::voice::Voices;
use crate::sound::water::Waterline;
use crate::sound::{Cue, Mix, Snapshot, SnapshotDef, Snapshots, CUE_COUNT, SAMPLE_RATE, VOICE_CAP};

use super::{Eye, Net};

/// Commands a frame may hand the renderer. Overflow policy: **drop the newest
/// and count it** ([`Engine::dropped`]) — the mixer's rule one queue up, and
/// for its reason: a cue's value is that it happened, so the earlier asks
/// win. A frame's worst case is arithmetic and well under it — three bed
/// gains, `STARTS_PER_FRAME` starts, one music play and four music gains or
/// stops, plus three loops or three stops and a cut on a screen transition
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

/// The looping beds, in the order [`Sound::bed_gain`] indexes them.
pub const BEDS: [Cue; 4] = [Cue::BedWind, Cue::BedSurf, Cue::BedUnder, Cue::BedRain];

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
        }
    }
}

impl Engine {
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
    /// The waterline, as a thing the local body crosses.
    pub waterline: Waterline,
    /// Each roster slot's voice clock (`sound::voice` — pure; [`voices`] is
    /// the producer that reads it against the drawn herd).
    pub voices: Voices,
    /// When a song plays and which piece it is (`sound::music`). Lives on
    /// the resource rather than on a world entity because it runs on the
    /// menus too, where there is no world.
    pub music: Director,
    /// The forest layer's clock (`sound::birds`), driven by [`bed`], which
    /// already has the cover score and the props to perch on.
    pub birds: Birds,
    /// Each bed's current gain, moving toward its target at
    /// [`BED_FADE_PER_S`]. Held rather than recomputed so the crossfade is
    /// state, not a function of a frame.
    bed_gain: [f32; BEDS.len()],
    bed_target: [f32; BEDS.len()],
    /// The level each bed was last SENT. A standing player sends nothing:
    /// the renderer holds a target until it is given another, so a gain
    /// equal to the last one is a command with no content.
    bed_sent: [f32; BEDS.len()],
    /// The snapshot crossfade, and this frame's resolved mixer state.
    snapshots: Snapshots,
    snap: SnapshotDef,
    /// The pieces sounding, one per held music slot (`engine::HELD_MUSIC`),
    /// and the round-robin cursor: with four slots the one a new piece
    /// overwrites has always ended (two overlap by the tail, a transition
    /// adds one orphan, one spare), and if it has not the renderer counts
    /// the cut (`Diag::stats.cut`) rather than this file assuming.
    music_slots: [MusicSlot; HELD_MUSIC],
    next_music: usize,
}

impl Sound {
    /// Ask for a cue. The one door — see [`Sound`].
    pub fn play(&mut self, req: Request) {
        self.mixer.push(req);
    }
}

/// A piece of music on one of the engine's held music slots.
///
/// **Deliberately a held slot and not a one-shot voice, and deliberately
/// untouched by [`teardown`]**, and both are load-bearing:
///
/// - A held slot, so music does not count against `VOICE_CAP` and cannot
///   be the reason an axe was refused (`sound::mixer` refuses to start a
///   music cue at all — `Cue::is_music`).
/// - Not cut on leaving a world (`teardown` stops the beds and the voices
///   and leaves the music slots alone), so a piece is not cut off
///   mid-phrase. A menu piece rings out over the loading screen, which is
///   how music is supposed to carry a transition.
#[derive(Clone, Copy, Debug)]
struct MusicSlot {
    /// Which piece this is, so its level comes from the cue table like every
    /// other level in the client rather than from a constant here.
    cue: Cue,
    /// The piece's own gain, 1 while it is playing normally and ramping to
    /// zero once [`ending`](Self::ending) is set.
    fade: f32,
    /// Set when a screen change orphaned this piece: the director under it
    /// has been reset, and something else is about to start on top of it.
    ending: bool,
    /// Seconds until the renderer frees the slot on its own — the bank's
    /// length, counted down here so the slot is released without reading
    /// the audio thread back.
    left_s: f32,
    /// The level last sent, so a steady piece sends nothing.
    sent: f32,
    active: bool,
}

impl Default for MusicSlot {
    fn default() -> Self {
        Self {
            cue: Cue::ALL[0],
            fade: 1.0,
            ending: false,
            left_s: 0.0,
            sent: 0.0,
            active: false,
        }
    }
}

/// How long an orphaned piece takes to fade out, seconds.
///
/// **The one place music fades, and the reference's rule is not being
/// broken.** `reference/AUDIO.md` §8 says pieces cut to each other *without*
/// fading, and they still do — the tail covers that join. This is the other
/// case: leaving a world starts the menu's music immediately, and two pieces
/// playing over each other is not a join, it is two songs. Short enough not
/// to be a swell, long enough not to click (`DECISIONS.md` §open,
/// "music v0").
pub const MUSIC_FADE_S: f32 = 1.2;

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
}

/// Reset what the world owned: stop the beds, cut every one-shot, and reset
/// the odometer — which is in a resource that outlives the world, and a
/// stale one measures the distance between two worlds as ground covered and
/// fires a burst of footsteps on the first frame of the next.
///
/// Music is not touched: a piece rings out over the transition
/// ([`MusicSlot`]), and [`music_mode`] decides whether it fades.
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
) {
    let body = &net.session.core.predict.body;
    let pos = net.session.core.predict.render_position();
    let Some(step) = sound.steps.sample(pos, body.grounded, time.delta_secs()) else {
        return;
    };
    // The same `splat` the ground under the player is DRAWN with, so the
    // sound cannot disagree with the picture — see `sound::steps`.
    let splat = sim_core::terrain::splat(world.seed, pos[0], pos[2]);
    let below_sea = pos[1] < sim_core::terrain::SEA_LEVEL;
    let cue = crate::sound::steps::surface_cue(splat, below_sea);
    sound.play(Request::own(cue).with_gain(step.gain));
    if let Some(mut fx) = fx {
        super::fx::world::footstep(&mut fx, cue, Vec3::from(pos), step.gain);
    }
}

/// One remote body's step odometer — the same `sound::steps::Steps` the
/// local player runs, one per drawn body, carried as a component so it dies
/// with the entity and a body that leaves AOI and returns starts fresh (no
/// map to sweep, no reset to remember). `bodies::stream` inserts it at
/// spawn.
#[derive(Component, Default)]
pub struct RemoteSteps(pub Steps);

/// Another player's footsteps — the sound that decides fights, and until
/// this system nothing produced it: only the local body has a predictor,
/// so a remote's cadence comes off the same distance-integrated odometer
/// fed the INTERPOLATED transform `bodies::stream` just wrote (this runs
/// after `Stream`). The surface is `terrain::splat` at THEIR position, the
/// cue is the local family's positional twin (`steps::remote`), and "only
/// nearby" is the mixer's own falloff at the cue's radius — the falling
/// tree's pattern: push with a position, let the one distance law cull.
///
/// Two honest gaps, both the wire's: there is no grounded bit (`NOW.md`
/// §0v item 1), so a jumping remote ticks the odometer by the horizontal
/// half of its arc; and a teleport (death, respawn) reads as ground
/// covered, which the odometer's own hitch cap bounds at ONE step. A
/// sleeper stands still and the speed floor keeps it silent for free.
pub fn remote_steps(
    world: Res<super::WorldId>,
    time: Res<Time>,
    mut bodies: Query<(&Transform, &mut RemoteSteps), With<super::bodies::Body>>,
    mut sound: ResMut<Sound>,
    eye: Res<Eye>,
    mut fx: Option<ResMut<super::fx::Fx>>,
) {
    let dt = time.delta_secs();
    for (t, mut steps) in bodies.iter_mut() {
        let pos = [t.translation.x, t.translation.y, t.translation.z];
        let Some(step) = steps.0.sample(pos, true, dt) else {
            continue;
        };
        let splat = sim_core::terrain::splat(world.seed, pos[0], pos[2]);
        let below_sea = pos[1] < sim_core::terrain::SEA_LEVEL;
        let cue = crate::sound::steps::remote(crate::sound::steps::surface_cue(splat, below_sea));
        sound.play(Request::at(cue, pos).with_gain(step.gain));
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

/// How often a round on stone or metal glances off whining, percent.
pub const RICOCHET_PCT: u32 = 30;

/// What a contact sounds like, by what struck it and what it struck: a
/// blow is the impact family, a round or an arrow the bullet family (a
/// `glance` on the hard two is a ricochet), a body is a body whatever hit
/// it, and a charge is a blast. Water makes no sound here.
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
        (_, Matter::Water) => None,
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
pub fn shot_cue(speed_mmpt: u16) -> Cue {
    if protocol::shot_is_instant(speed_mmpt) {
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
) {
    if feed.shots().is_empty() {
        return;
    }
    let core = &net.session.core;
    for &(shooter, _yaw, _pitch, speed_mmpt, _reach) in feed.shots() {
        // The one bit that separates the two weapons, and it is the same
        // bit the tracer reads: a projectile cannot leave a muzzle at rest,
        // so zero means the shot was instantaneous and instantaneous means
        // a firearm. No item on the wire and none needed.
        let cue = shot_cue(speed_mmpt);
        let at = if shooter == core.player_id {
            let p = core.predict.position();
            [p[0], p[1], p[2]]
        } else {
            let Some(t) = bodies
                .iter()
                .find(|(b, _)| b.0 == shooter)
                .map(|(_, t)| t.translation)
            else {
                continue;
            };
            [t.x, t.y, t.z]
        };
        let d = eye.pos.distance(Vec3::from(at));
        sound.play(Request::at(far_layer(cue, d), at));
    }
}

/// Past this far a gunshot is its far layer, metres: where the near
/// report's falloff and the far one's meet at about the same level
/// (`CUES`' two rows), so the switch is a change of colour and not a step.
pub const SHOT_FAR_M: f32 = 45.0;

/// The layer of a report heard from `d` metres. Only the gun has a far
/// layer; a bow does not carry far enough to need one.
pub fn far_layer(cue: Cue, d: f32) -> Cue {
    if cue == Cue::ShotGun && d > SHOT_FAR_M {
        Cue::ShotGunFar
    } else {
        cue
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
) {
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
    // A magazine seated: your own hands, so an own-fact. (It borrowed the
    // positional `Cue::Place` with no position, which the mixer refuses —
    // a reload was silent.)
    if feed.reloaded > 0 {
        sound.play(Request::own(Cue::Reload));
    }
    // A knock, at the door knocked on — broadcast, so it is often somebody
    // else's hand, and where the door is is the news.
    if let Some(world) = world {
        let core = &net.session.core;
        for &(cx, cz, level, loc, _who) in feed.knocks() {
            let (x, z) = sim_core::build::anchor(cx, cz, loc);
            let plate = core.pieces.cols().plate(cx, cz).unwrap_or(0);
            let y = super::structures::level_base_y(world.seed, &world.haven, cx, cz, level, plate);
            sound.play(Request::at(Cue::Knock, [x, y + 1.3, z]));
        }
    }
    // Every refusal kind, one sound. A player does not need to hear the
    // difference between a refused craft and a refused placement — the toast
    // already says which — they need to hear that the button did nothing.
    //
    // Deliberately not a count: this line said "three" while `Refused` held
    // four, and then five when the consume verbs joined (2026-08-15). The
    // predicate below is variant-agnostic, so a new refusal kind gets this
    // cue for free the moment `feed.rs` pushes it — which is the property
    // worth writing down, and a number here only ever goes stale against it.
    if feed.refusals().next().is_some() {
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
pub fn fell(q: Query<(Ref<super::props::Fellable>, &GlobalTransform)>, mut sound: ResMut<Sound>) {
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
        let cue = match f.part {
            super::props::FellPart::Trunk => Cue::TreeFall,
            super::props::FellPart::Vanish => Cue::ImpactStone,
            // …and `Far` with them: the hull is the trunk at a distance, so
            // a chop that felled both would play `TreeFall` twice on one
            // frame at one position — the exact defect this match was
            // written to end.
            super::props::FellPart::Canopy
            | super::props::FellPart::Stump
            | super::props::FellPart::Far => continue,
        };
        let p = t.translation();
        sound.play(Request::at(cue, [p.x, p.y, p.z]));
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
    for &(cx, cz, level, loc, _deploy) in feed.placed() {
        let plate = net.session.core.pieces.cols().plate(cx, cz).unwrap_or(0);
        let p = super::structures::base_transform(
            world.seed,
            &world.haven,
            (cx, cz, level, loc),
            plate,
        )
        .translation;
        sound.play(Request::at(Cue::Place, [p.x, p.y, p.z]));
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
    herd: Query<(&super::mobs::Animal, &Transform)>,
    eye: Res<Eye>,
    time: Res<Time>,
    feed: Res<super::feed::Feed>,
    mut sound: ResMut<Sound>,
) {
    let dt = time.delta_secs();
    let switch = crate::sound::voice::switch_m();
    for (animal, t) in herd.iter() {
        let Some(slot) = sim_core::mob::slot_of_id(animal.0) else {
            continue;
        };
        let p = t.translation;
        let at = [p.x, p.y + super::mobs::voice_h_of(slot), p.z];
        // A pack call the sim raised (wire v76): the howl goes up now, from
        // this animal, and its own clock restarts so the ambient cadence
        // does not answer it a second later. `Feed` is read, never drained
        // — `feed::drain` is the one reader of the core's rings.
        if feed.howls().contains(&animal.0) {
            sound.voices.called(slot);
            sound.play(Request::at(Cue::Howl, at));
            continue;
        }
        let d = [at[0] - eye.pos.x, at[1] - eye.pos.y, at[2] - eye.pos.z];
        let near = d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= switch * switch;
        let Some(cue) = sound.voices.due(slot, near, dt) else {
            continue;
        };
        sound.play(Request::at(cue, at));
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
/// Two producers, one voice. The fall carries coverage — see
/// [`crate::sound::hurt`] for why reading `EV_HURT` alone would silence
/// starvation, thirst and the keypad shock — and this frame's announced blows
/// carry the weight. `Feed` is taken immutably, which is the shape
/// `CLAUDE.md`'s two-drains trap requires of every reader that is not
/// `feed::drain` itself; scheduling puts this after it.
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
    if let Some(req) = crate::sound::hurt::request(fall, feed.hurt_damage, feed.hurts, core.hp_max)
    {
        sound.play(req);
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
// Nine: the mix state to write, where the ears are, which island this is, the
// cover query, the clock, the player's sliders, the tick, the day pin, and
// the engine the levels leave through.
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
    // Crickets are the night companion and still owed (`NOW.md` §0x).
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
    sound.bed_target[0] = (1.0 - 0.45 * cover) * (0.6 + 0.8 * weather.wind);
    // The surf reads how much sea is within earshot, from the same
    // `terrain::height` the water is drawn from — 24 taps, a fixed pattern, so
    // the level cannot flicker as a search finds different water.
    sound.bed_target[1] = crate::sound::water::surf_gain(crate::sound::water::shore_exposure(
        world.seed, eye.pos.x, eye.pos.z,
    ));
    // The submerged bed has no world level of its own: it is entirely the
    // snapshot's, which is the point of a snapshot.
    sound.bed_target[2] = 1.0;
    // The rain (weather v0): as hard as it falls, duller under a roof. The
    // snapshot takes it away underwater.
    sound.bed_target[3] = weather.rain * if weather.sheltered { 0.45 } else { 1.0 };

    // Thunder: each bolt's clap once its sound has crossed the distance
    // (`weather::update` queued it at the bolt's own time plus d / 343).
    let now_s = feed.server_tick_est / sim_core::limits::TICK_HZ as f64;
    while let Some(gain) = weather.take_thunder(now_s) {
        sound.play(Request::own(Cue::Thunder).with_gain(gain));
    }

    let d = BED_FADE_PER_S * time.delta_secs();
    for i in 0..BEDS.len() {
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
}

/// The waterline: which snapshot the mix is in, and the splash on crossing it.
///
/// **Runs before [`bed`] and [`pump`]**, because both read the snapshot this
/// resolves and a mix state one frame stale is a mix that comes up as your
/// head goes back under.
pub fn water(net: NonSend<Net>, eye: Res<Eye>, time: Res<Time>, mut sound: ResMut<Sound>) {
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
    let feet = net.session.core.predict.render_position()[1];
    if let Some(gain) = sound.waterline.sample(feet, dt) {
        sound.play(Request::own(Cue::Splash).with_gain(gain));
    }
}

/// The score: tick the director, start the piece it asks for, and hold every
/// sounding piece at the player's music level.
///
/// **Runs everywhere, unlike every other system in this file.** There is no
/// `world_running`, no `Net` and no `Eye` in its arguments, because music
/// plays on the menus too — `sound::music::Mode::Menu` is what makes that a
/// different behaviour rather than a different code path.
///
/// Starting a piece is a `Play` on the next music slot and nothing else: the
/// previous piece is left alone to ring out under it, which is the whole of
/// `reference/AUDIO.md` §8's transition design. The renderer frees the slot
/// when the piece ends and the countdown here releases it the same frame,
/// so the stop it sends then lands on a free slot and is a no-op by the
/// engine's own rule.
pub fn music(
    mut sound: ResMut<Sound>,
    mut engine: ResMut<Engine>,
    bank: Res<Bank>,
    time: Res<Time>,
    settings: Res<super::Settings>,
) {
    let dt = time.delta_secs();
    let mix = mix_of(&settings);
    if let Some(piece) = sound.music.tick(dt) {
        let i = sound.next_music;
        sound.next_music = (i + 1) % HELD_MUSIC;
        let def = piece.cue.def();
        // **Its real level, not silence.** A `Play` sets the slot's gain
        // immediately and the loop below only sends a CHANGE, so starting
        // it silent would put a step from zero to full one frame into every
        // piece, which is the click `synth::edges` exists to prevent at the
        // other end of the same sample. The loop's job is to FOLLOW the
        // slider, not to set the opening level.
        let level = def.gain * mix.bus_gain(def.bus);
        sound.music_slots[i] = MusicSlot {
            cue: piece.cue,
            fade: 1.0,
            ending: false,
            left_s: bank.len_s(piece.cue),
            sent: level,
            active: true,
        };
        engine.push(Cmd::Play {
            slot: (HELD_BEDS + i) as u8,
            cue: piece.cue,
            gain: level,
        });
    }

    let step = if MUSIC_FADE_S > 0.0 {
        dt / MUSIC_FADE_S
    } else {
        1.0
    };
    for (i, slot) in sound.music_slots.iter_mut().enumerate() {
        if !slot.active {
            continue;
        }
        slot.left_s -= dt;
        if slot.ending {
            slot.fade -= step;
        }
        if slot.fade <= 0.0 || slot.left_s <= 0.0 {
            slot.active = false;
            engine.push(Cmd::Stop {
                slot: (HELD_BEDS + i) as u8,
            });
            continue;
        }
        let def = slot.cue.def();
        let level = def.gain * mix.bus_gain(def.bus) * slot.fade;
        if level != slot.sent {
            slot.sent = level;
            engine.push(Cmd::Gain {
                slot: (HELD_BEDS + i) as u8,
                gain: level,
            });
        }
    }
}

/// Put the director on a new screen's schedule.
///
/// Called on entering the menu and on entering a world, which are the only
/// two transitions music has. One function because the rule is one rule, and
/// it is stated in the condition rather than in the caller: **a sounding
/// piece is ended only when the new mode is about to start one on top of
/// it.** Leaving a world does (the menu has no gap), so the old piece fades;
/// joining one does not (`music::FIRST_GAP_S` is half a minute), so the menu
/// piece rings out over the loading screen, which is what music is for.
pub fn music_mode(mode: music::Mode) -> impl Fn(ResMut<Sound>) {
    move |mut sound: ResMut<Sound>| {
        sound.music.reset(mode);
        if sound.music.next_in_s() > crate::sound::music::PIECE_S {
            return;
        }
        for slot in sound.music_slots.iter_mut() {
            if slot.active {
                slot.ending = true;
            }
        }
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
    {
        let starts = sound.mixer.tick(dt_ms, listener, live, &mix);
        for (slot, s) in chosen.iter_mut().zip(starts.iter()) {
            *slot = Some(*s);
        }
    }
    for start in chosen.into_iter().flatten() {
        // The rate is the mixer's speed resampled to the renderer's rate —
        // `engine::rate` is the one resampler in the chain (`audio_out.rs`).
        // The take is the bank's to count and never the last one played.
        let takes = bank.takes(start.cue);
        let take = sound.takes.pick(start.cue, takes);
        engine.push(engine::start_cmd(&start, listener, right, out_rate).with_take(take, takes));
        // The ledger counts what the renderer will actually sound: a cue
        // that has not landed (a spread bank's first frames) is silence
        // there, counted as `unbanked`, and must not hold a ledger slot for
        // its length. The ledger's own refusal (`false`) is the renderer's
        // refusal one frame early and needs no second count.
        if bank.installed(start.cue) {
            engine.live.start(bank.len_s(start.cue), start.speed);
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

/// The settings screen's three sliders as a [`Mix`].
fn mix_of(s: &super::Settings) -> Mix {
    Mix {
        master: s.vol_master,
        game: s.vol_game,
        ambience: s.vol_ambience,
        music: s.vol_music,
    }
}

/// The frame budget must not exceed the pool it draws from.
const _: () = assert!(crate::sound::STARTS_PER_FRAME <= VOICE_CAP);
