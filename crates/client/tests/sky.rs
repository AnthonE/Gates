//! The browser's sky — the deck bakes a clear sky behind its clouds on
//! wasm32 and nothing at all on the desktop, and this holds the two bakes
//! to each other (browser sky v0, `DECISIONS.md` §open).
//!
//! **Why both bakes are reachable natively.** `sky::BAKE_BACKDROP` is a
//! `cfg!` and a gate that only ran the desktop's arm would be green over a
//! browser sky that composited wrong, bit-for-bit — the shape `CLAUDE.md`
//! records for `render/verbs.rs`'s matches (green twice on the workspace,
//! red at the gate that costs five minutes). `cloud_cubemap_with(seed,
//! backdrop)` takes the choice as a value, so the browser's bake is run here
//! under the desktop's toolchain and compared texel for texel.
//!
//! Three mutants were run against this file before it was believed:
//! premultiplying the cloud over black in the browser arm (the "identical
//! where opaque" and "backdrop where clear" checks both go red), dropping
//! `HORIZON_DESAT` (the chroma check goes red), and forgetting the
//! below-horizon texels (the "never black" sweep goes red on face 3).
#![cfg(feature = "render")]

use bevy::math::Vec3;
use bevy::pbr::FogFalloff;
use client::render::fill::{linear_to_srgb, luminance, srgb_to_linear};
use client::render::rig::{island_medium, AIR_DENSITY, HAZE_SCALE};
use client::render::sky::{
    air_chroma, backdrop_at, browser_haze, cloud_cubemap_with, cube_dir, deck_hue, ground_air,
    haze_color, AIR_FLOOR, BAKE_BACKDROP, CLOUD_NITS, DECK_GAIN, HORIZON_DESAT, HORIZON_GAIN,
    SKY_FACE,
};

const SEED: u64 = 20_260_731;

fn texels(backdrop: bool) -> Vec<u8> {
    cloud_cubemap_with(SEED, backdrop)
        .data
        .expect("the deck is built with CPU-side data")
}

fn at(data: &[u8], face: usize, x: u32, y: u32) -> [u8; 4] {
    let n = SKY_FACE;
    let i = (((face as u32 * n + y) * n + x) * 4) as usize;
    [data[i], data[i + 1], data[i + 2], data[i + 3]]
}

fn encoded(rgb: [f32; 3]) -> [u8; 3] {
    let mut out = [0u8; 3];
    for c in 0..3 {
        out[c] = (linear_to_srgb(rgb[c].clamp(0.0, 1.0)) * 255.0).round() as u8;
    }
    out
}

/// The desktop's deck is what it always was: zero wherever there is no
/// cloud, including the whole lower hemisphere. The atmosphere paints that
/// sky, and a non-zero clear texel would be a second one added to it.
#[test]
fn the_desktop_deck_leaves_every_clear_texel_zero() {
    let d = texels(false);
    let n = SKY_FACE;
    // Face 3 is -Y: nothing above the horizon anywhere on it.
    for y in 0..n {
        for x in 0..n {
            assert_eq!(at(&d, 3, x, y), [0, 0, 0, 0], "a -Y texel is not zero");
        }
    }
    // And the upper hemisphere is mostly clear: a deck that covered every
    // texel would have no top stop either (`sky.rs`'s header).
    let clear = (0..n)
        .flat_map(|y| (0..n).map(move |x| (x, y)))
        .filter(|&(x, y)| at(&d, 2, x, y) == [0, 0, 0, 0])
        .count();
    assert!(
        clear > (n * n / 5) as usize,
        "only {clear} of {} +Y texels are clear sky — the deck has grown a backdrop natively",
        n * n
    );
}

