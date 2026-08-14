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
