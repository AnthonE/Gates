//! The watchdog that keeps a body moving.
//!
//! Every skill has its own way of noticing it is stuck (the route's stall
//! ladder, a door hop's stage clock, a goal's own timeout), and each covers
//! only the walk it knows about. This one asks the plain question from the
//! outside: has the goal been trying to move the body for
//! [`WEDGE_SECS`] while the body never left a [`WEDGE_BOX_M`] box? Then it is
//! stuck, whatever the skill thinks: the goal ends, the body backs off with
//! one jump, and the next choice starts from somewhere new.
//!
//! "Trying" is read off the intent (a travel bearing or move axes), so a goal
//! that stands still on purpose (a wait, a follow already beside its
//! teammate, a box panel open) is never called stuck. A bounding box rather
//! than net displacement, so pacing back and forth counts as stuck too.

use sim_core::limits::TICK_HZ;

/// How long trying without leaving the box is stuck.
pub const WEDGE_SECS: u32 = 30;
/// The box, per axis, metres.
pub const WEDGE_BOX_M: f32 = 1.0;
/// At least this share of the window's frames must have been trying.
pub const WEDGE_TRYING_PCT: u32 = 60;
/// The back-off after a trip: this many ticks walking away, a jump first.
pub const BACK_OFF_TICKS: u32 = 20;

#[derive(Clone, Copy, Debug, Default)]
pub struct Wedge {
    start: Option<u32>,
    lo: [f32; 2],
    hi: [f32; 2],
    frames: u32,
    trying: u32,
}

impl Wedge {
    /// Forget the window: a new goal, a fight, a death.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// One frame of a running goal: where the body stands (x, z) and whether
    /// the goal is trying to move it. True once a whole window of mostly
    /// trying kept it inside the box; the window starts again either way.
    pub fn observe(&mut self, tick: u32, at: [f32; 2], trying: bool) -> bool {
        let start = match self.start {
            Some(start) => start,
            None => {
                self.restart(tick, at);
                tick
            }
        };
        for i in 0..2 {
            self.lo[i] = self.lo[i].min(at[i]);
            self.hi[i] = self.hi[i].max(at[i]);
        }
        if self.hi[0] - self.lo[0] > WEDGE_BOX_M || self.hi[1] - self.lo[1] > WEDGE_BOX_M {
            // It got somewhere: the window starts from here.
            self.restart(tick, at);
        }
        self.frames += 1;
        self.trying += u32::from(trying);
        if tick.wrapping_sub(start) < WEDGE_SECS * TICK_HZ || self.start != Some(start) {
            return false;
        }
        let stuck = self.trying * 100 >= self.frames * WEDGE_TRYING_PCT;
        self.restart(tick, at);
        stuck
    }

    fn restart(&mut self, tick: u32, at: [f32; 2]) {
        *self = Self {
            start: Some(tick),
            lo: at,
            hi: at,
            frames: 0,
            trying: 0,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        w: &mut Wedge,
        from: u32,
        secs: u32,
        at: impl Fn(u32) -> [f32; 2],
        trying: bool,
    ) -> bool {
        let mut tripped = false;
        for t in from..from + secs * TICK_HZ {
            tripped |= w.observe(t, at(t), trying);
        }
        tripped
    }

    #[test]
    fn trying_in_place_for_the_window_trips_once() {
        let mut w = Wedge::default();
        // Pacing half a metre back and forth is still stuck.
        let pace = |t: u32| [100.0 + (t % 10) as f32 * 0.05, 50.0];
        assert!(
            !run(&mut w, 0, WEDGE_SECS - 1, pace, true),
            "not before the window"
        );
        assert!(run(&mut w, (WEDGE_SECS - 1) * TICK_HZ, 2, pace, true));
        // The window started again: a second trip needs another whole one.
        assert!(!run(&mut w, (WEDGE_SECS + 1) * TICK_HZ, 5, pace, true));
    }

    #[test]
    fn walking_standing_on_purpose_or_a_reset_never_trips() {
        let mut w = Wedge::default();
        let walk = |t: u32| [t as f32 * 0.02, 0.0];
        assert!(
            !run(&mut w, 0, 3 * WEDGE_SECS, walk, true),
            "metres every window"
        );
        let mut w = Wedge::default();
        assert!(
            !run(&mut w, 0, 2 * WEDGE_SECS, |_| [5.0, 5.0], false),
            "waiting"
        );
        let mut w = Wedge::default();
        assert!(!run(&mut w, 0, WEDGE_SECS - 5, |_| [5.0, 5.0], true));
        w.reset();
        assert!(!run(
            &mut w,
            (WEDGE_SECS - 5) * TICK_HZ,
            10,
            |_| [5.0, 5.0],
            true
        ));
    }
}
