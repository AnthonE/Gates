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
//! the fifth (`granite_reaches_the_ground_on_the_shipped_seed`) is independent
//! of it. A gate nobody has seen fail is not evidence.
//!
//! ⚠ That fifth name is not the one this line carried until 2026-08-15, and the
//! rename is the point rather than a typo. It was
//! `granite_never_reaches_the_ground_on_the_capture_seed` — a test asserting the
//! *opposite* fact — and it was renamed and inverted on 2026-08-14 when the
//! quadrant sweep behind it was retracted (`CLAUDE.md`'s trap list; the sweep
//! read `-1024..1024` on a world centred at 1024). The module doc kept naming
//! the retracted test for a day, which is the doc-reads-as-covered failure the
//! paragraph above it describes, one level in.
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

use client::render::fill::GROUND_MIX;
use client::render::terrain_mesh::GROUND_ALBEDO;
use sim_core::terrain;

/// The seed the shard ships, so the seed the capture probe shoots.
///
/// **Proposed for replacement on 2026-08-14 and kept.** The report that called
/// it the flattest of forty islands had swept `-1024..1024` — one quadrant of a
/// world whose coordinates run 0..2048 — and over the whole island it is
/// upper-third for granite. `sim-core/tests/relief.rs` carries the retraction
/// and the numbers.
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

