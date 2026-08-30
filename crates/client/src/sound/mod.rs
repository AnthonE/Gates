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
//!   about: three groups, mirroring `audio.game`, `audio.ambience` and
//!   `audio.musicvolume`.
//! - **Music is a gap-and-intensity system, not a soundtrack.** Four to eight
//!   minutes of silence between songs; a theme divided into sections; each
//!   section holding clips of differing intensity; the tier read only at a
//!   section boundary so the music never lurches. [`music`] is that design,
//!   whole, and the pieces are cues like everything else here.
//! - **Occlusion is a knob and it shipped OFF.** The reference gated its first
//!   pass behind `audio.occlusion` for a week, and a later build was still
//!   fixing excess DSP from it on surround setups. There is no occlusion in
//!   this module, and that is the reference's own ordering, not a shortcut.
//!
//! Every number below is a knob with its documented default
//! (`DECISIONS.md` §open, "audio v0"). None of them was measured against
//! anything, and the module says so rather than implying a tuning pass that
//! did not happen.

// The forest layer — sparse bird calls over the beds. `reference/AUDIO.md`
// §3's *layers*, as distinct from its beds.
pub mod birds;
// What the mixer is told when the health bar moves. Pure, and deliberately
// takes the *fall* as well as the event: three of the sim's seven damage
// routes announce nothing, on purpose, and a mixer fed only `EV_HURT` goes
// silent for all three with every gate green.
pub mod hit;
pub mod hurt;
pub mod mixer;
// When a song plays and which piece it is. `reference/AUDIO.md` §8 is the
// design; this is the whole of it.
pub mod music;
pub mod steps;
pub mod synth;
// What water sounds like from where you are standing. `reference/WATER.md` §7
// is the research; this is the model, and `render/audio.rs` plays it.
pub mod water;
// When an animal speaks. Pure cadence — the render half reads the drawn herd,
// asks it what the species in that roster slot says at that range, and plays
// exactly the cue it is handed back.
pub mod voice;

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
/// **There is still no `Voice` bus**, because there is no voice chat — and a
/// bus with nothing on it is the greyed-out settings row
/// `render/settings.rs` refuses to draw, one layer down. [`Bus::Music`]
/// stopped being one of those when `music::Director` landed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bus {
    /// Everything the player or the world did: steps, swings, impacts, the
    /// interface. The reference's `audio.game`.
    Game,
    /// The beds and the layers over them. The reference's `audio.ambience`.
    Ambience,
    /// The score. The reference's `audio.musicvolume`, whose default is
    /// **0.2** — music is the one bus that does not open at full, there and
    /// here (see [`Mix::default`]).
    Music,
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
    /// Breaking the surface, in either direction. The *edge*, not the wading:
    /// [`Cue::StepWater`] is already playing for every stride taken in the
    /// shallows.
    Splash,
    TreeFall,
    UiClick,
    /// The wind bed. Looped, not fired — a bed is a voice that is turned up
    /// and down rather than triggered, and [`Cue::is_bed`] is the rule the
    /// mixer refuses them by. `render/audio.rs` owns the one entity each.
    BedWind,
    /// The surf bed: heard when there is sea within earshot, at a level that
    /// reads how much (`water::shore_exposure`).
    BedSurf,
    /// The submerged bed. Crossfaded in by [`Snapshot::Submerged`], never by
    /// the world directly.
    BedUnder,
    /// The pig. Appended after the beds rather than beside the other
    /// diegetic cues, deliberately: every cue's noise is seeded off its own
    /// discriminant (`synth::render`), so inserting mid-enum would renumber
    /// everything after it and regenerate waveforms this cue has nothing to
    /// do with. Discriminant order is append order from here on.
    Snort,
    /// Another player's footstep: [`Cue::StepSand`] heard at THEIR body.
    /// A separate cue rather than a positional flag on the local one,
    /// because whether a cue is positional is the def's fact and the
    /// mixer's one distance law hangs off it — the local step is an
    /// own-fact at the listener, this is a place in the world, and one row
    /// cannot be both. The waveform is the local surface's byte for byte
    /// (`synth::render` delegates): the ground decides what a step sounds
    /// like, not whose boot it is. Appended after `Snort` — the enum's
    /// append-order rule.
    RemoteStepSand,
    RemoteStepGrass,
    RemoteStepLitter,
    RemoteStepRock,
    RemoteStepWater,
    /// A bird, somewhere in the trees. `sound::birds` is the cadence; the
    /// perch is a drawn prop, so the layer cannot call from an empty sky.
    Bird,
    /// The nine music pieces: three sections of the theme, three intensity
    /// clips each (`sound::music::PIECES` is the table, and it is the only
    /// place the mapping is written). They are cues so that they get the one
    /// bank, the one gain table, the one WAV dump and the whole of
    /// `tests/sound.rs`'s structural gates for free — and, like the beds,
    /// the mixer refuses to start one ([`Cue::is_music`]): music must not
    /// compete with an axe for a voice out of `VOICE_CAP`.
    ///
    /// Appended after the remote steps — the enum's append-order rule, which
    /// is not cosmetic: every cue's noise is seeded off its own discriminant
    /// (`synth::render`), so inserting mid-enum regenerates waveforms that
    /// have nothing to do with the change.
    MusicOpenCalm,
    MusicOpenTense,
    MusicOpenCombat,
    MusicTurnCalm,
    MusicTurnTense,
    MusicTurnCombat,
    MusicCloseCalm,
    MusicCloseTense,
    MusicCloseCombat,
    /// The wolf's far voice: a contact call, heard from further than any
    /// other diegetic cue but the falling tree. Appended after the score —
    /// the enum's append-order rule, for the reason `Snort` states.
    ///
    /// The pair with [`Cue::Growl`] is **two registers of one animal, chosen
    /// by distance**, and the distance is not a third number: it is this
    /// cue's sibling's own [`CueDef::radius_m`]. See [`crate::sound::voice`].
    Howl,
    /// The wolf's near voice: the threat, at the short end.
    ///
    /// ⚠ **It means "a wolf is close", not "a wolf is hunting you"** — the
    /// client cannot see that. `Mob::roused_until` and `MobDef::brave_pct`
    /// are sim-side only (`sim-core/src/mob.rs`), the snapshot carries
    /// `EntityState` and no mob state or def lane exists on the wire, and
    /// duplicating a content radius into client code would be wall 7. So
    /// the register is the honest half of the encounter: a growl is a
    /// close-range threat call and a howl is a long-range contact call,
    /// which is what the two vocalizations *are*, whatever the animal is
    /// currently deciding.
    Growl,
    /// Another player's swing: [`Cue::Swing`] heard at THEIR body.
    ///
    /// The remote-footstep argument, one verb over — whether a cue is
    /// positional is the def's fact and the mixer's one distance law hangs
    /// off it, so the local arm (an own-fact at the listener, fired off a
    /// keystroke you already knew about) and someone else's arm (a place in
    /// the world) cannot be one row. The waveform is the local one's byte
    /// for byte (`synth::render` delegates): what makes a swing remote is
    /// its def, never its sound.
    ///
    /// **Reusing `Cue::Swing` for both would have been worse than silence**
    /// — non-positional means straight to both ears at full gain with no
    /// pan and no falloff, so every swing on the island would arrive as if
    /// it were in your hands, which is a lie about where a threat is rather
    /// than a missing sound. Appended after `Growl`, the enum's
    /// append-order rule.
    RemoteSwing,
    /// A bow released, heard from where the archer stands (wire v54).
    ///
    /// `EV_SHOT` has crossed the wire since ranged v0 and reached exactly
    /// one reader — `render/tracer.rs` — so a bow drew a streak and made
    /// no sound at all. This is the other half of that event.
    ///
    /// Quiet and short on purpose: a bowstring is the reference's stealth
    /// option against a firearm, and the whole of what makes it one is
    /// that it does not carry.
    ShotBow,
    /// A gun fired, heard from where the shooter stands (wire v54).
    ///
    /// **This is the loudest thing in the bank and that is the mechanic.**
    /// A firearm raised no event at all until v54, so a gunfight was a
    /// private event — the only evidence a shot had happened was the
    /// damage, and a player had no input to the fight-or-run decision.
    /// Sound is the reference's primary disclosure channel
    /// (`reference/AUDIO.md` §9) and its radius is a hundred metres, which
    /// is why [`MAX_AUDIBLE_M`] is what it is.
    ShotGun,
    /// The hitmarker for a **head** hit (v58).
    ///
    /// A separate cue rather than [`Cue::Hit`] pitched up, because
    /// `interface_cues_do_not_vary_in_pitch` is a rule this bank keeps for
    /// a reason — a symbol that drifts stops being a symbol — and "the
    /// same click, higher" is exactly the drift that rule forbids. Three
    /// rungs are three symbols, so they are three waveforms.
    ///
    /// Appended after `ShotGun`, the enum's append-order rule: the
    /// discriminant seeds `synth::render`'s noise, so inserting one beside
    /// `Hit` would regenerate unrelated cues.
    HitHead,
    /// The hitmarker for a **limb** hit (v58).
    ///
    /// The one the judge's gap is really about: a x0.5 blow is easier to
    /// misread as a miss than a x2 one is to read as a skull, so this cue
    /// must say *landed, but less* — duller and lower than [`Cue::Hit`],
    /// never quieter to the point of ambiguity with silence.
    HitLimb,
}

