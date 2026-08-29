//! Gate: where the green goes.
//!
//! `NOW.md` §0gi item 3 asked one question — *"Something between
//! `vertex_color` and the pixel eats the green"* — and named four untested
//! suspects: the detail texture's chroma, the lighting, the tonemap, or a near
//! band that is not ground. **The answer is none of them. Nothing eats the
//! green.** The ground is a two-population mosaic, the capture camera stands
//! in one population per run, and which one it draws is not pinned.
//!
//! ## What was measured
//!
//! **1. The material side cannot rotate hue.** Everything `vertex_color` does
//! after the identity mix is either a scalar on all three channels (the
//! `hash01` macro break-up, and `wetted`'s `value`) or an add of grey plus a
//! scale (`wetted`'s `chroma`, which is `luma·(1−k)·1 + k·c`). Adding grey
//! leaves every channel *difference* untouched, and hue is a ratio of
//! differences, so both are exactly hue-preserving. Measured over the shipped
//! island: worst delta **0.000061°** across 39,300 land samples.
//!
//! **2. The detail map is neutral to the last texel.** `textures.rs` calls
//! `ground_detail.jpg` a "LUMINANCE-only map" and nothing checked it — the
//! shape `CLAUDE.md`'s ⚠ is about, and the sibling claim one screen away
//! (`ground_material`'s doc comment describes a granite per-channel gain the
//! code does not use) was already wrong. Measured on the shipped file:
//! per-channel linear mean `[0.24657, 0.24657, 0.24657]`, per-texel HSV
//! saturation **0.0 at every percentile**.
//!
//! **3. So the green was never in the frame.** `SPLAT_MOIST_BAND` is
//! `(0.01, 0.09)` — a ramp **0.08 wide across a moisture field that spans
//! about 0.9** (p05 −0.446, p95 +0.429 on the shipped seed). Grass and litter
//! are therefore a *switch*, not a blend: 48.8% of land reads as grass alone,
//! 34.7% as litter alone, and only **12.1% of land is inside the ramp at
//! all**. The two identities sit **30.5° apart** in hue (grass 68.5°, litter
//! 38.0°), so a frame shot from one place shows one of two colours, and
//! neither is an average of them.
//!
//! **4. And the mix is won by litter wherever they do meet.** Grass is the
//! darkest identity (linear luma 0.0515) and litter is 3.2× brighter (0.1660),
//! so a linear albedo average is dominated by litter long before the weights
//! are even. Derived from `GROUND_ALBEDO` in `grass_must_hold_most_of_a_mix`:
//! grass must hold **≥ 82.1%** of a grass/litter blend for the result to still
//! read green-dominant.
//!
//! ## Why three visual reports said "one painted material"
//!
//! The capture probe shoots all six vantages from the spawn point and never
//! walks (`render/capture.rs` — `VANTAGES` is six yaw/pitch pairs, no
//! translation), and `World::spawn_pos` draws a *bearing* from the player id.
//! Measured over 16 bearings on the shipped seed, the near band's
//! green-dominant share is **bimodal**: four of the first five sit at 78–85%,
//! and one sits at **0.0%**. So which of the two populations a visual report
//! is a picture of is a **draw**, redrawn every run — the same class as the
//! unpinned sun elevation the visual judge already carries as its own defect
//! 17, and it compounds with it.
//!
//! The judge's measurement ("the whole island is hue 29–35°, zero pixels in
//! the grass band") is correct *about the frames it was given*. It is a
//! correct reading of one tile of a mosaic, stated about the island.
//!
//! **What this file does NOT say.** It does not say the frames are fine, and
//! the visual judge's ranked gap 1 stands on its own merits — a litter floor
//! with no understory standing on it is still a defect, and nothing here
//! speaks to the splat layer count, the missing grass geometry or the
//! shoreline transition. What it retires is the *cause* three reports have
//! named. `CLAUDE.md`: a judge names the symptom, fix the cause — and a pass
//! that goes hunting between `vertex_color` and the framebuffer is hunting a
//! bug that is not there.
//!
//! Headless — no GPU, no window, no shard. Arithmetic over the shipped
//! constants, the shipped seed and the shipped texture.

