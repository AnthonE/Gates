//! The hitmarker's voice: which rung the ladder priced your shot at.
//!
//! **Aim was unlearnable in this game until v58, and the mechanism was that
//! nothing carried the rung.** A shot pays x2, x1 or x0.5 depending on where
//! the line crossed a 1.7 m cylinder (`sim_core::collide::Part`), and
//! `EV_HIT` carried the *product* — so a leg hit from a strong weapon and a
//! chest hit from a weak one were the same number on the same marker with the
//! same click. A player could not tell a graze from a solid hit, which means
//! they could not practise the difference.
//!
//! The asymmetry is the part worth stating, because it decides which rung
//! this module works hardest on: **a halved number is easier to misread as a
//! miss than a doubled one is to read as a skull.** A headshot announces
//! itself by killing things faster whether or not the client says anything;
//! a leg hit just looks like bad luck. So [`Cue::HitLimb`] is deliberately
//! *not* the quiet end of a fade — it is its own audible symbol, at a gain
//! that brackets the identity rather than trailing off toward silence.
//!
//! Three rungs, three waveforms, and none of them is another one pitched:
//! `tests/sound.rs`'s `interface_cues_do_not_vary_in_pitch` keeps signal cues
//! from wobbling, and "the same click, higher" is the same drift that rule
//! exists to forbid. See `sound::synth`'s three `chime` arms.
//!
//! [`request`] is pure and takes no Bevy types, for `sound::hurt`'s reason:
//! the decision is testable headless and the caller in `render/audio.rs` is
//! four lines that cannot hold a judgement.

use super::mixer::Request;
use super::Cue;
use sim_core::collide::Part;

/// The cue a rung is heard as.
///
/// Total on `Option<Part>`, and `None` — a hit on a *wall*, which shares the
/// core's hitmarker ring — maps to the identity [`Cue::Hit`]. That is the
/// right answer rather than a fallback: a structure has no head and no legs,
/// it takes the unscaled blow, and the identity cue is exactly what the
/// player heard for a wall before there were rungs at all.
#[inline]
pub fn cue(part: Option<Part>) -> Cue {
    match part {
        Some(Part::Head) => Cue::HitHead,
        Some(Part::Limb) => Cue::HitLimb,
        Some(Part::Chest) | None => Cue::Hit,
    }
}

/// One marker per frame, at the rung the frame's best hit landed on.
///
/// `None` when nothing landed. `hits` is the count and not the damage on
/// purpose: a blow that armor ate whole still *landed*, and the marker is a
/// statement about the shooter's aim rather than about the victim's health —
/// which is the same split `EV_HIT` and `EV_HEALTH` have always had.
///
/// One request however many hits arrived, because `Cue::Hit`'s cooldown would
/// refuse the rest anyway and three rungs in one frame are still one marker.
/// Which one is already decided upstream: `Feed::hit_part` is a `max` over
/// the frame, so this receives the most significant part and never has to
/// merge anything itself.
#[inline]
pub fn request(hits: u16, part: Option<Part>) -> Option<Request> {
    if hits == 0 {
        return None;
    }
    Some(Request::own(cue(part)))
}