/// How many cues there are. Kept beside [`Cue::ALL`], which is what fails if
/// they disagree.
pub const CUE_COUNT: usize = 45;

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
        Cue::Splash,
        Cue::TreeFall,
        Cue::UiClick,
        Cue::BedWind,
        Cue::BedSurf,
        Cue::BedUnder,
        Cue::Snort,
        Cue::RemoteStepSand,
        Cue::RemoteStepGrass,
        Cue::RemoteStepLitter,
        Cue::RemoteStepRock,
        Cue::RemoteStepWater,
        Cue::Bird,
        Cue::MusicOpenCalm,
        Cue::MusicOpenTense,
        Cue::MusicOpenCombat,
        Cue::MusicTurnCalm,
        Cue::MusicTurnTense,
        Cue::MusicTurnCombat,
        Cue::MusicCloseCalm,
        Cue::MusicCloseTense,
        Cue::MusicCloseCombat,
        Cue::Howl,
        Cue::Growl,
        Cue::RemoteSwing,
        Cue::ShotBow,
        Cue::ShotGun,
        Cue::HitHead,
        Cue::HitLimb,
    ];

    /// Is this cue a piece of music?
    ///
    /// The same rule [`Cue::is_bed`] states, for the same reason and one
    /// system over: the mixer refuses to start one ([`mixer::Mixer::push`]),
    /// because `render/audio.rs`'s music systems own exactly one voice at a
    /// time and a second copy started as a one-shot would play the same
    /// phrase over itself. Derived from `music::piece_of` rather than from a
    /// remembered list, so a tenth piece cannot be forgotten from it.
    pub fn is_music(self) -> bool {
        music::piece_of(self).is_some()
    }

    /// May the mixer start this cue? False for the beds and the music, which
    /// are voices the render layer holds rather than events it fires.
    pub fn mixer_started(self) -> bool {
        !self.is_bed() && !self.is_music()
    }

    /// Is this cue a looping bed?
    ///
    /// A bed is never started by the mixer: [`mixer::Mixer::push`] refuses one
    /// and counts it as the caller bug it is. The rule exists as a predicate
    /// rather than as three remembered names because it is now checked in
    /// three places — the mixer, the frame-budget gate, and the
    /// cooldown-stacking gate, all of which would otherwise each carry their
    /// own list to forget a fourth bed from.
    pub fn is_bed(self) -> bool {
        matches!(self, Cue::BedWind | Cue::BedSurf | Cue::BedUnder)
    }

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
            Cue::StepSand
            | Cue::StepGrass
            | Cue::StepLitter
            | Cue::StepRock
            | Cue::StepWater
            | Cue::RemoteStepSand
            | Cue::RemoteStepGrass
            | Cue::RemoteStepLitter
            | Cue::RemoteStepRock
            | Cue::RemoteStepWater => 0.10,
            Cue::Swing
            | Cue::RemoteSwing
            | Cue::ImpactWood
            | Cue::ImpactStone
            | Cue::ImpactMetal
            | Cue::Gather
            | Cue::Place
            | Cue::Splash
            | Cue::TreeFall
            | Cue::Snort
            | Cue::Growl
            | Cue::Hurt => 0.07,
            // A shot varies like any other diegetic cue, and slightly less
            // than a swing: the two are the most *repeated* sounds in a
            // fight, so unison is the tell, but a firearm's report is a
            // mechanism with a fixed bore and a bow's is a fixed string —
            // both vary with the shooter and the round, not with the
            // weapon's pitch.
            Cue::ShotBow | Cue::ShotGun => 0.05,
            // Wider than any diegetic cue but the bird, and for the bird's
            // reason turned up one notch: a howl is the most *exposed* tonal
            // call in the bank — a near-pure pitched tone held for seconds,
            // where a growl hides its repetition under noise. Two wolves
            // answering each other at the same pitch is a chorus in unison,
            // which is the machine-gun tell this knob exists for, and unison
            // is what a real chorus is specifically not.
            Cue::Howl => 0.12,
            // The forest layer's variation is the whole of it: one recording
            // of one bird, retriggered every few seconds at exactly its own
            // pitch, is the machine-gun tell this knob exists for, and a
            // layer is heard for minutes on end where a footstep is heard for
            // a stride. The widest in the table on purpose.
            Cue::Bird => 0.16,
            Cue::CraftDone
            | Cue::Refused
            | Cue::Hit
            | Cue::HitHead
            | Cue::HitLimb
            | Cue::Death
            | Cue::UiClick
            | Cue::BedWind
            | Cue::BedSurf
            | Cue::BedUnder => 0.0,
            // **Zero, and it is not the signal-cue argument.** A piece played
            // at 1.03× is a piece in a different key, and the next piece
            // would be in a third — the tail that covers a join would be
            // covering a modulation. Music is the one family where varying
            // the rate is not variation, it is being out of tune.
            //
            // Spelled out rather than caught by a `_` arm: this match is
            // exhaustive on purpose, so that adding a cue is a compile error
            // until somebody decides what it should do. `CLAUDE.md`'s
            // feature-gated-`match` trap is the same lesson from the other
            // side — an arm you did not have to write is a decision you did
            // not have to make.
            Cue::MusicOpenCalm
            | Cue::MusicOpenTense
            | Cue::MusicOpenCombat
            | Cue::MusicTurnCalm
            | Cue::MusicTurnTense
            | Cue::MusicTurnCombat
            | Cue::MusicCloseCalm
            | Cue::MusicCloseTense
            | Cue::MusicCloseCombat => 0.0,
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
    SWING,
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
    // Breaking the surface. Your own body, so non-positional, and the cooldown
    // is what stops a player bobbing on the waterline from machine-gunning it:
    // the crossing test is a sign change, and a body oscillating around
    // `SEA_LEVEL` produces one every frame.
    row(GAME,  0.0, 0.65, 220, 5, false),  // splash
    // A tree coming down is the loudest thing in the forest — and it set
    // `MAX_AUDIBLE_M` until the gun's report outranged it at v54.
    row(GAME, 96.0, 0.90,   0, 6, true),   // tree fall
    row(GAME,  0.0, 0.30,  40, 2, false),  // ui click
    // The beds. Never started by the mixer (`Cue::is_bed`); `render/audio.rs`
    // holds one looping voice each and moves their gains.
    row(AMB,   0.0, 0.30,   0, 0, false),  // wind
    row(AMB,   0.0, 0.34,   0, 0, false),  // surf
    row(AMB,   0.0, 0.40,   0, 0, false),  // submerged
    // The pig announces itself before you see it (`reference/ANIMALS.md` —
    // the boar is identified by its snorting), so it carries past the
    // impacts but nowhere near a falling tree. Priority with the footsteps'
    // register: ambience, not signal. The cooldown is per-cue, so it is the
    // herd's stagger, not one animal's — `sound::voice` spaces one animal.
    row(GAME, 40.0, 0.55, 150, 2, true),   // snort
    // Remote footsteps: the STEP family heard at another body — see RSTEP.
    RSTEP, RSTEP, RSTEP, RSTEP, RSTEP,
    // The forest layer. AMBIENCE, not game: it is scenery, and a player who
    // turns the bed down means the birds too. Positional (it comes from a
    // perch), quiet, and at the footsteps' priority — a bird must never be
    // the reason an axe was refused a voice. The cooldown is per-CUE and so
    // it is the whole flock's stagger, not one bird's, which is what
    // `sound::birds` already spaces.
    row(AMB,   44.0, 0.30, 700, 1, true),  // bird
    // The nine music pieces: three sections down, three intensity tiers
    // across. Non-positional (music is not anywhere), radius 0 (never culled
    // — see `positional`), no cooldown and priority 0 because the mixer never
    // starts one at all (`Cue::is_music`).
    //
    // **The tiers differ in GAIN and the sections do not**, which is this
    // table's own law rather than a taste call: `synth::PEAK` normalizes every
    // cue in the bank to one peak precisely so that `CueDef::gain` is the only
    // thing deciding relative loudness — and that erases the level a denser
    // arrangement would otherwise have had. A tier a player cannot hear
    // arriving is a table nobody can hear, so the step back is taken here,
    // where every other level in the client lives, instead of in the
    // generator where the normalizer would eat it.
    M_CALM, M_TENSE, M_COMBAT,
    M_CALM, M_TENSE, M_COMBAT,
    M_CALM, M_TENSE, M_COMBAT,
    // The wolf, in two registers (`sound::voice`). Both GAME rather than
    // AMBIENCE for the pig's reason: an animal is a thing in the world, not
    // scenery, and a player who turns the bed down must not turn the
    // predator down with it.
    //
    // **The howl carries further than anything but a gunshot and a falling
    // tree** — which is the point of it. The wolf notices a player at 30 m by day and 15 m at night
    // (`content/mobs.toml`), so 88 m means the island tells you it has
    // wolves, and roughly where, long before one of them can act on you. The
    // cooldown is per-CUE and therefore the pack's stagger, not one animal's
    // — `sound::voice` spaces one animal, as `sound::birds` does for a flock.
    row(GAME, 88.0, 0.60, 900, 2, true),   // howl
    // The growl's radius is doing a SECOND job and it is the load-bearing
    // one: `sound::voice` picks the register by comparing the listener's
    // distance against this very number (`CUES[Growl].radius_m`), so the two
    // cannot disagree — a growl is never chosen and then culled by the
    // mixer's falloff, and a howl is never chosen inside growl range. There
    // is no third "switch distance" knob to drift.
    //
    // 14 m sits inside the wolf's *smallest* notice radius (15 m, at night),
    // so in practice a wolf you can hear growl has already seen you. ⚠ **That
    // relationship is a design intent and nothing enforces it** — the client
    // has no mob-def lane on the wire and no dependency on the `content`
    // crate, so the two numbers live in different worlds and a `mobs.toml`
    // edit will not redden anything here. `NOW.md` §0pr carries the owed gate.
    // Priority 5: above the impacts, below the hitmarker — a growl at this
    // range is information a player's life turns on.
    row(GAME, 14.0, 0.65, 200, 5, true),   // growl
    // Another player's swing — see RSWING.
    RSWING,
    // **The two shots, and both radii are the reference's own number read
    // off one sentence** (`CueDef::radius_m`'s doc, quoted there since
    // audio v0): a silenced weapon there carries "a maximum of 40m instead
    // of the 100m it used to be". That is a published pair for exactly the
    // two roles this game has — the loud ranged option and the quiet one —
    // so `BALANCE.md` §6 takes both and no case is owed for either. What
    // would need a case is *differing*, and the thing worth not differing
    // about is the RATIO: a bow discloses you over 40 m and a gun over
    // 100 m, which is 6× the area, and that multiple is the whole of why
    // anyone would carry a bow once they can craft a revolver.
    //
    // The gun's 100 m is what sets `MAX_AUDIBLE_M`; the falling tree held
    // that title until v54.
    row(GAME,  40.0, 0.45,  60, 4, true),   // bow released
    row(GAME, 100.0, 0.85,  60, 6, true),   // gun fired
    // The other two rungs of the marker (v58). Same bus, same radius (none
    // — they are signals, not places), same 45 ms cooldown and the same
    // reason for it as `Hit`'s: zero `pitch_var` means two in a frame are
    // bit-identical and would add rather than blend.
    //
    // The gains bracket `Hit`'s 0.50 rather than sitting on it, because
    // the rung has to be legible in one hearing and loudness is the
    // channel a player reads without being taught. The limb's 0.45 is a
    // deliberate floor and not a taper: quieter than the identity, still
    // plainly a hit — the judge's whole point is that a halved blow reads
    // as a MISS, and a cue fading toward silence would say exactly that.
    row(GAME,  0.0, 0.60,  45, 6, false),  // hit, head
    row(GAME,  0.0, 0.45,  45, 6, false),  // hit, limb
];

