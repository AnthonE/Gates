//! The cold's frost on the frame (`render/frost.rs`): the curve from the
//! server's cold to how far the frost has crept in, the ease that moves it,
//! its brightness, and the node's bundle spawned once — a duplicate
//! component in a bundle panics at spawn, not at compile.
#![cfg(feature = "render")]

use bevy::prelude::*;
use client::render::frost::{
    bundle, lum, params, seen_light, step, target, FrostLens, FrostMaterial, FROST_COLD_MAX,
    FROST_FALL_PER_S, FROST_FROM_PCT, FROST_HURT_PCT, FROST_NIGHT_LUM, FROST_RISE_PER_S,
};
use client::render::rig::DAY_EV100;

#[test]
fn no_frost_until_past_the_cold_chip() {
    for pct in 0..=FROST_FROM_PCT {
        assert_eq!(target(pct, false, false), 0.0, "{pct}%");
    }
    // The COLD chip shows from 25 %: it says it before the frost does.
    const { assert!(FROST_FROM_PCT > 25) };
    assert!(target(FROST_FROM_PCT + 1, false, false) > 0.0);
}

/// The ramp tops out where the server starts taking hp, read off the
/// shipped balance: `cold_pct` is the chill over ten and the hurt is a
/// chill past `hurt_at`, so a ramp that ran on past it would have a top
/// no cold short of hurting can reach, and a jump where hurting begins.
#[test]
fn the_ramp_ends_where_the_cold_starts_to_hurt() {
    let content = content::Content::load_dir(std::path::Path::new("../../content"))
        .unwrap_or_else(|e| panic!("content/ does not load: {e}"));
    let hurt_at = content.balance.exposure.hurt_at;
    assert_eq!(u32::from(FROST_HURT_PCT), hurt_at / 10, "hurt_at {hurt_at}");
    const { assert!(FROST_HURT_PCT > FROST_FROM_PCT && FROST_HURT_PCT < 100) };
}

#[test]
fn the_cold_ramps_and_the_hurt_is_the_strongest() {
    let mut last = 0.0;
    for pct in 0..=100u8 {
        let t = target(pct, false, false);
        assert!(t >= last, "monotone at {pct}%: {t} < {last}");
        assert!(t <= FROST_COLD_MAX + 1e-6, "{pct}%: {t}");
        last = t;
    }
    assert!((target(FROST_HURT_PCT, false, false) - FROST_COLD_MAX).abs() < 1e-6);
    // Taking hp: never under the ramp's top, never under what the same cold
    // short of hurting shows, and full at a full meter.
    let mut last = 0.0;
    for pct in 0..=100u8 {
        let t = target(pct, true, false);
        assert!((FROST_COLD_MAX..=1.0).contains(&t), "{pct}%: {t}");
        assert!(t >= target(pct, false, false), "{pct}%");
        assert!(t >= last, "monotone at {pct}%");
        last = t;
    }
    assert!(target(FROST_HURT_PCT + 1, true, false) > FROST_COLD_MAX);
    assert_eq!(target(100, true, false), 1.0);
}

/// The hurting bit flips with the meter at the threshold: the frost must
/// not jump there, nor pulse while the chill hovers on it.
#[test]
fn no_step_where_the_hurt_begins() {
    let at = FROST_HURT_PCT;
    assert_eq!(target(at, true, false), target(at, false, false));
    for pct in at..=at + 1 {
        let a = target(pct, false, false);
        let b = target(pct, true, false);
        assert!((b - a).abs() < 0.02, "{pct}%: {a} vs {b}");
    }
    let into = target(at + 1, true, false) - target(at, false, false);
    assert!((0.0..0.02).contains(&into), "{into}");
}

#[test]
fn a_fire_thaws_the_frost_that_is_not_hurting() {
    for pct in 0..=100u8 {
        assert_eq!(target(pct, false, true), 0.0, "{pct}%");
        // Still freezing at the fire: the frost stays.
        assert_eq!(target(pct, true, true), target(pct, true, false), "{pct}%");
    }
}

#[test]
fn it_creeps_in_and_thaws_at_its_rates_and_never_overshoots() {
    let dt = 1.0 / 60.0;
    let up = step(0.0, 1.0, dt);
    assert!((up - FROST_RISE_PER_S * dt).abs() < 1e-6);
    let down = step(1.0, 0.0, dt);
    assert!((down - (1.0 - FROST_FALL_PER_S * dt)).abs() < 1e-6);
    assert_eq!(step(0.5, 0.501, 1.0), 0.501);
    assert_eq!(step(0.5, 0.499, 1.0), 0.499);
    assert_eq!(step(0.3, 0.3, dt), 0.3);
    // Full frost thaws in a few seconds, and creeps in slower than that.
    let thaw_s = 1.0 / FROST_FALL_PER_S;
    let creep_s = 1.0 / FROST_RISE_PER_S;
    assert!((2.0..=4.0).contains(&thaw_s), "{thaw_s} s");
    assert!(creep_s > thaw_s);
    // A long frame does not leave it below zero.
    assert_eq!(step(0.1, 0.0, 10.0), 0.0);
}

#[test]
fn as_bright_as_the_light_on_the_world() {
    // Noon is full, a sunless night the floor, and the floor dark enough
    // not to read as a lamp over a black frame.
    assert_eq!(lum(seen_light(1.0, DAY_EV100)), 1.0);
    assert_eq!(lum(seen_light(0.0, DAY_EV100)), FROST_NIGHT_LUM);
    const { assert!(FROST_NIGHT_LUM <= 0.15) };
    assert_eq!(lum(2.0), 1.0);
    let mut last = FROST_NIGHT_LUM;
    for i in 0..=100 {
        let l = lum(i as f32 / 100.0);
        assert!(l >= last, "{i}");
        last = l;
    }
    // The twilight tail at the horizon (4 % of the sun) through an exposure
    // opened 2.5 stops is a sunset, not the night.
    let sunset = seen_light(0.04, DAY_EV100 - 2.5);
    assert!((0.2..0.3).contains(&sunset), "{sunset}");
    assert!(lum(sunset) > 0.5, "{}", lum(sunset));
    // An exposure shut past the day's never brightens it.
    assert_eq!(seen_light(0.5, DAY_EV100 + 1.0), 0.5);
}

#[test]
fn the_uniform_moves_in_steps() {
    // A sky that moves a hair a frame through the dusk changes the
    // material a step at a time, not every frame.
    let mut changes = 0;
    let mut last = params(0.5, 1.0);
    for i in 0..=10_000 {
        let p = params(0.5, 1.0 - i as f32 / 10_000.0);
        assert_eq!(p.x, 0.5);
        assert_eq!((p.y * 128.0).fract(), 0.0, "{p}");
        if p != last {
            changes += 1;
            last = p;
        }
    }
    assert!(changes <= 128, "{changes}");
    assert_eq!(
        params(0.3, 0.0).y,
        (FROST_NIGHT_LUM * 128.0).round() / 128.0
    );
    assert_eq!(params(2.0, 1.0).x, 1.0);
}

#[test]
fn the_bundle_spawns() {
    let mut world = World::new();
    let e = world.spawn(bundle(Handle::<FrostMaterial>::default())).id();
    assert!(world.get::<FrostLens>(e).is_some());
    assert!(world.get::<MaterialNode<FrostMaterial>>(e).is_some());
    assert_eq!(world.get::<ZIndex>(e), Some(&ZIndex(-1)));
}
