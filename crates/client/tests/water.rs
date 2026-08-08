//! The sea's gate. **Renderer tier**: it links Bevy for `Image` and `Vec3`,
//! but it opens no window, needs no GPU and reads no pixel.
//!
//! `CLAUDE.md` is explicit that there is no visual gate here and that writing
//! one is forbidden — a pixel statistic cannot see whether the frame is a
//! picture of anything, and this repo has the beige smear to prove it. What
//! *may* be gated about a frame is arithmetic: that the mesh fits the volume
//! the sim blocks, in Rust, in the shape of `tests/tree.rs`. So nothing below
//! asks whether the water looks good. Every assertion is a property that,
//! violated, produces a specific named artefact — a sea that aliases into a
//! crawling ghost wave, that climbs the beach, that tiles with a seam, or that
//! is lit like polished plastic.
//!
//! The person who decides whether it looks good boots the game and looks.
//!
//! **`assertions_on_constants` is allowed here on purpose.** Most of this file
//! asserts relations *between* knobs — that green outlives red in the
//! extinction table, that the reflectance and the IOR agree — and clippy is
//! right that those fold at compile time. That is the point: a knob gate is a
//! statement about values somebody will edit, and a relation that folds to
//! `true` today is exactly the one that stops folding when they do.

#![allow(clippy::assertions_on_constants)]

use client::render::terrain_mesh::{self, WET_BAND_M, WET_VALUE};
use client::render::water::*;
use sim_core::terrain::{ISLAND_SIZE, SEA_LEVEL};

// ---------------------------------------------------------------------------
// The wave set.
// ---------------------------------------------------------------------------

/// Directions are unit vectors or every wave's speed and steepness is wrong by
/// its own length — and silently, because a 3% short direction just makes a
/// slightly slower wave.
#[test]
fn wave_directions_are_unit() {
    for (i, w) in WAVES.iter().enumerate() {
        let l = (w.dir[0] * w.dir[0] + w.dir[1] * w.dir[1]).sqrt();
        assert!(
            (l - 1.0).abs() < 1e-3,
            "wave {i}'s direction is {l} long, not 1"
        );
    }
}

/// **The swell must not break.** A wave whose maximum slope reaches 1 has a
/// vertical face; past it a height field folds through itself and the surface
/// self-intersects. The sum over the whole set is what binds, not each wave —
/// four gentle waves that crest together are one steep one.
///
/// `Σ A·k` is the classic Gerstner steepness limit; [`SHAPE_MAX_SLOPE`] is
/// what the crest sharpening multiplies it by, and leaving that factor out is
/// how the same arithmetic ends up 30% optimistic.
#[test]
fn the_swell_cannot_break() {
    let mut slope = 0.0f32;
    for w in WAVES {
        slope += w.amp_m * (std::f32::consts::TAU / w.len_m);
    }
    let peak = slope * SHAPE_MAX_SLOPE;
    assert!(
        peak < 1.0,
        "the swell's peak slope is {peak} - at or past 1 the surface folds through itself"
    );
    // And it must actually be a sea rather than a puddle: if this is near zero
    // the assertion above is passing on nothing.
    assert!(peak > 0.05, "the swell's peak slope is {peak} - it is flat");
}

/// The core must be fine enough to carry every wave in the set. A wave the
/// mesh cannot sample is not a subtle wave: [`band_weight`] retires it, so it
/// is a wave that is simply absent everywhere, in a table that says it is
/// there.
#[test]
fn the_core_carries_every_wave() {
    for (i, w) in WAVES.iter().enumerate() {
        assert!(
            band_weight(STEP_M, w.len_m) > 0.99,
            "wave {i} at {} m is retired by the {STEP_M} m core - it never draws",
            w.len_m
        );
    }
}

/// The mechanism that keeps the far sea from aliasing: past half a wavelength
/// of spacing a wave is gone, and it goes smoothly.
#[test]
fn a_wave_retires_before_its_own_nyquist() {
    let len = 20.0f32;
    assert_eq!(band_weight(len * 0.5, len), 0.0);
    assert_eq!(band_weight(len * 0.6, len), 0.0);
    assert_eq!(band_weight(len * 0.25, len), 1.0);
    assert_eq!(band_weight(1.0, len), 1.0);
    // Monotone across the transition, with no step at either end — a step is a
    // ring of water that visibly stops moving.
    let mut prev = 1.1f32;
    for i in 0..=40 {
        let s = len * (0.25 + 0.25 * i as f32 / 40.0);
        let w = band_weight(s, len);
        assert!(w <= prev + 1e-6, "band weight rose at spacing {s}");
        prev = w;
    }
}

