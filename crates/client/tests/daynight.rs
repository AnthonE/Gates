//! Day and night, gated headless (day/night v0; `tests/music.rs`' bare-App
//! pattern — no window, no GPU, no device).
//!
//! Two layers: the curve as arithmetic (a noon that is not the rig's
//! authored elevation, a dusk that is not the horizon, a night sun above
//! ground — each a lie the picture would tell), and the system as wiring —
//! `rig::day_night` reads the clock `Feed` carries and writes the sun, the
//! ambient and the shadow toggle, asserted on the entities it wrote.

#![cfg(feature = "render")]

use bevy::light::light_consts::lux;
use bevy::prelude::*;
use client::render::feed::Feed;
use client::render::rig::{
    self, day_night, sun_elevation, EyeCam, Sun, NIGHT_AMBIENT_LUX, RIG_SUN_ELEVATION,
};
use sim_core::limits::{DAY_PHASE_TICKS, DAY_PORTION, DAY_TICKS};
use sim_core::world::day_frac;

/// The tick whose `day_frac` is exactly `want`.
fn tick_at(want: f32) -> u64 {
    ((want * DAY_TICKS as f32) as u64 + DAY_TICKS - DAY_PHASE_TICKS) % DAY_TICKS
}

#[test]
fn the_curve_is_the_day_it_claims() {
    // Dawn and dusk sit on the horizon; noon is the rig's authored
    // elevation exactly (the sine's peak is the band ART.md measured).
    assert!(sun_elevation(0.0).abs() < 1e-6, "dawn is the horizon");
    let noon = sun_elevation(DAY_PORTION * 0.5);
    assert!(
        (noon - RIG_SUN_ELEVATION).abs() < 1e-6,
        "noon must be the authored elevation, got {noon}"
    );
    assert!(
        sun_elevation(DAY_PORTION - 1e-4).abs() < 0.01,
        "dusk returns to the horizon"
    );
    // Night: below the horizon the whole way through.
    for f in [0.75, 0.85, 0.95] {
        assert!(
            sun_elevation(f) < 0.0,
            "the night sun must be under the ground at {f}"
        );
    }
    // Daylight scalar: full at noon, zero all night.
    assert!((rig::daylight(DAY_PORTION * 0.5) - 1.0).abs() < 1e-6);
    assert_eq!(rig::daylight(0.85), 0.0);
}

#[test]
fn the_clock_boots_in_the_morning() {
    // Tick zero — a fresh world, every capture probe — lands mid-morning
    // with real daylight, not on the dawn terminator: the visual rubric
    // scores daylight frames and a capture at dusk would tank it for a
    // reason that is a clock, not a defect.
    let f = day_frac(0);
    assert!(f > 0.05 && f < DAY_PORTION * 0.5, "boot frac {f}");
    assert!(
        rig::daylight(f) > 0.5,
        "a fresh world must boot into real daylight, got {}",
        rig::daylight(f)
    );
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<Feed>();
    app.add_systems(Update, day_night);
    // The rig's two entities, at their spawn-time shapes.
    app.world_mut().spawn((
        Sun,
        DirectionalLight {
            illuminance: lux::DIRECT_SUNLIGHT,
            shadows_enabled: true,
            ..default()
        },
        Transform::default(),
    ));
    app.world_mut().spawn((
        EyeCam,
        AmbientLight {
            color: Color::srgb(0.80, 0.85, 0.95),
            brightness: lux::AMBIENT_DAYLIGHT * 1.7,
            affects_lightmapped_meshes: false,
        },
    ));
    app
}

fn set_tick(app: &mut App, tick: u64) {
    app.world_mut().resource_mut::<Feed>().server_tick_est = tick as f64;
    app.update();
}

#[test]
fn noon_and_midnight_reach_the_entities() {
    let mut app = app();

    set_tick(&mut app, tick_at(DAY_PORTION * 0.5));
    {
        let world = app.world_mut();
        let (t, d) = world
            .query_filtered::<(&Transform, &DirectionalLight), With<Sun>>()
            .single(world)
            .unwrap();
        assert!(
            (d.illuminance - lux::DIRECT_SUNLIGHT).abs() < 1.0,
            "noon is full sun, got {}",
            d.illuminance
        );
        assert!(d.shadows_enabled, "the noon sun casts shadows");
        // The light points down-ish at noon: its forward has a negative y.
        let fwd = t.rotation * Vec3::NEG_Z;
        assert!(fwd.y < -0.4, "the noon sun points down, fwd {fwd:?}");
        let amb = world
            .query_filtered::<&AmbientLight, With<EyeCam>>()
            .single(world)
            .unwrap();
        assert!(
            (amb.brightness - lux::AMBIENT_DAYLIGHT * 1.7).abs() < 1.0,
            "noon ambient is the rig's authored fill"
        );
    }

    set_tick(&mut app, tick_at(0.85));
    {
        let world = app.world_mut();
        let (t, d) = world
            .query_filtered::<(&Transform, &DirectionalLight), With<Sun>>()
            .single(world)
            .unwrap();
        assert_eq!(d.illuminance, 0.0, "midnight has no sun");
        assert!(!d.shadows_enabled, "no shadows from a sun under the ground");
        let fwd = t.rotation * Vec3::NEG_Z;
        assert!(fwd.y > 0.0, "the midnight sun shines up from below");
        let amb = world
            .query_filtered::<&AmbientLight, With<EyeCam>>()
            .single(world)
            .unwrap();
        assert!(
            (amb.brightness - NIGHT_AMBIENT_LUX).abs() < 1.0,
            "midnight ambient is the night floor, got {}",
            amb.brightness
        );
    }
}
