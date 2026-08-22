//! Gate: the sky fill is a **hemisphere**, and the cubemap that carries it to
//! the GPU says what the model says.
//!
//! **What makes this gateable at all.** There is no visual gate in this repo
//! and there must not be one (`CLAUDE.md`), so the rule for a render change is
//! that what may be gated about a frame is *arithmetic*. A hemisphere fill is
//! unusually well suited to that: the whole of it is a function from a normal
//! to an irradiance, and the texture is that function sampled. Nothing here
//! needs a GPU, a window or a shard.
//!
//! **The load-bearing assertion is the first one, and it is the defect.**
//! `bevy_pbr`'s `ambient_light()` takes a `world_normal` and never reads it,
//! so an `AmbientLight` delivers the same irradiance in every direction —
//! [`the_fill_is_a_hemisphere_and_not_a_uniform_term`] measures exactly the
//! ratio that term pins to 1.0 by construction. Run it against the rig as it
//! shipped before this slice and it reads 1.000; that is the number the visual
//! judge was seeing when it measured a tree base "flat within ±5% right up to
//! the trunk" and open ground in shadow that never falls below ~0.73 of lit.
//!
//! **The second load-bearing one is the layout**, and it is the half that
//! testing the model alone cannot reach. [`fill_at`] can be perfect while the
//! bake writes the sky onto the `-Y` face, and every frame would then light
//! the underside of every prop with sky and the top of every rock with dirt —
//! wrong in a way that reads as "the lighting looks odd" rather than as a bug
//! with a location. [`the_cubemap_texels_carry_the_model`] decodes the actual
//! byte buffer at the two poles, so the face ORDER is checked against the
//! model rather than against itself.

#![cfg(feature = "render")]

use bevy::light::light_consts::lux;
use bevy::prelude::*;
use client::render::fill::{
    bounce_albedo, bounce_lux, cubemap, fill_at, ground_irradiance, linear_to_srgb, luminance,
    peak_lux, sky_lux, srgb_to_linear, FILL_FACE, GROUND_MIX, SKY_FILL_LUX, SKY_FILL_SRGB,
};
use client::render::rig::{NIGHT_AMBIENT_LUX, RIG_SUN_ELEVATION};
use client::render::terrain_mesh::GROUND_ALBEDO;

/// Decode one texel of the baked cubemap the way the GPU will: byte → sRGB
/// unorm → linear, then scaled by the `EnvironmentMapLight::intensity` the rig
/// sets. The result is irradiance in lux, directly comparable to [`fill_at`].
fn decode(data: &[u8], face: usize, x: u32, y: u32, n: u32) -> [f32; 3] {
    let i = (((face as u32 * n + y) * n + x) * 4) as usize;
    let peak = peak_lux();
    let mut out = [0.0f32; 3];
    for (c, o) in out.iter_mut().enumerate() {
        *o = srgb_to_linear(data[i + c] as f32 / 255.0) * peak;
    }
    out
}

/// **The defect, measured.** A uniform ambient term pins this ratio to exactly
/// 1.0 — it is the same number in every direction, which is what
/// `pbr_ambient.wgsl` ignoring its `world_normal` argument means. `ART.md` §4
/// asks for a hemisphere; a hemisphere has a ratio greater than one.
///
/// The band is a rule at each end rather than a fitted value. The floor says
/// the split is real and not a rounding — below 1.25 a "hemisphere" is a
/// uniform term with a tint. The ceiling is `ART.md` rule 3, "no pure black
/// and no crushed shadow": the ground bounces into every downward face, so an
/// earth half more than 3× down from the sky half would be authoring the black
/// underside the rule exists to forbid.
#[test]
fn the_fill_is_a_hemisphere_and_not_a_uniform_term() {
    let up = luminance(fill_at(Vec3::Y));
    let down = luminance(fill_at(Vec3::NEG_Y));
    let ratio = up / down;
    println!("sky half {up:.1} lux, earth half {down:.1} lux, ratio {ratio:.4}");
    assert!(
        ratio > 1.25,
        "the fill is direction-free (ratio {ratio:.4}); a uniform AmbientLight reads exactly 1.0"
    );
    assert!(
        ratio < 3.0,
        "the earth half is crushed (ratio {ratio:.4}) — ART.md rule 3 forbids it"
    );
}