/// Every vertex of the mesh actually satisfies the rule above — the coordinate
/// table and the wave set are two independent knobs and nothing else checks
/// that they agree.
#[test]
fn every_vertex_respects_the_nyquist_rule() {
    let coords = axis_coords();
    let spacing = axis_spacing(&coords);
    for (i, s) in spacing.iter().enumerate() {
        for w in WAVES {
            if *s >= w.len_m * 0.5 {
                assert_eq!(
                    band_weight(*s, w.len_m),
                    0.0,
                    "coordinate {i} at {} m has {s} m spacing and still draws a {} m wave",
                    coords[i],
                    w.len_m
                );
            }
        }
    }
}

/// The grid must reach past the island from anywhere on it, or a player on one
/// shore sees the sea end before the far one does.
#[test]
fn the_grid_reaches_past_the_island() {
    let coords = axis_coords();
    let far = *coords.last().unwrap();
    assert!(
        far >= ISLAND_SIZE,
        "the sea reaches {far} m and the island is {ISLAND_SIZE} m across"
    );
    assert_eq!(coords[0], -far, "the coordinate table is not symmetric");
    // Ascending, or the triangle winding flips somewhere in the middle and
    // half the sea faces away.
    for w in coords.windows(2) {
        assert!(w[1] > w[0], "the coordinate table is not ascending");
    }
    // And the cost has to stay a cost: this is the one number that decides how
    // much work `animate` does every single frame.
    let verts = coords.len() * coords.len();
    assert!(
        verts < 12_000,
        "the sea is {verts} vertices - that is a per-frame budget, not a mesh"
    );
}

/// The core is uniform and the skirt only ever coarsens. A skirt step that
/// went *finer* would put a dense ring in the far field, which costs vertices
/// where nothing can see them.
#[test]
fn the_skirt_only_coarsens() {
    let coords = axis_coords();
    let mid = coords.len() / 2;
    let mut last = 0.0f32;
    for i in mid..coords.len() - 1 {
        let gap = coords[i + 1] - coords[i];
        assert!(
            gap >= last - 1e-3,
            "the skirt narrows from {last} to {gap} at coordinate {i}"
        );
        last = gap;
    }
}

// ---------------------------------------------------------------------------
// The surface.
// ---------------------------------------------------------------------------

const PHASE: [f32; WAVE_COUNT] = [0.3, 1.1, 2.4, 5.0];

/// The gradient must be the height field's own, or the specular disagrees with
/// the silhouette and the sea reads as a lit sheet of plastic laid over a wavy
/// one. Finite differences at fixed weights, which is the property
/// `wave_field` actually claims (see its note on locally-constant weights).
#[test]
fn the_normal_agrees_with_the_height_field() {
    let e = 0.01f32;
    for (x, z) in [(0.0, 0.0), (13.7, -22.1), (-101.5, 47.0), (3.3, 3.3)] {
        let (_, gx, gz) = wave_field(x, z, &PHASE, STEP_M, 1.0);
        let (hx1, _, _) = wave_field(x + e, z, &PHASE, STEP_M, 1.0);
        let (hx0, _, _) = wave_field(x - e, z, &PHASE, STEP_M, 1.0);
        let (hz1, _, _) = wave_field(x, z + e, &PHASE, STEP_M, 1.0);
        let (hz0, _, _) = wave_field(x, z - e, &PHASE, STEP_M, 1.0);
        let fd_x = (hx1 - hx0) / (2.0 * e);
        let fd_z = (hz1 - hz0) / (2.0 * e);
        assert!(
            (gx - fd_x).abs() < 5e-3 && (gz - fd_z).abs() < 5e-3,
            "at ({x}, {z}) the analytic gradient ({gx}, {gz}) disagrees with \
             finite differences ({fd_x}, {fd_z})"
        );
    }
    // And the normal it makes is a normal.
    let (_, gx, gz) = wave_field(9.0, -4.0, &PHASE, STEP_M, 1.0);
    let n = wave_normal(gx, gz);
    assert!((n.length() - 1.0).abs() < 1e-5);
    assert!(n.y > 0.0, "the surface normal points down");
}

