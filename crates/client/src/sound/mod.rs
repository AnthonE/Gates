//! The client's audio **model** — what should be heard, how loud, and how
//! many at once. Computed outside Bevy.
//!
//! **Bevy plays; it does not decide.** This is `RENDER.md` §1's rule applied
//! one surface over, and for the same reason: a mixer that lives in a system
//! can only be tested by a windowed run with a sound card, and this box has
//! neither. Everything here is a pure function of a cue request, a listener
//! position and a `dt`, so `crates/client/tests/sound.rs` drives all of it
//! **headless, with no `render` feature and no audio device** — which is also
//! why this module is not feature-gated. `ui/` is the precedent and this is
//! deliberately the same shape.
//!
//! What the reference game does, and which parts we took, is
//! `reference/AUDIO.md`. Three of its findings are load-bearing here and are
//! restated where they bind:
//!
//! - **The sound system is on a per-frame budget.** The reference ships
//!   `audio.framebudget 0.3` — 0.3 ms a frame for sound updates, a convar, not
//!   a comment. A pure module cannot measure milliseconds, so the budget here
//!   is in work items ([`STARTS_PER_FRAME`], [`VOICE_CAP`], [`CUE_QUEUE_CAP`]),
//!   which is what `CLAUDE.md` wall 4 asks for anyway: a cap and a stated
//!   overflow policy on every queue.
//! - **Buses before effects.** The reference moved to Unity 5's audio mixer —
//!   groups you can balance and snapshots you can fade between — before it had
//!   occlusion or reverb. [`Bus`] is that, at the size ours can be honest
//!   about: two groups, mirroring `audio.game` and `audio.ambience`.
//! - **Occlusion is a knob and it shipped OFF.** The reference gated its first
//!   pass behind `audio.occlusion` for a week, and a later build was still
//!   fixing excess DSP from it on surround setups. There is no occlusion in
//!   this module, and that is the reference's own ordering, not a shortcut.
//!
//! Every number below is a knob with its documented default
//! (`DECISIONS.md` §open, "audio v0"). None of them was measured against
//! anything, and the module says so rather than implying a tuning pass that
//! did not happen.

pub mod mixer;
pub mod steps;
pub mod synth;

/// The mixer's output sample rate, Hz.
///
/// One rate for the whole bank because [`synth`] generates every cue at boot
/// and rodio resamples whatever it is handed — a second rate would buy
/// nothing and would make the seam arithmetic in [`synth::loop_seam`] take a
/// parameter it does not need.
pub const SAMPLE_RATE: u32 = 44_100;

/// Which group a cue is balanced in. The reference's mixer groups, at the
/// size ours can be honest about.
///
/// **There is no `Music` bus and no `Voice` bus**, because there is no music
/// and no voice chat. A bus with nothing on it is the greyed-out settings row
/// `render/settings.rs` refuses to draw, one layer down.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bus {
    /// Everything the player or the world did: steps, swings, impacts, the
    /// interface. The reference's `audio.game`.
    Game,
    /// The bed. The reference's `audio.ambience`.
    Ambience,
}

/// Every sound this client can make, as an integer code.
///
/// Integer codes rather than strings for the sim's own reason (`CLAUDE.md`
/// wall 3): a cue crosses a queue every frame, and a `String` on that path is
/// an allocation per sound. The name is for the reader; the wire is the
/// discriminant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Cue {
    StepSand = 0,
    StepGrass,
    StepLitter,
    StepRock,
    StepWater,
    Swing,
    ImpactWood,
    ImpactStone,
    ImpactMetal,
    Gather,
    CraftDone,
    Refused,
    Hit,
    Hurt,
    Death,
    Place,
    TreeFall,
    UiClick,
    /// The wind bed. Looped, not fired — the only cue the mixer never starts,
    /// because a loop is a voice that is turned up and down rather than
    /// triggered. `render/audio.rs` owns its one entity.
    BedWind,
}

/// How many cues there are. Kept beside [`Cue::ALL`], which is what fails if
/// they disagree.
pub const CUE_COUNT: usize = 19;

impl Cue {
    /// Every cue, in discriminant order. The bank is built by walking this,
    /// and `tests/sound.rs` asserts it matches [`CUE_COUNT`] and that each
    /// entry's index is its own discriminant — which is the assertion that
    /// catches a cue added to the enum and forgotten here, the one mistake
    /// that would silently shift every table below by one.
    pub const ALL: [Cue; CUE_COUNT] = [
        Cue::StepSand,
        Cue::StepGrass,
        Cue::StepLitter,
        Cue::StepRock,
        Cue::StepWater,
        Cue::Swing,
        Cue::ImpactWood,
        Cue::ImpactStone,
        Cue::ImpactMetal,
        Cue::Gather,
        Cue::CraftDone,
        Cue::Refused,
        Cue::Hit,
        Cue::Hurt,
        Cue::Death,
        Cue::Place,
        Cue::TreeFall,
        Cue::UiClick,
        Cue::BedWind,
    ];

    /// The cue's index into every table in this module.
    pub fn idx(self) -> usize {
        self as usize
    }

