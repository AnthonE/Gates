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
use bevy::light::EnvironmentMapLight;
use bevy::prelude::*;
use client::render::feed::Feed;
use client::render::fill::peak_lux;
use client::render::rig::{
    self, day_night, sun_elevation, tick_at_frac as tick_at, DayPin, EyeCam, Sun, CAPTURE_DAY_FRAC,
    NIGHT_AMBIENT_LUX, RIG_SUN_ELEVATION,
};
use sim_core::limits::{DAY_PORTION, DAY_TICKS};
use sim_core::world::day_frac;

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
    app_pinned(DayPin::default())
}

fn app_pinned(pin: DayPin) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.init_resource::<Feed>();
    app.insert_resource(pin);
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
    // **Two fill terms since 2026-08-15, and the pair is the point.** The rig
    // spawns a hemisphere (`EnvironmentMapLight`, `render/fill.rs`) that carries
    // the day and a uniform `AmbientLight` that carries only the night floor.
    // This fixture must hold BOTH or `day_night`'s query does not match and the
    // camera half of the system silently does nothing — which is how this file
    // failed when the hemisphere landed: the ambient kept its stale spawn value
    // and the test reported it as a wrong number rather than as a missing
    // component. The assertions below now check both terms at both ends, so a
    // future drift of this shape fails loudly instead of no-opping.
    app.world_mut().spawn((
        EyeCam,
        AmbientLight {
            color: Color::srgb(0.80, 0.85, 0.95),
            brightness: 0.0,
            affects_lightmapped_meshes: false,
        },
        EnvironmentMapLight {
            diffuse_map: Handle::default(),
            specular_map: Handle::default(),
            intensity: peak_lux(),
            rotation: Quat::IDENTITY,
            affects_lightmapped_mesh_diffuse: false,
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
        // At noon the hemisphere carries the whole fill and the uniform term
        // carries none of it. Both halves are asserted: a regression that let
        // the uniform term keep contributing at midday would DOUBLE the fill,
        // and checking only the hemisphere would not see it.
        let (amb, env) = world
            .query_filtered::<(&AmbientLight, &EnvironmentMapLight), With<EyeCam>>()
            .single(world)
            .unwrap();
        assert_eq!(
            amb.brightness, 0.0,
            "the uniform term still carries fill at noon — the two would sum"
        );
        assert!(
            (env.intensity - peak_lux()).abs() < 1.0,
            "noon fill is the hemisphere at full scale, got {}",
            env.intensity
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
        // And at midnight the handover is complete the other way: the night
        // floor is the whole fill and the hemisphere is dark. Unchanged from
        // what this shipped before the hemisphere existed — which is the
        // property that matters, since night was not what this slice moved.
        let (amb, env) = world
            .query_filtered::<(&AmbientLight, &EnvironmentMapLight), With<EyeCam>>()
            .single(world)
            .unwrap();
        assert!(
            (amb.brightness - NIGHT_AMBIENT_LUX).abs() < 1.0,
            "midnight ambient is the night floor, got {}",
            amb.brightness
        );
        assert_eq!(
            env.intensity, 0.0,
            "the hemisphere still carries fill at midnight, got {}",
            env.intensity
        );
    }
}

// ── The capture probe's clock (capture clock v0) ─────────────────────────
//
// The defect these gate: a capture shard boots at tick 0 and the probe fires
// whenever the client finished building and settling, so the sun's height was
// a function of how long `cargo` took — and every frame a visual judge has
// ever scored was taken below the band the rig is authored for.

/// `ART.md` §1's measured band: shadows in the reference run 1.5–2× the
/// height of what casts them, which is a sun between these.
const BAND_LO: f32 = 30.0 * std::f32::consts::PI / 180.0;
const BAND_HI: f32 = 40.0 * std::f32::consts::PI / 180.0;

#[test]
fn the_inverse_round_trips_through_the_sims_own_map() {
    // `tick_at_frac` is now shared by the code and this file, so it may not
    // be checked by agreeing with itself: every case goes back through
    // `sim_core::world::day_frac`, which is the forward map and not ours.
    for want in [0.0, 0.1, CAPTURE_DAY_FRAC, 0.5, DAY_PORTION, 0.85, 0.99] {
        let got = day_frac(tick_at(want));
        assert!(
            (got - want).abs() < 2.0 / DAY_TICKS as f32,
            "tick_at_frac({want}) round-trips to {got}"
        );
    }
}

#[test]
fn the_capture_pin_is_the_register_the_rig_is_authored_for() {
    // Not a number picked for a picture: it is the one fraction at which the
    // arch returns the authored elevation and full daylight exactly.
    let elev = sun_elevation(CAPTURE_DAY_FRAC);
    assert!(
        (elev - RIG_SUN_ELEVATION).abs() < 1e-6,
        "the pin must be the authored elevation, got {elev}"
    );
    assert!(
        (rig::daylight(CAPTURE_DAY_FRAC) - 1.0).abs() < 1e-6,
        "the pin must be full daylight"
    );
    assert!(
        (BAND_LO..=BAND_HI).contains(&elev),
        "{}° is outside ART.md §1's 30–40° band",
        elev.to_degrees()
    );
    // And the pinned tick really lands there, through the sim's forward map.
    let f = day_frac(DayPin::capture().tick(0.0));
    assert!(
        (f - CAPTURE_DAY_FRAC).abs() < 2.0 / DAY_TICKS as f32,
        "the pinned tick reads {f}, wanted {CAPTURE_DAY_FRAC}"
    );
}

#[test]
fn a_pinned_clock_does_not_move_with_the_build_time() {
    let pinned = DayPin::capture();
    let live = DayPin::default();
    // The whole point: whatever the estimate says, the pinned hour is one
    // hour. Swept across a full day, including the negatives `max(0.0)`
    // clamps and the wrap at DAY_TICKS.
    let probe = [
        -5.0, 0.0, 1.0, 2_200.0, 5_000.0, 40_500.0, 81_000.0, 162_000.0,
    ];
    for t in probe {
        assert_eq!(
            pinned.tick(t),
            pinned.tick(0.0),
            "the pin moved at estimate {t}"
        );
    }
    // ...and the test is not vacuous: unpinned, the same sweep does move.
    assert_ne!(
        live.tick(0.0),
        live.tick(40_500.0),
        "an unpinned clock must still follow the server"
    );
}

#[test]
fn the_unpinned_probe_shot_below_the_band_and_that_is_the_defect() {
    // Measured on this tree, and the reason the pin exists. A capture shard
    // starts at tick 0; the probe fires after the build and the settle.
    let at = |tick: u64| sun_elevation(day_frac(tick));
    let boot = at(0); // 24.5°
    let typical = at(2_200); // 27.3° — what the visual judge measured
    let slow = at(5_000); // 30.4° — a slower box
    for (name, e) in [("boot", boot), ("typical", typical)] {
        assert!(
            e < BAND_LO,
            "{name} shot at {}°, which this gate exists because it is below the band",
            e.to_degrees()
        );
    }
    // Monotonically rising across the window, so a slower box scored a
    // brighter frame — the frame was a function of the box.
    assert!(boot < typical && typical < slow);
    assert!(
        slow - boot > 5.0 * std::f32::consts::PI / 180.0,
        "the unpinned swing was {}°",
        (slow - boot).to_degrees()
    );
    // The pin answers all of it.
    let pinned = sun_elevation(day_frac(DayPin::capture().tick(0.0)));
    assert!((BAND_LO..=BAND_HI).contains(&pinned));
}

#[test]
fn the_pin_reaches_the_sun_and_the_ambient() {
    // System level, on the entities `day_night` actually writes: a pinned run
    // draws the same sky at two ticks half a day apart.
    // Named `probe`, not `app` — the local would shadow the `app()` helper
    // the second half of this test needs.
    let mut probe = app_pinned(DayPin::capture());
    let read = |app: &mut App| {
        let world = app.world_mut();
        let (t, d) = world
            .query_filtered::<(&Transform, &DirectionalLight), With<Sun>>()
            .single(world)
            .unwrap();
        (t.rotation, d.illuminance, d.shadows_enabled)
    };
    set_tick(&mut probe, 0);
    let a = read(&mut probe);
    set_tick(&mut probe, 40_500);
    let b = read(&mut probe);
    assert_eq!(a, b, "a pinned capture run drew two different skies");
    assert!(
        (a.1 - lux::DIRECT_SUNLIGHT).abs() < 1.0,
        "the pinned run is full sun, got {}",
        a.1
    );
    assert!(a.2, "the pinned run casts shadows");

    // Unpinned, the same two ticks are day and night — so the assertion
    // above is about the pin and not about `day_night` being inert.
    let mut live = app();
    set_tick(&mut live, 0);
    let c = read(&mut live);
    set_tick(&mut live, 40_500);
    let d = read(&mut live);
    assert_ne!(c, d, "an unpinned run must still follow the server's clock");
}