/// The default bake is the target's: zero texels wherever the atmosphere
/// draws (here, and the browser's WebGPU module), a sky under WebGL2.
#[test]
fn the_default_bake_is_the_targets() {
    assert_eq!(BAKE_BACKDROP, cfg!(webgl2));
    // And `cloud_cubemap` goes through the switch rather than one arm — read
    // off the source, because a `cfg!` cannot be flipped from a test.
    let src =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/render/sky.rs")).unwrap();
    assert!(
        src.contains("cloud_cubemap_with(seed, BAKE_BACKDROP)"),
        "`sky::cloud_cubemap` no longer bakes through `BAKE_BACKDROP`"
    );
}

/// The browser's deck is the desktop's with a sky under it: the same cloud
/// wherever a cloud is opaque, exactly the backdrop wherever the desktop's
/// texel is zero, and never black anywhere. "The same cloud" once the
/// desktop's two corrections are undone — the atmosphere's hue it takes out
/// (`deck_hue`) and the headroom it keeps (`DECK_GAIN`); a browser has no
/// atmosphere and needs neither.
#[test]
fn the_browser_deck_is_the_desktops_over_a_sky() {
    let native = texels(false);
    let web = texels(true);
    let n = SKY_FACE;
    let (mut same, mut backdrop, mut blended) = (0usize, 0usize, 0usize);
    for face in 0..6 {
        for y in 0..n {
            for x in 0..n {
                let a = at(&native, face, x, y);
                let b = at(&web, face, x, y);
                assert_eq!(b[3], 255, "a browser texel is not opaque");
                assert!(
                    b[0] > 0 && b[1] > 0 && b[2] > 0,
                    "a browser texel is black at face {face} ({x}, {y}): {b:?}"
                );
                let d = cube_dir(face, x, y, n);
                let want = encoded(backdrop_at(d));
                if a == [0, 0, 0, 0] {
                    // No cloud here: the sky, exactly.
                    // Within one step: the composer encodes through a table,
                    // the reference here through the exact curve.
                    for c in 0..3 {
                        assert!(
                            (b[c] as i32 - want[c] as i32).abs() <= 1,
                            "clear texel {b:?} is not the backdrop {want:?}"
                        );
                    }
                    backdrop += 1;
                } else {
                    // The desktop's cloud, as the browser would store it.
                    // Decoded, because the bytes are sRGB and a sum is not.
                    let lin = |v: u8| srgb_to_linear(v as f32 / 255.0);
                    let hue = deck_hue(d.y);
                    let al: [f32; 3] = core::array::from_fn(|c| lin(a[c]) * DECK_GAIN / hue[c]);
                    let bl: [f32; 3] = core::array::from_fn(|c| lin(b[c]));
                    // One byte of the desktop's, undone by the same factors:
                    // where the air gives a channel back, the cube keeps
                    // little of it, and a byte there is worth a lot.
                    let tol: [f32; 3] = core::array::from_fn(|c| {
                        let step = srgb_to_linear((a[c] as f32 + 1.0) / 255.0) - lin(a[c]);
                        0.02 + step * DECK_GAIN / hue[c]
                    });
                    // A texel the desktop brought back in range (keeping
                    // its hue) says only "at least", in every channel.
                    let over = a[..3].contains(&255);
                    let clipped = |_c: usize| over;
                    if (0..3).all(|c| clipped(c) || (al[c] - bl[c]).abs() <= tol[c]) {
                        // An opaque cloud: the same cloud on both targets.
                        same += 1;
                    } else {
                        // A cloud's edge. The desktop stores `cloud · cov`
                        // over black; the browser `cloud · cov + sky ·
                        // (1 − cov)`, so the sky only ever adds, by at most
                        // the sky itself: `a ≤ b ≤ a + sky`.
                        let sky = backdrop_at(d);
                        for c in 0..3 {
                            assert!(
                                bl[c] >= al[c] - tol[c]
                                    && (clipped(c) || bl[c] <= al[c] + sky[c] + tol[c]),
                                "edge texel {b:?} is not cloud {a:?} plus at most the sky {want:?} (channel {c}: {} .. {})",
                                al[c],
                                al[c] + sky[c]
                            );
                        }
                        blended += 1;
                    }
                }
            }
        }
    }
    assert!(
        same > 1000,
        "no opaque cloud is byte-identical across targets ({same})"
    );
    assert!(
        blended > 1000,
        "no cloud edge blends into the sky ({blended})"
    );
    assert!(
        backdrop > 100_000,
        "too few clear texels carry the sky ({backdrop})"
    );
}

