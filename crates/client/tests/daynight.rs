//! Day and night, gated headless (day/night v0; `tests/music.rs`' bare-App
//! pattern — no window, no GPU, no device).
//!
//! Two layers: the curve as arithmetic (a noon that is not the rig's
//! authored elevation, a dusk that is not the horizon, a night sun above
//! ground — each a lie the picture would tell), and the system as wiring —
//! `rig::day_night` reads the clock `Feed` carries and writes the sun, the
//! ambient and the shadow toggle, asserted on the entities it wrote.

#![cfg(feature = "render")]

use bevy::core_pipeline::Skybox;
use bevy::light::light_consts::lux;
use bevy::light::EnvironmentMapLight;
use bevy::prelude::*;
use client::render::feed::Feed;
use client::render::fill::peak_lux;
use client::render::rig::{
    self, day_night, sun_elevation, tick_at_frac as tick_at, DayPin, EyeCam, Sun, CAPTURE_DAY_FRAC,
    NIGHT_AMBIENT_LUX, RIG_SUN_ELEVATION,
};
use client::render::sky;
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
    // Night: below the horizon the whole way through. **Sampled as a
    // fraction OF THE NIGHT, never as literals of the cycle.** It was
    // `[0.75, 0.85, 0.95]` until 2026-08-15, which were night while
    // `DAY_PORTION` was 0.70 and are broad daylight at 0.875 — the operator
    // lengthened the day and this gate went red asserting the sun was
    // underground at teatime. A literal here is a second, silent copy of
    // `DAY_PORTION`; deriving is what makes the gate follow the knob.
    for k in [0.1, 0.5, 0.9] {
        let f = DAY_PORTION + (1.0 - DAY_PORTION) * k;
        assert!(
            sun_elevation(f) < 0.0,
            "the night sun must be under the ground at {f} ({k} of the way through the night)"
        );
    }
    // Daylight scalar: full at noon, zero all night. Midnight derived for
    // the same reason as the loop above — `0.85` was night at 0.70 and is
    // afternoon at 0.875, and this line read `daylight` as 0.095 rather
    // than 0.
    assert!((rig::daylight(DAY_PORTION * 0.5) - 1.0).abs() < 1e-6);
    assert_eq!(rig::daylight(DAY_PORTION + (1.0 - DAY_PORTION) * 0.5), 0.0);
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
        // The cloud deck. Third component of `day_night`'s camera query, and
        // the one this fixture went WITHOUT until 2026-08-15 — the branch is
        // `Option<&mut Skybox>`, so with no `Skybox` here the deck half of the
        // system was dead in every suite: deleting the `sky::deck_rotation`
        // call site in `rig::day_night` left every client suite green,
        // proven at the time. Same silent-no-op shape
        // as the hemisphere note above, one component later. Sentinel values,
        // both wrong on purpose, so an assertion on them proves a WRITE and
        // not a lucky spawn: no hour's brightness is negative and no hour's
        // deck rotation is a half-turn (`RIG_SUN_ARC = π` sweeps the day, so
        // the daytime offset from noon stays strictly inside ±π/2).
        Skybox {
            image: Handle::default(),
            brightness: -1.0,
            rotation: Quat::from_rotation_y(std::f32::consts::PI),
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

    // Midnight is the MIDPOINT OF THE NIGHT, derived — `0.85` until
    // 2026-08-15, which was midnight at `DAY_PORTION = 0.70` and is
    // mid-afternoon at 0.875. See the same correction in
    // `the_curve_is_the_day_it_claims`.
    set_tick(&mut app, tick_at(DAY_PORTION + (1.0 - DAY_PORTION) * 0.5));
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

#[test]
fn the_deck_follows_the_sun_it_is_lit_by() {
    // The wiring gate for the Skybox branch (`rig.rs`, `day_night`'s camera
    // query). `sky::deck_rotation` is already gated as arithmetic
    // (`tests/sun.rs`); this holds the CALL SITE, which nothing did — the
    // branch is dead without a `Skybox` on the entity, and a dead branch's
    // deletion is green everywhere.
    let mut app = app();
    let read = |app: &mut App| {
        let world = app.world_mut();
        let sky = world
            .query_filtered::<&Skybox, With<EyeCam>>()
            .single(world)
            .unwrap();
        (sky.rotation, sky.brightness)
    };

    // Two hours, neither of them noon: at noon `deck_rotation` is the
    // identity and the brightness is full, both of which a stale value could
    // fake. Mid-morning and mid-afternoon sit on opposite sides of noon, so
    // the deck's offset changes sign between them and the two reads cannot
    // agree with each other either.
    let morning_tick = tick_at(DAY_PORTION * 0.25);
    let afternoon_tick = tick_at(DAY_PORTION * 0.75);

    set_tick(&mut app, morning_tick);
    let (rot_am, bright_am) = read(&mut app);
    set_tick(&mut app, afternoon_tick);
    let (rot_pm, bright_pm) = read(&mut app);

    // The deck moved between the two hours…
    assert!(
        rot_am.angle_between(rot_pm) > 0.1,
        "the deck did not move between morning and afternoon: {rot_am:?} vs {rot_pm:?}"
    );
    // …and each read is the arithmetic the sun gate already holds, computed
    // through the sim's own forward map — the same frac the system saw.
    for (name, tick, rot, bright) in [
        ("morning", morning_tick, rot_am, bright_am),
        ("afternoon", afternoon_tick, rot_pm, bright_pm),
    ] {
        let frac = day_frac(tick);
        let want = sky::deck_rotation(frac);
        assert!(
            rot.angle_between(want) < 1e-4,
            "{name}: deck at {rot:?}, deck_rotation says {want:?}"
        );
        // `sky.brightness` was ungated by the same dead branch; one fixture
        // closes both (NOW.md §0sun item 1).
        let want_b = sky::CLOUD_NITS * rig::daylight(frac);
        assert!(
            (bright - want_b).abs() < 1.0,
            "{name}: deck brightness {bright}, wanted {want_b}"
        );
        assert!(bright > 0.0, "{name} is daytime; the sentinel survived");
    }

    // The brightness half at its other endpoint: dark at midnight (derived
    // as the night's midpoint, same as every night sample in this file).
    set_tick(&mut app, tick_at(DAY_PORTION + (1.0 - DAY_PORTION) * 0.5));
    let (_, bright_night) = read(&mut app);
    assert_eq!(bright_night, 0.0, "the deck must go dark at midnight");
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
    // Measured under the 80-minute cycle spoken 2026-08-15. The figures in
    // the right margin were 24.5 / 27.3 / 30.4 under the 45-minute one, and
    // **boot barely moved** — `DAY_PHASE_TICKS` was re-derived with
    // `DAY_TICKS` rather than left behind, so tick zero still lands at the
    // same hour. What shrank is the SPREAD: a fixed build window is a
    // smaller slice of a longer cycle, so the sun climbs less while cargo
    // works.
    let boot = at(0); // 24.7°
    let typical = at(2_200); // 26.0° — what the visual judge measured, then
    let slow = at(5_000); // 27.6° — a slower box
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
    // **The threshold is scaled, not re-picked.** The defect this gate holds
    // is that the frame was a function of how long the box took, and that is
    // the ORDERING above; the magnitude is a property of the cycle length and
    // moves whenever the operator speaks it. 5° at `DAY_TICKS = 81_000`
    // becomes 5 × 81_000/144_000 = 2.81° at 80 minutes, and the measurement
    // is 2.88°. Written as the scaling so the next cycle change moves it
    // arithmetically instead of reddening a gate about something else.
    let floor = 5.0_f32.to_radians() * 81_000.0 / DAY_TICKS as f32;
    assert!(
        slow - boot > floor,
        "the unpinned swing was {}°, floor {}°",
        (slow - boot).to_degrees(),
        floor.to_degrees()
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
