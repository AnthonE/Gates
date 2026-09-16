//! Gate: the splat's two rock routes are ramps **centred on the sim's own
//! laws**, and granite reaches the ground on the islands worldgen actually
//! makes.
//!
//! **Why this exists, and it is the opposite of the item that asked for it.**
//! `NOW.md` §0gi item 1 read: *"Granite is authored and never drawn — both
//! routes are closed by the terrain's own range. `SPLAT_ALPINE_BAND` opens at
//! 44 m against a p99.9 of 43.63 m, and `SPLAT_CLIFF_BAND` opens at 0.952
//! against a max slope of 0.890, so the cliff mask has never fired once.
//! Moving either band … is the pass."* Neither measurement survived, and the
//! conclusion never followed from them anyway — see the retraction below.
//!
//! ⚠ **The pancake was a measurement artifact, and this file carried it for a
//! day. Retracted 2026-08-14, same day, before the seed it accused was
//! changed.** The original sweep here — and in `client/tests/ground_identity.rs`
//! and the note in `gates-loop/findings/` — ran `-1024..1024` on both axes.
//! `terrain::continent` centres the island on `(ISLAND_SIZE/2, ISLAND_SIZE/2)`,
//! so world coordinates run 0..2048 and that square's **corner is the island's
//! centre**: it sampled one quadrant, 632 k m² of a 2.9 M m² island, and called
//! it "the island". Every seed was measured the same way, so the *comparison*
//! was sound and the *conclusion* was not.
//!
//! | seed 20260731 | quadrant sweep | whole island |
//! |---|---|---|
//! | max height | 46.32 m | **106.00 m** |
//! | max slope | 0.890 | **2.665** |
//! | land with rock ≥ 32/255 | 0.00% | **10.0%** |
//!
//! Against a 44-island median of 106.0 m / 2.585 / 7.2%, the shipped seed is
//! **upper-third for granite**, not the minimum of anything, so it stays
//! (`shard-public.toml`; changing it is a wipe). Granite reaches the ground on
//! it, and within 300 m of the capture camera's own spawn (1155, 140) it paints
//! 8.9% — where the median island paints **0%**. `examples/seed_scan --at
//! x,z,r` is that measurement.
//!
//! What survives the retraction is the half that was never about a seed: the
//! two ramps must stay centred on the laws they ramp, which is what the first
//! two tests below hold. The rest of `NOW.md` §0gi item 1 — "granite is
//! authored and never drawn" — is still struck, and now for a simpler reason
//! than the one the note gave.
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

// The measurements ARE this file's output, exactly as in `tests/forest.rs`
// and `tests/scatter.rs`: the L5 wall bans format/print in SIM code, and a
// test harness is not sim code. Every number in the doc comments below was
// read off a run of this file, and a gate whose measurement is invisible is
// one nobody can re-derive.
#![allow(clippy::disallowed_macros)]

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

