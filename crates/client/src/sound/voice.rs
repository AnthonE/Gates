//! When an animal speaks: the roster's voice cadence, pure and headless.
//!
//! The reference identifies a boar by its snorting before you see it
//! (`NOW.md` §0m item 3), so the voice's job is presence, not reaction —
//! a call every handful of seconds from wherever the animal is. Two rules
//! shape everything here:
//!
//! - **Not a metronome.** A fixed period is the tell that gives away a
//!   generated voice the same way a clock-driven footstep gives away a
//!   footstep system (`steps.rs`). Every interval is drawn fresh from a
//!   hash of the roster slot and the cycle number, so one animal never
//!   repeats its own rhythm and two animals never share one.
//! - **Deterministic, from the world's own identities.** The variation is
//!   hashed from the mob's roster slot — never from an OS random source
//!   (the house style: `synth`'s bank and the mixer's pitch stream are both
//!   seeded xorshift). This is client presentation; the sim never sees it,
//!   and two clients hearing the same pig hear the same cadence.
//!
//! Dormancy is respected by construction rather than by a check: the render
//! half reads the *drawn* herd, which exists only for mobs inside AOI
//! (`AOI_EXIT_CM` = 208 m), and every mob a client can see is awake —
//! `MOB_WAKE_CM` (240 m) deliberately encloses AOI (`limits.rs`). An animal
//! this module voices is therefore always one the sim is simming, and the
//! cue's own radius culls most of those besides.
//!
//! # Which animal, and which register
//!
//! **The species is not passed in — it is read off the slot.** `mob::kind_of`
//! is pure and seedless and the wire deliberately carries no species field
//! (`sim-core/src/mob.rs`, protocol v29 refused one on cost), so the roster
//! slot *is* the species and a caller cannot hand this module the wrong one.
//! Until 2026-08-14 this file was `sound/pig.rs` and the render half asked it
//! nothing at all, so every wolf on the island snorted.
//!
//! The wolf has two registers and the pig has one. The pig's single voice is
//! not an omission: a boar's snort is the same call at any range, and the
//! `near` argument is ignored for it by [`cue_of`] rather than by a caller
//! remembering to. What the wolf's two registers are **not** is an aggression
//! read — see [`Cue::Growl`], which states in full what the client can and
//! cannot see. The register is distance, and distance is the honest half:
//! a growl is a close-range threat call and a howl is a long-range contact
//! call, which is what the two vocalizations are.

use sim_core::limits::MAX_MOBS;
use sim_core::mob;

use super::{Cue, CUES};

/// Mean seconds between one pig's snorts (`DECISIONS.md` §open,
/// "pig voice v0"). Not measured against anything — an opening value in
/// `CONTENT.md`'s sense, like every number in `sound/mod.rs`.
pub const SNORT_PERIOD_S: f32 = 9.0;

/// Mean seconds between one wolf's howls (`DECISIONS.md` §open, "wolf voice
/// v0"). **The rarest voice in the roster by an order of magnitude**, and
/// that is sourced rather than thrifty: real packs howl in bouts minutes to
/// hours apart, and a howl every ten seconds is the metronome failure at a
/// different frequency.
///
/// **Halved from the sourced figure, and the reason is a mechanism we do not
/// have** (`reference/BALANCE.md` §6.2's admissible kind of difference). The
/// research's ~165 s is the interval between *pack bouts* — several animals
/// answering each other, once. We have no bout: every wolf on the roster
/// keeps its own clock, so per-animal and per-island rates are the same
/// number divided by however many are in earshot at 88 m, and the island
/// would go quiet for minutes at a time. 75 s at ±50 % is 37–113 s per wolf.
/// It is a knob and it wants a person with ears (`NOW.md` §0pr).
pub const HOWL_PERIOD_S: f32 = 75.0;

/// Mean seconds between one wolf's growls, once it is inside growl range.
/// The shortest period here, for the opposite reason to the howl's: a growl
/// is not ambience, it is the animal that is about to reach you, and a
/// silence in the middle of that reads as the wolf having lost interest.
pub const GROWL_PERIOD_S: f32 = 2.5;

/// How far an interval may wander from its mean, as a fraction: each draw
/// lands in `PERIOD × [1−J, 1+J]`, so 4.5–13.5 s for a snort at the defaults.
///
/// One jitter for all three voices. The shape is the same argument every
/// time — a period a player can count along with is a period they stop
/// hearing — and it is not a number that wants a per-species opinion.
pub const VOICE_JITTER: f32 = 0.5;

/// Per-slot countdowns. One state for the whole roster because the wire's
/// mob ids name fixed roster slots (`mob::slot_of_id`), so a fixed array is
/// exact — no map, no allocation, wall 4's shape for free.
pub struct Voices {
    /// Seconds until the slot's next call; negative = never seen, primed
    /// silently on first sight (the `Steps`/`Waterline` pattern — the first
    /// observation establishes state and sounds like nothing, so a world
    /// join is not sixty-four animals clearing their throats at once).
    next: [f32; MAX_MOBS],
    /// Which interval draw the slot is on — the hash input that makes each
    /// interval its own.
    cycle: [u32; MAX_MOBS],
    /// Which register the slot's *current* countdown was drawn in, so a
    /// change can be noticed. Meaningless for a pig, which has one register
    /// and therefore never changes it.
    near: [bool; MAX_MOBS],
}