#![cfg(feature = "render")]

use client::render::terrain_mesh::{vertex_color, GROUND_ALBEDO};
use client::render::textures::GROUND_DETAIL_GAIN;
use sim_core::terrain;
use sim_core::world::World;

/// The seed the shard ships, so the seed the capture probe shoots. Same
/// constant and same reason as `ground_identity.rs`.
const SEED: u64 = 20260731;

/// `GROUND_ALBEDO`'s own order — `terrain::splat`'s order.
const GRASS: usize = 1;
const LITTER: usize = 2;

/// Where a channel stops being rounding and starts being a visible share of
/// the blend. Not chosen here: `relief.rs` and `ground_identity.rs` both use
/// this floor, and a third value would make three files disagree about what
/// "present" means.
const LEGIBLE: u8 = 32;

/// sRGB encode of one linear channel — the space `ART.md` §3 and the visual
/// judge both measure in.
fn srgb(u: f32) -> f32 {
    if u <= 0.003_130_8 {
        u * 12.92
    } else {
        1.055 * u.powf(1.0 / 2.4) - 0.055
    }
}

/// HSV hue in degrees. Restated rather than shared with `ground_identity.rs`:
/// a test that imports another test's helper is a test that fails for two
/// reasons.
fn hue(c: [f32; 3]) -> f32 {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let d = max - min;
    if d <= 1e-12 || max <= 1e-12 {
        return 0.0;
    }
    let h = if max == c[0] {
        60.0 * (((c[1] - c[2]) / d) % 6.0)
    } else if max == c[1] {
        60.0 * ((c[2] - c[0]) / d + 2.0)
    } else {
        60.0 * ((c[0] - c[1]) / d + 4.0)
    };
    (h + 360.0) % 360.0
}

fn saturation(c: [f32; 3]) -> f32 {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    if max <= 1e-12 {
        0.0
    } else {
        (max - min) / max
    }
}

/// Shortest angular distance, so a pair straddling 0° is not read as 359°
/// apart.
fn arc(a: f32, b: f32) -> f32 {
    let r = (a - b).abs();
    r.min(360.0 - r)
}

/// `vertex_color`'s first stage alone — the identity mix, linear, with the
/// break-up and the waterline left off.
fn mix_only(w: [u8; 4]) -> [f32; 3] {
    let inv = 1.0 / 255.0;
    let mut c = [0.0f32; 3];
    for (k, wk) in w.iter().enumerate() {
        let f = *wk as f32 * inv;
        for (ch, cc) in c.iter_mut().enumerate() {
            *cc += GROUND_ALBEDO[k][ch] * f;
        }
    }
    c
}

/// Walk the shipped island's land at `STEP`, handing each sample's height,
/// slope and splat weights to `f`.
///
/// ⚠ `[0, ISLAND_SIZE)`, never `-1024..1024`: `terrain::continent` centres the
/// island on `(ISLAND_SIZE/2, ISLAND_SIZE/2)`, so that square's *corner* is the
/// island's centre and it samples one quadrant. That mistake cost a retraction
/// on 2026-08-14 (`sim-core/tests/relief.rs`).
fn walk_land(step: f32, mut f: impl FnMut(f32, f32, f32, f32, [u8; 4])) {
    let mut z = step * 0.5;
    while z < terrain::ISLAND_SIZE {
        let mut x = step * 0.5;
        while x < terrain::ISLAND_SIZE {
            let y = terrain::height(SEED, x, z);
            if y > 0.5 {
                let sl = terrain::slope(SEED, x, z);
                f(
                    x,
                    z,
                    y,
                    sl,
                    terrain::splat_from(y, terrain::moisture(SEED, x, z), sl),
                );
            }
            x += step;
        }
        z += step;
    }
}

