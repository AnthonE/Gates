//! The sky fill — a **hemisphere**, which is what `ART.md` §4 asks for and
//! what Bevy's `AmbientLight` provably cannot be.
//!
//! **The defect this closes, and it is in `bevy_pbr`'s source rather than in
//! ours.** `pbr_ambient.wgsl`'s `ambient_light()` takes `world_normal` as its
//! second parameter and **never reads it** — the body is `(diffuse_ambient +
//! specular_ambient * specular_occlusion) * ambient_color * occlusion`, with
//! no `N` anywhere. So an `AmbientLight` delivers the same
//! irradiance to the underside of a boulder as to open ground facing the sky,
//! and no amount of tuning its brightness changes that: the term is
//! direction-free by construction. `rig.rs` has said so in a comment since the
//! Bevy port ("a uniform term rather than the browser's hemisphere, so it
//! cannot be split sky-half/ground-half — the down-facing-prop-face half of
//! that split is owed to a later slice"). This is that slice.
//!
//! **What `ART.md` §4 actually asks for**, verbatim: "**Sky fill**:
//! hemisphere, sky half cool blue, earth half warm — this is what keeps rule
//! 3's 0.30 floor reachable and what makes shadowed grass go cool." Rule 3
//! adds the reason a hemisphere is the *physical* answer and not a look: "Pure
//! black is unreachable outdoors: the sky fills every upward face and the
//! ground bounces into every downward one." Two different fills, named in the
//! rule, delivered by one term that cannot tell them apart.
//!
//! **How it reaches the shader without a line of WGSL.** `environment_map.wgsl`
//! samples the diffuse cubemap **by the world normal** — `var
//! irradiance_sample_dir = N;` then `textureSampleLevel(diffuse_environment_map,
//! …, irradiance_sample_dir, 0.0).rgb * intensity`. A cubemap whose texel in
//! direction `n` holds the irradiance arriving at a surface whose normal is `n`
//! IS a hemisphere light, exactly, with no custom material and no compute pass.
//! That matters twice over: `StandardMaterial` is `#[bindless]` in 0.18 so a
//! custom ground material is a slice rather than a knob (`terrain_mesh.rs`
//! says so), and the gate box renders on a CPU rasterizer where a compute
//! pipeline is a risk this does not need to take.
//!
//! **And it lands on the right side of `ART.md` §4's occlusion law.**
//! `pbr_functions.wgsl` accumulates `environment_light.diffuse *
//! diffuse_occlusion`, and `diffuse_occlusion` is where the SSAO the rig
//! already carries is folded in via `min()`. So the fill is indirect-only and
//! occlusion-weighted — §4's "`indirectDiffuse *= ao`, indirect only" and its
//! "never sum or multiply two occlusion terms of the same scale" both hold
//! without us writing either.
//!
//! **Nothing here is a chosen number.** The sky half is the magnitude and
//! chroma the uniform term already shipped, carried across unchanged so open
//! ground does not move; the earth half is the island's own measured albedo
//! under the island's own measured irradiance. The one thing this file
//! *decides* is that the two are blended by `n·Y`, which is what §4 says.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::light::light_consts::lux;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};

use super::rig::RIG_SUN_ELEVATION;
use super::terrain_mesh::GROUND_ALBEDO;

/// Cube face size in texels for the fill map. 6 × 32² = 6,144 texels, 24 KB.
///
/// Small on purpose and not a corner cut: the function being sampled is a
/// lerp in `n·Y`, so it is exactly representable at any resolution that
/// resolves the vertical gradient, and 32 is the conventional size for a
/// diffuse irradiance cubemap for precisely that reason (the quantity is
/// already the cosine-weighted integral over a hemisphere — it has no high
/// frequencies left in it). The cloud deck next door needs 256 because it
/// carries an fBm; this carries a ramp.
pub const FILL_FACE: u32 = 32;

/// The sky half's colour, sRGB — **carried across from the uniform term
/// unchanged**, deliberately.
///
/// Its own derivation, which stays true and is why it is not re-picked here:
/// "Cool, not blue. The first capture's boulder had a NAVY shaded face —
/// 0.62/0.72/0.92 at a third of daylight is a saturated blue doing all the
/// work on every unlit surface, and the frame measured 42% near-band
/// saturation against the reference's 33%."
///
/// **That defect was the uniform term's, and this slice removes its cause
/// rather than its symptom** — the navy was a *down*-facing face being fed a
/// blue sky. Down-facing faces now get [`bounce_lux`] instead, so the
/// constraint that forced this colour warm is gone and `ART.md` §3's
/// "shadowed grass goes COOL (hue 70° → 172°)" becomes reachable by cooling
/// it. Deliberately NOT done in the same pass: the hemisphere is the
/// structural change, re-chroming the sky half is a look, and this rig's own
/// header records what happens when coupled terms move together blind.
/// `NOW.md` carries it.
pub const SKY_FILL_SRGB: [f32; 3] = [0.80, 0.85, 0.95];

