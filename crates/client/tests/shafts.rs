//! The fog's sunlit share (`render/shafts.rs`): what the volume takes off
//! the flat fog has to come back as the same fog wherever it is SEEN, or the
//! shafts are a weather change nobody chose.
#![cfg(feature = "render")]

use bevy::math::Vec3;
use client::config::Quality;
use client::render::fill::luminance;
use client::render::quality::{components, effective, preset, tier, SHADOW_M_LADDER};
use client::render::rig::{FOV_DEG, RIG_SUN_ELEVATION, SHADOW_MAX_VEIL, SHADOW_MIN_ELEV};
use client::render::shafts::{
    fade, reach_m, split, steps, sun_share, volume, volume_transform, SHAFT_FULL_DEPTH,
    SHAFT_FULL_ELEV, SHAFT_MIN_DEPTH, SHAFT_MIN_REACH_M, SHAFT_MIN_T, SHAFT_STEPS,
};
use client::render::sky::{sun_transmittance, FOG_LAYER_M};
use client::render::weather::WeatherNow;

/// `(fog, rain)` per metre: the fog preset's ground fog, a fade halfway into
/// it, a thin mist, and the fog beside a heavy rain's haze.
const WEATHERS: [(f32, f32); 4] = [
    (0.9 * 0.9 * 0.9 * 3.0 / 110.0, 0.0),
    (0.6 * 0.6 * 0.6 * 3.0 / 110.0, 0.0),
    (0.3 * 0.3 * 0.3 * 3.0 / 110.0, 0.0),
    (0.4 * 0.4 * 0.4 * 3.0 / 110.0, 0.8 * 0.8 * 3.0 / 600.0),
];

/// A level ray's optical depth at `d` metres straight ahead: the flat fogs
/// all the way, the volume as far as its wall.
fn depth(flat: f32, rain: f32, volume: f32, reach: f32, d: f32) -> f32 {
    (flat + rain) * d + volume * d.min(reach)
}

#[test]
fn the_fog_is_the_same_fog_at_its_sight_line_straight_ahead() {
    for (fog, rain) in WEATHERS {
        for share in [0.2, 0.37, 0.7, 1.0] {
            for shadow_m in SHADOW_M_LADDER {
                let reach = reach_m(shadow_m);
                let s = split(fog, rain, share, reach);
                let sight = 3.0 / (fog + rain);
                let got = depth(s.flat, rain, s.volume, reach, sight);
                assert!(
                    (got - 3.0).abs() < 1e-3,
                    "fog {fog} rain {rain} share {share} reach {reach}: a hill at the \
                     sight line reads optical depth {got}, not the weather's 3"
                );
                assert!(s.flat >= 0.0 && s.volume >= 0.0 && s.flat <= fog);
                // A box that spans the sight line splits the fog exactly.
                if reach >= sight {
                    assert!((s.flat + s.volume - fog).abs() < 1e-6);
                }
            }
        }
    }
}

/// What `rig::day_night` hands the volume at noon in a weather of `fog` and
/// `rain` (‰ / 1000) under no cloud: the share of a sun the fog dims
/// (`WeatherNow::occlusion`) and the air reddens, and the two extinctions.
fn noon(fog: f32, rain: f32) -> (f32, f32, f32) {
    let mut w = WeatherNow::default();
    w.fog = fog;
    w.rain = rain;
    let t = sun_transmittance(RIG_SUN_ELEVATION.sin());
    let lux = 100_000.0 * (1.0 - 0.9 * w.occlusion()) * luminance(t);
    (sun_share(lux, 1.0), w.fog_part(), w.rain_part())
}

/// Bevy marches view DEPTH, so a ray off the view axis carries `cos θ` of
/// the volume it crosses and the flat fog does not make it up. The limit the
/// header names, bounded: at the frame's side edge (16:9 at the rig's FOV,
/// the camera facing down a box axis) a hill at the sight line keeps 2.7
/// of its 3 optical depths or more — at most a third more visible — in every
/// weather and on every rung that draws shafts.
#[test]
fn the_frame_edge_keeps_most_of_its_fog() {
    let edge = ((FOV_DEG.to_radians() * 0.5).tan() * 16.0 / 9.0).atan();
    let along = edge.cos();
    for fog in [0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0] {
        for rain in [0.0, 0.5, 0.8] {
            let (share, fog_m, rain_m) = noon(fog, rain);
            let sight = 3.0 / (fog_m + rain_m);
            for shadow_m in SHADOW_M_LADDER {
                let reach = reach_m(shadow_m);
                if reach < SHAFT_MIN_REACH_M {
                    continue;
                }
                let s = split(fog_m, rain_m, share, reach);
                let wall = reach / edge.cos().max(edge.sin());
                let got = (s.flat + rain_m) * sight + s.volume * wall.min(sight) * along;
                assert!(
                    got > 2.7 && got < 3.0 + 1e-3,
                    "fog {fog} rain {rain} share {share} shadows {shadow_m} m: a hill at \
                     the frame's edge reads optical depth {got} against the weather's 3"
                );
            }
        }
    }
}

