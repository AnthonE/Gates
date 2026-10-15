//! The fog's sunlit share (`render/shafts.rs`): what the volume takes off
//! the flat fog has to come back as the same fog wherever it is SEEN, or the
//! shafts are a weather change nobody chose.
#![cfg(feature = "render")]

use bevy::math::Vec3;
use client::config::Quality;
use client::render::quality::{tier, SHADOW_M_LADDER};
use client::render::shafts::{
    reach_m, split, sun_share, volume, volume_transform, SHAFT_MIN_REACH_M,
};
use client::render::sky::FOG_LAYER_M;

/// `(fog, rain)` per metre: the fog preset's ground fog, a fade halfway into
/// it, a thin mist, and the fog beside a heavy rain's haze.
const WEATHERS: [(f32, f32); 4] = [
    (0.9 * 0.9 * 0.9 * 3.0 / 110.0, 0.0),
    (0.6 * 0.6 * 0.6 * 3.0 / 110.0, 0.0),
    (0.3 * 0.3 * 0.3 * 3.0 / 110.0, 0.0),
    (0.4 * 0.4 * 0.4 * 3.0 / 110.0, 0.8 * 0.8 * 3.0 / 600.0),
];

/// A level ray's optical depth at `d` metres: the flat fogs all the way, the
/// volume as far as its wall.
fn depth(flat: f32, rain: f32, volume: f32, reach: f32, d: f32) -> f32 {
    (flat + rain) * d + volume * d.min(reach)
}

#[test]
fn the_fog_is_the_same_fog_at_its_sight_line() {
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

#[test]
fn off_is_the_weather_unchanged() {
    let fog = WEATHERS[0].0;
    let s = split(fog, 0.0, 0.0, 135.0);
    assert_eq!((s.flat, s.volume), (fog, 0.0));
    let s = split(0.0, 0.004, 0.7, 135.0);
    assert_eq!((s.flat, s.volume), (0.0, 0.0));
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
    }
    // HIGH's 200 m leaves a box worth drawing.
    assert!(reach_m(200.0) >= SHAFT_MIN_REACH_M);
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

/// `volumetric_fog.wgsl`'s lit, long-ray limit is `fog_color · light_intensity
/// · exposure · sun · HG(θ) · e^(−σR)`. Seen side-on a lit volume must read as
/// the flat fog's own colour, brighter into the sun and darker away from it.
#[test]
fn a_lit_volume_reads_as_the_flat_fog() {
    // `henyey_greenstein` in `volumetric_fog.wgsl`, at the cosine between
    // the view ray and the way to the sun.
    let hg = |g: f32, cos: f32| {
        (1.0 - g * g) / (4.0 * std::f32::consts::PI * (1.0 + g * g - 2.0 * g * cos).powf(1.5))
    };
    let rgb = [0.31, 0.33, 0.36];
    let scale = Vec3::new(270.0, 120.0, 270.0);
    for (sigma, sun, exposure) in [(0.0074, 19_000.0, 4.4e-5), (0.002, 100_000.0, 2.0e-4)] {
        let v = volume(rgb, sigma, scale, sun, exposure);
        let radius = 0.5 * scale.length();
        let c = v.fog_color.to_linear();
        let lit = |cos: f32| {
            v.light_intensity
                * exposure
                * sun
                * hg(v.scattering_asymmetry, cos)
                * (-sigma * radius * (v.absorption + v.scattering)).exp()
        };
        let k = lit(0.0);
        for (got, want) in [c.red * k, c.green * k, c.blue * k].into_iter().zip(rgb) {
            assert!((got - want).abs() < 1e-3 * want, "{got} against {want}");
        }
        // Into the light the beams carry more than the flat fog, and with
        // the light behind, less — but never nothing.
        assert!(lit(1.0) > 2.0 && lit(-1.0) > 0.4 && lit(-1.0) < 1.0);
        // The shader's ambient term is not density-weighted; it must be off,
        // and droplets do not absorb.
        assert_eq!(v.absorption, 0.0);
    }
}

#[test]
fn the_sun_carries_less_of_a_fog_it_is_dimmed_by() {
    let clear = sun_share(100_000.0, 1.0);
    let fog = sun_share(100_000.0 * (1.0 - 0.9 * 0.9), 1.0 - 0.35 * 0.2);
    assert!(clear > 0.5 && clear < 1.0, "clear noon {clear}");
    assert!(fog > 0.1 && fog < clear, "fog preset {fog}");
    assert_eq!(sun_share(0.0, 0.0), 0.0);
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
}