/// Irradiance reaching a zenith-facing surface from the sky at noon, lux —
/// **also carried across unchanged**, and that is the whole safety argument
/// for landing this without eyes on a frame.
///
/// An up-facing surface in the open receives exactly what it received before,
/// so the frame's overall level, its exposure and its highlight placement
/// cannot move. What moves is only how the fill is *distributed* over normal
/// direction, which is a change no global statistic can be made worse by: the
/// darks come back on down-facing faces and nowhere else.
pub const SKY_FILL_LUX: f32 = lux::AMBIENT_DAYLIGHT * 1.7;

/// The island's ground mix — **measured over the whole world square, which is
/// the half this constant used to get wrong**.
///
/// This is what makes the earth half a derivation rather than a knob. §4 asks
/// for "earth half warm"; it does not say how warm or how bright, and inventing
/// those two numbers would be exactly the invention `CLAUDE.md` forbids. The
/// island's own albedo under the island's own irradiance answers both, and it
/// answers them warm on its own — litter is 38% of the land and hue 38°.
///
/// ⚠ **Corrected 2026-08-15. It said `[0.008, 0.619, 0.373, 0.0]` and "granite
/// never reaches the ground at this seed at all", and both came from the
/// quadrant.** The old value cited "39,521 land samples at seed 20260731", and
/// that number is exactly reproducible: it is what `-1024..1024` on both axes
/// returns at a 4 m step. `terrain::continent` centres the island on
/// `(ISLAND_SIZE/2, ISLAND_SIZE/2)`, so world coordinates run `0..2048` and
/// that window's *corner* is the island's centre — `0..1024` alone returns the
/// identical 39,521, because the negative half is open sea. It is the same
/// window `CLAUDE.md`'s trap list retracted for the relief sweep on
/// 2026-08-14; the client's two copies were not re-measured then, and this is
/// that.
///
/// Whole-island, 157,198 land samples at the same step: sand 0.0113, grass
/// 0.5182, litter 0.3789, **rock 0.0916** — granite is the third identity by
/// area, eight times sand's share, and `sim-core/tests/relief.rs` (10.0% of
/// land carries a legible rock weight) and `examples/seed_scan.rs` (8.9%
/// within 300 m of the capture spawn) have both said so since the retraction.
/// Three sources that already knew where the world is, against one that did
/// not. Gated by `crates/client/tests/ground_mix.rs`, which reproduces the
/// retracted window's own numbers so the correction cannot quietly revert.
///
/// The values are the measurement normalised to a partition: the raw means sum
/// to 0.999999, because a splat row is four bytes that sum to 254 or 255 rather
/// than exactly 255, and `tests/fill.rs` requires a partition. The 1e-6 goes
/// into grass, the largest channel.
///
/// **What this costs, stated because it is not free.** `GROUND_ALBEDO`'s
/// granite was authored as a free parameter *because* this said its weight was
/// zero. It is not free now, and the area-weighted mean linear luma that whole
/// re-placement was pinned to (0.09390) is 14.4% low against the island's own
/// 0.1075. Re-placing four coupled albedos is the brightness owner's iteration
/// and not this one's — `NOW.md` carries it.
pub const GROUND_MIX: [f32; 4] = [0.011_284, 0.518_242, 0.378_859, 0.091_615];

/// sRGB → linear, the exact piecewise transfer (not the 2.2 approximation).
pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// linear → sRGB, the exact piecewise transfer. The inverse of
/// [`srgb_to_linear`], and gated as a round trip because the cubemap stores
/// the fill sRGB-encoded and the GPU decodes it back.
pub fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Rec.709 luminance of a linear RGB triple. The weights the frame is
/// ultimately measured with.
pub fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// The sky half: irradiance at a zenith-facing surface, linear lux per channel.
pub fn sky_lux() -> [f32; 3] {
    [
        srgb_to_linear(SKY_FILL_SRGB[0]) * SKY_FILL_LUX,
        srgb_to_linear(SKY_FILL_SRGB[1]) * SKY_FILL_LUX,
        srgb_to_linear(SKY_FILL_SRGB[2]) * SKY_FILL_LUX,
    ]
}

