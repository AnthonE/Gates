//! The deck's shadows on the land (`render/cloud_shadow.rs`): the drift the
//! shader runs on between writes, the weather's lanes, the one reading of
//! the deck the rig and the shader share, and the uniform's two sides.

#![cfg(all(feature = "render", not(target_arch = "wasm32")))]

use bevy::light::light_consts::lux;
use bevy::math::DVec2;
use bevy::prelude::*;
use client::config::{Clouds, Quality};
use client::render::cloud_shadow::{
    field_words, level, Anchor, CloudShadowParams, Lane, DRIFT_SLACK_M, LANE_SETTLE_S, LANE_STEP,
};
use client::render::feed::Feed;
use client::render::quality::{self, Gfx};
use client::render::rig::{self, day_night, DayPin, Sun};
use client::render::sky::{self, CloudField, CLOUD_SCALE_M, FIELD_N, FIELD_PERIOD};
use client::render::weather::WeatherNow;

const SHADER: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/shaders/cloud_shadow.wgsl"
);

/// The weather's drift, a frame on, the way `weather::update` keeps it:
/// metres, summed in f64 and wrapped a field over — and Bevy's clock,
/// which is not summed. Whether the anchor took this frame.
fn frame(a: &mut Anchor, drift: &mut DVec2, clock: &mut f64, wind: Vec2, sun: Vec3) -> bool {
    let period_m = f64::from(FIELD_PERIOD as f32 * CLOUD_SCALE_M);
    let dt = 1.0f32 / 60.0;
    *clock += f64::from(dt);
    let sum = *drift + wind.as_dvec2() * f64::from(dt);
    *drift = DVec2::new(sum.x.rem_euclid(period_m), sum.y.rem_euclid(period_m));
    a.track(
        *clock as f32,
        -drift.as_vec2() / CLOUD_SCALE_M,
        -wind / CLOUD_SCALE_M,
        sun,
    )
}