/// Leg 1, and the direct answer to `NOW.md` §0gi item 3. Every stage
/// `vertex_color` applies after the identity mix is either a scalar on all
/// three channels or an add of grey followed by a scale, and neither moves
/// hue — adding grey leaves every channel *difference* untouched, and hue is a
/// ratio of differences.
///
/// **Compared in LINEAR space, and the space is the whole point.** An earlier
/// draft of this test compared sRGB-encoded triples and read a worst delta of
/// 0.262°, which looks like a small tint and is not one: sRGB encoding is
/// per-channel and nonlinear, so a *scalar* multiply in linear space is not a
/// scalar multiply after encoding, and the curve moves hue by a fraction of a
/// degree on its own. That drift is real, understood, and four hundred times
/// too small to explain a 30° gap — but comparing in the encoded space would
/// have meant choosing a tolerance to absorb it, and a tolerance chosen to
/// absorb a known effect is a tolerance that also absorbs an unknown one.
///
/// **The bound is not tuned.** In linear space the invariance is exact in real
/// arithmetic, so the only residue is `f32` cancellation: measured worst is
/// **0.000061°** over 39,300 samples. `SLACK_DEG` is 0.01 — two orders above
/// that measurement and three below the smallest tint that could matter (the
/// grass↔litter gap this file is about is 30.5°). Nothing lands between those
/// two, which is what makes the number safe rather than lucky.
///
/// **Run red before it was run green, twice, and the two numbers are worth
/// keeping because they bound what this test can and cannot see.** Giving
/// `wetted` a per-channel `[1.0, 1.0 + wet * 0.35, 1.0]` in place of its
/// single `chroma` takes the worst delta to **0.32°** — red, but only that
/// large because `wetted` is a no-op wherever `wet_factor` is zero, which is
/// most of the island. Putting a 2% red tint on the macro break-up instead,
/// which applies to every vertex, takes it to **3.90°**. So the floor of what
/// this catches is a tint confined to the wet band, and it catches that at 32×
/// the bound.
#[test]
fn the_material_side_of_the_chain_cannot_rotate_hue() {
    // 8 m. Coarser than `ground_identity.rs`'s 4 m because every sample here
    // is its own independent check rather than one contribution to a
    // percentile — the pitch buys coverage, not resolution.
    const STEP: f32 = 8.0;
    /// Hue is not usefully defined on a near-grey sample, and `ART.md` §3
    /// measures none of them.
    const MIN_SAT: f32 = 0.02;
    const SLACK_DEG: f32 = 0.01;

    let mut worst = 0.0f32;
    let mut worst_at = (0.0f32, 0.0f32);
    let mut checked = 0u64;

    walk_land(STEP, |x, z, y, sl, w| {
        let mixed = mix_only(w);
        if saturation(mixed) < MIN_SAT {
            return;
        }
        let d = vertex_color(y, w, x, z, sl);
        let delta = arc(hue(mixed), hue([d[0], d[1], d[2]]));
        checked += 1;
        if delta > worst {
            worst = delta;
            worst_at = (x, z);
        }
    });

    assert!(
        checked > 20_000,
        "the sweep found almost no land to check ({checked} samples at {STEP} m) \
         — the window moved, not the colour"
    );
    assert!(
        worst <= SLACK_DEG,
        "the material side of the chain rotated hue by {worst:.6}° at \
         ({:.0}, {:.0}) over {checked} samples (measured worst: 0.000061°).\n\
         `vertex_color`'s macro break-up and waterline are supposed to be a \
         scalar and an add-grey-then-scale — one of them now tints.\n\
         `NOW.md` §0gi item 3 asked whether something between `vertex_color` \
         and the pixel eats the green; this test answering NO is what retired \
         that question, and it is live again.",
        worst_at.0,
        worst_at.1
    );
}