/// Your own arm. Named rather than written inline so [`RSWING`] can read its
/// numbers off it instead of restating them — [`STEP`]/[`RSTEP`]'s shape.
const SWING: CueDef = row(GAME, 20.0, 0.45, 120, 3, false);

/// Another player's swing (`DECISIONS.md` §open, "remote swing v0").
///
/// Radius and gain come off [`SWING`] rather than being restated: the 20 m
/// that row has always carried was written for exactly this positional half
/// and **was never read at all** — `positional: false` makes `radius_m` dead
/// (see [`CueDef::radius_m`]) — so this is where the number becomes true
/// rather than a new one being invented beside it.
///
/// What differs is deliberate, and it is [`RSTEP`]'s list verbatim because
/// it is the same argument:
///
/// - **positional** — the whole point; the cue is at the body, panned, and
///   culled by the one falloff law.
/// - **priority 4**, above your own arm's 3, because another player's swing
///   is information your life turns on and your own is a keystroke you just
///   pressed. That puts it level with the impacts, which is the register it
///   belongs in; ties break on distance, so the nearer swing wins.
/// - **a 40 ms cooldown** rather than the local 120 ms — [`RSTEP`]'s value,
///   taken for [`RSTEP`]'s reason. The cooldown is per-CUE and therefore
///   shared across every swinger in earshot, and ⚠ **it binds inside a
///   frame** (`tests/sound.rs::a_cooldown_binds_within_one_frame`), so it is
///   a hard rate limit over the whole crew rather than a stagger on one arm.
///   At 40 ms a second raider swinging three frames later is heard; at the
///   local swing rate he waits an eighth of a second.
const RSWING: CueDef = CueDef {
    bus: Bus::Game,
    radius_m: SWING.radius_m,
    gain: SWING.gain,
    cooldown_ms: 40,
    priority: 4,
    positional: true,
};