/// The gradient is the sky's: brighter and greyer at the horizon than at
/// the zenith, by the two constants that say so, along a curve that ends
/// exactly at both.
#[test]
fn the_backdrop_brightens_and_greys_toward_the_horizon() {
    let zenith = backdrop_at(Vec3::Y);
    let horizon = backdrop_at(Vec3::X);
    let lz = luminance(zenith);
    let lh = luminance(horizon);
    assert!(lz > 0.0);
    assert!(
        (lh / lz - HORIZON_GAIN).abs() < 1e-3,
        "horizon/zenith luminance is {} against HORIZON_GAIN {HORIZON_GAIN}",
        lh / lz
    );
    // Chroma: the horizon's blue lead over red shrinks by the desaturation.
    let cz = (zenith[2] - zenith[0]) / lz;
    let ch = (horizon[2] - horizon[0]) / lh;
    assert!(cz > 0.0, "the zenith is not blue: {zenith:?}");
    assert!(
        (ch / cz - (1.0 - HORIZON_DESAT)).abs() < 1e-3,
        "the horizon kept {} of the zenith's chroma against 1 - HORIZON_DESAT = {}",
        ch / cz,
        1.0 - HORIZON_DESAT
    );
    // Monotone in elevation, and flat below the horizon.
    let mut last = lh;
    for k in 1..=20 {
        let e = k as f32 / 20.0;
        let l = luminance(backdrop_at(Vec3::new((1.0 - e * e).sqrt(), e, 0.0)));
        assert!(l <= last + 1e-6, "luminance rose with elevation at e={e}");
        last = l;
    }
    let below = backdrop_at(Vec3::new(0.6, -0.8, 0.0));
    assert_eq!(
        below, horizon,
        "below the horizon is not the horizon colour"
    );
    // The curve's shape is the air-mass one: halfway up the sky it is much
    // nearer the zenith than the horizon, which a linear ramp would not be —
    // the floor is what concentrates the band at the skyline.
    let mid = luminance(backdrop_at(Vec3::new(0.5f32.sqrt(), 0.5f32.sqrt(), 0.0)));
    let linear_mid = (lz + lh) * 0.5;
    assert!(
        mid < linear_mid,
        "at 45° the sky reads {mid} against a linear ramp's {linear_mid} — the air-mass floor \
         {AIR_FLOOR} is not shaping the gradient"
    );
}

/// The zenith's BRIGHTNESS is derived from the fill and not typed: the fill's
/// sky irradiance over π, in the deck's units. Its colour stopped being the
/// fill's on 2026-09-13 (`the_browser_zenith_is_the_colour_the_air_scatters`);
/// until then this held all three channels to the fill's near-white tint.
#[test]
fn the_zenith_is_the_fills_brightness_over_pi() {
    let zenith = backdrop_at(Vec3::Y);
    let want = luminance(client::render::fill::sky_lux()) / core::f32::consts::PI / CLOUD_NITS;
    let got = luminance(zenith);
    assert!(
        (got / want - 1.0).abs() < 1e-4,
        "zenith luminance {got} against the fill's {want}"
    );
    // And it lands inside the texel range with room for the horizon's gain,
    // in EVERY channel: the bake clamps each one, so a blue that clipped while
    // the luminance did not would bend the hue where the eye looks most.
    let lh = luminance(backdrop_at(Vec3::X));
    assert!(lh < 1.0, "the horizon clips at {lh} of CLOUD_NITS");
    for (name, rgb) in [("zenith", zenith), ("horizon", backdrop_at(Vec3::X))] {
        for c in 0..3 {
            assert!(rgb[c] < 1.0, "the {name} clips in channel {c}: {rgb:?}");
        }
    }
    // Round trip the encoder this file compares through.
    let v = 0.3f32;
    assert!((srgb_to_linear(linear_to_srgb(v)) - v).abs() < 1e-5);
}