/// Leg 2. `textures.rs` says `ground_detail.jpg` is a LUMINANCE-only map and
/// that its scalar gain is "what makes the span 1". Both are claims about a
/// file on disk, and a claim about a file that nothing opens is exactly what
/// `CLAUDE.md`'s ⚠ is about.
///
/// If someone re-bakes this map from a colour source, the ground silently
/// acquires that source's chroma on top of the splat's — a tint no reviewer
/// can see in a diff, because the diff is a binary.
#[test]
fn the_ground_detail_map_is_neutral_and_the_gain_still_places_its_mean() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/textures/ground_detail.jpg"
    );
    let bytes = std::fs::read(path).unwrap_or_else(|e| {
        panic!(
            "the ground's detail map is not on disk at {path}: {e}\n\
             It ships — `assets/textures/MANIFEST.md` carries its row."
        )
    });
    let img = image::load_from_memory(&bytes)
        .expect("ground_detail.jpg did not decode as an image")
        .to_rgb8();

    let mut sum = [0.0f64; 3];
    let mut worst_sat = 0.0f32;
    let n = u64::from(img.width()) * u64::from(img.height());
    for px in img.pixels() {
        let mut lin = [0.0f32; 3];
        for (ch, l) in lin.iter_mut().enumerate() {
            let u = f32::from(px.0[ch]) / 255.0;
            *l = if u <= 0.040_45 {
                u / 12.92
            } else {
                ((u + 0.055) / 1.055).powf(2.4)
            };
            sum[ch] += f64::from(*l);
        }
        worst_sat = worst_sat.max(saturation(lin));
    }
    let mean = sum.map(|s| (s / n as f64) as f32);

    // Neutral per TEXEL, not merely on average — three equal channel means
    // would also be produced by a red half and a cyan half, which is precisely
    // the failure a mean cannot see.
    assert!(
        worst_sat <= 0.01,
        "ground_detail.jpg is not the luminance-only field `textures.rs` says \
         it is: worst per-texel HSV saturation {worst_sat:.4} (measured on the \
         shipped file: 0.0000 at every percentile).\n\
         A detail map with chroma multiplies its own colour into every ground \
         pixel on top of the splat's, and `ART.md` §7 bounds that at ×1."
    );

    let span = mean[0].max(mean[1]).max(mean[2]) / mean[0].min(mean[1]).min(mean[2]);
    assert!(
        span <= 1.001,
        "ground_detail.jpg's channel means are not equal: {mean:?}, span {span:.4}"
    );

    // The gain is the reciprocal that makes the delivered mean the authored
    // vertex colour, so a re-bake that moves the mean without moving the
    // constant darkens or brightens every ground pixel on the island.
    let want = 1.0 / mean[1];
    let drift = (GROUND_DETAIL_GAIN / want - 1.0).abs();
    assert!(
        drift <= 0.01,
        "GROUND_DETAIL_GAIN is {GROUND_DETAIL_GAIN}, but the shipped file's \
         linear mean is {:.5}, whose reciprocal is {want:.4} ({:.1}% off).\n\
         If the map was re-baked, re-measure it.",
        mean[1],
        drift * 100.0
    );
}

