//! Gate: the ground's four identities are the surfaces `ART.md` §3 measured,
//! and the island they mix into is more than one of them.
//!
//! **Why this exists.** `GROUND_ALBEDO`'s doc comment says two things in
//! plain words — *"the hue and saturation are the reference's"* and *"granite
//! is warm grey and roughly 2× turf's value"* — and until this file nothing
//! checked either. `CLAUDE.md`'s walls list has the general form of that
//! failure: a claim that something is enforced, with nothing enforcing it,
//! reads as covered while it drifts. It had drifted. The visual judge measured
//! the delivered frames at hue 29–35° across the whole island with **zero
//! pixels** in §3's grass band (63–74°), and the constants reproduce that
//! without a renderer: turf sat at 84.0° and 22.9% saturation against §3's
//! 63–74° and 29–33%, litter at 31.1°/29.6% against 34–42°/10.5–19.5%, granite
//! at 7.6% against 10–19%, the granite:turf value separation at 1.44× against
//! the claimed 2×, and litter — not grass — was the darkest identity.
//!
//! **Every test here was run red before it was run green**, against the
//! constants this pass replaced: 4 of 5 failed on the old `GROUND_ALBEDO` and
//! the fifth (`granite_never_reaches_the_ground_on_the_capture_seed`) is
//! independent of it. A gate nobody has seen fail is not evidence.
//!
//! **Why albedo may be compared to a lit measurement at all**, which is the
//! one thing that could make this gate bogus. It may for hue and for HSV
//! saturation and it may NOT for luma, and the reason is arithmetic: a white
//! light multiplies all three channels by one scalar, and both `hue` and
//! `S = (max − min) / max` are invariant under that multiply. Absolute luma is
//! not, so this file never asserts one — it asserts the *ratio* between two
//! identities, which is scale-invariant for the same reason.
//!
//! **The spread rule is the half that catches the mechanism**, and no
//! per-identity check can stand in for it. `vertex_color` mixes the four
//! albedos by the splat weights, so what the ground READS as is set by their
//! relative chroma rather than by any one of them: litter was the most
//! saturated identity on the island and it is warm, so it took the hue of every
//! mix it appeared in — and it appears in 37.6% of the land. Four identities in
//! the weights, one hue in the picture, and every identity in isolation
//! arguably fine. The rule is that the resolved hue's p10 and p90 must fall in
//! different `ART.md` §3 bands; it was red before this pass and is green after,
//! and it chooses no number to be either.
//!
//! Headless — no GPU, no window, no shard. Arithmetic over the shipped
//! constants and the shipped seed.

#![cfg(feature = "render")]

use client::render::terrain_mesh::GROUND_ALBEDO;
use sim_core::terrain;

/// The seed the capture probe shoots and every terrain golden pins.
const SEED: u64 = 20260731;

/// `ART.md` §3's measured surfaces, as the bands this gate holds the four
/// identities to. Hue in degrees, saturation as HSV S.
///
/// Three rows are quoted straight from §3's table. The fourth — forest litter
/// — has no row of its own there, because the reference frames sampled a
/// *dirt path* (139 luma, 38°, 15%) rather than needle litter under canopy.
/// It gets the dirt row's centre with the widest hue span §3 states anywhere
/// (granite's 8°), which is a derivation from the table rather than a number
/// chosen to fit what shipped; the alternative was to leave the one identity
/// that is 60% of a forest floor ungated.
///
/// The two single-valued rows (sand, dirt) are given the span of the nearest
/// banded row rather than a tolerance picked here: §3 states sand as 42°/10%
/// with no range, and granite — the neighbouring warm grey — as 35–43°/10–19%,
/// an 8° and 9-point span. Widths come from the document; centres come from
/// the document; nothing in this table is this file's opinion.
struct Band {
    name: &'static str,
    hue: (f32, f32),
    sat: (f32, f32),
}

const BANDS: [Band; 4] = [
    // §3 "beach sand — 117 luma, 42°, 10%", widened by granite's spans.
    Band {
        name: "sand",
        hue: (38.0, 46.0),
        sat: (0.055, 0.145),
    },
    // §3 "grass, lit — 59–70 luma, 63–74°, 29–33%", verbatim.
    Band {
        name: "grass",
        hue: (63.0, 74.0),
        sat: (0.29, 0.33),
    },
    // §3 "dirt path — 139 luma, 38°, 15%", widened by granite's spans.
    Band {
        name: "litter",
        hue: (34.0, 42.0),
        sat: (0.105, 0.195),
    },
    // §3 "granite, lit — 127–167 luma, 35–43°, 10–19%", verbatim.
    Band {
        name: "rock",
        hue: (35.0, 43.0),
        sat: (0.10, 0.19),
    },
];