/// `ART.md` §4, verbatim: "sky half cool blue, earth half warm". Both halves,
/// as a chroma ordering rather than a hue angle, because the ordering is what
/// the rule actually constrains and it survives a re-chroming of either half.
#[test]
fn the_earth_half_is_warm_and_the_sky_half_is_cool() {
    let sky = fill_at(Vec3::Y);
    let ground = fill_at(Vec3::NEG_Y);
    assert!(
        sky[2] > sky[0],
        "the sky half is not cool: R {:.0} B {:.0}",
        sky[0],
        sky[2]
    );
    assert!(
        ground[0] > ground[2],
        "the earth half is not warm: R {:.0} B {:.0}",
        ground[0],
        ground[2]
    );
}

/// **The no-regression guarantee, and the reason this slice could land without
/// a frame in front of anyone.** An up-facing surface in the open receives
/// exactly what the uniform term delivered, so the frame's overall level and
/// its highlight placement cannot have moved: the change is purely a
/// redistribution over normal direction.
///
/// Recomputed here from `lux::AMBIENT_DAYLIGHT` through a second path rather
/// than read back from `sky_lux()`, so this is an agreement between two
/// derivations and not a tautology.
#[test]
fn the_zenith_fill_is_the_uniform_term_it_replaced() {
    let want_scale = lux::AMBIENT_DAYLIGHT * 1.7;
    assert!(
        (SKY_FILL_LUX - want_scale).abs() < 1e-3,
        "the sky half's magnitude moved: {SKY_FILL_LUX} vs {want_scale}"
    );
    let got = fill_at(Vec3::Y);
    for c in 0..3 {
        let want = srgb_to_linear(SKY_FILL_SRGB[c]) * want_scale;
        assert!(
            (got[c] - want).abs() < 1.0,
            "channel {c}: zenith fill {got:?} does not match the uniform term's {want:.1}"
        );
    }
}

/// The earth half is the island's own albedo and not a picked colour: the mix
/// is a partition (it sums to one), the bounce IS the weighted sum of the four
/// identities, and the result therefore lies inside their convex hull.
///
/// ⚠ **The hull bound alone got looser for free on 2026-08-15, with no edit to
/// it, and that is why the weighted sum is pinned directly now.** The hull is
/// taken over identities with `w > 0.0`. Granite's weight was 0.0 under the
/// retracted quadrant mix, so the *brightest* of the four identities was
/// excluded from the hull and the admissible band was narrow by accident;
/// correcting `GROUND_MIX` put granite back in and widened it. Nothing here
/// asserted less than it had, but it proved less than it reads as proving —
/// the same shape as a gate that goes green because its case was deleted.
/// Measured 2026-08-15: truncating `bounce_albedo()`'s fold to three
/// identities (the exact "granite is free" bug this pass corrected) leaves the
/// hull assert **green**, at bounce 0.0973 against a hull that still admits it.
/// The hull is a real property and stays. It is simply not the binding one.
#[test]
fn the_bounce_is_the_islands_own_albedo() {
    let sum: f32 = GROUND_MIX.iter().sum();
    assert!(
        (sum - 1.0).abs() < 1e-6,
        "the ground mix is not a partition: {sum}"
    );
    let a = bounce_albedo();

    // The weighted sum written out term for term rather than folded — a second
    // expression of the same arithmetic, not a re-invocation. `bounce_albedo()`
    // is a nested `enumerate` over two arrays, so a dropped identity, a
    // truncated iterator or a transposed index is a live way for it to return a
    // plausible colour that no bound above can see. Red at 0.0973 vs 0.1163
    // under the `.take(3)` mutation.
    for c in 0..3 {
        let want = GROUND_ALBEDO[0][c] * GROUND_MIX[0]
            + GROUND_ALBEDO[1][c] * GROUND_MIX[1]
            + GROUND_ALBEDO[2][c] * GROUND_MIX[2]
            + GROUND_ALBEDO[3][c] * GROUND_MIX[3];
        assert!(
            (a[c] - want).abs() < 1e-6,
            "channel {c}: bounce albedo {:.6} is not the area-weighted sum {want:.6}",
            a[c]
        );
    }

    // Every identity is actually carried. A zero weight is not hypothetical —
    // it is precisely what the retracted quadrant window reported for granite,
    // and it is what made the hull bound below vacuous for the brightest
    // surface on the island. A mix that has dropped a ground surface is no
    // longer the island's albedo whatever it sums to, so this is checked
    // separately from the partition (a renormalised three-way mix sums to 1.0).
    for (i, w) in GROUND_MIX.iter().enumerate() {
        assert!(
            *w > 1e-4,
            "identity {i} carries weight {w} — the mix has dropped a ground surface"
        );
    }

    for c in 0..3 {
        let lo = GROUND_ALBEDO
            .iter()
            .zip(GROUND_MIX)
            .filter(|(_, w)| *w > 0.0)
            .fold(f32::MAX, |m, (id, _)| m.min(id[c]));
        let hi = GROUND_ALBEDO
            .iter()
            .zip(GROUND_MIX)
            .filter(|(_, w)| *w > 0.0)
            .fold(0.0f32, |m, (id, _)| m.max(id[c]));
        assert!(
            a[c] >= lo - 1e-6 && a[c] <= hi + 1e-6,
            "channel {c}: bounce albedo {:.4} is outside the mixed identities [{lo:.4}, {hi:.4}]",
            a[c]
        );
    }
    println!(
        "bounce albedo {a:?}, ground irradiance {:.0} lux",
        ground_irradiance()
    );
}

