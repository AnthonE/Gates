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
//! whether a swing happened, and the client's own audio cue has taken this
//! same liberty since audio v0 (`render::input` plays `Cue::Swing` on the
//! press). What is new is only that the *picture* now agrees with the
//! sound.
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
}

impl SwingCadence {
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
        self.next = 0.0;
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