/// The air at the ground is the published earthlike medium at `AIR_DENSITY`
/// — rebuilt here from Hillaire's numbers (Rayleigh as
/// `bevy_pbr::medium::ScatteringMedium::earthlike` has it, Mie as Bevy 0.16's
/// `Atmosphere::EARTH` had it before 0.18 swapped the pair), never from the
/// function under test. Counting the ozone tent, leaving absorption out of
/// extinction, or taking 0.18's swapped Mie each moves a channel.
#[test]
fn ground_air_is_the_published_medium_at_the_ground() {
    let (ext, scat) = ground_air(&island_medium());
    let rayleigh = Vec3::new(5.802e-6, 13.558e-6, 33.100e-6);
    let (mie_abs, mie_scat) = (Vec3::splat(0.444e-6), Vec3::splat(3.996e-6));
    let want_scat = (rayleigh + mie_scat) * AIR_DENSITY;
    let want_ext = (rayleigh + mie_scat + mie_abs) * AIR_DENSITY;
    for (got, want, what) in [
        (ext, want_ext, "extinction"),
        (scat, want_scat, "inscattering"),
    ] {
        // Relative: these are f32 near 1e-5, where one rounding is ~1e-12
        // absolute. Counting the ozone tent moves red and green by 6-10%.
        let rel = ((got - want) / want).abs().max_element();
        assert!(
            rel < 1e-5,
            "{what} is {got:?}; the published air at the ground is {want:?} ({rel} off)"
        );
    }
}

/// The browser's zenith is the colour the air scatters, at the fill's
/// luminance — and it is BLUE. Borrowing the fill's near-white tint gave a
/// blue lead of 0.42 and a sky the first real GPU drew as grey (2026-09-13).
#[test]
fn the_browser_zenith_is_the_colour_the_air_scatters() {
    let zenith = backdrop_at(Vec3::Y);
    let l = luminance(zenith);
    let want_l = luminance(client::render::fill::sky_lux()) / core::f32::consts::PI / CLOUD_NITS;
    assert!(
        (l / want_l - 1.0).abs() < 1e-4,
        "zenith luminance {l} against the fill's {want_l}"
    );
    let c = air_chroma();
    let (_, s) = ground_air(&island_medium());
    let ls = luminance(s.to_array());
    for i in 0..3 {
        assert!(
            (zenith[i] / l - c[i]).abs() < 1e-4,
            "zenith channel {i} is not the air's chroma"
        );
        assert!(
            (c[i] - s.to_array()[i] / ls).abs() < 1e-5,
            "air_chroma channel {i} is not the air's"
        );
    }
    let blue_lead = (zenith[2] - zenith[0]) / l;
    assert!(
        blue_lead > 1.5,
        "the zenith's blue lead is {blue_lead}: the sky has gone back toward the fill's grey"
    );
}