/// Leg 4 (stated before leg 3 because leg 3 cites it). Grass is the darkest
/// identity and litter is 3.2× brighter, so a *linear* albedo average is
/// dominated by litter long before the weights are even — the mix crosses out
/// of green-dominant while grass still holds four fifths of it.
///
/// This chooses no number: the crossover is solved from `GROUND_ALBEDO` by
/// bisection and the assertion is that it is far above half. It is the
/// quantitative form of the observation `ground_identity.rs`'s header already
/// makes in words — *"litter … took the hue of every mix it appeared in"* —
/// and it is why the spread rule there cannot be replaced by a per-identity
/// check.
#[test]
fn grass_must_hold_most_of_a_mix_for_the_ground_to_read_green() {
    // Bisect the grass fraction at which G overtakes R. sRGB is monotonic per
    // channel, so the crossover is the same in linear and encoded space.
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..40 {
        let g = 0.5 * (lo + hi);
        let r = GROUND_ALBEDO[GRASS][0] * g + GROUND_ALBEDO[LITTER][0] * (1.0 - g);
        let gg = GROUND_ALBEDO[GRASS][1] * g + GROUND_ALBEDO[LITTER][1] * (1.0 - g);
        if gg > r {
            hi = g;
        } else {
            lo = g;
        }
    }
    assert!(
        hi > 0.66,
        "grass now only needs {:.1}% of a grass/litter mix to read \
         green-dominant. That is a better ground than the one this file \
         documents — re-measure the mosaic numbers in the header before \
         citing them.",
        hi * 100.0
    );
    // And the reason: the two identities are nowhere near equal in value, so
    // the average is not near the midpoint.
    let luma = |i: usize| {
        0.2126 * GROUND_ALBEDO[i][0] + 0.7152 * GROUND_ALBEDO[i][1] + 0.0722 * GROUND_ALBEDO[i][2]
    };
    assert!(
        luma(LITTER) / luma(GRASS) > 2.0,
        "litter is only {:.2}× grass's value; the crossover above is explained \
         by that ratio and this file's account of it no longer holds",
        luma(LITTER) / luma(GRASS)
    );
}

/// Leg 3, and the finding. The island's ground is **two populations, not a
/// blend**, because `SPLAT_MOIST_BAND` is a ramp 0.08 wide across a moisture
/// field spanning about 0.9. So a frame shot from one place shows one of two
/// colours 30° apart, and no amount of looking between `vertex_color` and the
/// framebuffer will find the other one.
///
/// Three assertions, none of which picks a threshold out of the air: both
/// populations hold a real share of the land, the zone where they actually
/// blend is a minority of it, and their hues are far enough apart that landing
/// in one rather than the other changes what the frame is a picture of.
///
/// **This asserts the mechanism, not that it is desirable.** A mosaic is a
/// perfectly good thing for an island to be; `ART.md` §3 wants more than one
/// surface and this delivers exactly two. What it costs is stated in
/// `the_capture_spawn_decides_which_population_the_frame_shows` below.
#[test]
fn the_ground_is_two_populations_and_not_a_blend() {
    const STEP: f32 = 8.0;

    let (mut land, mut blending, mut grassy, mut littery) = (0u64, 0u64, 0u64, 0u64);
    walk_land(STEP, |_, _, _, _, w| {
        land += 1;
        if w[GRASS] >= LEGIBLE && w[LITTER] >= LEGIBLE {
            blending += 1;
        }
        // "Reads as this identity": the identity holds enough of the mix to
        // own the resulting hue, per `grass_must_hold_most_of_a_mix…` above.
        if w[GRASS] >= 209 {
            grassy += 1;
        }
        if w[LITTER] >= 209 {
            littery += 1;
        }
    });
    assert!(land > 30_000, "the island shrank: {land} land samples");

    let frac = |n: u64| n as f32 / land as f32;
    assert!(
        frac(grassy) > 0.2 && frac(littery) > 0.2,
        "the ground is no longer two populations: {:.1}% reads as grass alone \
         and {:.1}% as litter alone (measured: 48.8% and 34.7%).\n\
         This file's account of every visual report — that the frames show one \
         of two surfaces depending on where the camera spawned — rests on both \
         being common.",
        frac(grassy) * 100.0,
        frac(littery) * 100.0
    );
    assert!(
        frac(blending) < frac(grassy).min(frac(littery)),
        "the grass/litter boundary is now wider than either population: \
         {:.1}% of land has both legible, against {:.1}% grass and {:.1}% \
         litter. `SPLAT_MOIST_BAND` has stopped being a switch, which is a \
         change to what the island looks like, not a defect — but the header \
         of this file describes the old behaviour.",
        frac(blending) * 100.0,
        frac(grassy) * 100.0,
        frac(littery) * 100.0
    );

    let gap = arc(hue(GROUND_ALBEDO[GRASS]), hue(GROUND_ALBEDO[LITTER]));
    assert!(
        gap > 25.0,
        "grass and litter are only {gap:.1}° apart in hue (measured 30.5°), so \
         which population a frame lands in no longer decides its colour"
    );
}

