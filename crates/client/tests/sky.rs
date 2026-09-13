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
use client::render::rig::{island_medium, AIR_DENSITY};
use client::render::sky::{
    air_chroma, backdrop_at, browser_haze, cloud_cubemap_with, cube_dir, ground_air, haze_color,
    AIR_FLOOR, BAKE_BACKDROP, CLOUD_NITS, HORIZON_DESAT, HORIZON_GAIN, SKY_FACE,
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

/// The default bake is the target's: zero texels here, a sky in a browser.
#[test]
fn the_default_bake_is_the_targets() {
    assert_eq!(BAKE_BACKDROP, cfg!(target_arch = "wasm32"));
    // And `cloud_cubemap` goes through the switch rather than one arm — read
    // off the source, because a `cfg!` cannot be flipped from a test.
    let src =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/render/sky.rs")).unwrap();
    assert!(
        src.contains("cloud_cubemap_with(seed, BAKE_BACKDROP)"),
        "`sky::cloud_cubemap` no longer bakes through `BAKE_BACKDROP`"
    );
}

/// The browser's deck is the desktop's with a sky under it: identical bytes
/// wherever a cloud is opaque, exactly the backdrop wherever the desktop's
/// texel is zero, and never black anywhere.
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
                    assert_eq!(&b[..3], &want[..], "clear texel is not the backdrop");
                    backdrop += 1;
                } else if a[..3] == b[..3] {
                    // An opaque cloud: the same bytes on both targets.
                    same += 1;
                } else {
                    // A cloud's edge. The desktop stores `cloud · cov` over
                    // black; the browser stores `cloud · cov + sky · (1 − cov)`,
                    // so in LINEAR terms the sky only ever adds, by at most
                    // the sky itself: `a ≤ b ≤ a + sky`. Compared decoded,
                    // because the bytes are sRGB and a sum is not.
                    let lin = |v: u8| srgb_to_linear(v as f32 / 255.0);
                    let sky = backdrop_at(d);
                    for c in 0..3 {
                        let (al, bl) = (lin(a[c]), lin(b[c]));
                        assert!(
                            bl >= al - 0.02 && bl <= al + sky[c] + 0.02,
                            "edge texel {b:?} is not cloud {a:?} plus at most the sky {want:?} (channel {c}: {al} .. {})",
                            al + sky[c]
                        );
                    }
                    blended += 1;
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

/// The air at the ground is Bevy's published earthlike medium at
/// `AIR_DENSITY` — rebuilt here from the numbers in
/// `bevy_pbr::medium::ScatteringMedium::earthlike`, never from the function
/// under test. Counting the ozone tent, or leaving absorption out of
/// extinction, each moves a channel.
#[test]
fn ground_air_is_the_published_medium_at_the_ground() {
    let (ext, scat) = ground_air(&island_medium());
    let rayleigh = Vec3::new(5.802e-6, 13.558e-6, 33.100e-6);
    let (mie_abs, mie_scat) = (Vec3::splat(3.996e-6), Vec3::splat(0.444e-6));
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

/// The browser's haze is the desktop's air, coloured by the sky's own horizon,
/// dimmed with the day, and carries no sun glow nobody owns.
#[test]
fn the_browser_haze_is_the_desktops_air_and_the_skys_horizon() {
    let fog = browser_haze(1.0);
    let (ext, scat) = ground_air(&island_medium());
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