#[test]
fn off_is_the_weather_unchanged() {
    let fog = WEATHERS[0].0;
    let s = split(fog, 0.0, 0.0, 135.0);
    assert_eq!((s.flat, s.volume), (fog, 0.0));
    let s = split(0.0, 0.004, 0.7, 135.0);
    assert_eq!((s.flat, s.volume), (0.0, 0.0));
}

/// Every edge where the pass switches off is one where the volume is
/// already empty, and the way there is smooth: neither a cloud's edge
/// crossing the sun nor a dawn pops the fog.
#[test]
fn the_shafts_fade_out_before_they_switch_off() {
    // The sun too low to cast, too veiled to cast; a fog too thin to show.
    assert_eq!(fade(SHADOW_MIN_ELEV, 0.0, 1.0), 0.0);
    assert_eq!(fade(RIG_SUN_ELEVATION, SHADOW_MAX_VEIL, 1.0), 0.0);
    assert_eq!(fade(RIG_SUN_ELEVATION, 0.0, SHAFT_MIN_DEPTH), 0.0);
    assert_eq!(fade(-0.2, 1.0, 0.0), 0.0);
    // The whole share in fine conditions.
    assert_eq!(fade(SHAFT_FULL_ELEV, 0.0, SHAFT_FULL_DEPTH), 1.0);
    // No frame of a slow change moves it by more than a few percent.
    let n = 1000;
    for i in 0..n {
        let (a, b) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
        let elev = |x: f32| fade(x * RIG_SUN_ELEVATION, 0.3, 1.0);
        let veil = |x: f32| fade(RIG_SUN_ELEVATION, x, 1.0);
        let thick = |x: f32| fade(RIG_SUN_ELEVATION, 0.3, x * 2.0 * SHAFT_FULL_DEPTH);
        for f in [&elev as &dyn Fn(f32) -> f32, &veil, &thick] {
            assert!((f(b) - f(a)).abs() < 0.02, "step {i}: {} to {}", f(a), f(b));
        }
    }
    // Just inside the edge the flat fog is the weather's, near enough.
    let fog = WEATHERS[0].0;
    let s = split(
        fog,
        0.0,
        0.3 * fade(SHADOW_MIN_ELEV + 1e-3, 0.0, 1.0),
        135.0,
    );
    assert!(s.volume < 1e-3 * fog && (s.flat - fog).abs() < 1e-3 * fog);
}

/// Bevy's march reads the sun's shadow map at every step and answers ZERO
/// past the last cascade, so a box corner beyond it is a black corner.
#[test]
fn the_box_stays_inside_the_sun_shadow() {
    for shadow_m in SHADOW_M_LADDER {
        let r = reach_m(shadow_m);
        // The shortest rungs leave no box at all, and draw none.
        if r < SHAFT_MIN_REACH_M {
            continue;
        }
        let corner = Vec3::new(r, FOG_LAYER_M, r).length();
        assert!(
            corner <= shadow_m + 1e-3,
            "shadow distance {shadow_m} m: the box's top corner is {corner} m out"
        );
        // ~3 m a step wherever the cap allows it.
        let n = steps(shadow_m);
        assert!((16..=SHAFT_STEPS).contains(&n));
        assert!(
            corner / n as f32 <= 3.5 || n == SHAFT_STEPS,
            "{shadow_m} m: {n}"
        );
    }
    // HIGH's 200 m leaves a box worth drawing, at Bevy's default steps.
    assert!(reach_m(200.0) >= SHAFT_MIN_REACH_M);
    assert_eq!(steps(200.0), SHAFT_STEPS);
}

#[test]
fn the_box_holds_the_eye_and_never_the_sea() {
    for eye_y in [1.7_f32, 30.0, 150.0] {
        let eye = Vec3::new(1500.0, eye_y, 600.0);
        let t = volume_transform(eye, 135.0);
        let bottom = t.translation.y - 0.5 * t.scale.y;
        let top = t.translation.y + 0.5 * t.scale.y;
        assert!(
            bottom >= sim_core::terrain::SEA_LEVEL - 1e-4,
            "bottom {bottom}"
        );
        assert!((top - (eye_y + FOG_LAYER_M)).abs() < 1e-3, "top {top}");
        // Bevy draws the interior as a fullscreen plane only when the eye is
        // strictly inside the unit cube's local space.
        let local = (eye - t.translation) / t.scale;
        assert!(local.abs().max_element() < 0.5, "eye at {local} in the box");
    }
}

