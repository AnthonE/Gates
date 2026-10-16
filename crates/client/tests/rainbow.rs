//! Rainbows (`render/rainbow.rs`): where the rings sit, and when a sky has
//! one. Pure arithmetic, no GPU.

#![cfg(feature = "render")]

use bevy::math::Vec3;
use client::render::rainbow::{
    descartes, lingering, strength, strength_now, RainbowParams, BANDS, LINGER_S, WATER_N_RED,
    WATER_N_VIOLET,
};
use client::render::weather::WeatherNow;
use sim_core::limits::TICK_HZ;
use sim_core::weather::{blend, preset, CLEAR, RAIN_HEAVY, RAIN_MILD};

fn deg(r: f32) -> f32 {
    r.to_degrees()
}

/// A sun this many degrees up.
fn sun_up(d: f32) -> f32 {
    d.to_radians().sin()
}

/// The primary at ~42° with red outside, the secondary at ~51° with red
/// inside, and the dark band between them.
#[test]
fn the_rings_sit_where_the_drops_send_them() {
    let (p_red, p_violet) = (
        deg(descartes(WATER_N_RED, 1)),
        deg(descartes(WATER_N_VIOLET, 1)),
    );
    let (s_red, s_violet) = (
        deg(descartes(WATER_N_RED, 2)),
        deg(descartes(WATER_N_VIOLET, 2)),
    );
    assert!((41.8..42.8).contains(&p_red), "primary red at {p_red}°");
    assert!(
        (40.2..41.2).contains(&p_violet),
        "primary violet at {p_violet}°"
    );
    assert!((49.8..51.0).contains(&s_red), "secondary red at {s_red}°");
    assert!(
        (52.8..54.0).contains(&s_violet),
        "secondary violet at {s_violet}°"
    );
    // About two degrees of colour in the primary, more in the secondary.
    assert!((1.2..2.4).contains(&(p_red - p_violet)));
    assert!(s_violet - s_red > p_red - p_violet);
    // Alexander's band: the rings' inner edges are ~8° apart.
    assert!(s_red - p_red > 7.0);
}

/// The rings add up to the sun's white, violet first and red last.
#[test]
fn the_bands_sum_to_white_violet_to_red() {
    for c in 0..3 {
        let sum: f32 = BANDS.iter().map(|b| b[c]).sum();
        assert!((sum - 1.0).abs() < 0.01, "channel {c} sums to {sum}");
    }
    let first = BANDS[0];
    let last = BANDS[BANDS.len() - 1];
    assert!(
        first[2] > first[0] && first[2] > first[1],
        "violet ring {first:?}"
    );
    assert!(last[0] > last[1] && last[0] > last[2], "red ring {last:?}");
}

/// A shower's clearing in the afternoon has a bow; take away any one of
/// sun, rain or a gap for the sun and it has none.
#[test]
fn a_bow_needs_a_low_sun_rain_and_a_gap() {
    let k = strength(sun_up(20.0), 0.6, 0.2, 0.0);
    assert!(k > 0.4, "a clearing shower: {k}");
    // Never at night, nor with the sun on the horizon.
    assert_eq!(strength(sun_up(-5.0), 0.6, 0.2, 0.0), 0.0);
    assert_eq!(strength(0.0, 0.04, 0.2, 0.0), 0.0);
    // A sun higher than the bow is wide puts all of it underground.
    assert_eq!(strength(sun_up(43.0), 1.0, 0.2, 0.0), 0.0);
    assert!(strength(sun_up(35.0), 1.0, 0.2, 0.0) > 0.0);
    // No rain, or a downpour's haze.
    assert_eq!(strength(sun_up(20.0), 0.6, 0.0, 0.0), 0.0);
    assert_eq!(strength(sun_up(20.0), 0.6, 1.0, 0.0), 0.0);
    // The sun behind cloud or fog.
    assert_eq!(strength(sun_up(20.0), 0.6, 0.2, 0.8), 0.0);
    assert!(strength(sun_up(20.0), 0.6, 0.2, 0.5) < k);
    // Brighter with more sun.
    assert!(strength(sun_up(20.0), 0.9, 0.2, 0.0) > k);
}

/// The weather as the sky reads it: a steady shower under its own overcast
/// has no bow, nor does a storm, but the shower clearing does, wherever the
/// sun stands in a gap. Underwater nothing does.
#[test]
fn the_clearing_after_a_shower_has_one() {
    let sky = |wx: sim_core::weather::Wx| {
        let mut w = WeatherNow::default();
        w.cloud = wx.cloud as f32 * 0.001;
        w.dark = wx.dark as f32 * 0.001;
        w.rain = wx.rain as f32 * 0.001;
        w.fog = wx.fog as f32 * 0.001;
        w.sun = Vec3::new(0.0, sun_up(25.0), 25f32.to_radians().cos());
        w.sun_lux = 0.7;
        w
    };
    let steady = sky(preset(RAIN_MILD, 0));
    assert_eq!(strength_now(&steady, steady.rain), 0.0);
    let heavy = sky(preset(RAIN_HEAVY, 0));
    assert_eq!(strength_now(&heavy, heavy.rain), 0.0);
    let clearing = sky(blend(&preset(RAIN_MILD, 0), &preset(CLEAR, 0), 750));
    assert!(strength_now(&clearing, clearing.rain) > 0.2);
    let mut under = clearing;
    under.underwater = true;
    assert_eq!(strength_now(&under, under.rain), 0.0);
    // The uniform puts the bow opposite the sun, at that strength.
    let p = RainbowParams::for_weather(&clearing, clearing.rain);
    assert!((p.anti.truncate() + clearing.sun.normalize()).length() < 1e-5);
    assert_eq!(p.anti.w, strength_now(&clearing, clearing.rain));
}

/// Rain hangs in the air for a while after it stops, fading as it goes,
/// and fades smoothly rather than in steps.
#[test]
fn rain_lingers_and_fades_smoothly() {
    let span = LINGER_S as u64 * TICK_HZ as u64;
    let stop = 100_000u64;
    let rain = |t: u64| if t < stop { 0.35 } else { 0.0 };
    // Steady rain is itself.
    assert_eq!(lingering(stop - 1, rain), 0.35);
    assert_eq!(lingering(50, |_| 0.2), 0.2);
    // Half the linger after the rain stopped, about half of it is left.
    let half = lingering(stop + span / 2, rain);
    assert!((0.12..0.25).contains(&half), "half way: {half}");
    // And by the end of it none.
    assert_eq!(lingering(stop + span, rain), 0.0);
    // Never a step bigger than a tick's share of the fade.
    let most = (stop - 10..stop + span + 10)
        .map(|t| (lingering(t + 1, rain) - lingering(t, rain)).abs())
        .fold(0.0f32, f32::max);
    assert!(most < 0.35 * 2.0 / span as f32, "a step of {most}");
}