impl Default for Voices {
    fn default() -> Self {
        Self {
            next: [-1.0; MAX_MOBS],
            cycle: [0; MAX_MOBS],
            near: [false; MAX_MOBS],
        }
    }
}

impl Voices {
    /// Advance one drawn animal's clock by `dt_s`, and say what it says on
    /// the frame its call lands. `near` is the caller's distance test — see
    /// [`switch_m`], which is the only number it may test against.
    ///
    /// **Assign on fire, never accumulate**: a frame hitch that swallowed
    /// two intervals produces one call and a fresh countdown, not a banked
    /// burst — the same rule the step odometer states for a ten-metre
    /// hitch.
    ///
    /// Returning the cue rather than a `bool` is deliberate: the species and
    /// the register are decided in exactly one place ([`cue_of`]), so the
    /// render half cannot pick a different sound than the one this clock was
    /// timed for.
    pub fn due(&mut self, slot: usize, near: bool, dt_s: f32) -> Option<Cue> {
        if slot >= MAX_MOBS {
            return None;
        }
        let cue = cue_of(slot, near);
        if self.next[slot] < 0.0 {
            // First sight: prime silently, phase-offset by the slot's own
            // first draw so a herd walked into does not speak in unison.
            self.next[slot] = interval(slot, 0, cue);
            self.cycle[slot] = 1;
            self.near[slot] = near;
            return None;
        }
        if near != self.near[slot] {
            // The register changed under a countdown drawn for the other
            // one. **Shorten only, never extend.** A wolf closing on you
            // carries a howl's countdown — up to 60 s — and waiting that out
            // would make the one moment this cue exists for the one moment
            // it is silent. Walking back out of range must not stretch a
            // call that was nearly due, which is what an unconditional
            // redraw would do.
            self.near[slot] = near;
            let fresh = interval(slot, self.cycle[slot], cue);
            self.next[slot] = self.next[slot].min(fresh);
        }
        self.next[slot] -= dt_s;
        if self.next[slot] > 0.0 {
            return None;
        }
        self.next[slot] = interval(slot, self.cycle[slot], cue);
        self.cycle[slot] = self.cycle[slot].wrapping_add(1);
        Some(cue)
    }

    /// Forget every clock — leaving a world. A stale countdown carried into
    /// the next island would voice its animals on the last island's schedule.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// What the animal in this roster slot says at this range.
///
/// The pig ignores `near` — one call at every distance. The wolf does not,
/// and the boundary is [`switch_m`].
pub fn cue_of(slot: usize, near: bool) -> Cue {
    match mob::kind_of(slot) {
        mob::MOB_WOLF => {
            if near {
                Cue::Growl
            } else {
                Cue::Howl
            }
        }
        _ => Cue::Snort,
    }
}

/// The distance at which a wolf stops howling and starts growling, metres.
///
/// **Not a knob** — it is [`Cue::Growl`]'s own `radius_m`, read back out of
/// the one table that already had to have an opinion about how far a growl
/// carries. Deriving it means the two can never disagree, and the two ways
/// they could disagree are both bugs a player would hear: a switch wider than
/// the radius picks a growl the mixer then culls (a wolf that goes silent as
/// it closes), and a switch narrower than it leaves a band where a howl plays
/// from an animal near enough to bite you.
pub fn switch_m() -> f32 {
    CUES[Cue::Growl.idx()].radius_m
}

/// The slot's `cycle`-th interval for a given call, seconds: the cue's mean
/// period scaled into `[1−J, 1+J]` by a hash of (slot, cycle).
fn interval(slot: usize, cycle: u32, cue: Cue) -> f32 {
    let h = hash01(slot as u32, cycle);
    period_s(cue) * (1.0 - VOICE_JITTER + 2.0 * VOICE_JITTER * h)
}

/// The mean seconds between two of this call.
///
/// The wildcard arm is the pig, and it is safe by construction rather than by
/// hope: [`cue_of`] is the only thing that produces a cue here and it returns
/// exactly three, which `tests/sound.rs` pins.
pub fn period_s(cue: Cue) -> f32 {
    match cue {
        Cue::Howl => HOWL_PERIOD_S,
        Cue::Growl => GROWL_PERIOD_S,
        _ => SNORT_PERIOD_S,
    }
}

/// A uniform draw in [0, 1) from two integers. The same integer-mixing
/// register as `synth`'s seeding (splitmix-style multiplies), self-contained
/// so the sound model stays free of anyone else's stream.
///
/// Public because it is now the house convention for per-animal variation,
/// not only the voice's: the gait's phase offset (`render/mobs.rs::Gait`)
/// draws from the same well, so one roster slot's walk and snort are both
/// its own and neither needs a second hash to forget to match this one.
pub fn hash01(slot: u32, cycle: u32) -> f32 {
    let mut x = slot
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add(cycle.wrapping_mul(0x85EB_CA6B));
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    // Top 24 bits: exact in f32, uniform, and never 1.0.
    (x >> 8) as f32 / (1u32 << 24) as f32
}