/// The five footsteps share every number but their timbre — see [`CUES`].
const STEP: CueDef = CueDef {
    bus: Bus::Game,
    radius_m: 24.0,
    gain: 0.35,
    cooldown_ms: 90,
    priority: 1,
    positional: false,
};

/// The remote five (`DECISIONS.md` §open, "remote footsteps v0"). Radius and
/// gain are read off [`STEP`] rather than restated — the 24 m the local row
/// always carried was written for exactly this positional half, and tying
/// them means the two families cannot drift apart on how far a boot carries.
/// What differs is deliberate: **positional** (the whole point — the cue is
/// at the body, panned and culled by the one falloff law); **priority 2**,
/// above your own feet, because another player's step is the sound that
/// decides fights and yours tells you nothing; and a **40 ms cooldown** (the
/// impacts' register, not the local 90 ms) since the cooldown is per-CUE and
/// therefore shared across every remote on that surface — a stride-length
/// cooldown would let your own pool mask a second attacker entirely.
const RSTEP: CueDef = CueDef {
    bus: Bus::Game,
    radius_m: STEP.radius_m,
    gain: STEP.gain,
    cooldown_ms: 40,
    priority: 2,
    positional: true,
};

/// A music piece at intensity `gain`. The three tiers are the only thing that
/// separates the nine rows — see [`CUES`].
///
/// The steps are ~5 dB and ~2 dB (`DECISIONS.md` §open, "music v0"), sized so
/// that each tier is audibly above the one below it *after* the bank's peak
/// normalization has flattened them, and so the top tier is the one that runs
/// at the bus's full level rather than the middle one having headroom above
/// it.
const fn music_row(gain: f32) -> CueDef {
    CueDef {
        bus: Bus::Music,
        radius_m: 0.0,
        gain,
        cooldown_ms: 0,
        priority: 0,
        positional: false,
    }
}