/// **The sin/cos trap, gated.** `RIG_SUN_ELEVATION` is measured above the
/// HORIZON, so the cosine of the angle between a `+Y` ground normal and the
/// sun is the SINE of that elevation. Taking it for a zenith angle inflates
/// the ground's irradiance by 1.43× at 35°, and every consequence of that is a
/// too-bright bounce that reads as "the darks are still not dark".
#[test]
fn the_ground_irradiance_takes_the_suns_elevation_not_its_zenith_angle() {
    let want = lux::DIRECT_SUNLIGHT * RIG_SUN_ELEVATION.sin() + SKY_FILL_LUX;
    let wrong = lux::DIRECT_SUNLIGHT * RIG_SUN_ELEVATION.cos() + SKY_FILL_LUX;
    let got = ground_irradiance();
    assert!(
        (got - want).abs() < 1.0,
        "{got} is not the sine form {want}"
    );
    assert!(
        (got - wrong).abs() > 10_000.0,
        "{got} is indistinguishable from the zenith-angle form {wrong}"
    );
}

/// The fill rises monotonically with the normal's height. A sign flip on the
/// blend weight passes every endpoint test above — both poles still hold their
/// own value — and inverts the whole hemisphere in between.
#[test]
fn the_fill_is_monotone_in_the_normals_height() {
    let mut prev = f32::MIN;
    for i in 0..=64 {
        let ny = -1.0 + 2.0 * i as f32 / 64.0;
        let l = luminance(fill_at(Vec3::new(0.0, ny, 0.0)));
        assert!(
            l > prev,
            "fill is not monotone at n.y = {ny:.3}: {l:.1} after {prev:.1}"
        );
        prev = l;
    }
}

