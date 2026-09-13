//! The sentences of being down (wounded v0) — pure, so a headless test can
//! read them. `render/wounded.rs` draws them; `render/hud.rs` toasts them.
//!
//! Two numbers and no more, because two is what the reference put on the
//! screen when it finally put anything there (`reference/WOUNDED.md` §2.5,
//! August 2023: *"the probability of recovery as well as how many seconds
//! are left"*). The clock is in whole seconds — a tenth would read as a
//! stopwatch and the decision it informs (crawl for the door or wait) is
//! not made at that resolution.

use sim_core::limits::TICK_HZ;

/// Per mille to a whole percent, rounded to nearest.
pub fn pct(chance_pm: u16) -> u16 {
    (chance_pm + 5) / 10
}

/// Ticks to whole seconds, rounded up so the readout never says `0 s` while
/// a tick is still owed.
pub fn secs(ticks: u16) -> u16 {
    ticks.div_ceil(TICK_HZ as u16)
}

/// The toast on the way down.
pub fn down_line(ticks: u16, chance_pm: u16) -> String {
    format!(
        "you are down - {} s to the roll, {}% to get up",
        secs(ticks),
        pct(chance_pm)
    )
}

/// The toast on the way up.
pub fn up_line(chance_pm: u16, hp: u16) -> String {
    format!("you got up - beat {}%, {hp} hp", pct(chance_pm))
}

/// The standing line under the vignette while down. `secs_left` is the
/// client's own countdown from the fall; it is clamped at zero rather than
/// allowed to read negative while the sim's roll is a packet away.
pub fn readout(secs_left: f32, chance_pm: u16) -> String {
    let s = secs_left.max(0.0).ceil() as u32;
    format!("WOUNDED   {s} s   {}% to get up", pct(chance_pm))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_mille_reads_as_a_whole_percent() {
        assert_eq!(pct(200), 20);
        assert_eq!(pct(450), 45);
        assert_eq!(pct(325), 33, "rounded to nearest, not floored");
        assert_eq!(pct(0), 0);
    }

    #[test]
    fn the_clock_rounds_up_to_the_second_it_is_inside() {
        assert_eq!(secs(1_200), 40);
        assert_eq!(secs(1_500), 50);
        assert_eq!(secs(1_201), 41, "a tick into the 41st second is 41");
        assert_eq!(secs(1), 1, "never zero while a tick is owed");
    }

    #[test]
    fn the_lines_carry_both_numbers() {
        assert_eq!(
            down_line(1_350, 325),
            "you are down - 45 s to the roll, 33% to get up"
        );
        assert_eq!(up_line(287, 10), "you got up - beat 29%, 10 hp");
        assert_eq!(readout(40.2, 200), "WOUNDED   41 s   20% to get up");
        assert_eq!(
            readout(-0.5, 450),
            "WOUNDED   0 s   45% to get up",
            "clamped, never negative"
        );
    }
}
