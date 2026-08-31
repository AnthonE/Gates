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

//! ⚠ **Feature-gated, and it was not until this merge.** `client::render` is
//! behind `--features render` (`crates/client/Cargo.toml` says why), so a
//! file here that names it is red on a plain `cargo test --workspace` and on
//! `ci/gates.sh`'s first clippy pass — which is what happened: this suite
//! landed on `main` unguarded and the gate has been red since. 34 of the
//! other test files carry this line; `tests/viewmodel_arms.rs`'s header
//! spells out the failure. Added on the merge rather than left for main,
//! because a branch cannot be green on top of a red base.

#![cfg(feature = "render")]

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use client::render::mipmap::{
    chain, chain_bytes, coverage, is_translucent, levels, wants, Filter, MASK_CUT,
};

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
        assert_eq!(
            &out[..level0.len()],
            &level0[..],
            "{filter:?} moved level 0"
        );
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
    assert_eq!(
        out[16], 146,
        "expected 146, the linear mean 0.2892 re-encoded"
    );
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
        let level0: Vec<u8> = std::iter::repeat_n([v, v, v, 255], (n * n) as usize)
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
    for (path, format, translucent, want) in [
        (
            "textures/rock_albedo.jpg",
            Rgba8UnormSrgb,
            false,
            Filter::Srgb,
        ),
        (
            "textures/rock_normal.jpg",
            Rgba8Unorm,
            false,
            Filter::Normal,
        ),
        ("textures/rock_rough.jpg", Rgba8Unorm, false, Filter::Linear),
        ("textures/rock_ao.jpg", Rgba8Unorm, false, Filter::Linear),
        // A normal map that somehow arrived sRGB is still a normal map. The
        // path is the stronger statement.
        (
            "textures/grass_normal.jpg",
            Rgba8UnormSrgb,
            false,
            Filter::Normal,
        ),
        // The cutout. It is an sRGB albedo like the first row and differs only
        // by carrying alpha, which is measured and not guessed.
        (
            "textures/grass_card_albedo.png",
            Rgba8UnormSrgb,
            true,
            Filter::Mask,
        ),
    ] {
        assert_eq!(
            Filter::pick(path, format, translucent),
            want,
            "{path} picked wrong"
        );
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
    assert!(
        !wants(&img),
        "an image with a chain was accepted for a second"
    );
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
    let bytes = if format == TextureFormat::R8Unorm {
        1
    } else {
        4
    };
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

// ---------------------------------------------------------------------------
// The cutout: coverage, not alpha.
// ---------------------------------------------------------------------------

/// **A box-filtered cutout goes bald, and this is the assertion that says so.**
///
/// An alpha-tested surface does not draw its alpha — it draws the share of
/// texels that survive the cutoff. Averaging a sparse mask drives every texel
/// toward the mask's mean, and a grass card's mean is about 0.22, well under
/// the 0.5 test. So each level loses coverage against the one above and the
/// loss compounds; on the page that reads as grass THINNING with distance,
/// which looks like a density bug and is a filtering one.
/// `tree::needle_mips` measured its own version at 0.53x after one halving.
#[test]
fn a_cutout_keeps_its_coverage_all_the_way_down() {
    let (w, h) = (256u32, 256u32);
    let level0 = blades(w, h);
    let want = coverage(&level0);
    assert!(
        (0.10..0.40).contains(&want),
        "the fixture is not a sparse mask ({want}) and so cannot show the defect"
    );

    let out = chain(&level0, w, h, Filter::Mask);
    let mut off = 0usize;
    for lvl in 0..levels(w, h) {
        let (lw, lh) = ((w >> lvl).max(1), (h >> lvl).max(1));
        let n = (lw as usize) * (lh as usize) * 4;
        let got = coverage(&out[off..off + n]);
        // **Asymmetric, because the two errors are not the same defect.**
        // Losing coverage is the baldness this whole filter exists to stop and
        // is held tight; gaining a little is `preserve_coverage` taking `hi`
        // over the midpoint on purpose, and it costs a texel of extra grass at
        // range. The bottom levels are a handful of texels where exact
        // coverage is unreachable at any scale — a 4x4 can only express
        // sixteenths — so they are only held against LOSS.
        let (lose, gain) = if lw >= 16 { (0.02, 0.10) } else { (0.10, 1.0) };
        assert!(
            got >= want - lose,
            "level {lvl} ({lw}x{lh}) draws {got:.3} of its texels against level \
             0's {want:.3} — the chain is losing coverage and the grass thins \
             with distance"
        );
        assert!(
            got <= want + gain,
            "level {lvl} ({lw}x{lh}) draws {got:.3} against level 0's \
             {want:.3} — the rescale is inventing coverage, not preserving it"
        );
        off += n;
    }
}

/// The plain sRGB filter on the same fixture is what the Mask filter is NOT.
///
/// This is the mutant, run as a test rather than by hand: it proves the
/// defect above is real and that `Filter::Mask` is the thing preventing it.
/// If this ever stops failing to hold coverage, the two filters have become
/// the same and the one above is gating nothing.
#[test]
fn the_plain_filter_would_have_gone_bald() {
    let (w, h) = (256u32, 256u32);
    let level0 = blades(w, h);
    let want = coverage(&level0);
    let out = chain(&level0, w, h, Filter::Srgb);
    // Level 1 alone, one halving from full detail.
    let n1 = ((w >> 1) as usize) * ((h >> 1) as usize) * 4;
    let off = (w as usize) * (h as usize) * 4;
    let got = coverage(&out[off..off + n1]);
    assert!(
        got < want * 0.9,
        "a plain average held {got:.3} against {want:.3}; if it no longer \
         loses coverage then `Filter::Mask` is not buying anything"
    );
}

/// Alpha is measured, not assumed from the extension. A fully opaque RGBA
/// image is an ordinary albedo and must not be put through a coverage
/// bisection — which would rescale nothing but would say the wrong thing
/// about what this module does.
#[test]
fn opacity_is_measured_and_not_guessed() {
    let opaque: Vec<u8> = std::iter::repeat_n([12u8, 34, 56, 255], 64)
        .flatten()
        .collect();
    assert!(!is_translucent(&opaque));
    // 254 is not a cutout: a lossy re-encode of a solid interior lands there,
    // and one such texel must not promote an albedo to a mask.
    let nearly: Vec<u8> = std::iter::repeat_n([12u8, 34, 56, 254], 64)
        .flatten()
        .collect();
    assert!(!is_translucent(&nearly));
    let mut cut = opaque.clone();
    cut[3] = 0;
    assert!(is_translucent(&cut));
}

/// The cutoff this module preserves against and the one the frame tests with
/// are one number. A drift between them preserves a coverage nothing draws.
#[test]
fn the_cutoff_is_the_one_the_frame_tests_with() {
    // `clutter::CARD_ALPHA_CUT` is the runtime's `AlphaMode::Mask` value, 0..1.
    let as_byte = (client::render::clutter::CARD_ALPHA_CUT * 255.0).round() as u8;
    assert_eq!(
        as_byte, MASK_CUT,
        "the mip chain preserves coverage above {MASK_CUT} and the frame keeps \
         texels above {as_byte} — the grass thins with distance by exactly \
         that gap"
    );
}

/// A sparse vertical-blade mask, deterministic — the shape a grass card's
/// alpha actually has, which a uniform noise field is not: what makes a cutout
/// hard to filter is that its coverage is well under the cutoff.
///
/// **The edges are anti-aliased and that is not cosmetic.** The first version
/// of this fixture drew hard 2-texel bars, so its alpha histogram had three
/// values and its coverage could only be about 0.19 or about 0.40 — no scale
/// existed that landed between them, and the test was demanding one. A
/// photographic cutout is a photograph: its edge texels are partial coverage,
/// its alpha is continuous, and a scale that hits the target exists. The hard
/// fixture was still worth writing, because it is what found the midpoint bug
/// in `preserve_coverage`.
fn blades(w: u32, h: u32) -> Vec<u8> {
    let mut v = vec![0u8; (w * h * 4) as usize];
    let mut seed = 0x9e37_79b9u32;
    for _ in 0..(w / 3) {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let x0 = (seed >> 16) % w;
        let bh = h / 2 + (seed >> 8) % (h / 2);
        // **Sub-texel wide, and that is the whole mechanism.** A box filter
        // does not lose coverage on a feature wider than its kernel — a soft
        // 3-texel bar keeps its 0.5 crossing under halving and comes through
        // fine. What a chain destroys is a feature THINNER than a texel: its
        // peak alpha is averaged against its empty neighbour and drops under
        // the cutoff, so the blade stops being drawn at all. A grass card is
        // full of those (blade tips, the gaps between blades), which is why
        // `tree::needle_mips` measured 0.53x after one halving and why this
        // fixture measures 0.30x. Widths vary so the mask is not one spatial
        // frequency.
        let half = 0.15 + ((seed >> 4) % 3) as f32 * 0.15;
        for y in (h - bh)..h {
            for dx in -3i32..=3 {
                let cover = (half - dx.abs() as f32 + 0.5).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    continue;
                }
                let x = ((x0 as i32 + dx).rem_euclid(w as i32)) as u32;
                let i = ((y * w + x) * 4) as usize;
                let a = (cover * 255.0) as u8;
                if a > v[i + 3] {
                    v[i] = 90;
                    v[i + 1] = 140;
                    v[i + 2] = 60;
                    v[i + 3] = a;
                }
            }
        }
    }
    v
}
