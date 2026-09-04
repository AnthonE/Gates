//! Being hurt: what the mixer is told when the health bar moves.
//!
//! **The fall is the coverage, the event is the detail, and having those the
//! wrong way round is the whole trap in this file.** Seven damage routes reach
//! the sim's funnel, and `sim-core/tests/damage_routes.rs`'s `ROUTES` table
//! splits them in two. Four *announce*: melee, a shot, a blast and a bite push
//! [`EV_HURT`](sim_core::world::EV_HURT), so the client learns a bearing, a
//! damage and a hit count. Three are *silent on purpose* and that table says
//! why, per row: starving, dehydrating and the keypad shock have no direction
//! to point a player at, so inventing one would be a lie rather than a
//! feature.
//!
//! So the obvious edit — "read `EV_HURT` instead of watching `core.hp`" —
//! makes three routes inaudible with every gate in the tree green. That is the
//! same shape as the eleven days the bite and the blast debited hp and told
//! the body nothing (`ROUTES`' own doc records it): nothing could have caught
//! it, because the event queue is outside `state_hash` and no encoder ever saw
//! a missing message.
//!
//! [`request`] refuses that **by construction** rather than by a gate that has
//! to be remembered: it takes the fall *and* the announced blows, and a fall on
//! its own is still a sound. A route added next year cannot opt out of debiting
//! hp — that is what the funnel is — so it cannot opt out of being heard. The
//! event is only ever allowed to make the sound *better*, never to be the
//! reason there is one.
//!
//! Two facts the mixer did not have before, both from the announced side:
//!
//! - **How hard.** Every hurt in the tree played at the cue's full 0.80,
//!   whether it was a starve tick or a headshot. The weight is the frame's
//!   damage against the player's own `hp_max` — the server's number, not a
//!   constant here — so a solid melee blow still plays at 0.80 and lighter
//!   things are quieter. The ceiling does not move; only the floor of the
//!   range opens up.
//! - **A blow armor ate completely was silent.** `hp` does not fall, so the
//!   old fall-watcher had nothing to see, and being shot at while wearing a
//!   chest plate made no sound at all. An announced blow is audible whether or
//!   not it cost anything, which is the point of hearing it.
//!
//! What this does **not** do, and `NOW.md` §0hrt item 1 keeps: the cue is still
//! non-positional, so the bearing `Feed` merges per sector is read by the arc
//! and not by the mixer, and [`Cue::Hurt`]'s 120 ms cooldown still means two
//! blows in one frame start one voice. They start a *heavier* one now — the
//! damage is summed before the weight is taken — but that is a different thing
//! from two sounds.

use super::mixer::Request;
use super::Cue;

/// The share of a full health bar one frame's damage must reach before the
/// hurt cue plays at full weight (`DECISIONS.md` §open, "hurt weight v0").
///
/// Not a taste call in isolation: `content/weapons.toml` prices every melee
/// row at 20–35 damage against `player_hp = 100` (its own §comment states the
/// band), so a third of the bar puts the heaviest ordinary swing at the
/// ceiling, spreads the rest of the table across the live part of the curve,
/// and leaves headshots — `headshot_mult` doubles — pinned at full.
pub const HURT_FULL_FRAC: f32 = 0.35;

/// The quietest a hurt is allowed to get, as a share of the cue's own gain.
///
/// A floor rather than a taper to nothing, for a mechanical reason as well as
/// a design one: [`super::mixer::Mixer`] refuses any request whose composed
/// gain is not `> 0.0`, so a weight that reached zero would not be a very
/// quiet hurt, it would be a **silent** one — the exact failure the rest of
/// this module is built to make impossible. The design half is
/// `reference/AUDIO.md` §8's published order, which puts taking damage above
/// everything else a player hears: a metabolic tick may be small, never gone.
pub const HURT_MIN_GAIN: f32 = 0.40;

/// How loud a frame's damage is, against the player's own health ceiling.
///
/// `hp_max == 0` is "the server has not said yet" (`render/hud.rs` handles the
/// same case for the vitals bar) — weigh it at full rather than divide, since
/// the safe direction for a cue that must never be missed is loud.
pub fn weight(damage: u16, hp_max: u16) -> f32 {
    let full = hp_max as f32 * HURT_FULL_FRAC;
    if full <= 0.0 {
        return 1.0;
    }
    (damage as f32 / full).clamp(HURT_MIN_GAIN, 1.0)
}

/// What the mixer should be told this frame, or nothing.
///
/// `fall` is how far `core.hp` dropped since the last frame — **every** route
/// produces it, which is why it is first and why it alone is enough. The two
/// `announced_*` arguments are this frame's `EV_HURT` blows, already merged by
/// `render::feed::Feed`.
///
/// The damage taken is `max(announced, fall)` rather than either one:
///
/// - normally they agree, and it does not matter which is read;
/// - when armor ate the blow, `fall` is 0 and the event is the only witness;
/// - when a silent route fired, the event is 0 and the fall is the only one.
///
/// A sum would double-count the ordinary case, which is every case but two.
pub fn request(
    fall: u16,
    announced_damage: u16,
    announced_hits: u16,
    hp_max: u16,
) -> Option<Request> {
    if fall == 0 && announced_hits == 0 {
        return None;
    }
    let damage = announced_damage.max(fall);
    Some(Request::own(Cue::Hurt).with_gain(weight(damage, hp_max)))
}
