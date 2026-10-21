//! A use that takes its time in the hands, on this side: Rust's
//! `MedicalTool`, the bandage's wrap (`ItemRow::use_ticks`, wire v135).
//!
//! The sim lands it on its tick and cancels it when the hand that began it
//! is put away (`sim_core::survival::tick_use`). This is the clock the
//! viewmodel draws the wrap off and the HUD counts down, started by the click
//! that sends `ACT_CONSUME` and put away on the same rule the sim uses, so
//! the arm stops wrapping on the frame the player switches.

/// How many times the hand goes round the forearm in one wrap.
pub const WRAP_TURNS: f32 = 3.0;
/// The share of the wrap the left arm takes to come up into view, and to go
/// back down at its end.
pub const WRAP_RAISE: f32 = 0.14;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Run {
    t0: f64,
    secs: f64,
    slot: u8,
    item: u16,
}

/// The wrap under way, if there is one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WrapClock {
    run: Option<Run>,
}

impl WrapClock {
    /// A wrap of `item` in hotbar `slot` starts at `now` and lands `secs`
    /// later.
    pub fn begin(&mut self, now: f64, secs: f64, slot: u8, item: u16) {
        self.run = Some(Run {
            t0: now,
            secs: secs.max(1e-3),
            slot,
            item,
        });
    }

    /// One frame: the wrap ends when it has run its time, and is put away
    /// when the hand up is not the slot it began in (`sel`) or that slot no
    /// longer holds the item (`held`, the slot's item and count).
    pub fn step(&mut self, now: f64, sel: u8, held: (u16, u16)) {
        let Some(r) = self.run else { return };
        if now - r.t0 >= r.secs || sel != r.slot || held.1 == 0 || held.0 != r.item {
            self.run = None;
        }
    }

    /// Put the wrap away (the body went down, the screen changed).
    pub fn cancel(&mut self) {
        self.run = None;
    }

    /// How far through the wrap `now` is, 0..1; `None` with none under way.
    pub fn progress(&self, now: f64) -> Option<f32> {
        self.run
            .map(|r| ((now - r.t0) / r.secs).clamp(0.0, 1.0) as f32)
    }

    /// Seconds until it lands.
    pub fn left(&self, now: f64) -> Option<f64> {
        self.run.map(|r| (r.secs - (now - r.t0)).max(0.0))
    }
}

/// How far up the left arm is at progress `p`: up over the first
/// [`WRAP_RAISE`], held, and down over the last.
pub fn raise(p: f32) -> f32 {
    let s = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    s(p / WRAP_RAISE) * s((1.0 - p) / WRAP_RAISE)
}

/// How far round the forearm the right hand has gone at progress `p`, in
/// turns: the arm comes up before the hand starts round, and it is finished
/// before the arm goes down.
pub fn turns(p: f32) -> f32 {
    let span = 1.0 - 2.0 * WRAP_RAISE;
    ((p - WRAP_RAISE) / span).clamp(0.0, 1.0) * WRAP_TURNS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrap_runs_its_time_and_is_put_away_with_the_hand() {
        let mut w = WrapClock::default();
        w.begin(10.0, 4.0, 2, 7);
        w.step(12.0, 2, (7, 3));
        assert_eq!(w.progress(12.0), Some(0.5));
        assert!(raise(0.5) == 1.0 && raise(0.0) == 0.0 && raise(1.0) == 0.0);
        assert_eq!(turns(1.0 - WRAP_RAISE), WRAP_TURNS);
        w.step(12.5, 3, (7, 3));
        assert_eq!(w.progress(12.5), None, "switching hands puts it away");
        w.begin(20.0, 4.0, 2, 7);
        w.step(24.0, 2, (7, 3));
        assert_eq!(w.progress(24.0), None, "done");
    }
}