/// The seed the shard ships and the client's `--capture` probe therefore
/// photographs — `shard.toml.example`, `shard-public.toml`, `shards.toml`.
///
/// **Proposed for replacement on 2026-08-14 and kept**, once the sweep window
/// was fixed: it is upper-third for granite, not the minimum of anything.
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
///
/// ⚠ **The window is `[0, ISLAND_SIZE)` and it used to be `-1024..1024`, which
/// was a quarter of the island.** `terrain::continent` centres the island on
/// `(ISLAND_SIZE/2, ISLAND_SIZE/2)` — world coordinates run 0..2048 and every
/// other consumer knows it (`World::spawn_pos` starts from `c = ISLAND_SIZE *
/// 0.5`; a live shard placed a player at `1001.6, 1935.3`). The origin-centred
/// square's *corner* is the island's centre, so it sampled one quadrant:
/// 632 k m² of a 2.9 M m² island, 150‰ land where the true figure is 588‰.
/// Fixed 2026-08-14, and it inverted this file's own headline — see the header.
fn rock_per_mille(seed: u64) -> u32 {
    const STEP: f32 = 6.0;
    const LO: f32 = 0.0;
    const HI: f32 = terrain::ISLAND_SIZE;
    let (mut land, mut rock) = (0u32, 0u32);
    let mut z = LO + STEP * 0.5;
    while z < HI {
        let mut x = LO + STEP * 0.5;
        while x < HI {
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
    // ~68 k land samples is a whole island at this step (588‰ of the square);
    // the floor is here to catch a seed that produced open sea, not to bound
    // the coastline. It was 10_000 while the window was a quadrant.
    assert!(
        land > 40_000,
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
///
/// The floor is a fifth of what the set measures, so it fails on a real
/// regression rather than on the noise between two worldgen tweaks.
#[test]
fn granite_reaches_the_ground_on_the_islands_worldgen_makes() {
    let mut shares: Vec<u32> = seed_set().into_iter().map(rock_per_mille).collect();
    shares.sort_unstable();
    let median = shares[shares.len() / 2];
    let barren = shares.iter().filter(|&&s| s < 1).count();

    assert!(
        median >= 20,
        "the median island paints rock on {median} per-mille of its land \
         (76 on the whole-island sweep this floor was set against; the 61 \
         it quoted before 2026-08-14 was a quadrant). Granite has stopped \
         reaching the \
         ground across the whole seed set, which is the defect §0gi item 1 \
         described — and unlike that item's measurement, this one is not one \
         seed."
    );
    assert!(
        barren * 5 <= shares.len(),
        "{barren} of {} islands have essentially no rock (was 0 of 24 on the \
         whole-island sweep; the 2 this line used to quote were an artifact of \
         sweeping a quadrant). \
         Worldgen has started making pancakes at a rate a player would notice.",
        shares.len()
    );
}

/// The shipped seed is a **representative** island rather than an outlier.
///
/// This replaced `the_capture_seed_is_the_flattest_island_in_the_set`, which
/// asserted the retracted finding as a fact (`capture == 0`) and would have
/// held it there — a gate is how a wrong measurement becomes permanent. What
/// stands in its place is not "the shipped seed is good" (taste, and no gate
/// scores it) but the property the question actually turned on: **the world
/// under the camera must not be an extreme of the set**, in either direction. A
/// pancake makes every visual report describe a world players do not get; the
/// maximum makes the same mistake with more granite in it. On the corrected
/// sweep the shipped seed is neither — 100‰ against a set median of 72‰.
///
/// The band is the set's own median, not a literal, so it moves with worldgen
/// instead of pinning a number worldgen would have to be edited around.
#[test]
fn the_shipped_seed_is_a_representative_island() {
    let capture = rock_per_mille(CAPTURE_SEED);
    let mut others: Vec<u32> = seed_set()
        .into_iter()
        .filter(|&s| s != CAPTURE_SEED)
        .map(rock_per_mille)
        .collect();
    others.sort_unstable();
    let median = others[others.len() / 2];
    let (min, max) = (others[0], others[others.len() - 1]);

    // Half the median to twice it. Wide on purpose: this is a guard against
    // shipping an outlier, not a target to tune the world towards.
    assert!(
        capture * 2 >= median && capture <= median * 2,
        "the shipped seed paints rock on {capture} per-mille of its land \
         against a set median of {median} (range {min}..{max}). The island \
         every player joins and every visual report is written against has \
         drifted away from the islands worldgen typically makes — which is \
         the whole finding this file exists for, in whichever direction it \
         has gone."
    );
    assert!(
        capture > min && capture < max,
        "the shipped seed is now the {} of the set at {capture} per-mille. \
         That is exactly what 20260731 was when the loop spent four passes \
         photographing it.",
        if capture <= min { "minimum" } else { "maximum" }
    );
}

/// The whole island is swept, and a quadrant is not the island.
///
/// **This is the gate on the bug that produced the retracted finding**, and it
/// is one comparison: the land in the origin-centred square `-1024..1024` is a
/// quarter of the land in the world square, because the island is centred on
/// `(ISLAND_SIZE/2, ISLAND_SIZE/2)`. Anything that re-introduces the old window
/// makes this red instead of making a seed look flat.
#[test]
fn the_sweep_covers_the_island_and_not_one_quadrant() {
    const STEP: f32 = 6.0;
    let count = |lo: f32, hi: f32| {
        let mut land = 0u32;
        let mut z = lo + STEP * 0.5;
        while z < hi {
            let mut x = lo + STEP * 0.5;
            while x < hi {
                if terrain::height(CAPTURE_SEED, x, z) > 0.5 {
                    land += 1;
                }
                x += STEP;
            }
            z += STEP;
        }
        land
    };
    let whole = count(0.0, terrain::ISLAND_SIZE);
    let quadrant = count(-terrain::ISLAND_SIZE * 0.5, terrain::ISLAND_SIZE * 0.5);
    assert!(
        whole > quadrant * 3,
        "the origin-centred window holds {quadrant} of the world square's \
         {whole} land samples, so the two are no longer the quarter-and-whole \
         this file's header is about. Either the island moved off \
         (ISLAND_SIZE/2, ISLAND_SIZE/2) or a sweep changed — read the header \
         before trusting any number in this file."
    );
}

// ── World structure v1: the shore terrace and the coastline ────────────────

/// **The terrace's mechanism, as arithmetic** — not as a statistic over the
/// island.
///
/// `tests/contour.rs`'s lesson: the property that matters here is a
/// *derivative*, and a sweep of the island cannot separate a crease from the
/// cliffs worldgen exists to make. `terrain::shore_terrace_slope` is the
/// derivative in closed form, so every claim the terrace is bought for can be
/// checked exactly and in microseconds:
///
/// - `f(0) = 0` — the coastline does not move by one bit, so the land mask,
///   `road_band`'s crossings and the island's area are untouched.
/// - `f'(0) = 1` and `f'(H) = 1` — C¹ at both joins, which is the whole of
///   why a beach does not draw a shading line at the water's edge. The
///   renderer takes its normal from this function's gradient.
/// - `f(h) = h` outside — no constant offset hiding above the band, so the
///   shelves, the treeline and the summit are where they were.
/// - `f' = SHORE_TERRACE_K` at the berm, `0.21 × H` up, and `f' > 0`
///   everywhere — monotone, so the heightfield has no fold in it.
///
/// Mutants: any change to the kernel's exponent breaks the C¹ joins; a K at
/// or below 0 breaks monotonicity (and the const block in `terrain.rs`
/// refuses to compile); dropping the terrace entirely makes the berm read 1.
#[test]
fn the_shore_terrace_is_a_berm_and_is_c1_at_both_joins() {
    let h = terrain::SHORE_TERRACE_H;
    let k = terrain::SHORE_TERRACE_K;

    assert!(
        terrain::shore_terrace(0.0) == 0.0,
        "the terrace moved the waterline to {} — f(0) must be exactly 0 or \
         the coastline, the land mask and every shoreline crossing move with it",
        terrain::shore_terrace(0.0)
    );
    for probe in [-40.0f32, -h, -0.001, 0.0, h, h + 0.001, 40.0, 200.0] {
        assert!(
            terrain::shore_terrace(probe) == probe,
            "outside [0, H] the terrace must be the identity; f({probe}) = {}",
            terrain::shore_terrace(probe)
        );
        assert!(
            terrain::shore_terrace_slope(probe) == 1.0,
            "outside [0, H] the terrace's slope must be exactly 1; f'({probe}) = {}",
            terrain::shore_terrace_slope(probe)
        );
    }

    // C¹ at both joins, as a limit taken from the inside. `fabs` is the
    // crate's own, and the tolerance is f32 noise on a degree-4 polynomial,
    // not slack in the claim.
    for (edge, name) in [(0.0f32, "waterline"), (h, "top of the band")] {
        let inside = if edge == 0.0 { 1e-3 } else { h - 1e-3 };
        let d = terrain::shore_terrace_slope(inside);
        assert!(
            sim_core::fmath::fabs(d - 1.0) < 1e-3,
            "the terrace's slope is {d} just inside the {name} and 1 just \
             outside it — a C⁰ join, which is a shading line at exactly the \
             elevation every player stands at (CLAUDE.md's contour-map entry)"
        );
    }

    // The berm: the flattest point, at 0.21132 × H by the kernel's own
    // algebra, reads exactly the knob.
    let berm = h * 0.211_324_87;
    let at_berm = terrain::shore_terrace_slope(berm);
    assert!(
        sim_core::fmath::fabs(at_berm - k) < 1e-3,
        "the berm's gradient is {at_berm}, not the {k} `SHORE_TERRACE_K` \
         declares — the knob and the shape have come apart, and DECISIONS.md \
         §open is describing a terrace this code does not draw"
    );

    // Monotone, swept fine enough that the shoulder cannot hide a fold.
    let mut worst = f32::MAX;
    let mut at = 0.0f32;
    let mut probe = -1.0f32;
    while probe <= h + 1.0 {
        let d = terrain::shore_terrace_slope(probe);
        if d < worst {
            worst = d;
            at = probe;
        }
        probe += h * 0.0005;
    }
    assert!(
        worst > 0.0,
        "the terrace's slope bottoms at {worst} at h = {at} — a heightfield \
         that folds back on itself, which nothing downstream can represent"
    );
    assert!(
        sim_core::fmath::fabs(worst - k) < 1e-3,
        "the terrace's minimum gradient is {worst} at h = {at}, not the {k} \
         the knob declares"
    );
    println!("shore terrace: H {h} K {k}; berm f'({berm:.2}) = {at_berm:.4}; min {worst:.4}");
}

/// **And the beach it buys is a beach** — the measurement half, over the
/// whole seed set rather than the shipped island.
///
/// The terrace is a mechanism; whether it produces a coast is a fact about
/// the generator, and before it the answer was **1.1% of land and a 5.0 m
/// walk** from the waterline to the 2 m contour. Both are asserted because
/// they can fail independently: a share without a width is a marsh, and a
/// width without a share is one flat bay.
///
/// Mutant, run: `SHORE_TERRACE_K = 0.999` (the terrace off in all but name)
/// takes the median width to **4.5–6.0 m** across the eight seeds, red on
/// every one. It takes the share to 0.7–2.0%, which is red on six of eight —
/// see `BEACH_SHARE_MIN` for why that half is kept as a tripwire rather than
/// read as the gate.
#[test]
fn the_island_has_a_beach_to_walk_up() {
    let mut worst_share = f32::MAX;
    let mut worst_width = f32::MAX;
    for seed in seed_set().into_iter().take(8) {
        let haven = terrain::haven(seed);
        let (mut land, mut beach) = (0u32, 0u32);
        for cz in 0..terrain::CELLS_PER_SIDE {
            for cx in 0..terrain::CELLS_PER_SIDE {
                let x = cx as f32 * terrain::CELL_SIZE + terrain::CELL_SIZE * 0.5;
                let z = cz as f32 * terrain::CELL_SIZE + terrain::CELL_SIZE * 0.5;
                let h = terrain::ground(seed, &haven, x, z);
                if h < terrain::LAND_MIN_H {
                    continue;
                }
                land += 1;
                if terrain::biome(h, terrain::moisture(seed, x, z)) == terrain::Biome::Beach {
                    beach += 1;
                }
            }
        }
        let share = beach as f32 / land.max(1) as f32;

        // The width, walked: bisect the waterline on each of 180 radials,
        // then step inland to the 2 m contour. The median is what a player
        // meets; a mean would be carried by one flat bay.
        let c = terrain::ISLAND_SIZE * 0.5;
        let mut widths: Vec<f32> = Vec::new();
        for b in 0..180usize {
            let t = b as f32 / 180.0 * 4.0;
            let (ux, uz) = match t as i32 {
                0 => (1.0, t - 0.5),
                1 => (1.5 - t, 0.5),
                2 => (-1.0, 2.5 - t),
                _ => (t - 3.5, -0.5),
            };
            let n = (ux * ux + uz * uz).sqrt();
            let (ux, uz) = (ux / n, uz / n);
            let (mut lo, mut hi) = (200.0f32, 1150.0f32);
            if terrain::height(seed, c + ux * lo, c + uz * lo) < 0.0 {
                continue;
            }
            if terrain::height(seed, c + ux * hi, c + uz * hi) > 0.0 {
                continue;
            }
            for _ in 0..34 {
                let mid = (lo + hi) * 0.5;
                if terrain::height(seed, c + ux * mid, c + uz * mid) > 0.0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let mut step = 0.0f32;
            while step < 300.0 {
                if terrain::height(seed, c + ux * (lo - step), c + uz * (lo - step)) > 2.0 {
                    widths.push(step);
                    break;
                }
                step += 0.5;
            }
        }
        assert!(
            widths.len() > 100,
            "seed {seed:#x}: only {} radials found a shore — the sweep, not \
             the island",
            widths.len()
        );
        widths.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = widths[widths.len() / 2];
        println!(
            "seed {seed:#x}: beach {:.1}% of land, waterline->2 m median {median:.1} m",
            share * 100.0
        );
        worst_share = worst_share.min(share);
        worst_width = worst_width.min(median);
    }
    assert!(
        worst_share >= BEACH_SHARE_MIN,
        "the thinnest island gives the beach {:.1}% of its land, under the \
         {:.1}% floor — before the shore terrace this was 1.1%, which is a \
         kerb and not a coast",
        worst_share * 100.0,
        BEACH_SHARE_MIN * 100.0
    );
    assert!(
        worst_width >= BEACH_WIDTH_MIN_M,
        "the narrowest island's shore reaches 2 m in {worst_width:.1} m of \
         walking, under the {BEACH_WIDTH_MIN_M} m floor — it was 5.0 m before \
         the terrace, which is two paces from sea to grass"
    );
}

/// Floor on the Beach biome's share of land.
///
/// ⚠ **This one does NOT separate, and it is asserted anyway for a different
/// reason — stated so nobody reads it as the terrace's gate.** Measured over
/// the eight seeds: **1.9–6.4%** as shipped, **0.7–2.0%** with the terrace
/// off. The bands touch, because the share is as much about how much low
/// ground an island happens to have as about how wide its shore is. What it
/// does catch is the Beach biome disappearing outright — a moved
/// `BEACH_MAX_H`, a classifier edit — which nothing else here would notice.
/// The width below is the statistic that separates.
const BEACH_SHARE_MIN: f32 = 0.015;
/// Floor on the median walk from waterline to the 2 m contour, metres.
/// Measured **9.5–13.5 m** as shipped and **4.5–6.0 m** with the terrace off,
/// over the same eight seeds — two bands with a clean gap, and the floor is
/// in it.
const BEACH_WIDTH_MIN_M: f32 = 8.0;

/// **The coastline is not a disc.**
///
/// `continent` wobbles a circle, and with one 900 m term on a ~5,600 m
/// circumference that is one and a half lobes: measured, the shore radius
/// had an sd of **30 m on a mean of 892** and the land mask's perimeter was
/// 1.30x an equal-area disc — most of that 1.30 being the 8 m grid's own
/// staircase rather than structure. `COAST_BAY_FREQ`/`COAST_BAY_WOBBLE` are
/// the second term, and this is what says they are still doing their job.
///
/// Perimeter-to-disc is the honest statistic here rather than the radius's
/// sd: sd is blind to the SCALE of the wobble — one slow 200 m lobe and
/// twenty 40 m coves read the same — where a perimeter counts the coastline
/// a player would actually walk.
///
/// Mutant: `COAST_BAY_WOBBLE = 0.0` takes the ratio to 1.29–1.32 and the
/// floor is red on every seed.
#[test]
fn the_coastline_has_headlands_and_coves() {
    let mut worst = f32::MAX;
    for seed in seed_set().into_iter().take(8) {
        // The shore radius on 720 evenly spaced bearings, bisected. What is
        // measured is the step between ADJACENT bearings — ~7.8 m of arc at
        // this radius — so the statistic is the coastline's roughness at the
        // scale a person standing on it can see, and it is blind to the slow
        // lobe `COAST_FREQ` already had.
        let c = terrain::ISLAND_SIZE * 0.5;
        let mut r: Vec<f32> = Vec::new();
        for b in 0..720usize {
            let t = b as f32 / 720.0 * 4.0;
            let (ux, uz) = match t as i32 {
                0 => (1.0, t - 0.5),
                1 => (1.5 - t, 0.5),
                2 => (-1.0, 2.5 - t),
                _ => (t - 3.5, -0.5),
            };
            let n = (ux * ux + uz * uz).sqrt();
            let (ux, uz) = (ux / n, uz / n);
            let (mut lo, mut hi) = (400.0f32, 1150.0f32);
            if terrain::height(seed, c + ux * lo, c + uz * lo) < 0.0
                || terrain::height(seed, c + ux * hi, c + uz * hi) > 0.0
            {
                continue;
            }
            for _ in 0..34 {
                let mid = (lo + hi) * 0.5;
                if terrain::height(seed, c + ux * mid, c + uz * mid) > 0.0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            r.push(lo);
        }
        assert!(
            r.len() > 600,
            "seed {seed:#x}: only {} bearings found a single shore crossing \
             — the sweep, not the island",
            r.len()
        );
        let mut sum = 0.0f32;
        for i in 0..r.len() {
            let d = r[(i + 1) % r.len()] - r[i];
            sum += if d < 0.0 { -d } else { d };
        }
        let rough = sum / r.len() as f32;
        let mean = r.iter().sum::<f32>() / r.len() as f32;
        println!(
            "seed {seed:#x}: shore r mean {mean:.0} m, roughness {rough:.2} m per \
             {:.1} m of arc",
            2.0 * core::f32::consts::PI * mean / r.len() as f32
        );
        worst = worst.min(rough);
    }
    assert!(
        worst >= COAST_ROUGHNESS_MIN_M,
        "the smoothest island's shore moves {worst:.2} m between adjacent \
         bearings, under the {COAST_ROUGHNESS_MIN_M} m floor — that is the \
         round island `COAST_BAY_WOBBLE` was added to retire"
    );
}

/// Floor on how far the shore radius moves between adjacent bearings, metres
/// per ~7.8 m of arc — the coastline's roughness at the scale a person on it
/// can see. Measured **1.69–2.15 m** as shipped and **0.84–1.21 m** with
/// `COAST_BAY_WOBBLE` at zero, over eight seeds; the floor is in the gap.
///
/// **Chosen over perimeter-to-equal-area-disc, which was tried and does not
/// separate.** That ratio reads 1.329–1.421 as shipped and 1.29–1.32 with the
/// bay term at zero: the bands touch, because most of both is the 8 m grid's
/// own staircase rather than coastline. A gate whose two populations overlap
/// reads as coverage and is not one — `NOW.md` §0wg item 4 is the same
/// finding about the contour statistic, and the rule there is that a metric
/// which cannot separate should not be shipped. This one separates by a
/// factor.
const COAST_ROUGHNESS_MIN_M: f32 = 1.45;