/// The crest-sharpened sine has zero mean, and that is not cosmetic: a DC
/// offset here moves the whole sea off `SEA_LEVEL`, so the drawn waterline and
/// the sim's own would sit at different heights.
#[test]
fn the_wave_shape_has_no_dc_offset() {
    let n = 4096;
    let mut sum = 0.0f64;
    let mut peak = 0.0f32;
    for i in 0..n {
        let theta = std::f32::consts::TAU * i as f32 / n as f32;
        let (s, _) = shape(theta);
        sum += s as f64;
        peak = peak.max(s.abs());
    }
    let mean = (sum / n as f64) as f32;
    assert!(
        mean.abs() < 1e-3,
        "the wave shape has a DC offset of {mean}"
    );
    assert!(
        (peak - SHAPE_PEAK).abs() < 1e-3,
        "the wave shape peaks at {peak}, not the documented {SHAPE_PEAK}"
    );
}

/// `shape`'s derivative is its own, checked the same way the field's is —
/// `SHAPE_MAX_SLOPE` is a documented constant that the breaking gate rests on.
#[test]
fn the_wave_shape_slope_is_what_it_claims() {
    let n = 8192;
    let mut peak = 0.0f32;
    for i in 0..n {
        let theta = std::f32::consts::TAU * i as f32 / n as f32;
        let (_, ds) = shape(theta);
        peak = peak.max(ds.abs());
        let e = 1e-3;
        let fd = (shape(theta + e).0 - shape(theta - e).0) / (2.0 * e);
        assert!((ds - fd).abs() < 5e-3, "shape' disagrees at {theta}");
    }
    assert!(
        (peak - SHAPE_MAX_SLOPE).abs() < 1e-2,
        "shape' peaks at {peak}, not the documented {SHAPE_MAX_SLOPE}"
    );
}

/// **The sea must be exactly flat at the waterline.** Any amplitude left at
/// zero depth lifts the surface above the sand it is meeting, and the water
/// visibly climbs the beach and sits there — the artefact `shoal` exists to
/// prevent, and the reason the reference needed two simulations.
#[test]
fn the_swell_dies_at_the_waterline() {
    assert_eq!(shoal(0.0), 0.0);
    assert_eq!(shoal(-3.0), 0.0);
    assert_eq!(shoal(SHOAL_FULL_M), 1.0);
    let (h, gx, gz) = wave_field(20.0, 20.0, &PHASE, STEP_M, shoal(0.0));
    assert_eq!((h, gx, gz), (0.0, 0.0, 0.0));
    // Monotone in between, so the shore is a ramp and not a step.
    let mut prev = -1.0f32;
    for i in 0..=20 {
        let d = SHOAL_FULL_M * i as f32 / 20.0;
        let s = shoal(d);
        assert!(s >= prev - 1e-6, "shoaling fell at depth {d}");
        prev = s;
    }
}

// ---------------------------------------------------------------------------
// The optics.
// ---------------------------------------------------------------------------

/// Both outputs come from one transmittance and both must grade with depth: a
/// shoreline whose alpha ramp and colour ramp disagree about where the
/// shallows end has two visible waterlines.
///
/// **The colour goes UP with depth, and that is the correction the first
/// capture forced.** What the water sends you is scatter, and scatter
/// accumulates; a model where depth darkens the body is treating extinction as
/// if it applied to the water's own light, and it renders a grey sheet.
#[test]
fn depth_grades_colour_and_alpha_together() {
    let mut prev = depth_tint(0.0);
    assert_eq!(prev[3], 0.0, "the waterline is not fully transparent");
    assert_eq!(
        [prev[0], prev[1], prev[2]],
        [0.0; 3],
        "water of zero depth is still contributing colour"
    );
    for i in 1..=60 {
        let d = i as f32 * 0.5;
        let t = depth_tint(d);
        assert!(t[3] >= prev[3] - 1e-6, "alpha fell at {d} m");
        assert!(t[3] <= ALPHA_MAX + 1e-6, "alpha passed ALPHA_MAX at {d} m");
        for c in 0..3 {
            assert!(t[c] >= prev[c] - 1e-6, "channel {c} lost scatter at {d} m");
            assert!(
                t[c] <= SCATTER_ALBEDO[c] + 1e-6,
                "channel {c} scattered back more than it took out, at {d} m"
            );
        }
        prev = t;
    }
    // Deep water must actually be deep-looking by the time the seabed is at
    // its own floor: `terrain::SEA_FLOOR_DEPTH` is 12 m.
    assert!(
        depth_tint(12.0)[3] > 0.8,
        "the open sea is still see-through over a 12 m floor"
    );
}