/// **No two identities that actually share the island may be the same paint.**
///
/// This is the gate that was missing on 2026-08-15, and the defect it now holds
/// is the one the visual judge measured without being able to name:
/// `pass-20260815-042118-10-visual.md` gap 1 read the ground as hue 33–37° and
/// luma 96–113 at *every* sample of six frames, with ~0.4% of pixels reading as
/// granite where `ART.md` §0 records 8.9% of the land within 300 m of the
/// capture spawn carrying it. Granite was not missing. **Forest litter and
/// granite were 1.0° apart in hue, 0.5 points in saturation and 1.059× in
/// value**, and those two identities own 89.4% of the land inside that radius,
/// so an island with four identities was painted with three.
///
/// Every per-identity check in this file passed throughout, and so did
/// [`the_island_is_more_than_one_surface`] — the island's hue spread is fine,
/// because grass is genuinely far from the warm three. The defect is *local*:
/// two identities that are individually in band and are indistinguishable from
/// each other. Nothing here could see a pair.
///
/// **Two numbers, neither of them §3's, both in `DECISIONS.md` §open.**
///
///  * **Which pairs are tested** — those where *both* members cover at least 5%
///    of the island's land, by `fill::GROUND_MIX`. Not the combined share: that
///    is dominated by the larger member and would drag beach sand into a rule
///    about the forest floor. The cut is deliberately insensitive — the four
///    shares are 1.1%, 51.8%, 37.9% and 9.2%, so **any threshold between 1.1%
///    and 9.2% selects the same three identities**, which is the strongest
///    thing that can be said about a number nobody measured. Sand is excluded
///    on its own share and because `wetted` darkens the band it lives in.
///  * **How far apart is far enough** — 1.25× in Rec.601 luma. §3 states no
///    such ratio; what it states is a *table*, and the tightest pair in it that
///    the document names as two different substances is beach sand (117) and
///    granite (147), at 1.256×. Anything a reader of `spawnedrock.jpg` can tell
///    apart clears that. §3's own dirt-path/granite pair is 1.058× and is
///    deliberately not the anchor: a path is a ribbon, and the identity that
///    borrowed its row is a third of the world.
///
/// Red on the constants this replaced (litter:granite 1.059×), green now at
/// 1.429× — 44.2 luma of separation where there were 6.7.
#[test]
fn granite_stands_clear_of_the_ground_it_shares() {
    /// Both members of a pair must cover at least this much land for the pair
    /// to be one a player sees adjacent. `DECISIONS.md` §open.
    const SHARE_MIN: f32 = 0.05;
    /// §3's tightest named pair of distinct substances, sand:granite.
    /// `DECISIONS.md` §open.
    const TWIN_RATIO_MIN: f32 = 1.25;

    let mut same_paint = Vec::new();
    let mut tested = 0;
    for i in 0..4 {
        for j in (i + 1)..4 {
            if GROUND_MIX[i] < SHARE_MIN || GROUND_MIX[j] < SHARE_MIN {
                continue;
            }
            tested += 1;
            let (a, b) = (luma(encoded(i)), luma(encoded(j)));
            let ratio = a.max(b) / a.min(b);
            println!(
                "{:>6} ({:.1}%) vs {:>6} ({:.1}%): {ratio:.3}×, {:.1} luma apart",
                BANDS[i].name,
                GROUND_MIX[i] * 100.0,
                BANDS[j].name,
                GROUND_MIX[j] * 100.0,
                (a - b).abs()
            );
            if ratio < TWIN_RATIO_MIN {
                same_paint.push(format!(
                    "{} ({:.1} luma, {:.1}% of the land) and {} ({:.1} luma, \
                     {:.1}% of the land) are {ratio:.3}× apart — the same paint. \
                     `ART.md` §8: materials must read as distinct substances at \
                     a glance, separated by value.",
                    BANDS[i].name,
                    a,
                    GROUND_MIX[i] * 100.0,
                    BANDS[j].name,
                    b,
                    GROUND_MIX[j] * 100.0
                ));
            }
        }
    }
    // The rule is worthless if the share cut selects nothing — that is how a
    // wall goes green by deleting its own cases (`CLAUDE.md`, and this repo has
    // done it once already this month).
    assert_eq!(
        tested, 3,
        "the 5% share cut should select grass, litter and granite — three \
         pairs. It selected {tested}, so either GROUND_MIX moved or the cut is \
         no longer insensitive, and the rule is not testing what it says."
    );
    assert!(
        same_paint.is_empty(),
        "two identities that share the island are indistinguishable:\n  {}",
        same_paint.join("\n  ")
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
    //
    // ⚠ **`[0, ISLAND_SIZE)`, and it read `-1024..1024` until 2026-08-14**,
    // which is a square whose corner is the island's centre: one quadrant,
    // 632 k m² of a 2.9 M m² island. `terrain::continent` centres on
    // `(ISLAND_SIZE/2, ISLAND_SIZE/2)` and world coordinates run 0..2048.
    // Everything this file measured was a quarter-island statistic.
    const STEP: f32 = 4.0;
    const LO: f32 = 0.0;
    const HI: f32 = terrain::ISLAND_SIZE;

    let mut hues: Vec<f32> = Vec::new();
    let mut impure = Vec::new();

    let mut z = LO + STEP * 0.5;
    while z < HI {
        let mut x = LO + STEP * 0.5;
        while x < HI {
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
        hues.len() > 100_000,
        "the island shrank: {} land samples at {STEP} m (a whole island is \
         ~154 k here — 30_000 was the floor while this swept a quadrant)",
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

/// Granite is authored, gated above, and **reaches the ground on the island we
/// ship** — both of `splat_from`'s routes to it are open on *this island's*
/// range.
///
/// ⚠ **This test used to assert the opposite, on the same seed, and the reason
/// is a sweep window rather than a world.** It read: the height route tops out
/// at 46.32 m against a 44 m band opening (p99.9 43.63 m), the slope route
/// reaches 0.890 against a 0.952 band, the cliff mask has never fired once, max
/// rock anywhere is 15/255. Every one of those numbers came from a
/// `-1024..1024` sweep — **one quadrant** of an island centred on (1024, 1024),
/// not even the quadrant this file's camera stands in. Over the world square
/// the same seed reaches 106.00 m, slope 2.665 and granite on 10.0% of its
/// land, against a 44-island median of 7.2%. Fixed and retracted 2026-08-14;
/// `sim-core/tests/relief.rs` holds the full retraction and a gate against the
/// window coming back.
///
/// `NOW.md` §0gi item 1 proposed the third option — move the bands so the flat
/// island paints rock — and that is the one that is actively wrong: it
/// decouples the cliff ramp from `CLIFF_SLOPE_RATIO` and the alpine ramp from
/// `biome()`'s Highland edge, a relationship `DECISIONS.md` materials v0 and
/// `TERRAIN.md` §7.1 both state. `relief.rs` is red under exactly that edit.
///
/// **What this asserts is still the mechanism, not the picture**: that both
/// routes are reachable on the shipped world. Whether the frame *looks* like
/// granite is the operator's eye and the visual judge's, and no arithmetic here
/// can stand in for either (`CLAUDE.md`, the beige-smear trap).
#[test]
fn granite_reaches_the_ground_on_the_shipped_seed() {
    const STEP: f32 = 4.0;
    // The world square, `[0, ISLAND_SIZE)` — see the sweep above for why this
    // is not `-1024..1024`, and what it cost.
    const LO: f32 = 0.0;
    const HI: f32 = terrain::ISLAND_SIZE;
    /// `SPLAT_CLIFF_BAND`'s opening: tan(50°) × 0.8. The mask that FORCES rock.
    const CLIFF_OPENS: f32 = 0.952;
    /// `SPLAT_ALPINE_BAND`'s opening.
    const ALPINE_OPENS: f32 = 44.0;
    /// Where a channel stops being rounding and starts being a visible share
    /// of the blend — `relief.rs` uses the same floor.
    const LEGIBLE: u8 = 32;

    let mut hmax = f32::MIN;
    let mut smax = f32::MIN;
    let mut rock_max = 0u8;
    let (mut land, mut rock, mut cliff) = (0u64, 0u64, 0u64);

    let mut z = LO + STEP * 0.5;
    while z < HI {
        let mut x = LO + STEP * 0.5;
        while x < HI {
            if terrain::height(SEED, x, z) > 0.5 {
                land += 1;
                hmax = hmax.max(terrain::height(SEED, x, z));
                let s = terrain::slope(SEED, x, z);
                smax = smax.max(s);
                if s >= CLIFF_OPENS {
                    cliff += 1;
                }
                let r = terrain::splat(SEED, x, z)[3];
                rock_max = rock_max.max(r);
                if r >= LEGIBLE {
                    rock += 1;
                }
            }
            x += STEP;
        }
        z += STEP;
    }

    assert!(
        land > 100_000,
        "the island shrank: {land} land samples at {STEP} m (a whole island \
         is ~154 k here)"
    );
    assert!(
        smax > CLIFF_OPENS,
        "the island's max slope is {smax:.3}, short of SPLAT_CLIFF_BAND's \
         {CLIFF_OPENS} — the cliff mask cannot fire anywhere, which is the \
         pancake condition the shipped seed was changed to leave"
    );
    assert!(
        cliff > 0,
        "no land sample on the whole island is at or past the cliff band"
    );
    assert!(
        hmax > ALPINE_OPENS + 20.0,
        "the island tops out at {hmax:.2} m, barely past SPLAT_ALPINE_BAND's \
         {ALPINE_OPENS} — the alpine route reaches almost nothing again"
    );
    assert_eq!(
        rock_max, 255,
        "granite's strongest weight anywhere is {rock_max}/255, so no ground \
         on this island reads as granite outright"
    );
    // The share, not just the extreme: one cliff face at the map edge would
    // satisfy every assert above and put no rock in any frame.
    let permille = 1000 * rock / land;
    assert!(
        permille >= 40,
        "granite is legible on {permille} per-mille of the land (was 72 when \
         this seed was chosen, against a 44-island median of 63). The world \
         under the camera has drifted back towards the pancake."
    );
}
