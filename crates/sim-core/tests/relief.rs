//! Gate: the splat's two rock routes are ramps **centred on the sim's own
//! laws**, and granite reaches the ground on the islands worldgen actually
//! makes.
//!
//! **Why this exists, and it is the opposite of the item that asked for it.**
//! `NOW.md` §0gi item 1 read: *"Granite is authored and never drawn — both
//! routes are closed by the terrain's own range. `SPLAT_ALPINE_BAND` opens at
//! 44 m against a p99.9 of 43.63 m, and `SPLAT_CLIFF_BAND` opens at 0.952
//! against a max slope of 0.890, so the cliff mask has never fired once.
//! Moving either band … is the pass."* Both measurements are correct and the
//! conclusion does not follow, because **both were taken on one seed**.
//!
//! Measured across 40 seeds at 4 m (the numbers are in
//! `gates-loop/findings/note-20260814-granite-and-the-flattest-island.md`):
//!
//! | | capture seed 20260731 | median of 40 | max of 40 |
//! |---|---|---|---|
//! | max height | **46.32 m** | 93.89 m | 106.00 m |
//! | max slope | **0.890** | 2.203 | 2.870 |
//! | land with rock ≥ 32/255 | **0.00%** | 6.11% | 23.86% |
//!
//! The capture seed is the **minimum on all three axes of the forty**. Granite
//! reaches the ground on 38 of 40 islands; it does not reach it on the one the
//! visual judge photographs. `splat_from` is not the defect — the world under
//! the camera is a 1-in-40 pancake, and every visual report this loop has
//! written was written against it.
//!
//! So the bands must NOT move, and this file is the gate that says why. They
//! are not free numbers: `DECISIONS.md` §open materials v0 authored them as
//! *"soft ramps **centred on `sim-core` `biome()`'s own hard edges**: … alpine
//! 44–60 m (edge 52) … cliff 0.8–1.2 × tan 50°"*, and `TERRAIN.md` §7.1 says
//! *"the blend math mirrors the biome function. Cliff mask forces rock."*
//! Three documents state the relationship and **nothing checked it**, which is
//! `CLAUDE.md`'s own named failure — a claim that reads as enforced while it
//! drifts. A pass chasing granite would have moved one band, left the law it
//! ramps where it was, and stayed green.
//!
//! Every assertion here is counted or structural, so it is worth the same on
//! this box as on the reference VPS.

use sim_core::terrain::{self, Biome, CLIFF_SLOPE_RATIO};

/// The rock channel of the splat — `splat_from`'s fourth byte.
const ROCK: usize = 3;

/// Bisect a monotone predicate: `pred(lo)` false, `pred(hi)` true.
fn bisect(lo: f32, hi: f32, pred: impl Fn(f32) -> bool) -> f32 {
    let (mut a, mut b) = (lo, hi);
    for _ in 0..64 {
        let m = 0.5 * (a + b);
        if pred(m) {
            b = m;
        } else {
            a = m;
        }
    }
    0.5 * (a + b)
}

/// The rock weight on ground that is neither beach nor summit, at a slope —
/// so the only channel moving is the cliff mask.
fn rock_at_slope(slope: f32) -> u8 {
    terrain::splat_from(20.0, 0.0, slope)[ROCK]
}

/// The rock weight on flat, dry ground at a height — so the only channel
/// moving is the alpine ramp.
fn rock_at_height(h: f32) -> u8 {
    terrain::splat_from(h, 0.0, 0.0)[ROCK]
}

/// Where a ramp's byte output leaves 0 and where it reaches 255.
///
/// Byte rounding moves each end inward by the same amount — `round(255·s)`
/// leaves 0 at `s = 1/510` and reaches 255 at `s = 1 − 1/510`, and smoothstep
/// is odd-symmetric about its own midpoint — so the two measured ends are
/// displaced symmetrically and **their midpoint is the band's true centre**.
/// That is the only property either test below reads.
fn ramp_ends(lo: f32, hi: f32, f: impl Fn(f32) -> u8) -> (f32, f32) {
    (
        bisect(lo, hi, |v| f(v) > 0),
        bisect(lo, hi, |v| f(v) == 255),
    )
}

/// The cliff ramp is centred on the threshold the *collision* law uses.
///
/// `CLIFF_SLOPE_RATIO` is what `movement.rs` refuses to walk up and what
/// `scatter` refuses to stand on. `TERRAIN.md` §1 stage 5 calls the same
/// number "unclimbable, unbuildable, **distinct material**" — one threshold
/// wearing three hats, and this is the hat nothing was holding on.
#[test]
fn the_cliff_ramp_is_centred_on_the_collision_threshold() {
    assert_eq!(rock_at_slope(0.0), 0, "flat ground is not rock");
    assert_eq!(rock_at_slope(3.0), 255, "a vertical face is all rock");

    let (open, full) = ramp_ends(0.0, 3.0, rock_at_slope);
    let centre = 0.5 * (open + full);

    // The shading constant is deliberately two decimals (`SPLAT_CLIFF` 1.19)
    // where the collision constant is seven (1.191_753_6) — terrain.rs says so
    // in place, because promoting it would move every ground pixel. So the
    // tolerance is the precision the shading band carries, not equality. It is
    // written as an interval because `f32::abs` is not one of wall 1's
    // permitted float operations, and a test in this crate obeys the wall.
    const TOL: f32 = 0.01;
    assert!(
        centre > CLIFF_SLOPE_RATIO - TOL && centre < CLIFF_SLOPE_RATIO + TOL,
        "the cliff ramp is centred on {centre:.4} but the sim's cliff threshold \
         is {CLIFF_SLOPE_RATIO:.4}. The ramp and the law it ramps have come \
         apart: ground the player cannot walk up no longer reads as the \
         material that says so. If the island needs more granite, that is \
         worldgen's relief — not this band (see this file's header)."
    );
}