/// The sea is Atlantic, not Caribbean — the reference's own stated retune
/// (`reference/WATER.md` §2). Green must reach at least as far as blue and
/// come back strongest, which is what coastal water does and what tropical
/// water does not.
#[test]
fn the_sea_is_a_coastal_green_not_a_tropical_blue() {
    assert!(
        EXTINCT[1] <= EXTINCT[2],
        "blue penetrates further than green - that is open ocean, not a coast"
    );
    assert!(
        EXTINCT[0] > EXTINCT[1] * 2.0,
        "red is not being eaten fast enough for this to be water"
    );
    assert!(
        SCATTER_ALBEDO[1] > SCATTER_ALBEDO[2] && SCATTER_ALBEDO[2] > SCATTER_ALBEDO[0],
        "the scattering albedo is not green > blue > red"
    );
    // And the deep sea has to be a colour at all, not a dark grey: the whole
    // reason the first cut was replaced.
    let deep = depth_tint(12.0);
    let luma = 0.2126 * deep[0] + 0.7152 * deep[1] + 0.0722 * deep[2];
    assert!(
        luma > 0.03,
        "the deep sea's own colour is {luma} - it is black"
    );
    assert!(
        deep[1] > deep[0] * 3.0,
        "the deep sea is grey: {:?}",
        &deep[..3]
    );
}

/// Transmittance is a transmittance: 1 at the surface, monotone, and it never
/// leaves [0, 1] however deep the sentinel goes.
#[test]
fn transmittance_is_bounded_and_monotone() {
    let t0 = transmittance(0.0);
    for v in t0 {
        assert!((v - 1.0).abs() < 1e-6);
    }
    let mut prev = t0;
    for i in 1..=200 {
        let d = i as f32 * 0.5;
        let t = transmittance(d);
        for c in 0..3 {
            assert!(
                t[c] <= prev[c] + 1e-9 && t[c] >= 0.0,
                "channel {c} at {d} m"
            );
        }
        prev = t;
    }
    let far = transmittance(DEEP_SENTINEL_M);
    for (c, v) in far.iter().enumerate() {
        assert!(
            *v < 1e-3,
            "the sentinel depth does not saturate channel {c}"
        );
    }
    // Negative depth is dry land, not a light source.
    assert_eq!(transmittance(-5.0), t0);
}

/// Foam is a *shore* effect weighted by the LAND's slope — the reference's own
/// published observation about its ocean foam, and the reason this function
/// takes a terrain slope at all.
#[test]
fn foam_is_a_shore_effect_keyed_to_the_land() {
    assert_eq!(shore_foam(FOAM_DEPTH_M, 1.0), 0.0);
    assert_eq!(shore_foam(20.0, 1.0), 0.0);
    let flat = shore_foam(0.2, 0.0);
    let steep = shore_foam(0.2, FOAM_SLOPE_FULL);
    assert!(flat > 0.0, "a flat shore gets no foam at all");
    assert!(steep > flat, "a steep shore is not foamier than a flat one");
    assert!(steep <= 1.0, "foam is over-unity at {steep}");
    // Deeper is less foamy, all the way out of the band.
    let mut prev = f32::MAX;
    for i in 0..=20 {
        let d = FOAM_DEPTH_M * i as f32 / 20.0;
        let f = shore_foam(d, 0.5);
        assert!(f <= prev + 1e-6, "foam rose with depth at {d} m");
        prev = f;
    }
    // Whitecaps are a fraction of a crest, not a paint bucket.
    assert_eq!(crest_foam(0.0, 1.0), 0.0);
    assert_eq!(crest_foam(1.0, 0.0), 0.0);
    assert!(crest_foam(1.0, 1.0) <= CREST_FOAM_MAX + 1e-6);
    assert!(crest_foam(0.5, 1.0) < crest_foam(0.95, 1.0));
}

/// Mixing foam in must not take the colour anywhere a colour cannot go, and it
/// must move the alpha up rather than down — foam is aerated water, which is
/// the one thing on the surface you cannot see through.
#[test]
fn foam_stays_a_colour() {
    for d in [0.0f32, 0.5, 3.0, 30.0] {
        let base = depth_tint(d);
        for f in [0.0f32, 0.4, 1.0] {
            let out = with_foam(base, f);
            for (c, v) in out.iter().enumerate() {
                assert!(
                    (0.0..=1.0).contains(v),
                    "foam {f} at {d} m put channel {c} at {v}"
                );
            }
            assert!(out[3] >= base[3] - 1e-6, "foam made the water clearer");
        }
        assert_eq!(with_foam(base, 0.0), base);
    }
}

