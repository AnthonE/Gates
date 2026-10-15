//! The moon's month (`world::moon_phase`): a pure function of the day clock,
//! one cycle every `MOON_DAYS`, full on day 0's midnight, new half a month
//! on, and never a jump between one tick and the next.

use sim_core::limits::{DAY_PHASE_TICKS, DAY_PORTION, DAY_TICKS, MOON_DAYS};
use sim_core::world::{day_frac, moon_phase};

const MONTH: u64 = DAY_TICKS * MOON_DAYS;

/// The tick of day `day`'s midnight, halfway through its night, checked
/// against the day clock rather than trusted.
fn midnight(day: u64) -> u64 {
    let want = (1.0 + DAY_PORTION) * 0.5;
    let t = day * DAY_TICKS + (DAY_TICKS as f32 * want) as u64 - DAY_PHASE_TICKS;
    let off = day_frac(t) - want;
    assert!(
        off < 1e-6 && off > -1e-6,
        "day {day}'s midnight is {off} off"
    );
    t
}

#[test]
fn the_month_repeats_every_moon_days_and_no_sooner() {
    for t in [0, 1, 77_777, DAY_TICKS * 3 + 5, MONTH - 1, 10_000_000] {
        let p = moon_phase(t);
        assert!((0.0..1.0).contains(&p), "phase {p} at {t}");
        assert_eq!(p, moon_phase(t + MONTH), "the month does not repeat at {t}");
    }
    let first = moon_phase(midnight(0));
    for day in 1..MOON_DAYS {
        assert_ne!(
            moon_phase(midnight(day)),
            first,
            "day {day} wears day 0's moon"
        );
    }
}

#[test]
fn full_on_the_first_midnight_and_new_half_a_month_on() {
    assert_eq!(moon_phase(midnight(0)), 0.5, "day 0's midnight is not full");
    assert_eq!(
        moon_phase(midnight(MOON_DAYS / 2)),
        0.0,
        "half a month on is not new"
    );
    assert_eq!(moon_phase(midnight(MOON_DAYS)), 0.5);
    // Waning after the full moon, waxing toward the next.
    assert!(moon_phase(midnight(1)) > 0.5);
    assert!(moon_phase(midnight(MOON_DAYS - 1)) < 0.5);
}

/// One tick moves the phase by one tick of the month, across dawn and the
/// month's turn at new moon too: the moon never steps, so the night's light
/// never does.
#[test]
fn the_phase_runs_on_without_a_step() {
    let dawn = DAY_TICKS - DAY_PHASE_TICKS;
    let new_moon = midnight(MOON_DAYS / 2);
    for t in [
        dawn - 1,
        dawn,
        midnight(2),
        new_moon - 1,
        new_moon,
        MONTH * 3 - 7,
    ] {
        let mut d = moon_phase(t + 1) - moon_phase(t);
        if d < 0.0 {
            d += 1.0;
        }
        // In ticks of the month: one, not none and not a jump.
        let ticks = d * MONTH as f32;
        assert!(
            ticks > 0.5 && ticks < 1.5,
            "the phase moved {ticks} ticks at {t}"
        );
    }
}