    /// The cue's static properties.
    pub fn def(self) -> &'static CueDef {
        &CUES[self.idx()]
    }

    /// How far this cue's playback rate may wander from 1.0, as a fraction.
    ///
    /// **The cheapest fix for the worst tell in a generated bank.** One
    /// footstep sample retriggered every 0.85 m is a machine gun firing the
    /// identical waveform, and no amount of synthesis quality hides it; ±10%
    /// on the rate makes each step a different length and a different pitch
    /// for one multiply. It is also how the reference gets variation, in the
    /// expensive way — it records "multiple variations per action"
    /// (`reference/AUDIO.md` §5), which is a bank we do not have.
    ///
    /// **Signal cues get zero, and that is the interesting half.** The
    /// hitmarker, the craft chime, the refusal and the UI click are the four
    /// sounds a player learns as *symbols*, and a symbol that changes pitch
    /// every time is a symbol that takes longer to learn. Diegetic sounds
    /// vary; the interface does not.
    pub fn pitch_var(self) -> f32 {
        match self {
            Cue::StepSand | Cue::StepGrass | Cue::StepLitter | Cue::StepRock | Cue::StepWater => {
                0.10
            }
            Cue::Swing
            | Cue::ImpactWood
            | Cue::ImpactStone
            | Cue::ImpactMetal
            | Cue::Gather
            | Cue::Place
            | Cue::TreeFall
            | Cue::Hurt => 0.07,
            Cue::CraftDone | Cue::Refused | Cue::Hit | Cue::Death | Cue::UiClick | Cue::BedWind => {
                0.0
            }
        }
    }
}

/// What is fixed about a cue: which bus it is balanced in, how far it carries,
/// how loud it is at the listener, how often it may retrigger, and how much it
/// is worth when the frame budget is full.
#[derive(Clone, Copy, Debug)]
pub struct CueDef {
    pub bus: Bus,
    /// How far the cue carries, metres. **Only read when `positional`** — a
    /// cue that happens to the local player has no distance to it.
    ///
    /// The reference's own datum for the scale of this number: a silenced
    /// weapon there is "a maximum of 40m instead of the 100m it used to be",
    /// so tens of metres for a gunshot and a couple of dozen for a footstep is
    /// the register, not hundreds.
    pub radius_m: f32,
    /// Gain at the listener, before the bus and the master. 0..1.
    pub gain: f32,
    /// The shortest interval between two starts of this cue, ms. **This is
    /// the cheap half of the reference's frame budget**: a machine-gun retrigger
    /// is what actually fills a voice pool, and refusing it costs one
    /// comparison.
    pub cooldown_ms: u32,
    /// Who survives when more cues want to start than the frame allows.
    /// Higher wins. Ties break on distance, then on queue order, so the
    /// choice is a pure function of the frame and never of a `HashMap`'s mood.
    pub priority: u8,
    /// Does this cue happen somewhere, or does it happen to *you*?
    ///
    /// Own-facts are non-positional on purpose. Your own footstep is at the
    /// listener, where an inverse-square law divides by zero and a panner has
    /// no side to pick.
    pub positional: bool,
}

/// One row of [`CUES`], positionally.
///
/// A builder rather than a struct literal for one reason and it is not
/// brevity: rustfmt expands a struct literal to seven lines, which turns a
/// nineteen-row table into a 130-line wall nobody can scan for the row that is
/// wrong. The column legend above the table is what a positional call costs,
/// and a table you can read down a column is what it buys.
const fn row(
    bus: Bus,
    radius_m: f32,
    gain: f32,
    cooldown_ms: u32,
    priority: u8,
    positional: bool,
) -> CueDef {
    CueDef {
        bus,
        radius_m,
        gain,
        cooldown_ms,
        priority,
        positional,
    }
}

use Bus::Ambience as AMB;
use Bus::Game as GAME;

