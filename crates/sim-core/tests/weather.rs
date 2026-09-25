//! Weather v0: the schedule is a pure function of the seed and the tick,
//! never jumps, rolls its presets at their weights, and the admin's force
//! and clock land where they say.

use sim_core::limits::DAY_TICKS;
use sim_core::weather::{self, Env, Wx, FADE_TICKS, FORCE_FADE_TICKS, SEG_TICKS};
use sim_core::world::day_frac;

fn fields(w: &Wx) -> [u16; 6] {
    [w.cloud, w.dark, w.rain, w.fog, w.wind, w.thunder]
}

#[test]
fn the_schedule_is_a_function_of_seed_and_tick() {
    for tick in [0, 1, 12_345, SEG_TICKS * 7 + 99, 10_000_000] {
        assert_eq!(weather::auto(42, tick), weather::auto(42, tick));
    }
    // Two seeds disagree somewhere in a day's worth of segments.
    let differs = (0..32u64).any(|s| {
        weather::auto(1, s * SEG_TICKS + FADE_TICKS) != weather::auto(2, s * SEG_TICKS + FADE_TICKS)
    });
    assert!(differs, "two seeds share one sky");
}

#[test]
fn the_sky_never_jumps_between_ticks() {
    // Across several segment edges, including the fades, no per-mille field
    // moves more than a smoothstep's steepest slope allows in one tick.
    let seed = 0x5EED;
    let max_step = 1000 * 3 / 2 / FADE_TICKS + 2;
    let mut prev = weather::auto(seed, 0);
    for tick in 1..(SEG_TICKS * 6) {
        let w = weather::auto(seed, tick);
        for (a, b) in fields(&prev).into_iter().zip(fields(&w)) {
            assert!(
                (a as i64 - b as i64).unsigned_abs() <= max_step,
                "a field jumped {a} -> {b} at tick {tick}"
            );
        }
        assert!(w.in_range());
        prev = w;
    }
}

#[test]
fn presets_roll_at_their_weights() {
    let mut n = [0u32; 7];
    let segs = 20_000u64;
    for seg in 0..segs {
        n[weather::preset_of(77, seg) as usize] += 1;
    }
    let want = [0, 55, 12, 7, 13, 8, 5];
    for code in 1..=6 {
        let pct = n[code] as f64 * 100.0 / segs as f64;
        assert!(
            (pct - want[code] as f64).max(want[code] as f64 - pct) < 3.0,
            "{} rolled {pct:.1}% against {}%",
            weather::preset_name(code as u8),
            want[code]
        );
    }
}

#[test]
fn a_forced_preset_fades_in_then_holds() {
    let seed = 9;
    let mut env = Env::default();
    let at = 1_000_000;
    let before = weather::now(seed, at, &env);
    env.force(seed, at, weather::STORM);
    // The tick of the order still shows the sky it was given.
    assert_eq!(weather::now(seed, at, &env), before);
    let storm = weather::now(seed, at + FORCE_FADE_TICKS, &env);
    assert_eq!(storm.rain, 1000);
    assert_eq!(storm.thunder, 1000);
    assert_eq!(
        weather::now(seed, at + FORCE_FADE_TICKS * 50, &env).rain,
        1000
    );
    // Halfway, it is on its way.
    let mid = weather::now(seed, at + FORCE_FADE_TICKS / 2, &env);
    assert!(mid.rain > before.rain && mid.rain < 1000);
    // Handing it back fades to the schedule.
    env.force(seed, at + 5_000, weather::MODE_AUTO);
    let back = at + 5_000 + FORCE_FADE_TICKS;
    assert_eq!(weather::now(seed, back, &env), weather::auto(seed, back));
    assert!(env.valid());
}

#[test]
fn set_time_lands_the_day_clock_where_asked() {
    let mut env = Env::default();
    for tick in [0u64, 17, 99_999, 1_234_567] {
        for frac_pm in [0u16, 250, 437, 855, 937, 999] {
            env.set_time(tick, frac_pm);
            assert!((env.day_offset as u64) < DAY_TICKS);
            let f = day_frac(weather::day_tick(tick, &env));
            assert!(
                sim_core::fmath::fabs(f - frac_pm as f32 / 1000.0) < 1e-4,
                "asked {frac_pm}‰ at {tick}, got {f}"
            );
        }
    }
}

#[test]
fn fog_and_rain_shorten_an_animals_reach() {
    let clear = weather::preset(weather::CLEAR, 0);
    let fog = weather::preset(weather::FOG, 0);
    let storm = weather::preset(weather::STORM, 0);
    assert_eq!(weather::sense_pm(&clear), 1000);
    assert!(weather::sense_pm(&fog) < 700);
    assert!(weather::sense_pm(&storm) >= 400);
}

#[test]
fn lightning_is_the_same_bolt_everywhere_and_only_in_thunder() {
    let mut bolts = 0;
    for sec in 0..10_000u64 {
        assert_eq!(weather::bolt(3, sec, 1000), weather::bolt(3, sec, 1000));
        assert_eq!(weather::bolt(3, sec, 0), None);
        if weather::bolt(3, sec, 1000).is_some() {
            bolts += 1;
        }
    }
    assert!(
        (600..1200).contains(&bolts),
        "{bolts} bolts in 10k storm seconds"
    );
}
