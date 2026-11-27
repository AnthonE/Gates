//! When to draw a swing.
//!
//! ## The gap this closes
//!
//! `render::viewmodel` animated the arm off [`Feed`] alone — a landed hit
//! or a gather payout — and its header argued the case: *"the swing worth
//! drawing is the one that LANDED. Reading the button animates swings the
//! server refused."*
//!
//! That is right about refusals and wrong about **misses**, and a miss is
//! not a refusal. A rock swung at open air is a swing the sim fully
//! accepted: it paid the cadence, it ran `gather::swing` (a whiff — "the
//! cooldown is paid, the arm is free"), it ran `combat::strike` (nothing in
//! reach), and it announced nothing, because there is nothing to announce.
//! So the commonest swing in the game — the first one a new player makes,
//! at nothing, to find out what the button does — was the one swing that
//! drew no motion at all. The item sat still in frame and the game read as
//! broken.
//!
//! ## What replaces it
//!
//! The client predicts the sim's own cadence. It already knows the whole
//! rule, because there is only one: `gather::swing` takes the arm when
//! `BTN_PRIMARY` is down and `tick >= next_swing`, then pushes `next_swing`
//! out by [`SWING_INTERVAL_TICKS`]. That is four lines of arithmetic over a
//! tick estimate the client is already given
//! (`Feed::server_tick_est`), so this mirrors it rather than approximating
//! it with a wall-clock timer.
//!
//! **It decides nothing** (`RENDER.md` §1). No gameplay state hangs off
//! this — it picks a pose. The sim is still the only thing that says
//! whether a swing happened. The client's own audio cue takes the same
//! liberty from the same line: `render::viewmodel::animate` plays
//! `Cue::Swing` on the frame this predictor fires. It used to fire on the
//! mouse press (`render::input`, audio v0 → 2026-09-13), which agreed with
//! this predictor exactly as long as nobody clicked faster than the sim
//! swings — spam the button and the ear heard a whoosh per click over an
//! arm that moved once per interval. One trigger for both is the fix.
//!
//! ## Why it is here and not in the Bevy system
//!
//! This crate's standing rule: arithmetic in `crate::ui`, headless and
//! gated; nodes and handles in `render`. A cadence predictor can be
//! silently wrong in two directions — firing every frame (a wind-up that
//! never resolves) or never firing after the first press — and neither is
//! visible to any gate if it lives inside a system that needs a window.
//!
//! [`Feed`]: crate::render::feed::Feed
//! [`SWING_INTERVAL_TICKS`]: sim_core::gather::SWING_INTERVAL_TICKS

use sim_core::gather::SWING_INTERVAL_TICKS;
use sim_core::limits::MAX_ARROW_LIFE_TICKS;

/// The client's mirror of `Player::next_swing`.
///
/// Held across button releases **on purpose**, exactly as the sim holds
/// its own: click, release, click again 5 ticks later, and the sim eats
/// the second one on the cooldown. A predictor that reset on release would
/// draw a swing there and the player would watch their arm move through a
/// swing the world never took.
#[derive(Debug, Default)]
pub struct SwingCadence {
    /// The estimated server tick the next swing may be taken on. Zero
    /// before the first one, which is correct: `Player::next_swing` starts
    /// at zero too, so a fresh body's first press swings.
    next: f64,
    /// The estimated tick past which a spear this hand threw can no longer
    /// be in the air ([`MAX_ARROW_LIFE_TICKS`] after the throw), zero for
    /// none. A hit landing before it may be the spear's, not a swing's.
    flying: f64,
    /// The aim was held for a throw on the last [`Self::hurl`]: the sim's
    /// `Player::draw_on`, which says whether an aim carries on.
    aiming: bool,
}