/// The browser's haze is the desktop's air over the desktop's sightline,
/// coloured by the sky's own horizon, dimmed with the day, and carries no sun
/// glow nobody owns.
#[test]
fn the_browser_haze_is_the_desktops_air_and_the_skys_horizon() {
    let fog = browser_haze(1.0);
    let (ext, scat) = ground_air(&island_medium());
    let (ext, scat) = (ext * HAZE_SCALE, scat * HAZE_SCALE);
    match fog.falloff {
        FogFalloff::Atmospheric {
            extinction,
            inscattering,
        } => {
            assert_eq!(
                extinction, ext,
                "the haze's extinction is not the island's air"
            );
            assert_eq!(
                inscattering, scat,
                "the haze's inscattering is not the island's air"
            );
        }
        other => panic!("the browser haze is not atmospheric: {other:?}"),
    }
    let h = backdrop_at(Vec3::X);
    let c = fog.color.to_linear();
    for (got, want) in [(c.red, h[0]), (c.green, h[1]), (c.blue, h[2])] {
        assert!(
            (got - want).abs() < 1e-6,
            "the haze is not the sky's horizon: {c:?} against {h:?}"
        );
    }
    assert_eq!(
        fog.directional_light_color,
        bevy::color::Color::NONE,
        "the haze grew a sun glow"
    );
    let night = haze_color(0.0).to_linear();
    assert!(
        night.red == 0.0 && night.green == 0.0 && night.blue == 0.0,
        "the haze glows at night"
    );
}

/// The stars are points, not texels: above the horizon, unit directions,
/// and mostly faint with a few bright — and the cube no longer carries any,
/// so a night deck is only the moon and the cloud.
#[test]
fn the_stars_are_a_mesh_above_the_horizon() {
    use bevy::mesh::VertexAttributeValues;
    use client::render::stars::{star_mesh, STARS};
    let mesh = star_mesh(STARS);
    let Some(VertexAttributeValues::Float32x3(pos)) =
        mesh.attribute(bevy::mesh::Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("no positions");
    };
    let Some(VertexAttributeValues::Float32x4(col)) =
        mesh.attribute(bevy::mesh::Mesh::ATTRIBUTE_COLOR)
    else {
        panic!("no colours");
    };
    assert_eq!(pos.len(), STARS as usize * 4);
    for p in pos {
        let d = Vec3::from_array(*p);
        assert!(
            (d.length() - 1.0).abs() < 1e-4 && d.y > 0.02,
            "a star at {d}"
        );
    }
    let bright = col.iter().step_by(4).filter(|c| c[0] > 0.5).count();
    assert!(
        bright * 10 < STARS as usize && bright * 30 > STARS as usize,
        "{bright} of {STARS} stars are bright"
    );
}

/// Without dual-source blending the atmosphere lays the sky over the deck
/// through the MEAN of its transmittance, so the deck's lift there is one
/// number in all three channels: it evens the brightness and tints nothing.
#[test]
fn a_grey_composite_gets_a_grey_lift() {
    for i in 0..=20 {
        let y = i as f32 / 20.0;
        let g = client::render::sky::deck_hue_for(y, false);
        assert!(g[0].is_finite() && g[0] > 0.0, "lift {g:?} at y {y}");
        assert_eq!(g[0], g[1]);
        assert_eq!(g[1], g[2]);
        // And the per-channel lift is a hue: blue lifted most, red least.
        let h = client::render::sky::deck_hue_for(y, true);
        assert!(
            h[2] >= h[1] && h[1] >= h[0],
            "per-channel lift {h:?} at y {y}"
        );
    }
}