/// A steady wind is written once, across both of the drift's wraps and out
/// where f32 metres are coarsest: the weather sums in f64 and the run stays
/// on it. A wind that changes, a sun that moves and a clock that comes
/// round are each one more. Every write re-prepares the ground and every
/// rock on it, so a write a frame would be the cost this anchor exists to
/// avoid.
#[test]
fn a_steady_wind_is_written_rarely() {
    let sun = Vec3::new(0.3, 0.6, -0.74).normalize();
    let mut a = Anchor::default();
    let mut drift = DVec2::new(100.0, 28_700.0);
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
        wrote <= 2,
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
    let off = -drift.as_vec2() / CLOUD_SCALE_M;
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

/// The shader's floor is the rig's occlusion with no deck over the sun:
/// nothing on a clear sky whatever the deck does, all of it in a storm,
/// the fog's own share in the fog. Off is the one constant.
#[test]
fn the_floor_is_the_rigs_occlusion_without_the_deck() {
    let weather = |cloud: f32, fog: f32, sun_cover: f32| {
        let mut w = WeatherNow::default();
        w.cloud = cloud;
        w.fog = fog;
        w.sun_cover = sun_cover;
        w
    };
    let clear = weather(0.35, 0.0, 1.0);
    assert_eq!(
        clear.occlusion_floor(),
        0.0,
        "a clear sky has no floor, whatever the deck over the sun"
    );
    assert_eq!(clear.occlusion(), 1.0);
    assert_eq!(weather(1.0, 0.0, 0.0).occlusion_floor(), 1.0);
    let fog = weather(0.35, 0.9, 0.3);
    assert_eq!(fog.occlusion_floor(), 0.9);
    assert_eq!(fog.occlusion(), 0.9);

    let a = Anchor::default();
    assert_eq!(
        CloudShadowParams::new(false, &a, 0.4, 0.6),
        CloudShadowParams::off()
    );
    let on = CloudShadowParams::new(true, &a, 0.4, 0.6);
    assert_eq!(on.cover, Vec4::new(1.0, 0.4, 0.6, sky::EDGE));
}

/// A lane follows a weather change in steps, never further than one from
/// the weather, at most a step's worth of writes — and once the sky holds
/// still it is the weather's own value, which is what the rig reads.
#[test]
fn a_lane_settles_on_the_weathers_own_value() {
    let mut lane = Lane::default();
    let dt = 1.0f32 / 60.0;
    let mut t = 5.0f32;
    assert!(lane.track(t, 0.35), "the first frame writes");
    // A minute of steady sky: nothing.
    let wrote: usize = (0..3600)
        .map(|_| {
            t += dt;
            usize::from(lane.track(t, 0.35))
        })
        .sum();
    assert_eq!(wrote, 0);
    // A minute's change from 0.35 to 1.0 in the weather's per-mille steps,
    // a step every few frames as the sim's ticks deliver them.
    let mut wrote = 0;
    for k in 0..3600u32 {
        t += dt;
        let v = (350 + 650 * k / 3600) as f32 * 0.001;
        wrote += usize::from(lane.track(t, v));
        assert!(
            (lane.written - v).abs() < LANE_STEP,
            "{} behind {v}",
            lane.written
        );
    }
    let steps = (0.65 / LANE_STEP).ceil() as usize;
    assert!(wrote <= steps + 2, "a minute's change wrote {wrote} times");
    // Held still: the exact value, once, inside the settle.
    let v = 1.0;
    let mut wrote = 0;
    for _ in 0..((LANE_SETTLE_S / dt) as usize + 2) {
        t += dt;
        wrote += usize::from(lane.track(t, v));
    }
    assert_eq!(lane.written, v);
    assert!(wrote <= 1);
    // The clock comes round: still the value.
    lane.track(0.25, v);
    assert_eq!(lane.written, v);
}

/// The rig's sun at the eye and the shader's reference for the eye are one
/// reading: the mip the shader binds is the one the sky's image holds, byte
/// for byte, its size and texel offset are the uniform's, and it reads its
/// own texels at their centres.
#[test]
fn the_rig_and_the_shader_read_one_field() {
    let field = CloudField::new(20260731, FIELD_N);
    let soft = field.soft(level());
    let off = CloudShadowParams::off();
    assert_eq!(soft.n() as f32, off.deck.w);
    assert_eq!(soft.offset(), off.fade.w);
    // The same bytes the GPU image keeps at that level.
    let image = field.image();
    let data = image.data.as_ref().expect("image data");
    let start: usize = (0..level()).map(|l| (FIELD_N >> l) * (FIELD_N >> l)).sum();
    assert_eq!(&data[start..start + soft.n() * soft.n()], soft.bytes());
    // Four to a word, little end first, as `field_texel` unpacks them.
    let words = field_words(&soft);
    assert_eq!(words.len() % 4, 0);
    assert_eq!(&words[..soft.bytes().len()], soft.bytes());
    // Texel (i, j)'s centre reads texel (i, j), wrapping both ways.
    let n = soft.n();
    let per = FIELD_PERIOD as f32 / n as f32;
    for (i, j) in [(0usize, 0usize), (3, 7), (n - 1, n - 1), (n / 2, 1)] {
        for wrap in [0.0, FIELD_PERIOD as f32, -(FIELD_PERIOD as f32)] {
            let q = Vec2::new(
                (i as f32 + soft.offset()) * per + wrap,
                (j as f32 + soft.offset()) * per,
            );
            let want = soft.bytes()[j * n + i] as f32 / 255.0;
            assert!((soft.at(q) - want).abs() < 1e-4, "texel ({i}, {j}) at {q}");
        }
    }
}

/// Where the ground draws the deck's shadows, the eye's own cloud no longer
/// puts the sun's shadows out — the ground a hundred metres off is lit back
/// up to its own sky, and a sunlit hill with no shadow map is the frame
/// this rules out. A heavy sky and the fog still do; and without the
/// ground's shadows (LOW or MEDIUM clouds, a browser) the eye's cloud does
/// as it did.
#[test]
fn the_eyes_cloud_keeps_the_shadows_where_the_ground_draws_the_decks() {
    let run = |gfx: Gfx, w: WeatherNow| {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<Feed>();
        app.insert_resource(DayPin::capture());
        let settings = client::render::Settings {
            gfx,
            ..Default::default()
        };
        app.insert_resource(settings);
        app.init_resource::<rig::FlameGain>();
        app.insert_resource(w);
        app.add_systems(Update, day_night);
        app.world_mut().spawn((
            Sun,
            DirectionalLight {
                illuminance: lux::DIRECT_SUNLIGHT,
                shadows_enabled: true,
                ..default()
            },
            Transform::default(),
        ));
        app.update();
        let world = app.world_mut();
        let d = world
            .query_filtered::<&DirectionalLight, With<Sun>>()
            .single(world)
            .unwrap();
        (d.shadows_enabled, d.illuminance)
    };
    let high = Gfx {
        clouds: Clouds::High,
        ..quality::preset(Quality::High)
    };
    assert!(quality::cloud_shadows(high));
    let low = Gfx {
        clouds: Clouds::Low,
        ..high
    };
    assert!(!quality::cloud_shadows(low));
    let medium = Gfx {
        clouds: Clouds::Medium,
        ..high
    };
    assert!(!quality::cloud_shadows(medium));

    let mut under = WeatherNow::default();
    under.cloud = 0.35;
    under.sun_cover = 1.0;
    let (shadows, lit) = run(high, under);
    assert!(shadows, "the eye under a cloud put the land's shadows out");
    let mut clear = under;
    clear.sun_cover = 0.0;
    let (_, full) = run(high, clear);
    assert!(
        lit < full * 0.2,
        "the eye's cloud still dims the sun ({lit} of {full})"
    );
    assert!(
        !run(low, under).0,
        "LOW keeps the eye's cloud putting them out"
    );
    let mut fog = under;
    fog.fog = 0.9;
    assert!(!run(high, fog).0, "the fog puts the shadows out");
    let mut storm = under;
    storm.cloud = 1.0;
    assert!(!run(high, storm).0, "a storm puts the shadows out");
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
