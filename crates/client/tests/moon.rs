//! The moon's phases, gated headless: where the calendar puts the moon, how
//! its disk is lit, and what it gives the night — down to a floor a moonless
//! night never goes under, asserted on the light `rig::day_night` writes.

#![cfg(feature = "render")]

use bevy::light::EnvironmentMapLight;
use bevy::prelude::*;
use client::render::feed::Feed;
use client::render::moon::{above, lighting_sun, lit_fraction, shine, sunlight_on_disk, toward};
use client::render::rig::{
    self, day_arc, day_night, tick_at_frac, to_sun, DayPin, EyeCam, NEW_MOON_SHARE,
    NIGHT_AMBIENT_LUX,
};
use sim_core::limits::{DAY_PORTION, DAY_TICKS, MOON_DAYS};
use sim_core::world::moon_phase;

/// `k` of the way through the night, as a day fraction.
fn night(k: f32) -> f32 {
    DAY_PORTION + (1.0 - DAY_PORTION) * k
}

#[test]
fn the_disk_is_lit_from_none_to_all_of_it() {
    assert_eq!(lit_fraction(0.0), 0.0, "a new moon shows a lit edge");
    assert_eq!(lit_fraction(0.5), 1.0, "a full moon is not whole");
    for q in [0.25, 0.75] {
        assert!((lit_fraction(q) - 0.5).abs() < 1e-6, "a quarter at {q}");
    }
    // Waxing and waning are mirror images.
    for p in [0.05, 0.2, 0.4] {
        assert!((lit_fraction(p) - lit_fraction(1.0 - p)).abs() < 1e-6);
    }
}

/// The calendar puts the moon on the sun's path behind it: a full moon up
/// all night and high, a new one under the horizon all night, and every
/// moon from first quarter to last up for the whole of it.
#[test]
fn a_full_moon_rides_all_night_and_a_new_one_never_rises() {
    for i in 0..8 {
        let f = night(i as f32 / 8.0);
        let full = toward(f, 0.5);
        assert!(full.y > 0.3, "the full moon is at {} at {f}", full.y);
        assert!(shine(f, 0.5) > 0.999, "a full moon gives {}", shine(f, 0.5));
        // With the sun: on the horizon at dusk, under it after.
        assert!(toward(f, 0.0).y <= 1e-6, "the new moon is up at {f}");
        assert_eq!(shine(f, 0.0), 0.0, "a new moon lights the night");
        for p in [0.25, 0.375, 0.625, 0.75] {
            assert!(shine(f, p) > 0.4, "the moon at {p} is down at {f}");
        }
    }
}

/// The light on the disk turned back into the world: the side of the disk
/// it comes from, in the frame `moon.wgsl` builds.
fn lit_side(moon: Vec3, l: Vec3) -> Vec3 {
    let right = moon.cross(Vec3::Y).normalize();
    let up = right.cross(moon);
    right * l.x + up * l.y
}

/// The full moon is lit from the eye, as it was drawn before it had phases,
/// and a crescent's lit side faces the sun: the sunset a waxing one follows,
/// the sunrise a waning one leads.
#[test]
fn the_terminator_faces_the_sun() {
    let f = night(0.5);
    let l = sunlight_on_disk(toward(f, 0.5), lighting_sun(f, 0.5), 1.0);
    assert!((l - Vec3::Z).length() < 1e-6, "the full moon is lit by {l}");

    for (f, p, when) in [
        (night(0.05), 0.12, "after sunset"),
        (night(0.95), 0.88, "before sunrise"),
    ] {
        let (moon, sun) = (toward(f, p), to_sun(f));
        assert!(above(moon) > 0.99, "the crescent {when} is down");
        let k = lit_fraction(p);
        let l = sunlight_on_disk(moon, lighting_sun(f, p), k);
        assert!(
            (l.z - (2.0 * k - 1.0)).abs() < 1e-6,
            "the phase angle is {}",
            l.z
        );
        assert!((l.length() - 1.0).abs() < 1e-5, "the light is {l}");
        assert!(
            lit_side(moon, l).dot(sun) > 0.0,
            "the crescent {when} is lit from away from the sun"
        );
    }
}

/// The sun the moon is lit by keeps to the day's arc, which is the sun's
/// own path by day, and it is the sky's sun at the sunset a waxing moon
/// follows and the sunrise a waning one leads.
#[test]
fn the_moon_is_lit_by_the_day_arc() {
    for i in 0..=16 {
        let f = DAY_PORTION * i as f32 / 16.0;
        assert!(
            (day_arc(f) - to_sun(f)).length() < 1e-5,
            "the arc leaves the sun at {f}"
        );
    }
    let (dusk, dawn) = (night(0.0), night(1.0));
    for p in [0.1, 0.25, 0.4] {
        let off = (lighting_sun(dusk, p) - to_sun(dusk)).length();
        assert!(off < 1e-4, "the moon at {p} is lit {off} off the sunset");
    }
    for p in [0.6, 0.75, 0.9] {
        let off = (lighting_sun(dawn, p) - to_sun(dawn)).length();
        assert!(off < 1e-4, "the moon at {p} is lit {off} off the sunrise");
    }
}