/// The consequence, and the reason this pass exists. The capture probe takes
/// all six vantages from the spawn point and never walks
/// (`render/capture.rs` — `VANTAGES` is six yaw/pitch pairs and no
/// translation), while `World::spawn_pos` draws a *bearing* from the player
/// id. Combined with the mosaic above, that makes the ground colour of a
/// visual report a **draw**, redrawn every run.
///
/// So two consecutive `-visual.md` reports are not comparable on anything the
/// ground's colour touches, and three of them in a row have now called the
/// island "one painted material" while measuring one tile of it. This is the
/// same class as the unpinned sun elevation the visual judge carries as its
/// own defect 17, and it compounds with it.
///
/// **The assertion is the spread, and it survives the fix.** It measures the
/// spawn ring, not the capture harness, so pinning the probe's spawn — the
/// proposal in `NOW.md` §0gi and `DECISIONS.md` §open — leaves it green. What
/// turns it red is the mosaic going away, at which point the finding above is
/// stale and should be re-read rather than trusted.
#[test]
fn the_capture_spawn_decides_which_population_the_frame_shows() {
    /// The near-ground band: `VANTAGES` sit at pitch −0.15 rad under a 75°
    /// vertical FOV from a 1.6 m eye, so the bottom of a frame meets the
    /// ground about 1.5 m out, and "near" (−0.85 rad) is looking straight at
    /// it. 30 m is well past where that band stops resolving.
    const NEAR_M: f32 = 30.0;
    const STEP: f32 = 2.0;
    /// The spawn ring is per player id; a capture run gets whichever the shard
    /// hands it. 16 is a sample of the ring, not the whole of it.
    const BEARINGS: u32 = 16;

    let world = Box::new(World::new(SEED));
    let mut shares: Vec<f32> = Vec::new();

    for id in 0..BEARINGS {
        let (sx, sz) = world.spawn_pos(id);
        let (mut land, mut green) = (0u64, 0u64);
        let mut dz = -NEAR_M;
        while dz <= NEAR_M {
            let mut dx = -NEAR_M;
            while dx <= NEAR_M {
                if dx * dx + dz * dz <= NEAR_M * NEAR_M {
                    let (x, z) = (sx + dx, sz + dz);
                    let y = terrain::height(SEED, x, z);
                    if y > 0.5 {
                        land += 1;
                        let sl = terrain::slope(SEED, x, z);
                        let w = terrain::splat_from(y, terrain::moisture(SEED, x, z), sl);
                        // "Green-dominant" is the visual judge's own test: G
                        // above R in the delivered, encoded pixel. It measured
                        // ZERO of them in the near band of four of six frames.
                        let d = vertex_color(y, w, x, z, sl);
                        if srgb(d[1]) > srgb(d[0]) {
                            green += 1;
                        }
                    }
                }
                dx += STEP;
            }
            dz += STEP;
        }
        if land >= 50 {
            shares.push(green as f32 / land as f32);
        }
    }

    assert!(
        shares.len() >= 8,
        "only {} of {BEARINGS} spawn bearings found land — the spawn selector \
         or the island moved, and this measurement is about neither",
        shares.len()
    );

    let lo = shares.iter().copied().fold(f32::MAX, f32::min);
    let hi = shares.iter().copied().fold(f32::MIN, f32::max);
    assert!(
        hi - lo > 0.5,
        "the near-ground green share no longer depends on which spawn bearing \
         the capture probe drew: every bearing is within {:.1} points \
         ({:.1}%–{:.1}% over {} bearings; measured 0.0%–85.3%).\n\
         That would make visual reports comparable run to run, which is the \
         outcome `NOW.md` §0gi asks for — but this file's account of why three \
         reports disagreed is then history, and its header should say so.",
        (hi - lo) * 100.0,
        lo * 100.0,
        hi * 100.0,
        shares.len()
    );
}