impl SwingCadence {
    /// One frame with a throwable weapon in hand (`ranged::hurl`, the
    /// spear): `aim` is `BTN_AIM` as the sim will read it, `held` is
    /// `BTN_PRIMARY`, `windup` is the row's `hurl_ticks`. Mirrors the
    /// sim's two pushes of `next_swing`, since the swing reads that clock
    /// too: the aim's first tick puts the throw `windup` away (and keeps
    /// it there after a release, so a plain click just after an aborted
    /// windup is refused), and a throw pays a swing's interval (so a click
    /// on the empty hand after it is refused as well).
    ///
    /// Returns whether the throw went this frame. Call it, not
    /// [`Self::poll`], on a frame the aim is held: the click throws.
    pub fn hurl(&mut self, aim: bool, held: bool, tick_est: f64, windup: u8) -> bool {
        let began = aim && !self.aiming;
        self.aiming = aim;
        if !aim {
            return false;
        }
        if began {
            self.next = self.next.max(tick_est + f64::from(windup));
        }
        if !held || tick_est < self.next {
            return false;
        }
        self.next = tick_est + SWING_INTERVAL_TICKS as f64;
        self.flying = tick_est + f64::from(MAX_ARROW_LIFE_TICKS);
        true
    }

    /// May a hit landing now be a spear this hand threw? While one can
    /// still be in the air, a hit is no proof of a swing the predictor
    /// missed (the viewmodel's landed-at-rest backstop).
    pub fn spear_aloft(&self, tick_est: f64) -> bool {
        tick_est < self.flying
    }

    /// Should a swing be drawn this frame?
    ///
    /// `held` is the button as the input layer read it (`pressed`, not
    /// `just_pressed` — the sim swings again on a held button and so does
    /// this). `tick_est` is the client's smoothed server-tick estimate.
    ///
    /// Fires at most once per [`SWING_INTERVAL_TICKS`] of estimated server
    /// time, whatever the frame rate: the next window is measured from the
    /// tick the swing was taken on, not from the tick this happened to be
    /// called at, so a 144 fps client and a 30 fps one chop at the same
    /// speed.
    ///
    /// **A backwards clock waits rather than firing.** `server_est` is
    /// smoothed and can retreat a little; `>=` on a shrinking value simply
    /// stays false until it catches up, which costs at most one swing's
    /// worth of animation and can never double-fire.
    pub fn poll(&mut self, held: bool, tick_est: f64) -> bool {
        if !held || tick_est < self.next {
            return false;
        }
        self.next = tick_est + SWING_INTERVAL_TICKS as f64;
        true
    }

    /// Forget the cadence — a new session, a new body, a new clock.
    ///
    /// The tick estimate is zero until the first snapshot of a session and
    /// then jumps to the shard's own tick, which for a long-running shard
    /// is a large number. Without this, reconnecting to a *fresher* shard
    /// would leave `next` in the future and swallow every swing until real
    /// time caught up with a tick count from somewhere else.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IVL: f64 = SWING_INTERVAL_TICKS as f64;

    /// The whole point: a held button chops at the sim's cadence and not
    /// at the frame rate. Sixty polls across one interval is one swing.
    #[test]
    fn a_held_button_swings_once_per_interval_not_once_per_frame() {
        let mut c = SwingCadence::default();
        let mut fired = 0;
        // Two intervals of estimated server time, sampled 120 times — the
        // shape of a 60 fps client at a 30 Hz tick.
        for i in 0..120 {
            let t = i as f64 * (2.0 * IVL / 120.0);
            if c.poll(true, t) {
                fired += 1;
            }
        }
        assert_eq!(
            fired, 2,
            "a held button drew {fired} swings across two intervals — the \
             predictor is running on frames instead of on ticks"
        );
    }

    /// The first press of a session swings immediately. `Player::next_swing`
    /// starts at zero and so does this, so a fresh body does not stand there
    /// for a second and a bit wondering.
    #[test]
    fn the_first_press_swings_at_once() {
        let mut c = SwingCadence::default();
        assert!(c.poll(true, 0.0));
        assert!(!c.poll(true, 0.0), "one press drew two swings");
    }