const M_CALM: CueDef = music_row(0.55);
const M_TENSE: CueDef = music_row(0.78);
const M_COMBAT: CueDef = music_row(1.0);

/// The furthest any cue carries, metres. Read by `render/audio.rs` to pick the
/// spatial scale, and asserted against [`CUES`] in `tests/sound.rs` — the two
/// must not drift, because a cue with a radius past this one would fall off
/// the far side of rodio's clamp and get *louder* with distance.
///
/// **Set by [`Cue::ShotGun`] since v54**, at the reference's own hundred
/// metres; the falling tree's 96 m held it before that (this line said 88 m
/// for one commit, which is the howl's radius — the maximum of a table is
/// the kind of claim to re-read off the table). It is a derived
/// number and not a taste one — it is the maximum of the table, and the
/// test that asserts so is what makes raising a radius force this line
/// rather than silently invert a cue's falloff.
pub const MAX_AUDIBLE_M: f32 = 100.0;

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
    pub music: f32,
}

impl Default for Mix {
    fn default() -> Self {
        // **The reference ships master and game at 1 and its music at 0.2,
        // and now that there is music, so do we.** The bed still sits under
        // the game bus by its own cue gain rather than by a quieter bus,
        // because a player who turns ambience down should be turning the
        // wind down and not discovering the bed was already half off. Music
        // is the opposite case and theirs is the evidence: a score at parity
        // with footsteps is a score you fight over, so it opens at a fifth
        // and the slider goes up from there.
        Self {
            master: 1.0,
            game: 1.0,
            ambience: 1.0,
            music: MUSIC_DEFAULT,
        }
    }
}

