//! The deck's shadows on the land (`render/cloud_shadow.rs`): the drift the
//! shader runs on between writes, and the uniform's two sides.

#![cfg(all(feature = "render", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use client::render::cloud_shadow::{level, Anchor, CloudShadowParams, DRIFT_SLACK_M};
use client::render::sky::{CLOUD_SCALE_M, FIELD_N, FIELD_PERIOD};
use client::render::weather::WeatherNow;

const SHADER: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/shaders/cloud_shadow.wgsl"
);

/// The weather's drift, a frame on, the way `weather::update` keeps it:
/// metres, integrated in f32 and wrapped a field over — and Bevy's clock,
/// which is not integrated. Whether the anchor took this frame.
fn frame(a: &mut Anchor, drift: &mut Vec2, clock: &mut f64, wind: Vec2, sun: Vec3) -> bool {
    let period_m = FIELD_PERIOD as f32 * CLOUD_SCALE_M;
    let dt = 1.0f32 / 60.0;
    *clock += f64::from(dt);
    *drift += wind * dt;
    drift.x = drift.x.rem_euclid(period_m);
    drift.y = drift.y.rem_euclid(period_m);
    a.track(
        *clock as f32,
        -*drift / CLOUD_SCALE_M,
        -wind / CLOUD_SCALE_M,
        sun,
    )
}

/// A steady wind is a write every half minute at most, across both of the
/// drift's wraps and out where the weather's f32 metres are coarsest; a
/// wind that changes, a sun that moves and a clock that comes round are
/// each one more. Every write re-prepares the ground and every rock on it,
/// so a write a frame would be the cost this anchor exists to avoid.
#[test]
fn a_steady_wind_is_written_rarely() {
    let sun = Vec3::new(0.3, 0.6, -0.74).normalize();
    let mut a = Anchor::default();
    let mut drift = Vec2::new(100.0, 28_700.0);
    let mut clock = 10.0f64;
    let wind = Vec2::new(-6.0, 2.5);
    assert!(
        frame(&mut a, &mut drift, &mut clock, wind, sun),
        "the first frame anchors"
    );
    // Ten minutes at 60 Hz.
    let wrote: usize = (0..36_000)
        .map(|_| usize::from(frame(&mut a, &mut drift, &mut clock, wind, sun)))
        .sum();
    assert!(
        wrote <= 20,
        "a steady wind wrote {wrote} times in ten minutes"
    );
    // The wind backs: the run wanders off and is taken again.
    let backed = Vec2::new(2.0, 6.0);
    let wrote: usize = (0..600)
        .map(|_| usize::from(frame(&mut a, &mut drift, &mut clock, backed, sun)))
        .sum();
    assert!(
        (1..=2).contains(&wrote),
        "a changed wind wrote {wrote} times in ten seconds"
    );
    // A few degrees of the day is nothing to write; ten is.
    let soon = Vec3::new(0.32, 0.6, -0.74).normalize();
    assert!(!frame(&mut a, &mut drift, &mut clock, backed, soon));
    let later = Vec3::new(0.45, 0.55, -0.70).normalize();
    assert!(
        frame(&mut a, &mut drift, &mut clock, backed, later),
        "a moved sun is written"
    );
    // `elapsed_secs_wrapped` comes round.
    let off = -drift / CLOUD_SCALE_M;
    assert!(
        a.track(0.5, off, -backed / CLOUD_SCALE_M, later),
        "a wrapped clock anchors"
    );
    let run = a.off + a.vel * (0.5 - a.t);
    assert!((run - off).length() * CLOUD_SCALE_M < DRIFT_SLACK_M);
}

/// A shadow is as soft as `SOFT_M` says on every build: one mip finer on
/// the field a debug build halves.
#[test]
fn the_shadow_reads_the_mip_nearest_its_softness() {
    let texel = CLOUD_SCALE_M * FIELD_PERIOD as f32 / FIELD_N as f32;
    let soft = texel * (1u32 << level()) as f32;
    assert!(
        (40.0..90.0).contains(&soft),
        "the shadows read {soft} m texels"
    );
}

/// Off, and under a full overcast or the fog, the weather's floor covers
/// the sky and the term is nothing; clear, the deck alone decides.
#[test]
fn the_floor_is_the_rigs_occlusion_without_the_deck() {
    let a = Anchor::default();
    let sky = |cloud: f32, fog: f32, sun_cover: f32| {
        let mut w = WeatherNow::default();
        w.cloud = cloud;
        w.fog = fog;
        w.sun_cover = sun_cover;
        w
    };
    let clear = sky(0.35, 0.0, 1.0);
    let p = CloudShadowParams::new(true, &clear, &a);
    assert_eq!(p.cover.x, 1.0);
    assert_eq!(
        p.cover.y, 0.0,
        "a clear sky has no floor, whatever the deck over the sun"
    );
    assert_eq!(CloudShadowParams::new(false, &clear, &a).cover.x, 0.0);
    let storm = sky(1.0, 0.0, 0.0);
    assert_eq!(CloudShadowParams::new(true, &storm, &a).cover.y, 1.0);
    let fog = sky(0.35, 0.9, 0.0);
    assert!((CloudShadowParams::new(true, &fog, &a).cover.y - 0.9).abs() < 1.0 / 256.0);
}

/// `CloudShadowParams` and `CloudShadow` in the shader name the same lanes
/// in the same order — a uniform whose sides disagree binds and reads
/// garbage.
#[test]
fn the_uniform_declares_the_same_fields_on_both_sides() {
    fn fields(src: &str, header: &str, strip: &str) -> Vec<String> {
        let body = src
            .split_once(header)
            .unwrap_or_else(|| panic!("no `{header}` in the source"))
            .1;
        let body = body.split_once("\n}").expect("unterminated struct").0;
        body.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("//"))
            .filter_map(|l| l.trim_start_matches(strip).trim().split(':').next())
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .collect()
    }
    let wgsl = std::fs::read_to_string(SHADER).expect("shader");
    let rust = std::fs::read_to_string("src/render/cloud_shadow.rs").expect("cloud_shadow.rs");
    let shader_fields = fields(&wgsl, "struct CloudShadow {", "");
    let rust_fields = fields(&rust, "pub struct CloudShadowParams {", "pub ");
    assert_eq!(shader_fields, rust_fields);
    assert_eq!(shader_fields.len(), 5);
}
