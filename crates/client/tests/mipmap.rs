//! The mip chain's gate. **Renderer tier**: it links Bevy for `Image`, but it
//! opens no window, needs no GPU and reads no pixel of a frame.
//!
//! `CLAUDE.md` is explicit that there is no visual gate here and that writing
//! one is forbidden. What may be gated about a frame is arithmetic, and a mip
//! chain is almost entirely arithmetic: every assertion below is a property
//! that, violated, produces a specific named artefact — the island crawling
//! with static at range, distant ground drifting dark and muddy, a normal map
//! shortening toward the origin and taking the lighting with it, or wgpu
//! reading past the end of a buffer at first draw.
//!
//! **Nothing here calls the thing it is checking to build its expectation.**
//! That is `CLAUDE.md`'s `lattice.rs` trap in full: a naive rebuild that calls
//! the function under test carries the same mutant on both sides and the gate
//! is green for the wrong reason. The byte totals below are closed-form sums
//! written out here; the sRGB constants are the IEC 61966-2-1 transfer
//! function evaluated by hand (188, and 146) and stated with their derivation
//! so the next person can check them without running anything.
//!
//! The person who decides whether it looks good boots the game and looks.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use client::render::mipmap::{chain, chain_bytes, levels, wants, Filter};

// ---------------------------------------------------------------------------
// Shape: the chain is complete, and it says how long it is.
// ---------------------------------------------------------------------------

/// A chain must run to a 1×1 level. A short chain is not a soft failure —
/// wgpu is told `mip_level_count` and reads that many levels out of the
/// buffer, so a count that disagrees with the data is a read past the end at
/// first draw, far from the cause.
#[test]
fn the_chain_runs_to_one_by_one() {
    // Written out rather than derived, so a change to `levels` has to face a
    // number somebody chose.
    for (w, h, want) in [
        (1024u32, 1024u32, 11u32),
        (512, 512, 10),
        (256, 256, 9),
        (4, 4, 3),
        (2, 2, 2),
    ] {
        assert_eq!(levels(w, h), want, "{w}x{h} should hold {want} levels");
    }
}

/// The byte total, as an independent closed-form sum. `chain_bytes` is what
/// the module uses to size its buffer and what a reviewer checks a descriptor
/// against; if the two disagree the texture is malformed in a way no value
/// assertion can see.
#[test]
fn the_bytes_are_the_sum_of_the_levels() {
    // Σ over levels of (w/2^i)(h/2^i)·4, spelled out for the two sizes that
    // actually ship: 1024² albedo/normal and 512² rough/ao.
    assert_eq!(chain_bytes(1024, 1024), 5_592_404);
    assert_eq!(chain_bytes(512, 512), 1_398_100);
    assert_eq!(chain_bytes(4, 4), (16 + 4 + 1) * 4);
    assert_eq!(chain_bytes(2, 2), (4 + 1) * 4);
}

/// What [`chain`] returns has to be exactly what [`chain_bytes`] promised, for
/// every filter — the buffer and the count are handed to wgpu together.
#[test]
fn what_it_builds_is_what_it_promised() {
    for filter in [Filter::Srgb, Filter::Normal, Filter::Linear] {
        for n in [2u32, 4, 8, 64] {
            let level0 = ramp(n, n);
            let out = chain(&level0, n, n, filter);
            assert_eq!(
                out.len(),
                chain_bytes(n, n),
                "{filter:?} at {n}x{n} built {} bytes, promised {}",
                out.len(),
                chain_bytes(n, n)
            );
        }
    }
}

/// **Level 0 is copied, never filtered.** The chain exists for what the
/// camera sees at range; disturbing the level it samples up close would trade
/// the static for a blurrier game, which is not the deal.
#[test]
fn level_zero_is_untouched() {
    for filter in [Filter::Srgb, Filter::Normal, Filter::Linear] {
        let level0 = ramp(8, 8);
        let out = chain(&level0, 8, 8, filter);
        assert_eq!(&out[..level0.len()], &level0[..], "{filter:?} moved level 0");
    }
}

// ---------------------------------------------------------------------------
// sRGB: the half a naive fix gets wrong.
// ---------------------------------------------------------------------------