    /// Releasing does not refund the cooldown, because the sim does not.
    /// This is the assertion that separates a mirror of `next_swing` from
    /// a per-click animation.
    #[test]
    fn letting_go_does_not_refund_the_cooldown() {
        let mut c = SwingCadence::default();
        assert!(c.poll(true, 100.0));
        assert!(!c.poll(false, 105.0), "a release is not a swing");
        assert!(
            !c.poll(true, 110.0),
            "click-release-click inside the cooldown drew a swing the sim ate"
        );
        assert!(c.poll(true, 100.0 + IVL), "the cooldown never lapsed");
    }

    /// An idle button never fires, however long the clock runs.
    #[test]
    fn a_button_nobody_pressed_never_swings() {
        let mut c = SwingCadence::default();
        for i in 0..1_000 {
            assert!(!c.poll(false, i as f64 * IVL));
        }
    }

    /// A smoothed estimate can retreat. Waiting is the only safe answer:
    /// never a second swing, and never a stall past the real window.
    #[test]
    fn a_clock_that_slips_backwards_waits_rather_than_double_firing() {
        let mut c = SwingCadence::default();
        assert!(c.poll(true, 1_000.0));
        assert!(
            !c.poll(true, 999.0),
            "a backwards clock drew a second swing"
        );
        assert!(!c.poll(true, 1_000.0 + IVL - 1.0));
        assert!(c.poll(true, 1_000.0 + IVL));
    }

    /// A shard whose tick is far ahead of the last one's must not swallow
    /// every swing until real time catches up.
    #[test]
    fn a_reset_lets_a_fresher_shard_swing_again() {
        let mut c = SwingCadence::default();
        assert!(c.poll(true, 9_000_000.0));
        // The next session's clock starts at zero and climbs.
        c.reset();
        assert!(
            c.poll(true, 0.0),
            "a reconnect after a long-running shard left the arm on a cooldown \
             measured in somebody else's ticks"
        );
    }

    /// The spear's two pushes (`ranged::hurl`): an aim let go before its
    /// windup still holds a plain click until the windup is up, and a
    /// throw pays a swing's interval, so the empty hand after it does not
    /// punch at once. A hit while the spear can be aloft is not a swing's.
    #[test]
    fn a_throw_holds_the_arm_as_the_sim_does() {
        const WINDUP: u8 = 15;
        let w = f64::from(WINDUP);
        // Aim held 5 ticks of a 15-tick windup, then let go.
        let mut c = SwingCadence::default();
        assert!(!c.hurl(true, false, 100.0, WINDUP));
        assert!(!c.hurl(true, false, 105.0, WINDUP));
        assert!(!c.hurl(false, false, 106.0, WINDUP));
        assert!(!c.poll(true, 107.0), "a click inside the aborted windup");
        assert!(c.poll(true, 100.0 + w), "the windup lapsed");

        // A full windup, then the click throws.
        let mut c = SwingCadence::default();
        assert!(
            !c.hurl(true, true, 200.0, WINDUP),
            "thrown before the windup"
        );
        assert!(!c.hurl(true, true, 200.0 + w - 1.0, WINDUP));
        assert!(c.hurl(true, true, 200.0 + w, WINDUP), "the throw");
        assert!(!c.hurl(true, true, 201.0 + w, WINDUP), "two throws");
        assert!(c.spear_aloft(210.0 + w));
        // The hand is empty now: no aim, a held click.
        assert!(!c.hurl(false, true, 202.0 + w, WINDUP));
        assert!(!c.poll(true, 202.0 + w), "the empty hand punched at once");
        assert!(c.poll(true, 200.0 + w + IVL), "the interval lapsed");
        assert!(!c.spear_aloft(200.0 + w + f64::from(MAX_ARROW_LIFE_TICKS)));
    }

    /// One long stall is one swing, not the dozen the elapsed ticks would
    /// buy. The window is measured from the swing taken, so a hitch cannot
    /// bank arm-swings and spend them all in one frame.
    #[test]
    fn a_stall_does_not_bank_swings() {
        let mut c = SwingCadence::default();
        assert!(c.poll(true, 0.0));
        assert!(c.poll(true, 50.0 * IVL), "the swing after the stall");
        assert!(
            !c.poll(true, 50.0 * IVL),
            "the stall banked a second swing for the same frame"
        );
    }
}