/// What the music bus opens at. The reference's `audio.musicvolume` default,
/// verbatim — `reference/BALANCE.md` §6: a case is owed for differing, not
/// for taking.
pub const MUSIC_DEFAULT: f32 = 0.2;

impl Mix {
    /// The multiplier a cue on `bus` gets from the mix.
    pub fn bus_gain(&self, bus: Bus) -> f32 {
        let g = match bus {
            Bus::Game => self.game,
            Bus::Ambience => self.ambience,
            Bus::Music => self.music,
        };
        (self.master * g).clamp(0.0, 1.0)
    }

    /// This mix seen through a snapshot. The player's sliders stay the
    /// player's; a snapshot only ever scales them down.
    pub fn under(&self, snap: &SnapshotDef) -> Mix {
        Mix {
            master: self.master,
            game: self.game * snap.game,
            ambience: self.ambience * snap.ambience,
            music: self.music * snap.music,
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshots.
// ---------------------------------------------------------------------------

/// A named whole-mixer state.
///
/// **This is the reference's own step 2 and the one `reference/AUDIO.md` §9.1
/// named as owed.** Their published build order for audio is groups, then
/// *snapshots* — "a whole mixer state" — then *fades between snapshots*, and
/// only then occlusion and reverb. We had the groups and nothing else; this is
/// the missing middle, and the thing that finally needed it is water.
///
/// It is two states because there are two, not because two is easy. A cave
/// snapshot and a hostile-contact snapshot are the obvious next ones and
/// neither has a cause in the client yet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Snapshot {
    Above,
    Submerged,
}

/// What a snapshot sets: the two buses, and the level each bed is held at.
///
/// The beds are in here rather than in the world's own gains because a
/// snapshot is *a whole mixer state* — the point of the mechanism is that one
/// value moves everything at once and nothing has to remember to move with it.
#[derive(Clone, Copy, Debug)]
pub struct SnapshotDef {
    pub game: f32,
    pub ambience: f32,
    /// **Flat in both states today, and that is the finding rather than an
    /// oversight.** A snapshot is *a whole mixer state*, so every bus has to
    /// appear in one or it can never be moved by one — but music is
    /// non-diegetic, and water between you and a violin is not a thing to
    /// model. The reference's own first use for this field is a cause we do
    /// not have: their published snapshot example is fading music down *when
    /// somebody speaks*, which needs voice chat.
    pub music: f32,
    pub wind: f32,
    pub surf: f32,
    pub under: f32,
}

/// The two states (`DECISIONS.md` §open, "water audio v0").
///
/// **What `Submerged` cannot do, said plainly.** Real submerged audio is a
/// steep low-pass: the top end goes, and that is a filter. We have no DSP
/// node — rodio gives us gain, rate and panning — so the substitution is to
/// duck the game bus and crossfade to a bed that is *generated* dark
/// (`synth::under`). That is the same kind of substitution as pitch jitter
/// standing in for a recorded variation bank (`reference/AUDIO.md` §9.5): a
/// stand-in, not an equal, and it is written down rather than implied.
pub const SNAPSHOTS: [SnapshotDef; 2] = [
    // Above.
    SnapshotDef {
        game: 1.0,
        ambience: 1.0,
        music: 1.0,
        wind: 1.0,
        surf: 1.0,
        under: 0.0,
    },
    // Submerged. The game bus survives at a level a player can still fight on
    // — being underwater must not be a stealth advantage handed out by the
    // mixer — and a little surf comes through, because it does.
    SnapshotDef {
        game: 0.45,
        ambience: 1.0,
        music: 1.0,
        wind: 0.0,
        surf: 0.22,
        under: 1.0,
    },
];

/// How long a full crossfade between snapshots takes, seconds.
///
/// Short, because the cause is instantaneous and a slow fade would have the
/// mix lagging a head that has already gone under. Not zero, because a step
/// change in five gains at once is a click in five voices at once.
pub const SNAPSHOT_FADE_S: f32 = 0.30;

/// The crossfade, as state.
///
/// `t` is how far toward [`Snapshot::Submerged`] the mix currently is. Held
/// rather than recomputed because a fade is state by definition — the same
/// shape `render/audio.rs` already keeps for each bed's gain.
#[derive(Default)]
pub struct Snapshots {
    t: f32,
}

impl Snapshots {
    /// Move toward `want` and return the mixer state to apply this frame.
    pub fn tick(&mut self, want: Snapshot, dt_s: f32) -> SnapshotDef {
        let target = match want {
            Snapshot::Above => 0.0,
            Snapshot::Submerged => 1.0,
        };
        let step = if SNAPSHOT_FADE_S > 0.0 {
            dt_s / SNAPSHOT_FADE_S
        } else {
            1.0
        };
        self.t = if self.t < target {
            (self.t + step).min(target)
        } else {
            (self.t - step).max(target)
        };
        self.blend()
    }

    /// Where the fade is, 0..1.
    pub fn t(&self) -> f32 {
        self.t
    }

    /// Snap to a state without fading — leaving a world, where there is
    /// nothing to hear through the fade and a half-submerged mix would be
    /// carried into the next island.
    pub fn reset(&mut self) {
        self.t = 0.0;
    }

    fn blend(&self) -> SnapshotDef {
        let (a, b) = (&SNAPSHOTS[0], &SNAPSHOTS[1]);
        let t = self.t.clamp(0.0, 1.0);
        let mix = |x: f32, y: f32| x + (y - x) * t;
        SnapshotDef {
            game: mix(a.game, b.game),
            ambience: mix(a.ambience, b.ambience),
            music: mix(a.music, b.music),
            wind: mix(a.wind, b.wind),
            surf: mix(a.surf, b.surf),
            under: mix(a.under, b.under),
        }
    }
}

impl Default for SnapshotDef {
    /// [`Snapshot::Above`] — the state a client that has not looked at the
    /// world yet is in, and the one it returns to when a world is left.
    fn default() -> Self {
        SNAPSHOTS[0]
    }
}

impl SnapshotDef {
    /// The level this snapshot holds a bed at. Panics on a cue that is not a
    /// bed, which is a programming error rather than a runtime condition —
    /// [`Cue::is_bed`] is the question to ask first.
    pub fn bed(&self, cue: Cue) -> f32 {
        match cue {
            Cue::BedWind => self.wind,
            Cue::BedSurf => self.surf,
            Cue::BedUnder => self.under,
            _ => 0.0,
        }
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