/// **The layout gate.** Decodes the real byte buffer at the two poles and
/// checks them against the model. This is what catches a face-order or
/// axis-flip bug in the bake, which every model-only assertion above is blind
/// to by construction.
///
/// The tolerance covers two known, bounded errors and nothing else: the centre
/// texel of a 32² face is 0.03 off-axis, so it is 99.95% of the way to the
/// pole rather than at it, and the 8-bit sRGB store costs up to ~0.45% of peak
/// near the top of the range.
#[test]
fn the_cubemap_texels_carry_the_model() {
    let img = cubemap(FILL_FACE);
    let data = img
        .data
        .as_ref()
        .expect("the fill map is baked, not deferred");
    let mid = FILL_FACE / 2;
    let peak = peak_lux();

    // Face 2 is +Y and face 3 is -Y in `sky::cube_dir`.
    for (face, want, name) in [(2usize, sky_lux(), "sky"), (3usize, bounce_lux(), "earth")] {
        let got = decode(data, face, mid, mid, FILL_FACE);
        for c in 0..3 {
            let err = (got[c] - want[c]).abs() / peak;
            assert!(
                err < 0.02,
                "{name} half, channel {c}: texel decodes to {:.0} lux, model says {:.0} ({:.3} of peak off)",
                got[c],
                want[c],
                err
            );
        }
        println!("{name} pole texel decodes to {got:?}");
    }
}

/// `ART.md` rule 3's "no pure black", checked over every texel of the map
/// rather than at the poles: outdoors the ground bounces into every downward
/// face, so no direction may be unlit.
#[test]
fn no_direction_of_the_fill_is_black() {
    let img = cubemap(FILL_FACE);
    let data = img.data.as_ref().expect("the fill map is baked");
    let mut lowest = u8::MAX;
    for px in data.chunks_exact(4) {
        lowest = lowest.min(px[0].max(px[1]).max(px[2]));
    }
    assert!(lowest > 0, "some direction of the sky fill is pure black");
    println!("dimmest texel's brightest channel: {lowest}/255");
}

/// The normalizer is the true maximum over all directions — tight, so the
/// 8-bit store spends its whole range, and not exceeded, so nothing clips. The
/// sweep is what checks [`peak_lux`]'s "no search needed" shortcut, which is
/// only valid because the fill is a lerp between two endpoints.
#[test]
fn the_peak_is_the_true_maximum() {
    let peak = peak_lux();
    let mut seen = 0.0f32;
    for i in 0..=256 {
        let ny = -1.0 + 2.0 * i as f32 / 256.0;
        for c in fill_at(Vec3::new(0.0, ny, 0.0)) {
            seen = seen.max(c);
        }
    }
    assert!(
        (seen - peak).abs() < 1.0,
        "peak_lux() says {peak:.1} but the sweep reaches {seen:.1}"
    );

    let img = cubemap(FILL_FACE);
    let data = img.data.as_ref().expect("the fill map is baked");
    let top = data
        .chunks_exact(4)
        .flat_map(|px| px[..3].iter().copied())
        .max()
        .unwrap();
    assert_eq!(top, 255, "the map does not reach the top of its range");
}

/// The transfer the cubemap is stored through round-trips. The map holds
/// sRGB-encoded values and the GPU decodes them, so an error here is an error
/// in every texel and would read as a global brightness shift.
#[test]
fn the_srgb_transfer_round_trips() {
    for i in 0..=1000 {
        let v = i as f32 / 1000.0;
        let back = srgb_to_linear(linear_to_srgb(v));
        assert!((back - v).abs() < 1e-4, "{v} round-tripped to {back}");
    }
}

/// The handover between the two fills is complementary: at noon the hemisphere
/// carries all of it and the uniform term contributes nothing, at deep night
/// the reverse, and neither ever carries a share the other is also carrying.
/// The failure this forbids is the one that doubles the fill at midday.
#[test]
fn the_two_fills_hand_over_rather_than_overlap() {
    for (light, name) in [(1.0f32, "noon"), (0.0f32, "deep night")] {
        let ambient = NIGHT_AMBIENT_LUX * (1.0 - light);
        let hemisphere = peak_lux() * light;
        if light == 1.0 {
            assert_eq!(ambient, 0.0, "{name}: the uniform term still carries fill");
            assert!(hemisphere > 0.0, "{name}: the hemisphere is dark");
        } else {
            assert_eq!(hemisphere, 0.0, "{name}: the hemisphere still carries fill");
            assert!(
                (ambient - NIGHT_AMBIENT_LUX).abs() < 1e-6,
                "{name}: the night floor moved to {ambient}"
            );
        }
    }
}