/// The island's area-weighted ground albedo — [`GROUND_MIX`] against
/// [`GROUND_ALBEDO`], which is where "earth half warm" comes from.
pub fn bounce_albedo() -> [f32; 3] {
    let mut a = [0.0f32; 3];
    for (i, w) in GROUND_MIX.iter().enumerate() {
        for (c, a_c) in a.iter_mut().enumerate() {
            *a_c += GROUND_ALBEDO[i][c] * w;
        }
    }
    a
}

/// What the ground itself receives at the rig's noon register, lux: the sun
/// projected onto flat ground, plus the sky.
///
/// `sin(RIG_SUN_ELEVATION)` and not `cos`: `RIG_SUN_ELEVATION` is measured
/// **above the horizon**, so the cosine of the angle between a `+Y` ground
/// normal and the sun is the sine of the sun's elevation. Getting this
/// backwards inflates the bounce by 1.75× at 35°.
pub fn ground_irradiance() -> f32 {
    lux::DIRECT_SUNLIGHT * RIG_SUN_ELEVATION.sin() + SKY_FILL_LUX
}

/// The earth half: irradiance at a nadir-facing surface, linear lux per
/// channel — the ground's own radiosity, i.e. what it reflects of what it
/// receives.
///
/// The Lambertian result for a surface facing a large uniform ground of
/// radiosity `B` is `B` itself, so there is no extra geometric factor to
/// apply here and adding one would be a second fudge.
pub fn bounce_lux() -> [f32; 3] {
    let e = ground_irradiance();
    let a = bounce_albedo();
    [a[0] * e, a[1] * e, a[2] * e]
}

/// **The hemisphere.** Irradiance arriving at a surface whose world normal is
/// `n`, linear lux per channel — sky at the zenith, ground bounce at the
/// nadir, blended by height exactly as `ART.md` §4 words it.
///
/// The `0.5 + 0.5·n·Y` weight is the standard hemisphere form and it is the
/// one thing in this file that is a modelling choice rather than a measured
/// number. A cosine-weighted integral over a split hemisphere gives this same
/// linear-in-`n·Y` result for two uniform halves, so it is the analytic answer
/// and not an eyeballed ramp.
pub fn fill_at(n: Vec3) -> [f32; 3] {
    let w = 0.5 + 0.5 * n.y.clamp(-1.0, 1.0);
    let sky = sky_lux();
    let ground = bounce_lux();
    [
        ground[0] + (sky[0] - ground[0]) * w,
        ground[1] + (sky[1] - ground[1]) * w,
        ground[2] + (sky[2] - ground[2]) * w,
    ]
}

/// The brightest channel the fill reaches in any direction.
///
/// The map stores `fill_at(n) / peak` in `0..=1` and `EnvironmentMapLight::
/// intensity` carries the scale back, which is what lets an 8-bit sRGB texture
/// hold a quantity measured in tens of thousands of lux. Because
/// [`fill_at`] is a lerp between two endpoints, the extremum over all
/// directions is an extremum of one of the two ends — no search needed.
pub fn peak_lux() -> f32 {
    let sky = sky_lux();
    let ground = bounce_lux();
    sky.iter()
        .chain(ground.iter())
        .fold(0.0f32, |m, v| m.max(*v))
}

/// Bake the hemisphere into a cubemap.
///
/// Written through [`super::sky::cube_dir`] rather than a local face table:
/// the face convention is one owner's, and a disagreement between the two
/// bakers would put the sky on the underside of every prop with both files
/// reading correct alone.
pub fn cubemap(n: u32) -> Image {
    let peak = peak_lux();
    let mut data = vec![0u8; (n * n * 6 * 4) as usize];

    for face in 0..6usize {
        for y in 0..n {
            for x in 0..n {
                let d = super::sky::cube_dir(face, x, y, n);
                let f = fill_at(d);
                let i = (((face as u32 * n + y) * n + x) * 4) as usize;
                for c in 0..3 {
                    let v = (f[c] / peak).clamp(0.0, 1.0);
                    data[i + c] = (linear_to_srgb(v) * 255.0).round() as u8;
                }
                data[i + 3] = 255;
            }
        }
    }

    let mut image = Image::new(
        Extent3d {
            width: n,
            height: n,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image
}