/// **Black and white in equal measure is 188, not 128.**
///
/// An `Rgba8UnormSrgb` texel is *encoded*, so averaging the bytes averages the
/// wrong quantity. Half black and half white is linear 0.5, and
/// `1.055 · 0.5^(1/2.4) − 0.055 = 0.7354`, which is byte **188**. A byte-wise
/// average gives 128 — 0.23 of the range too dark — and it compounds on every
/// level, so distant ground would sink into mud as it recedes. That is a
/// different visible defect from the static, and it is the one a naive chain
/// ships.
///
/// This is the assertion that fails if somebody replaces the sRGB path with a
/// plain average.
///
/// **188 is corroborated in the tree**, which is worth saying because a
/// constant nobody can check twice is a constant nobody checks:
/// `tree::NEEDLE_MASK_BYTE`'s doc reaches the same number from the other
/// side — "Alpha is linear even in an sRGB-encoded texture … so 0.5 is 128
/// and not 188" — and `tests/tree.rs` has gated it since the canopy landed.
#[test]
fn srgb_averages_in_linear_and_not_in_bytes() {
    // A 2×2 checkerboard: two black texels, two white.
    let mut level0 = Vec::new();
    for v in [0u8, 255, 255, 0] {
        level0.extend_from_slice(&[v, v, v, 255]);
    }
    let out = chain(&level0, 2, 2, Filter::Srgb);
    let one = &out[16..20];
    assert_eq!(
        one[0], 188,
        "the 1x1 level is {}, and a linear-space average is 188 (a byte-space one is 128)",
        one[0]
    );
    assert_eq!(one[3], 255, "alpha is not coverage-preserving");
}

/// A second sRGB case that is not at the rails, because 0 and 255 are where
/// the transfer function is least interesting. 64 and 192 in equal measure is
/// linear 0.2892, which encodes to **146**; a byte average is 128.
#[test]
fn srgb_holds_away_from_the_rails() {
    let mut level0 = Vec::new();
    for v in [64u8, 192, 192, 64] {
        level0.extend_from_slice(&[v, v, v, 255]);
    }
    let out = chain(&level0, 2, 2, Filter::Srgb);
    assert_eq!(out[16], 146, "expected 146, the linear mean 0.2892 re-encoded");
}

// ---------------------------------------------------------------------------
// Invariants that hold for every filter.
// ---------------------------------------------------------------------------

/// **A flat colour must survive every level.** This is the round-trip check:
/// an sRGB decode followed by an encode has to return the byte it started
/// from, or the whole chain drifts even where there is nothing to average.
/// Measured over all 256 codes it does, exactly, which is why this can assert
/// equality rather than a tolerance.
#[test]
fn a_flat_colour_survives_every_level() {
    for v in [0u8, 1, 17, 64, 128, 191, 254, 255] {
        let n = 16;
        let level0: Vec<u8> = std::iter::repeat([v, v, v, 255])
            .take((n * n) as usize)
            .flatten()
            .collect();
        for filter in [Filter::Srgb, Filter::Linear] {
            let out = chain(&level0, n, n, filter);
            for (i, texel) in out.chunks(4).enumerate() {
                assert_eq!(
                    texel[0], v,
                    "{filter:?} moved a flat {v} to {} at texel {i}",
                    texel[0]
                );
            }
        }
    }
}

/// **Every normal in every level is unit length.** `water::ripple_map` states
/// the house rule: a normal map averaged and left unnormalized shortens
/// toward the origin and reads as a loss of *lighting*, not of detail. A
/// shortened normal is not a wrong-looking normal, which is what makes it the
/// kind of defect that survives a review.
#[test]
fn normals_stay_unit_length() {
    let n = 32;
    let mut level0 = Vec::new();
    for y in 0..n {
        for x in 0..n {
            // A varied but valid tangent-space set: z stays positive, x and y
            // sweep, and no two neighbours agree — so averaging genuinely
            // shortens unless something renormalizes.
            let ux = (x as f32 / n as f32) * 1.6 - 0.8;
            let uy = (y as f32 / n as f32) * 1.6 - 0.8;
            let uz = (1.0f32 - ux * ux - uy * uy).max(0.05).sqrt();
            let l = (ux * ux + uy * uy + uz * uz).sqrt();
            for c in [ux / l, uy / l, uz / l] {
                level0.push(((c * 0.5 + 0.5) * 255.0).round() as u8);
            }
            level0.push(255);
        }
    }
    let out = chain(&level0, n, n, Filter::Normal);
    // Skip level 0 — it is the fixture, copied.
    for (i, texel) in out.chunks(4).enumerate().skip((n * n) as usize) {
        let v: Vec<f32> = texel[..3]
            .iter()
            .map(|b| *b as f32 / 255.0 * 2.0 - 1.0)
            .collect();
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        // 1/255 of a component is ~0.008 of length, and three of them
        // quadrature to ~0.014. The tolerance is the byte quantization and
        // nothing else — an unnormalized average of this fixture lands near
        // 0.9, which is far outside it.
        assert!(
            (l - 1.0).abs() < 0.02,
            "texel {i} is {l} long, not 1 — an averaged normal was not renormalized"
        );
    }
}