/// §3 states the granite/turf separation as "roughly 2×" and `GROUND_ALBEDO`'s
/// own doc comment repeats it. "Roughly" is what the floor here is for: the
/// claim is a value separation, not a target to hit on the nose.
const VALUE_SEPARATION_MIN: f32 = 1.8;

/// Linear → sRGB, the encode the swapchain applies. §3's numbers were read off
/// PNGs, so the comparison has to happen on this side of it.
fn srgb(c: f32) -> f32 {
    let c = c.max(0.0);
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Hue in degrees and HSV saturation of an sRGB triple. Both are invariant
/// under a scalar multiply, which is what lets an albedo be compared to a lit
/// measurement (module header).
fn hue_sat(rgb: [f32; 3]) -> (f32, f32) {
    let max = rgb[0].max(rgb[1]).max(rgb[2]);
    let min = rgb[0].min(rgb[1]).min(rgb[2]);
    let d = max - min;
    if d <= 0.0 || max <= 0.0 {
        return (0.0, 0.0);
    }
    let h = if max == rgb[0] {
        60.0 * (((rgb[1] - rgb[2]) / d) % 6.0)
    } else if max == rgb[1] {
        60.0 * ((rgb[2] - rgb[0]) / d + 2.0)
    } else {
        60.0 * ((rgb[0] - rgb[1]) / d + 4.0)
    };
    ((h + 360.0) % 360.0, d / max)
}

/// Rec.601 luma over 0..255, the estimator `ART.md` §3 and the visual judge
/// both use.
fn luma(rgb: [f32; 3]) -> f32 {
    255.0 * (0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2])
}

/// One identity's albedo, encoded.
fn encoded(i: usize) -> [f32; 3] {
    let a = GROUND_ALBEDO[i];
    [srgb(a[0]), srgb(a[1]), srgb(a[2])]
}

/// The mix `terrain_mesh::vertex_color` performs, encoded. Deliberately not a
/// call into it: this gate is about the *identities and the mix*, and
/// `vertex_color` also applies the macro break-up and the waterline, neither
/// of which is what §3 measured.
fn resolved(w: [u8; 4]) -> [f32; 3] {
    let inv = 1.0 / 255.0;
    let mut c = [0.0f32; 3];
    for (k, wk) in w.iter().enumerate() {
        let f = *wk as f32 * inv;
        for (ch, cc) in c.iter_mut().enumerate() {
            *cc += GROUND_ALBEDO[k][ch] * f;
        }
    }
    [srgb(c[0]), srgb(c[1]), srgb(c[2])]
}