/// The sky's fog and the ground's are one air: level with the horizon a sky
/// ray is fogged exactly as much as the island's far edge (`rig::FAR_M`
/// through the same extinction), straight up it crosses only the layer's
/// depth, and in clear weather there is none.
#[test]
fn the_sky_fog_meets_the_ground_fog_at_the_horizon() {
    use client::render::rig::FAR_M;
    use client::render::sky::{SkyFog, DUST_LAYER_M, FOG_LAYER_M};
    assert_eq!(SkyFog::CLEAR.band(0.0), 0.0);
    // The fog preset's ground fog, a storm's rain haze, and the dust of a
    // windy overcast over the sand, alone and with a little fog.
    for (fog, rain, dust) in [
        (0.0199, 0.0, 0.0),
        (0.0, 0.005, 0.0),
        (0.0007, 0.005, 0.0),
        (0.0, 0.0, 0.0043),
        (0.0007, 0.0, 0.0026),
    ] {
        let air = SkyFog::new(fog, rain).with_dust(dust);
        let far = 1.0 - (-(fog + rain + dust) * FAR_M).exp();
        assert!(
            (air.band(-0.02) - far).abs() < 1e-5,
            "the horizon read {} where the far land reads {far}",
            air.band(-0.02)
        );
        let mut last = 1.0;
        for i in 0..=50 {
            let b = air.band(i as f32 / 50.0);
            assert!(b <= last + 1e-6, "the fog thickened going up at {i}");
            last = b;
        }
    }
    let ground = SkyFog::new(0.02, 0.0);
    let up = 1.0 - (-(0.02 * FOG_LAYER_M) / 1.02f32).exp();
    assert!((ground.band(1.0) - up).abs() < 1e-5);
    // Dust stands deeper than the ground fog: overhead it crosses its own
    // layer, and the sky there is pale rather than gone.
    let dusty = SkyFog::CLEAR.with_dust(0.004);
    let up = 1.0 - (-(0.004 * DUST_LAYER_M) / 1.02f32).exp();
    assert!((dusty.band(1.0) - up).abs() < 1e-5);
    assert!(dusty.band(1.0) < 0.9 && dusty.band(-0.02) > 0.99);
}

const DESERT: (f32, f32) = (2048.0, 3400.0);
const MEADOW: (f32, f32) = (1500.0, 600.0);

/// A dry wind of `wind` blowing toward `toward`, under overcast at noon.
fn dry_wind(wind: f32, toward: bevy::math::Vec2) -> client::render::weather::WeatherNow {
    use client::render::weather::WeatherNow;
    // A private field: built and then set, not by struct update.
    let mut w = WeatherNow::default();
    w.wind = wind;
    w.wind_dir = toward;
    w.cloud = 0.9;
    w.dark = 0.35;
    w.sun = Vec3::new(0.3, 0.9, 0.3).normalize();
    w.sun_lux = 1.0;
    w
}

/// Dust is the sand's and the wind's: a windy dry day over the desert
/// carries it, and a calm one, a rainy one, one just after the rain and
/// the temperate south carry none.
#[test]
fn dust_rides_a_dry_wind_over_the_sand_and_nowhere_else() {
    use bevy::math::Vec2;
    use client::render::weather::dust_at;
    use sim_core::terrain::aridity;
    let (dx, dz) = DESERT;
    let (mx, mz) = MEADOW;
    assert_eq!(aridity(SEED, dx, dz), 1.0, "the desert point is not desert");
    assert_eq!(aridity(SEED, mx, mz), 0.0, "the meadow point is arid");
    // Blowing north: off the desert's interior, not in off the sea.
    let north = Vec2::new(0.0, 1.0);
    // Overcast's wind over the sand: a beige wall a few hundred metres off.
    let w = dry_wind(0.35, north);
    let d = dust_at(SEED, dx, dz, &w);
    assert!(d > 0.6, "a windy dry desert carries {d}");
    // Clear weather's breeze: a haze, not a wall.
    let light = dust_at(SEED, dx, dz, &dry_wind(0.2, north));
    assert!(light > 0.0 && light < 0.4 * d, "a breeze carries {light}");
    // Calm.
    for wind in [0.0, 0.1] {
        assert_eq!(
            dust_at(SEED, dx, dz, &dry_wind(wind, north)),
            0.0,
            "calm at {wind}"
        );
    }
    // Rain lays it on its own, the moment it starts.
    let mut rain = dry_wind(0.4, north);
    rain.rain = 0.35;
    assert_eq!(dust_at(SEED, dx, dz, &rain), 0.0, "dust in the rain");
    // A drizzle thins it.
    let mut drizzle = dry_wind(0.35, north);
    drizzle.rain = 0.05;
    let damp = dust_at(SEED, dx, dz, &drizzle);
    assert!(damp > 0.0 && damp < 0.8 * d, "a drizzle carries {damp}");
    // The schedule's rain of the last few minutes keeps the sand down.
    let mut lately = dry_wind(0.35, north);
    lately.rain_lately = 0.3;
    assert_eq!(dust_at(SEED, dx, dz, &lately), 0.0, "dust off wet sand");
    // This client's own wet ground does not: one who stood through the
    // shower and one who just joined see one haze.
    let mut echo = dry_wind(0.35, north);
    echo.ground_wet = 0.35;
    assert_eq!(dust_at(SEED, dx, dz, &echo), d, "the local wet moved it");
    // The temperate south, in any wind.
    for toward in [north, -north, Vec2::X, -Vec2::X] {
        assert_eq!(dust_at(SEED, mx, mz, &dry_wind(0.35, toward)), 0.0);
    }
}