/// The alpine ramp is centred on `biome()`'s own Highland edge.
///
/// `scatter_row` blends the four `ScatterTable` rows by these same weights and
/// `Biome`'s four identities are the splat's four channels in order, so the
/// ramp's centre and the classifier's edge disagreeing would put the Highland
/// *props* on different ground than the Highland *surface*.
#[test]
fn the_alpine_ramp_is_centred_on_the_biome_classifier_edge() {
    assert_eq!(rock_at_height(4.0), 0, "low ground is not alpine");
    assert_eq!(rock_at_height(70.0), 255, "a summit is all rock");

    let (open, full) = ramp_ends(4.0, 70.0, rock_at_height);
    let centre = 0.5 * (open + full);

    // The classifier's edge, measured rather than quoted — a literal copied
    // out of `biome()` into this file would be a second copy of the number the
    // test exists to hold equal.
    let edge = bisect(4.0, 70.0, |h| terrain::biome(h, 0.0) == Biome::Highland);

    const TOL: f32 = 0.25;
    assert!(
        centre > edge - TOL && centre < edge + TOL,
        "the alpine ramp is centred on {centre:.3} m but `biome()` turns \
         Highland at {edge:.3} m. The surface and the scatter table would \
         disagree about where the highland is."
    );
}

// --- What the island actually delivers ------------------------------------

/// The seed the client's `--capture` probe photographs, and every visual
/// report this loop has written.
const CAPTURE_SEED: u64 = 20260731;

/// A fixed, arbitrary spread of islands. Golden-adjacent seeds are included by
/// name so the set is not quietly all one family.
fn seed_set() -> Vec<u64> {
    let mut s = vec![CAPTURE_SEED, 0x0047_4154_4553, 0x1, 0xDEAD_BEEF];
    for i in 1..=20u64 {
        s.push(i.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    }
    s
}

/// Share of an island's land carrying a legible rock weight, in per-mille.
fn rock_per_mille(seed: u64) -> u32 {
    const STEP: f32 = 6.0;
    const HALF: f32 = 1024.0;
    let (mut land, mut rock) = (0u32, 0u32);
    let mut z = -HALF + STEP * 0.5;
    while z < HALF {
        let mut x = -HALF + STEP * 0.5;
        while x < HALF {
            if terrain::height(seed, x, z) > 0.5 {
                land += 1;
                // 32/255 is where a channel stops being rounding and starts
                // being a visible share of the blend.
                if terrain::splat(seed, x, z)[ROCK] >= 32 {
                    rock += 1;
                }
            }
            x += STEP;
        }
        z += STEP;
    }
    // ~17.5 k land samples is a whole island at this step; the floor is here
    // to catch a seed that produced open sea, not to bound the coastline.
    assert!(
        land > 10_000,
        "seed {seed:#x} produced no island: {land} samples"
    );
    (1000 * rock as u64 / land as u64) as u32
}

/// Granite reaches the ground on the islands worldgen makes — the assertion
/// `NOW.md` §0gi item 1 assumed was false.
///
/// This is the whole refutation, and it is one number: the typical island
/// paints a legible rock weight on a real share of its land. If this ever goes
/// red, `splat_from` really has stopped delivering granite and the item is
/// live again — but it has to be red *here*, across the set, not on one seed.
#[test]
fn granite_reaches_the_ground_on_the_islands_worldgen_makes() {
    let mut shares: Vec<u32> = seed_set().into_iter().map(rock_per_mille).collect();
    shares.sort_unstable();
    let median = shares[shares.len() / 2];
    let barren = shares.iter().filter(|&&s| s < 1).count();

    assert!(
        median >= 20,
        "the median island paints rock on {median} per-mille of its land \
         (was 61 when this gate landed). Granite has stopped reaching the \
         ground across the whole seed set, which is the defect §0gi item 1 \
         described — and unlike that item's measurement, this one is not one \
         seed."
    );
    assert!(
        barren * 5 <= shares.len(),
        "{barren} of {} islands have essentially no rock (was 2 of 24). \
         Worldgen has started making pancakes at a rate a player would notice.",
        shares.len()
    );
}

/// The capture seed is the flattest island in the set — the reason the visual
/// judge has never seen granite, stated as a measurement rather than a story.
///
/// **This test asserts the mechanism, not a defect to be fixed here.** Which
/// island the probe photographs is not a builder's call: it decides what every
/// visual report is about. `DECISIONS.md` §open carries the question. When it
/// is answered — a different capture seed, or worldgen with less relief
/// variance — this goes red with the new numbers in hand, which is the point.
#[test]
fn the_capture_seed_is_the_flattest_island_in_the_set() {
    let seeds = seed_set();
    let capture = rock_per_mille(CAPTURE_SEED);
    let mut others: Vec<u32> = seeds
        .iter()
        .copied()
        .filter(|&s| s != CAPTURE_SEED)
        .map(rock_per_mille)
        .collect();
    others.sort_unstable();
    let median = others[others.len() / 2];

    assert_eq!(
        capture, 0,
        "the capture seed now paints rock on {capture} per-mille of its land \
         (was 0). The island under the camera has changed and every visual \
         report older than this commit was written about a different world."
    );
    assert!(
        median >= 20,
        "the rest of the set has fallen to {median} per-mille, so the capture \
         seed is no longer the outlier this test names — re-read the header."
    );
}