#[test]
fn each_ground_identity_carries_the_hue_and_saturation_art_md_measured() {
    let mut wrong = Vec::new();
    for (i, band) in BANDS.iter().enumerate() {
        let (h, s) = hue_sat(encoded(i));
        if h < band.hue.0 || h > band.hue.1 {
            wrong.push(format!(
                "{}: hue {h:.1}° outside ART.md §3's {:.0}–{:.0}°",
                band.name, band.hue.0, band.hue.1
            ));
        }
        if s < band.sat.0 || s > band.sat.1 {
            wrong.push(format!(
                "{}: saturation {:.1}% outside ART.md §3's {:.1}–{:.1}%",
                band.name,
                s * 100.0,
                band.sat.0 * 100.0,
                band.sat.1 * 100.0
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "GROUND_ALBEDO says its hue and saturation are the reference's; they are not:\n  {}",
        wrong.join("\n  ")
    );
}

#[test]
fn granite_stands_clear_of_turf_in_value() {
    let rock = luma(encoded(3));
    let grass = luma(encoded(1));
    let ratio = rock / grass;
    assert!(
        ratio >= VALUE_SEPARATION_MIN,
        "ART.md §3: granite is 'much brighter than turf — a value separation of \
         roughly 2×'. Measured {ratio:.2}× (granite {rock:.1}, turf {grass:.1}). \
         Luma-identical ground is what makes the whole island read as one surface."
    );
}

#[test]
fn grass_is_the_darkest_identity() {
    // `GROUND_ALBEDO`'s own doc comment: "grass is the darkest thing on the
    // island". Held so that a future pass brightening turf to reach the band
    // above cannot quietly retire the statement it was authored under.
    let grass = luma(encoded(1));
    for (i, band) in BANDS.iter().enumerate() {
        if i == 1 {
            continue;
        }
        let other = luma(encoded(i));
        assert!(
            grass < other,
            "grass ({grass:.1}) is not darker than {} ({other:.1}), which \
             GROUND_ALBEDO's doc comment states it is",
            band.name
        );
    }
}

/// The island must be more than one surface, and where a mix is pure it must
/// be the surface `ART.md` §3 measured.
///
/// **This is the visual judge's finding turned into arithmetic.** It measured
/// the delivered frames at *"hue 29–35°, and zero pixels of `ART.md` §3's
/// grass band (63–74°) exist on the ground"* — one identity across a whole
/// island whose splat weights are four. Nothing in this repo could see that:
/// every per-identity check can pass while the mix collapses, because what
/// collapses it is the *relative* chroma of the identities, not any one of
/// them.
///
/// Two rules, and neither chooses a number:
///
///  * **Spread.** The resolved hue's p10 and p90 over the island's land must
///    fall in *different* §3 bands. Bands partition the document, p10/p90 are
///    not tuned, and an island that reads as one surface fails this however
///    that surface was arrived at. Measured on the constants this pass
///    replaced: p10 **31.1°** and p90 **84.0°**, both outside every band §3
///    states, so the rule read them as the same nothing and went red. After:
///    p10 38.0° (litter) and p90 68.5° (grass).
///
/// One honest gap, because it bounds what this file may be cited for. The old
/// vertex colours held two hue populations (31.1° and 84.0°) while the visual
/// judge measured the delivered *pixels* at 29–35° with nothing above it — so
/// something between `vertex_color` and the framebuffer was already eating the
/// green, and this gate cannot see it. Candidates, none tested: the granite
/// photograph's own chroma multiplying through `base_color_texture`, the
/// lighting, the tonemap, or a near band that is mostly clutter and props
/// rather than ground. **This file proves the authored identities, not the
/// picture.** `NOW.md` §0gi carries the residual.
///  * **Purity.** Where one identity holds the entire weight, the ground is
///    that identity and must land inside its own band.
///
/// What is deliberately NOT asserted is where a *blend* lands. A mix of two
/// identities resolves between them; demanding it sit in the majority's band
/// would be demanding the splat stop ramping, and three successive attempts to
/// state such a rule here all reduced to either a tautology or a coin-flip on
/// near-tied mixes. The spread rule catches the same defect without one.
#[test]
fn the_island_is_more_than_one_surface() {
    // A 4 m lattice over the island's full 2 km extent — the pitch the
    // measurements in this file's comments were taken at. Offset by half a
    // step so it never samples the origin or a chunk boundary.
    const STEP: f32 = 4.0;
    const HALF: f32 = 1024.0;

    let mut hues: Vec<f32> = Vec::new();
    let mut impure = Vec::new();

    let mut z = -HALF + STEP * 0.5;
    while z < HALF {
        let mut x = -HALF + STEP * 0.5;
        while x < HALF {
            // Land only: the sea has its own material and §3 measured ground.
            if terrain::height(SEED, x, z) <= 0.5 {
                x += STEP;
                continue;
            }
            let w = terrain::splat(SEED, x, z);
            let (hue, _) = hue_sat(resolved(w));
            hues.push(hue);

            // Purity: one identity holding all 255 IS that identity.
            if let Some(k) = (0..4).find(|k| w[*k] == 255) {
                let band = &BANDS[k];
                if (hue < band.hue.0 || hue > band.hue.1)
                    && impure.iter().all(|s: &String| !s.starts_with(band.name))
                {
                    {
                        impure.push(format!(
                            "{name} is pure at ({x:.0},{z:.0}) and resolves to hue \
                             {hue:.1}°, outside its own {lo:.0}–{hi:.0}°",
                            name = band.name,
                            lo = band.hue.0,
                            hi = band.hue.1
                        ));
                    }
                }
            }
            x += STEP;
        }
        z += STEP;
    }

    assert!(
        hues.len() > 30_000,
        "the island shrank: {} land samples at {STEP} m",
        hues.len()
    );
    assert!(
        impure.is_empty(),
        "a pure identity does not read as the surface ART.md §3 measured:\n  {}",
        impure.join("\n  ")
    );

    hues.sort_by(|a, b| a.partial_cmp(b).expect("hue is never NaN"));
    let at = |q: f32| hues[((hues.len() - 1) as f32 * q) as usize];
    let (p10, p90) = (at(0.10), at(0.90));
    let band_of = |h: f32| {
        BANDS
            .iter()
            .position(|b| h >= b.hue.0 && h <= b.hue.1)
            .map_or("no band", |k| BANDS[k].name)
    };
    // Both ends must land in a NAMED band before their difference means
    // anything. Without this, one end resolving to "no band" satisfies the
    // `assert_ne!` below on its own — the rule would pass on an island whose
    // p10 sits in no §3 surface at all, which is not "more than one surface",
    // it is one surface and one unrecognised colour. Named by the merge-gate
    // judge of pass 20260814-142610-02 as a hole in this test.
    for (label, h) in [("p10", p10), ("p90", p90)] {
        assert_ne!(
            band_of(h),
            "no band",
            "resolved hue {label} {h:.1}° falls in none of ART.md §3's bands, \
             so this test cannot say the island is more than one surface"
        );
    }
    assert_ne!(
        band_of(p10),
        band_of(p90),
        "the island reads as one surface: resolved hue p10 {p10:.1}° and p90 \
         {p90:.1}° are both '{}'. This is the defect the visual judge measured \
         as 'hue 29–35° and zero pixels of the grass band'.",
        band_of(p10)
    );
}

/// Granite is authored, gated above, and **never drawn on this seed** — both
/// of `splat_from`'s routes to it are closed by *this island's* range.
///
/// Measured over 39,521 land samples at 4 m, seed 20260731:
///
/// | route | band opens at | the island's extreme |
/// |---|---|---|
/// | `SPLAT_ALPINE_BAND` (height) | 44.0 m | max 46.32 m, p99.9 **43.63 m** |
/// | `SPLAT_CLIFF_BAND` (slope) | 0.952 | max **0.890** — never reached, once |
///
/// ⚠ **"On this seed" is the whole of the correction, and the sentence this
/// comment used to open with — "granite is never drawn" — was false.** It read
/// as a fact about `splat_from`; it is a fact about seed 20260731. Re-measured
/// 2026-08-14 across 40 seeds (`crates/sim-core/tests/relief.rs`, and the note
/// in `gates-loop/findings/`): the median island tops out at **93.89 m** with a
/// max slope of **2.203** and paints a legible rock weight on **6.11%** of its
/// land, and this seed is the **minimum of the forty on all three axes**.
/// Granite reaches the ground on 38 of 40 islands. `NOW.md` §0gi item 1 read
/// the table above as "move the bands"; moving them decouples the cliff ramp
/// from `CLIFF_SLOPE_RATIO` and the alpine ramp from `biome()`'s Highland edge,
/// which is a relationship `DECISIONS.md` §open materials v0 and `TERRAIN.md`
/// §7.1 both state — so `relief.rs` gates it, and it is red under exactly that
/// edit.
///
/// **This test asserts the mechanism, not the defect**, and the mechanism it
/// now holds is *the outlier*: these four numbers are why every visual report
/// this loop has written was written against a pancake. Which island the probe
/// photographs is `DECISIONS.md` §open, not a builder's call. When that is
/// answered this goes red with the new numbers in hand.
#[test]
fn granite_never_reaches_the_ground_on_the_capture_seed() {
    const STEP: f32 = 4.0;
    const HALF: f32 = 1024.0;

    let mut hmax = f32::MIN;
    let mut smax = f32::MIN;
    let mut rock_max = 0u8;
    let mut land = 0u64;

    let mut z = -HALF + STEP * 0.5;
    while z < HALF {
        let mut x = -HALF + STEP * 0.5;
        while x < HALF {
            if terrain::height(SEED, x, z) > 0.5 {
                land += 1;
                hmax = hmax.max(terrain::height(SEED, x, z));
                smax = smax.max(terrain::slope(SEED, x, z));
                rock_max = rock_max.max(terrain::splat(SEED, x, z)[3]);
            }
            x += STEP;
        }
        z += STEP;
    }

    assert!(
        land > 30_000,
        "the island shrank: {land} land samples at {STEP} m"
    );
    // The cliff mask is the one that FORCES rock, and it is the one that never
    // fires: SPLAT_CLIFF_BAND opens at tan(50°) × 0.8 = 0.952.
    assert!(
        smax < 0.952,
        "the island's max slope is now {smax:.3}, at or past SPLAT_CLIFF_BAND's \
         0.952 — the cliff mask can fire, so granite is reachable and this test \
         and ABSENT above are both stale"
    );
    // The alpine ramp opens at 44 m and the island tops out just past it, which
    // is why granite exists in the weights and not in the picture.
    assert!(
        (44.0..48.0).contains(&hmax),
        "the island's max height is now {hmax:.2} m against SPLAT_ALPINE_BAND's \
         44.0–60.0 — the alpine route's reach has changed materially"
    );
    assert!(
        rock_max < 16,
        "granite now reaches {rock_max}/255 somewhere (was 15) — if the bands \
         moved, drop \"rock\" from ABSENT and let the plurality rule cover it"
    );
}
