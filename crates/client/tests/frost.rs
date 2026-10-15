//! The cold's frost on the frame (`render/frost.rs`): the curve from the
//! server's cold to how far the frost has crept in, the ease that moves it,
//! and the node's bundle spawned once — a duplicate component in a bundle
//! panics at spawn, not at compile.
#![cfg(feature = "render")]

use bevy::prelude::*;
use client::render::frost::{
    bundle, lum, step, target, FrostLens, FrostMaterial, FROST_COLD_MAX, FROST_FALL_PER_S,
    FROST_FROM_PCT, FROST_HURT_MIN, FROST_NIGHT_LUM, FROST_RISE_PER_S,
};

#[test]
fn no_frost_until_past_the_cold_chip() {
    for pct in 0..=FROST_FROM_PCT {
        assert_eq!(target(pct, false, false), 0.0, "{pct}%");
    }
    // The COLD chip shows from 25 %: it says it before the frost does.
    const { assert!(FROST_FROM_PCT > 25) };
    assert!(target(FROST_FROM_PCT + 1, false, false) > 0.0);
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
    assert!((target(100, false, false) - FROST_COLD_MAX).abs() < 1e-6);
    // Taking hp: never under the floor, past anything a cold short of
    // hurting shows, and full at a full meter.
    let mut last = 0.0;
    for pct in 0..=100u8 {
        let t = target(pct, true, false);
        assert!((FROST_HURT_MIN..=1.0).contains(&t), "{pct}%: {t}");
        assert!(t > target(pct, false, false), "{pct}%");
        assert!(t >= last, "monotone at {pct}%");
        last = t;
    }
    assert_eq!(target(100, true, false), 1.0);
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
fn dimmer_at_night() {
    assert_eq!(lum(0.0), 1.0);
    assert!((lum(1.0) - FROST_NIGHT_LUM).abs() < 1e-6);
    assert!(lum(0.5) < 1.0 && lum(0.5) > FROST_NIGHT_LUM);
    assert_eq!(lum(2.0), lum(1.0));
}

#[test]
fn the_bundle_spawns() {
    let mut world = World::new();
    let e = world.spawn(bundle(Handle::<FrostMaterial>::default())).id();
    assert!(world.get::<FrostLens>(e).is_some());
    assert!(world.get::<MaterialNode<FrostMaterial>>(e).is_some());
    assert_eq!(world.get::<ZIndex>(e), Some(&ZIndex(-1)));
}