/// The sea lifts no sand, though the desert's band of latitude runs on out
/// over it: on the north beach a wind in off the water is far cleaner than
/// one off the dunes, and out at sea a sea wind carries none.
#[test]
fn a_sea_wind_carries_no_sand() {
    use bevy::math::Vec2;
    use client::render::weather::dust_at;
    use sim_core::terrain::{aridity, height};
    let x = DESERT.0;
    let coast = (3000..4096)
        .step_by(10)
        .rev()
        .map(|z| z as f32)
        .find(|&z| height(SEED, x, z) > 2.0)
        .expect("no north coast");
    let beach = coast - 50.0;
    assert_eq!(aridity(SEED, x, coast + 600.0), 1.0, "the sea is not arid");
    let (from_north, from_south) = (Vec2::new(0.0, -1.0), Vec2::new(0.0, 1.0));
    let off_sea = dust_at(SEED, x, beach, &dry_wind(0.35, from_north));
    let off_sand = dust_at(SEED, x, beach, &dry_wind(0.35, from_south));
    assert!(off_sand > 0.6, "a wind off the dunes carries {off_sand}");
    assert!(
        off_sea < 0.25 * off_sand,
        "a sea wind carries {off_sea} against the dunes' {off_sand}"
    );
    let at_sea = dust_at(SEED, x, coast + 600.0, &dry_wind(0.35, from_north));
    assert_eq!(at_sea, 0.0, "dust over the open sea");
}

/// The air at the eye came from upwind: just short of the desert's edge,
/// a wind off the sand brings the haze and a wind toward it does not.
#[test]
fn dust_spills_downwind_of_the_desert() {
    use bevy::math::Vec2;
    use client::render::weather::dust_at;
    use sim_core::terrain::aridity;
    let x = DESERT.0;
    let edge = (2048..4000)
        .step_by(10)
        .map(|z| z as f32)
        .find(|&z| aridity(SEED, x, z) > 0.0)
        .expect("no desert north of the centre");
    let z = edge - 100.0;
    let off_the_sand = dust_at(SEED, x, z, &dry_wind(0.35, Vec2::new(0.0, -1.0)));
    let toward_it = dust_at(SEED, x, z, &dry_wind(0.35, Vec2::new(0.0, 1.0)));
    assert!(off_the_sand > 0.0, "no dust downwind of the desert's edge");
    assert_eq!(toward_it, 0.0, "dust upwind of the desert");
}