/// The cue table. Indexed by [`Cue::idx`] — order must match [`Cue::ALL`].
///
/// **Not measured against anything.** These are opening values in the sense
/// `CONTENT.md` uses the phrase: each is a knob whose default ships until
/// someone with ears moves it (`DECISIONS.md` §open, "audio v0").
///
/// ```text
///      bus   radius  gain  cool  prio  positional
/// ```
#[rustfmt::skip]
pub const CUES: [CueDef; CUE_COUNT] = [
    // Footsteps. One bus, one radius, one gain: the surfaces differ in
    // TIMBRE, which is `synth`'s job, not in how far they carry.
    STEP, STEP, STEP, STEP, STEP,
    // Swing: yours, so non-positional, and the cooldown is the swing rate.
    row(GAME, 20.0, 0.45, 120, 3, false),
    // Impacts happen at a thing, so they carry and they pan.
    row(GAME, 40.0, 0.70,  40, 4, true),   // wood
    row(GAME, 40.0, 0.70,  40, 4, true),   // stone
    row(GAME, 48.0, 0.70,  40, 4, true),   // metal
    // Gather / craft / refusal are the interface answering you.
    row(GAME,  0.0, 0.50,  60, 4, false),  // gather
    row(GAME,  0.0, 0.55,  80, 5, false),  // craft done
    row(GAME,  0.0, 0.45, 250, 5, false),  // refused
    // Combat. `Hurt` and `Death` outrank everything: a player who cannot hear
    // that they are being killed has lost the round to the mixer.
    //
    // **`Hit` has a cooldown and it is not about pacing.** It is a signal cue,
    // so `pitch_var` is zero — four hits landing in one frame would start four
    // BIT-IDENTICAL 2 kHz clicks in phase, and identical samples summed in a
    // mixer add rather than blend: four of them is four times the amplitude,
    // straight into the limiter. The cooldown is what makes "one marker per
    // volley" true, and every zero-cooldown row in this table is one that is
    // either positional (so no two are identical) or happens once.
    row(GAME,  0.0, 0.50,  45, 6, false),  // hit (the marker)
    row(GAME,  0.0, 0.80, 120, 7, false),  // hurt
    row(GAME,  0.0, 0.90,   0, 8, false),  // death
    // Placement happens at a cell — the reference shipped this one wrong for a
    // while, with placement effects firing at the world origin instead of at
    // the socket (`reference/AUDIO.md` §6). Ours carries a position or it does
    // not fire.
    row(GAME, 32.0, 0.60,  30, 4, true),   // place
    // A tree coming down is the loudest thing in the forest and the one cue
    // whose radius sets `MAX_AUDIBLE_M`.
    row(GAME, 96.0, 0.90,   0, 6, true),   // tree fall
    row(GAME,  0.0, 0.30,  40, 2, false),  // ui click
    // The bed. Never started by the mixer; `render/audio.rs` holds one looping
    // voice and moves its gain.
    row(AMB,   0.0, 0.30,   0, 0, false),  // wind
];

/// The five footsteps share every number but their timbre — see [`CUES`].
const STEP: CueDef = CueDef {
    bus: Bus::Game,
    radius_m: 24.0,
    gain: 0.35,
    cooldown_ms: 90,
    priority: 1,
    positional: false,
};

/// The furthest any cue carries, metres. Read by `render/audio.rs` to pick the
/// spatial scale, and asserted against [`CUES`] in `tests/sound.rs` — the two
/// must not drift, because a cue with a radius past this one would fall off
/// the far side of rodio's clamp and get *louder* with distance.
pub const MAX_AUDIBLE_M: f32 = 96.0;

/// How many voices may sound at once.
///
/// The overflow policy is **refuse the new one**, not steal the oldest: a
/// stolen voice is an audible cut, and at this cap the thing being refused is
/// the 25th simultaneous sound, which nobody can pick out of the other 24.
pub const VOICE_CAP: usize = 24;

/// How many voices may START in one frame.
///
/// This is the reference's `audio.framebudget` in the only unit a pure module
/// has. Four is not a measurement; it is a bound (`DECISIONS.md` §open).
pub const STARTS_PER_FRAME: usize = 4;

/// How many cue requests may be queued between two mixer ticks.
///
/// **Overflow policy: drop the newest and count it.** Drop-oldest is wrong
/// here for the reason it is right for datagrams — a datagram's value is its
/// freshness, and a cue's value is that it happened. Dropping the newest keeps
/// the sounds that were requested first in the frame, which is the order the
/// world produced them in. [`mixer::Mixer::dropped`] is the counter, and a
/// non-zero one is a bug in the caller, not a load condition.
pub const CUE_QUEUE_CAP: usize = 32;

/// The bus and master volumes. The reference's `audio.master`, `audio.game`
/// and `audio.ambience`, one for one.
#[derive(Clone, Copy, Debug)]
pub struct Mix {
    pub master: f32,
    pub game: f32,
    pub ambience: f32,
}

impl Default for Mix {
    fn default() -> Self {
        // The reference ships master and game at 1 and its music at 0.2. Ours
        // has no music; the bed sits under the game bus by its own cue gain
        // rather than by a quieter bus, so both buses open at 1.
        Self {
            master: 1.0,
            game: 1.0,
            ambience: 1.0,
        }
    }
}

impl Mix {
    /// The multiplier a cue on `bus` gets from the mix.
    pub fn bus_gain(&self, bus: Bus) -> f32 {
        let g = match bus {
            Bus::Game => self.game,
            Bus::Ambience => self.ambience,
        };
        (self.master * g).clamp(0.0, 1.0)
    }
}

/// How a positional cue loses volume with distance.
///
/// `(1 - d/R)²` — quadratic, and **exactly zero at the radius**. The second
/// property is the one that matters and it is why this is not an inverse
/// square: a law that is merely small at the cull radius clicks when a source
/// crosses it, and a source crossing the radius is what a player walking
/// away from a chopping sound does continuously.
///
/// Outside the radius this returns 0, which is also the mixer's cull test —
/// one law, one place, so a cue can never be culled at one distance and
/// attenuated by a different curve at another.
pub fn falloff(dist_m: f32, radius_m: f32) -> f32 {
    if radius_m <= 0.0 || dist_m >= radius_m {
        return 0.0;
    }
    let t = 1.0 - (dist_m / radius_m);
    t * t
}