/// The linear filter is a plain average and nothing else. Roughness and AO
/// are already linear data and neither is a vector; decoding either as sRGB
/// would brighten every distant surface's roughness and flatten its shading.
#[test]
fn the_linear_filter_is_a_plain_average() {
    let mut level0 = Vec::new();
    for v in [10u8, 20, 30, 40] {
        level0.extend_from_slice(&[v, v, v, 255]);
    }
    let out = chain(&level0, 2, 2, Filter::Linear);
    // (10+20+30+40)/4 = 25, exactly.
    assert_eq!(out[16], 25, "expected the arithmetic mean of 10,20,30,40");
}

// ---------------------------------------------------------------------------
// Which images get one, and which filter.
// ---------------------------------------------------------------------------

/// **The path decides before the format does, and it has to.** A normal map
/// and a roughness map are both loaded `is_srgb = false`, so they arrive as
/// the same `Rgba8Unorm` and the format alone cannot separate them. If the
/// order flipped, every normal map in the game would be plain-averaged and
/// every distant surface would lose the length of its normals.
#[test]
fn the_filter_is_picked_from_the_path_then_the_format() {
    use TextureFormat::{Rgba8Unorm, Rgba8UnormSrgb};
    for (path, format, want) in [
        ("textures/rock_albedo.jpg", Rgba8UnormSrgb, Filter::Srgb),
        ("textures/rock_normal.jpg", Rgba8Unorm, Filter::Normal),
        ("textures/rock_rough.jpg", Rgba8Unorm, Filter::Linear),
        ("textures/rock_ao.jpg", Rgba8Unorm, Filter::Linear),
        // A normal map that somehow arrived sRGB is still a normal map. The
        // path is the stronger statement.
        ("textures/grass_normal.jpg", Rgba8UnormSrgb, Filter::Normal),
    ] {
        assert_eq!(Filter::pick(path, format), want, "{path} picked wrong");
    }
}

/// **An image that brought its own chain must never be filtered again.**
/// `water::ripple_map` and `tree::needle_mips` both build theirs by hand; a
/// second pass would read their level 1 as part of level 0 and produce a
/// texture that is garbage from the second level down — and it would look
/// like a water bug, not like this file.
#[test]
fn an_image_that_brought_its_own_chain_is_refused() {
    let mut img = flat_image(8, 8, TextureFormat::Rgba8Unorm);
    assert!(wants(&img), "a plain single-level image should be wanted");
    img.texture_descriptor.mip_level_count = 4;
    assert!(!wants(&img), "an image with a chain was accepted for a second");
}

/// Non-power-of-two is refused rather than filtered badly. A 2×2 box on an
/// odd side drops a row, which is a subtly wrong chain — the kind that looks
/// like a slight drift rather than an error.
#[test]
fn a_non_power_of_two_is_refused() {
    assert!(!wants(&flat_image(12, 8, TextureFormat::Rgba8Unorm)));
    assert!(!wants(&flat_image(8, 12, TextureFormat::Rgba8Unorm)));
    assert!(wants(&flat_image(8, 8, TextureFormat::Rgba8Unorm)));
    // 1×1 has no level below it, so there is no chain to build.
    assert!(!wants(&flat_image(1, 1, TextureFormat::Rgba8Unorm)));
}

/// A format this filter cannot read is refused. Every arm of `wants` is a
/// skip and not a failure, because it runs over every image the asset server
/// produces and most of them are none of its business.
#[test]
fn a_format_it_cannot_read_is_refused() {
    assert!(!wants(&flat_image(8, 8, TextureFormat::R8Unorm)));
    assert!(wants(&flat_image(8, 8, TextureFormat::Rgba8UnormSrgb)));
}

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

/// A deterministic non-flat RGBA8 level 0 — every channel varies, so a filter
/// that ignores one is visible.
fn ramp(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            v.push((x * 7 % 256) as u8);
            v.push((y * 11 % 256) as u8);
            v.push(((x + y) * 13 % 256) as u8);
            v.push(255);
        }
    }
    v
}

/// A mid-grey image of a given size and format, for the `wants` screens.
fn flat_image(w: u32, h: u32, format: TextureFormat) -> Image {
    let bytes = if format == TextureFormat::R8Unorm { 1 } else { 4 };
    Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![128u8; (w * h * bytes) as usize],
        format,
        RenderAssetUsages::RENDER_WORLD,
    )
}