/// The angle the lit side faces on the disk, degrees: 0 right, 90 up.
fn terminator_deg(f: f32, p: f32) -> f32 {
    let l = sunlight_on_disk(toward(f, p), lighting_sun(f, p), lit_fraction(p));
    l.y.atan2(l.x).to_degrees()
}

/// Through a whole night the lit side of a moon from first quarter to last
/// barely turns. Facing the sun's own night position it swung through 130°
/// in a couple of minutes, since that sun sweeps half the sky's bearing
/// under the world in a night, passing nearly behind the moon.
#[test]
fn the_terminator_holds_still_through_a_night() {
    // A night is this much of the month (`moon_phase` runs on with it).
    let month_per_night = (1.0 - DAY_PORTION) / MOON_DAYS as f32;
    for p0 in [0.25, 0.4, 0.6, 0.75] {
        let start = terminator_deg(night(0.0), p0);
        let mut worst = 0.0f32;
        for i in 0..=40 {
            let k = i as f32 / 40.0;
            let (f, p) = (night(k), p0 + k * month_per_night);
            assert!(above(toward(f, p)) > 0.99, "the moon at {p} is down at {f}");
            let turn = (terminator_deg(f, p) - start + 540.0).rem_euclid(360.0) - 180.0;
            worst = worst.max(turn.abs());
        }
        assert!(
            worst < 30.0,
            "the moon at {p0} turns its lit side {worst}° in a night"
        );
    }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<Feed>();
    app.insert_resource(DayPin(Some(0)));
    app.init_resource::<client::render::Settings>();
    app.init_resource::<rig::FlameGain>();
    app.add_systems(Update, day_night);
    app.world_mut().spawn((
        EyeCam,
        AmbientLight {
            brightness: -1.0,
            ..default()
        },
        EnvironmentMapLight::default(),
    ));
    app
}

fn ambient_at(app: &mut App, tick: u64) -> f32 {
    app.world_mut().resource_mut::<DayPin>().0 = Some(tick);
    app.update();
    let world = app.world_mut();
    world
        .query_filtered::<&AmbientLight, With<EyeCam>>()
        .single(world)
        .unwrap()
        .brightness
}

/// Through a whole month of nights the ambient `day_night` writes stays
/// between the moonless floor and the full moon's night — the full moon on
/// the first midnight, the floor at the new moon half a month on — and the
/// floor is a night, not a black frame.
#[test]
fn the_night_stays_above_the_moonless_floor() {
    let floor = NIGHT_AMBIENT_LUX * NEW_MOON_SHARE;
    assert!(
        floor >= 0.25 * NIGHT_AMBIENT_LUX,
        "the floor is {floor} lux"
    );
    let mut app = app();
    let midnight = tick_at_frac(night(0.5));
    let full = ambient_at(&mut app, midnight);
    assert!(
        (full - NIGHT_AMBIENT_LUX).abs() < 0.5,
        "the first midnight's full moon gives {full}, not the tuned night"
    );
    let new = ambient_at(&mut app, midnight + DAY_TICKS * MOON_DAYS / 2);
    assert!(
        (new - floor).abs() < 0.5,
        "the new moon's midnight gives {new}, not the floor {floor}"
    );
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for day in 0..MOON_DAYS {
        for i in 0..8 {
            let t = tick_at_frac(night(i as f32 / 8.0)) + day * DAY_TICKS;
            let lux = ambient_at(&mut app, t);
            lo = lo.min(lux);
            hi = hi.max(lux);
            assert!(
                lux >= floor - 0.01,
                "day {day} ({} of its month) at {}: {lux} lux, under the floor",
                moon_phase(t),
                night(i as f32 / 8.0)
            );
            assert!(lux <= NIGHT_AMBIENT_LUX + 0.01, "{lux} lux at day {day}");
        }
    }
    assert!(
        hi - lo > 0.5 * NIGHT_AMBIENT_LUX,
        "the month moves the night {lo}..{hi}"
    );
}

/// A new moon shows every star; a full one washes out the faint ones.
#[test]
fn a_moonless_night_shows_more_stars() {
    use client::render::stars::{washed, MOON_WASH};
    use client::render::weather::WeatherNow;
    let mut w = WeatherNow::default();
    w.night = 1.0;
    w.moonlight = 0.0;
    assert_eq!(washed(&w), 0.0);
    w.moonlight = 1.0;
    assert!((washed(&w) - MOON_WASH).abs() < 1.0 / 64.0);
}
