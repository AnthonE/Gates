//! The client's mirror of the bow's draw (`sim_core::ranged::draw`,
//! `reference/PROJECTILES.md` §6): when the bow in hand is ready to loose,
//! and how far drawn it should look on the way there.
//!
//! The sim's rule, restated in seconds: a bow looses only from a full draw,
//! and it is full `draw` after the aim began or `nock` after the last loose,
//! whichever is later. Nothing here decides a shot — the sim does — but the
//! viewmodel reaching full draw on the frame the sim would let it loose is
//! what makes the click at full draw the click that fires.

/// The draw clock. Seconds are the frame clock's (`Time::elapsed_secs`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DrawClock {
    /// When the aim now held began, or `None` while the bow is relaxed.
    aim_since: Option<f32>,
    /// When the last loose was, or `None` if there has not been one.
    loosed_at: Option<f32>,
}

impl DrawClock {
    /// Advance to `now` with the aim held or not, and return how far drawn
    /// the bow is, 0..=1 — full exactly when [`DrawClock::ready`] turns
    /// true. A relaxed bow is at 0 and forgets its aim.
    pub fn step(&mut self, now: f32, aiming: bool, draw_s: f32, nock_s: f32) -> f32 {
        if !aiming {
            self.aim_since = None;
            return 0.0;
        }
        let since = *self.aim_since.get_or_insert(now);
        let loosed = self.loosed_at.unwrap_or(f32::NEG_INFINITY);
        // The draw runs from the later of the aim and the loose to the
        // moment the sim will let it loose.
        let start = since.max(loosed);
        let ready = (since + draw_s).max(loosed + nock_s);
        if ready <= start {
            return 1.0;
        }
        ((now - start) / (ready - start)).clamp(0.0, 1.0)
    }

    /// Is the bow drawn full at `now`? Only while aiming.
    pub fn ready(&self, now: f32, draw_s: f32, nock_s: f32) -> bool {
        let Some(since) = self.aim_since else {
            return false;
        };
        let loosed = self.loosed_at.unwrap_or(f32::NEG_INFINITY);
        now >= (since + draw_s).max(loosed + nock_s)
    }

    /// The arrow left at `now`: the next draw is full a nock later.
    pub fn loose(&mut self, now: f32) {
        self.loosed_at = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAW: f32 = 1.0;
    const NOCK: f32 = 2.0;

    #[test]
    fn a_draw_is_full_a_draw_after_the_aim() {
        let mut c = DrawClock::default();
        assert_eq!(c.step(10.0, true, DRAW, NOCK), 0.0);
        assert!((c.step(10.5, true, DRAW, NOCK) - 0.5).abs() < 1e-6);
        assert!(!c.ready(10.99, DRAW, NOCK));
        assert_eq!(c.step(11.0, true, DRAW, NOCK), 1.0);
        assert!(c.ready(11.0, DRAW, NOCK));
    }

    #[test]
    fn letting_go_throws_the_draw_away() {
        let mut c = DrawClock::default();
        c.step(0.0, true, DRAW, NOCK);
        c.step(0.9, true, DRAW, NOCK);
        assert_eq!(c.step(1.0, false, DRAW, NOCK), 0.0);
        assert!(!c.ready(1.0, DRAW, NOCK));
        c.step(1.2, true, DRAW, NOCK);
        assert!(
            !c.ready(2.1, DRAW, NOCK),
            "the draw restarted at the new aim"
        );
        assert!(c.ready(2.2, DRAW, NOCK));
    }

    /// Held through a loose, the next draw is full a nock after it — the
    /// sim's cadence, not a second draw.
    #[test]
    fn a_draw_held_through_a_loose_is_full_when_the_nock_ends() {
        let mut c = DrawClock::default();
        c.step(0.0, true, DRAW, NOCK);
        assert!(c.ready(1.0, DRAW, NOCK));
        c.loose(1.0);
        assert_eq!(c.step(1.0, true, DRAW, NOCK), 0.0);
        assert!((c.step(2.0, true, DRAW, NOCK) - 0.5).abs() < 1e-6);
        assert!(!c.ready(2.9, DRAW, NOCK));
        assert!(c.ready(3.0, DRAW, NOCK));
    }

    /// Aimed again after the nock, it is a plain draw from the new aim.
    #[test]
    fn a_late_aim_after_a_loose_is_a_plain_draw() {
        let mut c = DrawClock::default();
        c.step(0.0, true, DRAW, NOCK);
        c.loose(1.0);
        c.step(1.1, false, DRAW, NOCK);
        c.step(5.0, true, DRAW, NOCK);
        assert!(!c.ready(5.9, DRAW, NOCK));
        assert!(c.ready(6.0, DRAW, NOCK));
    }
}