/// The vertex buffer carries **premultiplied** colour, and that is not a
/// format detail: it is what keeps the Fresnel reflection of the sky alive in
/// shallow water, where the alpha that carries the *depth* is near zero and
/// under straight blending would scale the specular away with it.
///
/// The invariant a premultiplied buffer has to hold is that no channel exceeds
/// its own alpha — a surface that does is one adding more light to the frame
/// than it transmitted, which is emission, not water. Foam is the case that
/// nearly broke it: it is the brightest thing on the sea and it is mixed in
/// after the grading.
#[test]
fn the_surface_never_emits() {
    for d in [0.0f32, 0.2, 0.4, 2.0, 5.0, 12.0, DEEP_SENTINEL_M] {
        for f in [0.0f32, 0.3, 0.7, 1.0] {
            let c = with_foam(depth_tint(d), f);
            for (ch, v) in c[..3].iter().enumerate() {
                assert!(
                    *v <= c[3] + 1e-6,
                    "at {d} m with foam {f}, channel {ch} is {v} against an alpha of {}",
                    c[3]
                );
                assert!(*v >= 0.0);
            }
        }
    }
    // At the waterline the surface contributes nothing at all — everything you
    // see there is the sand behind it plus the sky on it.
    assert_eq!(depth_tint(0.0), [0.0; 4]);
}

// ---------------------------------------------------------------------------
// The ripple map.
// ---------------------------------------------------------------------------

/// The map tiles over the whole ocean, so a discontinuity at its edge is a
/// grid of seams every [`RIPPLE_TILE_M`] as far as the eye can see. It tiles
/// iff every harmonic is an integer, which is exactly what this measures.
#[test]
fn the_ripple_map_tiles() {
    let img = ripple_map();
    let n = RIPPLE_TEX as usize;
    let data = img.data.as_ref().expect("the ripple map has no data");
    let texel = |x: usize, y: usize| {
        let i = (y * n + x) * 4;
        [
            data[i] as f32 / 255.0 * 2.0 - 1.0,
            data[i + 1] as f32 / 255.0 * 2.0 - 1.0,
            data[i + 2] as f32 / 255.0 * 2.0 - 1.0,
        ]
    };
    // The wrap must be no bigger a step than an ordinary neighbouring pair.
    let mut inner = 0.0f32;
    let mut wrap = 0.0f32;
    for i in 0..n {
        let d = |a: [f32; 3], b: [f32; 3]| {
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
        };
        inner = inner.max(d(texel(i, n / 2), texel(i, n / 2 + 1)));
        wrap = wrap.max(d(texel(i, n - 1), texel(i, 0)));
        wrap = wrap.max(d(texel(n - 1, i), texel(0, i)));
    }
    assert!(
        wrap <= inner * 1.5 + 1e-3,
        "the ripple map's wrap steps by {wrap} against an inner step of {inner} - it does not tile"
    );
}

/// A normal map is unit vectors around `+Z`, and it is a *perturbation*: if
/// the mean leaned off the surface normal the whole sea would be tilted by the
/// texture.
#[test]
fn the_ripple_map_is_a_perturbation() {
    let img = ripple_map();
    let n = RIPPLE_TEX as usize;
    let data = img.data.as_ref().expect("the ripple map has no data");
    let mut mean = [0.0f64; 3];
    let mut worst = 0.0f32;
    for i in 0..n * n {
        let v = [
            data[i * 4] as f32 / 255.0 * 2.0 - 1.0,
            data[i * 4 + 1] as f32 / 255.0 * 2.0 - 1.0,
            data[i * 4 + 2] as f32 / 255.0 * 2.0 - 1.0,
        ];
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        worst = worst.max((l - 1.0).abs());
        assert!(v[2] > 0.0, "a ripple normal faces into the surface");
        for c in 0..3 {
            mean[c] += v[c] as f64 / (n * n) as f64;
        }
    }
    // 1/255 of quantization on each channel is the floor here.
    assert!(worst < 0.02, "a ripple normal is {worst} off unit length");
    assert!(
        mean[0].abs() < 0.02 && mean[1].abs() < 0.02,
        "the ripple map leans: mean ({}, {})",
        mean[0],
        mean[1]
    );
    assert!(
        mean[2] > 0.9,
        "the ripple map is not a perturbation, it is terrain"
    );
}