/// Dust goes through the one fog path: it is part of the extinction the
/// rig gives the ground's `DistanceFog`, the sky's band at the horizon is
/// that fog at the island's far edge, the two are one colour, and that
/// colour is sand rather than sky.
#[test]
fn the_dust_is_the_ground_fogs_and_the_skys_alike() {
    use bevy::math::Vec2;
    use client::render::deck::SkyFogParams;
    use client::render::rig::FAR_M;
    use client::render::sky::{fog_color, fog_color_dust, fog_grey, fog_rgb, ComposeParams};
    let mut w = dry_wind(0.35, Vec2::new(0.0, 1.0));
    w.dust = client::render::weather::dust_at(SEED, DESERT.0, DESERT.1, &w);
    assert!(w.dust_part() > 0.0);
    assert_eq!(w.fog_sigma(), w.dust_part(), "dust is not the fog's");
    let p = ComposeParams::from_weather(&w);
    let far = 1.0 - (-w.fog_sigma() * FAR_M).exp();
    assert!(
        (p.fog.band(-0.02) - far).abs() < 1e-5,
        "the dusty horizon read {} where the far land reads {far}",
        p.fog.band(-0.02)
    );
    let sky = SkyFogParams::new(&p, false).color;
    let ground = fog_color_dust(w.sun_lux, w.dark, w.fog_sigma(), w.dust_part()).to_linear();
    for (a, b) in [
        (sky.x, ground.red),
        (sky.y, ground.green),
        (sky.z, ground.blue),
    ] {
        assert!(
            (a - b).abs() < 1e-6,
            "sky fog {sky} against ground fog {ground:?}"
        );
    }
    // Sand: warm where the sky's horizon is blue, and no brighter.
    assert!(
        sky.x > sky.y && sky.y > sky.z,
        "the dust is not sand: {sky}"
    );
    let plain = fog_rgb(w.sun_lux, w.dark, fog_grey(w.fog_sigma()));
    let l = luminance([sky.x, sky.y, sky.z]);
    assert!(
        (l / luminance(plain) - 1.0).abs() < 1e-3,
        "the dust lit the fog"
    );
    // And no dust, no tint: a clear sky's colour is what it always was.
    let clear = fog_color_dust(1.0, 0.0, 0.0, 0.0).to_linear();
    assert_eq!(clear, fog_color(1.0, 0.0, 0.0).to_linear());
}

/// The deck drifts the way the wind blows — the way the rain slants
/// (`rain::params_for` throws drops along `wind_dir`). It is read at
/// `plane + drift`, so its offset runs against the distance travelled.
#[test]
fn the_deck_drifts_downwind() {
    use bevy::math::Vec2;
    use client::render::sky::ComposeParams;
    use client::render::weather::WeatherNow;
    // A private field: built and then moved, not by struct update.
    let mut w = WeatherNow::default();
    w.drift = Vec2::new(900.0, -450.0);
    let p = ComposeParams::from_weather(&w);
    assert!(p.drift.x < 0.0 && p.drift.y > 0.0, "{:?}", p.drift);
}

/// The detail noise rides the deck's drift and the clock's rise, and both
/// wrap: the drift at the field's period, `Time::elapsed_secs_wrapped` at
/// Bevy's hour. A tile that did not divide both would jump at each wrap.
#[test]
fn the_cloud_detail_wraps_where_the_deck_does() {
    use client::render::deck::{detail_noise, BOIL_MS, DETAIL_TILE_M};
    use client::render::sky::{CLOUD_SCALE_M, FIELD_PERIOD};
    let field_m = FIELD_PERIOD as f32 * CLOUD_SCALE_M;
    assert_eq!((field_m / DETAIL_TILE_M).fract(), 0.0);
    assert_eq!((3600.0 * BOIL_MS / DETAIL_TILE_M).fract(), 0.0);
    // And the volume itself tiles: across its seam it steps no more than
    // between any two neighbours inside it.
    let n = 16;
    let v = detail_noise(SEED, n);
    let at = |x: usize, y: usize, z: usize| v[(z * n + y) * n + x];
    let (mut seam, mut inside) = (0.0f32, 0.0f32);
    for z in 0..n {
        for y in 0..n {
            seam = seam.max((at(n - 1, y, z) - at(0, y, z)).abs());
            for x in 1..n {
                inside = inside.max((at(x, y, z) - at(x - 1, y, z)).abs());
            }
        }
    }
    assert!(seam <= inside + 1e-6, "seam {seam} against {inside} inside");
    assert!(v.iter().all(|x| (0.0..=1.0).contains(x)));
}