/// `volumetric_fog.wgsl`'s lit, long-ray limit is `fog_color · light_tint ·
/// light_intensity · exposure · sun · HG(θ) · e^(−σR) · T`, the last the
/// atmosphere's transmittance toward the sun (`#ifdef ATMOSPHERE`, on every
/// view that draws shafts). Seen side-on a lit volume must read as the flat
/// fog's own colour at every hour it is drawn, brighter into the sun and
/// darker away from it.
#[test]
fn a_lit_volume_reads_as_the_flat_fog() {
    // `henyey_greenstein` in `volumetric_fog.wgsl`, at the cosine between
    // the view ray and the way to the sun.
    let hg = |g: f32, cos: f32| {
        (1.0 - g * g) / (4.0 * std::f32::consts::PI * (1.0 + g * g - 2.0 * g * cos).powf(1.5))
    };
    let rgb = [0.31, 0.33, 0.36];
    let scale = Vec3::new(270.0, 120.0, 270.0);
    for elev in [SHAFT_FULL_ELEV, 0.2, 0.4, RIG_SUN_ELEVATION] {
        let t = sun_transmittance(elev.sin());
        for (sigma, sun, exposure) in [(0.0074, 19_000.0, 4.4e-5), (0.002, 100_000.0, 2.0e-4)] {
            let v = volume(rgb, sigma, scale, sun, exposure, t);
            let radius = 0.5 * scale.length();
            let c = v.fog_color.to_linear();
            let tint = v.light_tint.to_linear();
            let lit = |cos: f32| {
                v.light_intensity
                    * exposure
                    * sun
                    * hg(v.scattering_asymmetry, cos)
                    * (-sigma * radius * (v.absorption + v.scattering)).exp()
            };
            let k = lit(0.0);
            let side = [
                c.red * tint.red * t[0] * k,
                c.green * tint.green * t[1] * k,
                c.blue * tint.blue * t[2] * k,
            ];
            for (got, want) in side.into_iter().zip(rgb) {
                assert!(
                    (got - want).abs() < 1e-3 * want,
                    "sun {elev} rad up: {got} against {want}"
                );
            }
            // Into the light the beams carry more than the flat fog, and with
            // the light behind, less — but never nothing.
            assert!(lit(1.0) > 2.0 && lit(-1.0) > 0.4 && lit(-1.0) < 1.0);
            // The shader's ambient term is not density-weighted; it must be
            // off, and droplets do not absorb.
            assert_eq!(v.absorption, 0.0);
        }
    }
    // Where the shafts are gone the lift stops: no blue lifted a thousandfold
    // off an air mass that far from the zenith.
    let t = sun_transmittance(SHADOW_MIN_ELEV.sin());
    let v = volume(rgb, 0.001, scale, 1_000.0, 4.4e-5, t);
    assert!(t[2] < SHAFT_MIN_T);
    assert!(v.light_tint.to_linear().blue <= 1.0 / SHAFT_MIN_T + 1e-3);
}

/// The sun's share is of what reaches the ground: the fog dims it, and so
/// does the air, more the lower it stands.
#[test]
fn the_sun_carries_less_of_a_fog_it_is_dimmed_by() {
    let (clear, _, _) = noon(0.0, 0.0);
    let (fog, _, _) = noon(0.9, 0.0);
    assert!(clear > 0.4 && clear < 1.0, "clear noon {clear}");
    assert!(fog > 0.1 && fog < clear, "fog preset {fog}");
    assert_eq!(sun_share(0.0, 0.0), 0.0);
    let low = luminance(sun_transmittance(SHAFT_FULL_ELEV.sin()));
    let high = luminance(sun_transmittance(RIG_SUN_ELEVATION.sin()));
    assert!(low < high && high < 1.0, "{low} at 6°, {high} at noon");
}

/// The shafts are the CLOUDS row's top rung: HIGH draws them, nothing below
/// it pays for them.
#[test]
fn only_the_top_rung_draws_shafts() {
    assert!(!tier(Quality::Low).shafts);
    assert!(!tier(Quality::Medium).shafts);
    // A browser's HIGH marches its clouds at MEDIUM, and so has none.
    assert_eq!(
        tier(Quality::High).shafts,
        cfg!(not(target_arch = "wasm32"))
    );
    // A shadow distance with no room for the box carries no fog on the
    // camera, and so leaves the fog preset's shadows out as before.
    for shadow_m in SHADOW_M_LADDER {
        let mut g = preset(Quality::High);
        g.shadow_m = shadow_m;
        let t = components(effective(g));
        if reach_m(shadow_m) < SHAFT_MIN_REACH_M {
            assert!(!t.shafts, "{shadow_m} m");
        }
    }
}