/// **The mip chain must be complete or wgpu rejects the texture at first
/// draw**, which on a box with no display is a black window and a backtrace.
/// Bevy builds no mips for an `Image` made in code, so the count and the byte
/// length are ours to keep in step.
#[test]
fn the_ripple_map_carries_its_own_mips() {
    let img = ripple_map();
    let levels = img.texture_descriptor.mip_level_count;
    assert_eq!(levels, RIPPLE_TEX.trailing_zeros() + 1);
    assert_eq!(
        img.data.as_ref().map(|d| d.len()),
        Some(ripple_bytes(RIPPLE_TEX)),
        "the ripple map's bytes do not match its declared mip chain"
    );
    // A one-level map on a surface that reaches kilometres is a shimmering
    // carpet, so the chain has to go all the way down.
    assert!(levels > 1);
}

// ---------------------------------------------------------------------------
// The material, and the land side of the waterline.
// ---------------------------------------------------------------------------

/// Water's Fresnel reflectance at normal incidence is 2%, and Bevy's field is
/// `F0 = 0.16·reflectance²`. The plane this replaced shipped 0.55 — `F0` of
/// 4.8%, nearly two and a half times too specular. That is not a look, it is a
/// different material.
#[test]
fn the_sea_is_lit_like_water() {
    let f0 = 0.16 * WATER_REFLECTANCE * WATER_REFLECTANCE;
    let ior_f0 = ((WATER_IOR - 1.0) / (WATER_IOR + 1.0)).powi(2);
    assert!(
        (f0 - ior_f0).abs() < 2e-3,
        "reflectance {WATER_REFLECTANCE} is F0 {f0}, but IOR {WATER_IOR} says {ior_f0}"
    );
    assert!(
        WATER_ROUGHNESS > 0.0,
        "a perfectly smooth sea has one pixel of sun on it and nothing else"
    );
}

/// The land half of the shoreline: wet at and below the waterline, dry above
/// the band, monotone between, and never outside `ART.md`'s albedo band on the
/// dark side — a beach that goes black at the tideline is worse than one that
/// stays dry.
#[test]
fn the_shore_is_wet_and_stays_a_surface() {
    assert_eq!(terrain_mesh::wet_factor(SEA_LEVEL), 1.0);
    assert_eq!(terrain_mesh::wet_factor(SEA_LEVEL - 4.0), 1.0);
    assert_eq!(terrain_mesh::wet_factor(SEA_LEVEL + WET_BAND_M), 0.0);
    assert_eq!(terrain_mesh::wet_factor(SEA_LEVEL + 40.0), 0.0);
    let mut prev = 1.1f32;
    for i in 0..=40 {
        let y = SEA_LEVEL + WET_BAND_M * i as f32 / 40.0;
        let w = terrain_mesh::wet_factor(y);
        assert!(w <= prev + 1e-6, "wetness rose with height at {y}");
        prev = w;
    }

    // Every ground identity, soaked, has to stay a surface.
    let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    for (i, dry) in terrain_mesh::GROUND_ALBEDO.iter().enumerate() {
        let wet = terrain_mesh::wetted(*dry, 1.0);
        assert!(
            luma(wet) < luma(*dry),
            "identity {i} does not darken when it is wet"
        );
        assert!(
            luma(wet) >= terrain_mesh::ALBEDO_LUMA_FLOOR - 1e-6,
            "identity {i} soaks to {} luma, under ART.md's albedo floor",
            luma(wet)
        );
        for (c, v) in wet.iter().enumerate() {
            assert!(
                (0.0..=1.0).contains(v),
                "identity {i} channel {c} left [0,1]"
            );
        }
        // Dry is dry: the modifier has to be a no-op above the band.
        assert_eq!(terrain_mesh::wetted(*dry, 0.0), *dry);
        // And it must be a *value* change, not a hue rotation: the darkest
        // channel stays the darkest.
        let order = |c: [f32; 3]| {
            let mut ix = [0usize, 1, 2];
            ix.sort_by(|a, b| c[*a].partial_cmp(&c[*b]).unwrap());
            ix
        };
        assert_eq!(order(*dry), order(wet), "identity {i} changed hue when wet");
    }
    // The soak is the documented one, within the saturation stretch.
    let grey = [0.3f32, 0.3, 0.3];
    assert!((luma(terrain_mesh::wetted(grey, 1.0)) / luma(grey) - WET_VALUE).abs() < 1e-3);
}
