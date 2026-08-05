//! The scatter field as arithmetic: does the forest cluster, and did making
//! it cluster cost any density (TERRAIN.md §1 stage 9, `reference/SPAWN.md`
//! §9.3).
//!
//! Same standard as `tests/road.rs` and `tests/haven.rs` — counts and
//! distributions that fail in seconds, re-derived from the public surface.
//! What is different here is that the thing under test is *statistical*, so
//! the suite carries its own null: a per-cell independent draw at the
//! measured density is binomial, and a binomial's variance and empty share
//! are closed forms. Nothing is compared against a remembered number from a
//! previous build, and no control run is needed — the null is arithmetic.
//!
//! The measurement that motivated the field is in the same units as the one
//! that gates it. Before `clump` existed, on the same windows:
//!
//!   seed 0      dispersion 1.051   empty 0.0003 (null 0.0006)
//!   seed 1      dispersion 1.027   empty 0.0007 (null 0.0008)
//!   seed 7      dispersion 0.978   empty 0.0003 (null 0.0006)
//!
//! That is white noise, to three decimal places, and a forest with no
//! clearings in it — `SPAWN.md` §9.3's "orchard", measured. The thresholds
//! below sit far above that band on purpose: a change that quietly reverted
//! the field would not squeak past them, it would fall through the floor.

// The measurements ARE the gate's output — same reasoning and same allow as
// `tests/haven.rs`: the L5 wall bans format/print in SIM code, and a test
// harness is not sim code. The float walls are NOT relaxed here: `powf`,
// `f64::sqrt` and `abs` stay banned, so the binomial null is an integer
// power by repeated multiply and every magnitude is `max` of a signed pair.
#![allow(clippy::disallowed_macros)]

use sim_core::terrain::{
    self, Biome, Occupant, ScatterTable, CELLS_PER_SIDE, CELL_SIZE, HAVEN_CRATES, LAND_MIN_H,
};

/// Seeds every statistical check runs over. More than one because a single
/// seed's island can be atypical — seed 7 carries a third less forest than
/// seed 1 — and a threshold that only holds on the shipped seed is a pin,
/// not a gate.
const SEEDS: [u64; 4] = [0, 1, 7, 12345];

/// Window half-width in cells. 5x5 cells is 40 m, which is the range
/// `TERRAIN.md` §1 stage 6 is talking about when it asks forest for "cover,
/// low visibility": the question is whether a player standing here can see
/// out, and that is decided within about forty meters.
const R: i32 = 2;
const WIN_CELLS: i32 = (2 * R + 1) * (2 * R + 1);

/// One island's tree field, plus the forest mask the windows are read on.
struct Field {
    tree: Vec<bool>,
    forest: Vec<bool>,
    counts: [u32; 11],
}

fn build(seed: u64) -> Field {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);
    let n = (CELLS_PER_SIDE * CELLS_PER_SIDE) as usize;
    let mut f = Field {
        tree: vec![false; n],
        forest: vec![false; n],
        // Indexed by `Occupant as usize`, which skips 8: sized to the
        // largest discriminant + 1, not to the number of variants.
        counts: [0u32; 11],
    };
    for cz in 0..CELLS_PER_SIDE {
        for cx in 0..CELLS_PER_SIDE {
            let slot = terrain::scatter(seed, &table, &haven, cx, cz);
            f.counts[slot.occupant as usize] += 1;
            let i = (cz * CELLS_PER_SIDE + cx) as usize;
            f.tree[i] = slot.occupant == Occupant::Tree;
            let x = cx as f32 * CELL_SIZE + CELL_SIZE * 0.5;
            let z = cz as f32 * CELL_SIZE + CELL_SIZE * 0.5;
            let h = terrain::height(seed, x, z);
            f.forest[i] = h >= LAND_MIN_H
                && terrain::biome(h, terrain::moisture(seed, x, z)) == Biome::Forest;
        }
    }
    f
}

/// Tree counts in every window that lies wholly inside the forest biome.
///
/// Conditioning on one biome is the whole reason this measurement means
/// anything. Measured over all land instead, the dispersion of the *old*,
/// provably independent field read 2.7 — not because it clustered but
/// because a meadow draws trees at 70 per-mille and a forest at 260, and a
/// statistic that cannot tell biome structure from clumping would have
/// scored the orchard as a forest. Inside one biome the per-cell
/// probability is near-constant, so what is left is clumping.
fn forest_windows(f: &Field) -> Vec<u32> {
    let mut out = Vec::new();
    for cz in R..CELLS_PER_SIDE - R {
        for cx in R..CELLS_PER_SIDE - R {
            let (mut c, mut all_forest) = (0u32, true);
            for wz in cz - R..=cz + R {
                for wx in cx - R..=cx + R {
                    let j = (wz * CELLS_PER_SIDE + wx) as usize;
                    all_forest &= f.forest[j];
                    c += u32::from(f.tree[j]);
                }
            }
            if all_forest {
                out.push(c);
            }
        }
    }
    out
}

fn mean_var(xs: &[u32]) -> (f64, f64) {
    let n = xs.len() as f64;
    let sum: f64 = xs.iter().map(|&c| f64::from(c)).sum();
    let sumsq: f64 = xs.iter().map(|&c| f64::from(c) * f64::from(c)).sum();
    let mean = sum / n;
    (mean, sumsq / n - mean * mean)
}

/// (1−p)^WIN_CELLS — the share of windows an independent draw at the same
/// density leaves empty. Repeated multiply because `powf` is walled.
fn null_empty_share(p: f64) -> f64 {
    let mut e = 1.0f64;
    for _ in 0..WIN_CELLS {
        e *= 1.0 - p;
    }
    e
}

/// The forest clusters, against a null that is computed rather than
/// remembered.
///
/// Two independent readings of the same field, because one alone is
/// gameable. Dispersion says the counts are overspread; the empty share
/// says the low tail is actually reaching zero. A field that only widened
/// the distribution symmetrically would pass the first and fail the second,
/// and that field is not a forest with clearings in it.
#[test]
fn test_scatter_clusters() {
    for seed in SEEDS {
        let f = build(seed);
        let w = forest_windows(&f);

        // Non-vacuity first: a statistic over three windows is not a gate,
        // and a mask bug that emptied the sample would otherwise make every
        // assertion below pass by matching nothing.
        assert!(
            w.len() >= 5_000,
            "seed {seed}: only {} all-forest windows — the sample is too small \
             for the statistics below to mean anything (expected >= 5,000)",
            w.len()
        );

        let (mean, var) = mean_var(&w);
        let p = mean / f64::from(WIN_CELLS);
        let null_var = f64::from(WIN_CELLS) * p * (1.0 - p);
        let dispersion = var / null_var;

        let empty = w.iter().filter(|&&c| c == 0).count() as f64 / w.len() as f64;
        let null_empty = null_empty_share(p);

        println!(
            "seed {seed}: {} windows, mean {mean:.3} var {var:.3} \
             dispersion {dispersion:.3} (null 1.000), empty {empty:.4} \
             (null {null_empty:.5}, ratio {:.1}x)",
            w.len(),
            empty / null_empty
        );

        // Measured 2.90–3.34 over these four seeds; the pre-`clump` field
        // read 0.98–1.05. 2.0 sits in the empty middle of that gap, so
        // neither a small regression nor a small improvement moves it, but
        // losing the field entirely does.
        assert!(
            dispersion >= 2.0,
            "seed {seed}: tree-count dispersion in a 40 m forest window is \
             {dispersion:.3}, and an independent per-cell draw is 1.000. \
             Below 2.0 the forest is back to the orchard `SPAWN.md` §9.3 \
             names — check `terrain::clump` still scales the biome row."
        );

        // Clearings, the half dispersion cannot see. Measured 37–52x the
        // null here, against 0.5–1.2x before the field existed.
        assert!(
            empty >= null_empty * 10.0,
            "seed {seed}: {empty:.4} of forest windows are empty against a \
             binomial null of {null_empty:.5} — under 10x, the low tail is \
             not reaching zero, so there are no clearings inside the forest."
        );
    }
}

/// The field redistributes density; it does not spend it.
///
/// `TERRAIN.md` §6's live-slot band is a budget the whole world shares, and
/// a texture change that quietly took a third of the trees would still look
/// better in a screenshot while costing every downstream number — gather
/// rates, the snapshot budget, the fleet triangle count. This is the assert
/// that makes `CLUMP_NORM` a derivation instead of a preference.
#[test]
fn test_scatter_density_preserved() {
    for seed in SEEDS {
        let f = build(seed);
        let live: u32 = f.counts[1..].iter().sum();
        println!(
            "seed {seed}: live {live} (tree {} bush {} rock {} stone {} barrel {} crate {})",
            f.counts[Occupant::Tree as usize],
            f.counts[Occupant::Bush as usize],
            f.counts[Occupant::Rock as usize],
            f.counts[Occupant::StoneNode as usize],
            f.counts[Occupant::BarrelSlot as usize],
            f.counts[Occupant::CrateSlot as usize],
        );
        // TERRAIN.md §6. `tests/terrain_golden.rs` holds the same band on
        // the shipped seed; this holds it on four, because a field with a
        // seed-dependent mean would pass there and fail in play.
        assert!(
            (8_000..=12_000).contains(&live),
            "seed {seed}: {live} live slots is outside TERRAIN.md §6's \
             8,000–12,000 — the clump field is meant to move density around, \
             not change how much there is (check `CLUMP_NORM`)."
        );
        assert!(
            f.counts[Occupant::Tree as usize] > 1_000,
            "seed {seed}: only {} trees",
            f.counts[Occupant::Tree as usize]
        );
    }
}

/// `CLUMP_NORM` is re-derived here rather than trusted, on the same grid the
/// constant's doc comment claims it was measured on.
///
/// The constant is the reciprocal of the field's island mean, which makes it
/// exactly the kind of number that is right when written and wrong two
/// passes later — change the frequency, the gain or the floor and the mean
/// moves, silently, taking the world's density with it. So the gate does the
/// division again.
#[test]
fn test_clump_normalizer_holds() {
    for seed in SEEDS {
        let (mut sum, mut sumsq) = (0f64, 0f64);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for gz in 0..CELLS_PER_SIDE {
            for gx in 0..CELLS_PER_SIDE {
                let g = terrain::clump(
                    seed,
                    gx as f32 * CELL_SIZE + CELL_SIZE * 0.5,
                    gz as f32 * CELL_SIZE + CELL_SIZE * 0.5,
                );
                sum += f64::from(g);
                sumsq += f64::from(g) * f64::from(g);
                lo = lo.min(g);
                hi = hi.max(g);
            }
        }
        let n = f64::from(CELLS_PER_SIDE) * f64::from(CELLS_PER_SIDE);
        let mean = sum / n;
        let sd = ((sumsq / n - mean * mean) as f32).sqrt();
        println!("seed {seed}: clump mean {mean:.4} sd {sd:.4} range {lo:.3}..{hi:.3}");

        // Measured 0.9968–1.0082. The tolerance is the field's own
        // seed-to-seed wobble, not slack: one constant cannot centre four
        // different islands exactly.
        assert!(
            (0.97..=1.03).contains(&mean),
            "seed {seed}: the clump field means {mean:.4}, so it multiplies \
             the world's density by that. `CLUMP_NORM` is the reciprocal of \
             this mean and has drifted — re-derive it."
        );
        // Non-vacuity, and the whole point: a `clump` that returned the
        // constant 1.0 would pass the mean check above and change nothing.
        assert!(
            sd >= 0.4,
            "seed {seed}: clump sd {sd:.4} — the field is nearly flat, so it \
             is not making groves. Density would be preserved and the world \
             would be white noise again."
        );
        assert!(lo >= 0.0, "seed {seed}: clump went negative at {lo}");
    }
}

/// The field cannot silently clip the density it is supposed to preserve.
///
/// The row is scaled by up to `max(clump)` and the draw compares a
/// per-mille roll against the running total, so once a scaled row reaches
/// 1,000 the tail entries stop being reachable and the extra weight
/// evaporates — density would fall while every mean above still read 1.0.
/// Nothing else in the suite can see that, because it looks exactly like a
/// slightly thinner forest.
#[test]
fn test_no_biome_row_saturates() {
    let table = ScatterTable::alpha_default();
    // The field's ceiling, taken from the field rather than from the
    // constant, so a change to the shaping is caught too.
    let mut hi = 0f32;
    for seed in SEEDS {
        for gz in 0..CELLS_PER_SIDE {
            for gx in 0..CELLS_PER_SIDE {
                hi = hi.max(terrain::clump(
                    seed,
                    gx as f32 * CELL_SIZE + CELL_SIZE * 0.5,
                    gz as f32 * CELL_SIZE + CELL_SIZE * 0.5,
                ));
            }
        }
    }
    for (b, row) in table.weights.iter().enumerate() {
        let total: u32 = row.iter().map(|&w| u32::from(w)).sum();
        let scaled = total as f32 * hi;
        println!("biome {b}: row total {total} per-mille, x{hi:.3} = {scaled:.0}");
        assert!(
            scaled < 1_000.0,
            "biome {b}: its weight row totals {total} per-mille and the clump \
             field peaks at {hi:.3}, so a grove cell asks for {scaled:.0} of \
             1,000. Past the rail the last entries in the row become \
             unreachable and density falls with nothing reporting it — lower \
             `CLUMP_NORM`'s ceiling or the row, do not raise the rail."
        );
    }
}

/// Authored placement sits above the field and is not weather.
///
/// `clump` scales the biome draw only. The pad's container ring is placed
/// because the pad is there and the road's shoulder barrels are drawn at the
/// road's own rate, so a grove must not thin either of them — and the
/// carriageway must stay clear whatever the field says.
#[test]
fn test_clump_leaves_authored_slots_alone() {
    let table = ScatterTable::alpha_default();
    for seed in SEEDS {
        let f = build(seed);
        // Both authored tiers, counted together, because the field must sit
        // below BOTH branches: the pad's five and the lesser tier's pairs are
        // placed because a site is there, and a grove may thin neither.
        // `tests/haven.rs` is what proves the split between them is right;
        // this only proves the field never reached either.
        let live = terrain::haven(seed).minor.iter().filter(|w| w.live).count() as u32;
        let authored = HAVEN_CRATES as u32 + live * terrain::WAYSTATION_CRATES as u32;
        assert_eq!(
            f.counts[Occupant::CrateSlot as usize],
            authored,
            "seed {seed}: an authored container ring lost a crate — the clump \
             field is meant to sit below the haven branch in `scatter`."
        );
        assert_eq!(
            f.counts[Occupant::HavenShelter as usize],
            1,
            "seed {seed}: the pad's shelter is not standing — same branch, \
             same rule: authored slots sit above the field, not in it."
        );
        assert!(
            f.counts[Occupant::BarrelSlot as usize] > 50,
            "seed {seed}: only {} barrels — the road's own rate is not the \
             field's to scale.",
            f.counts[Occupant::BarrelSlot as usize]
        );

        // And the road is still walkable: no clump value lets a slot back
        // onto the carriageway.
        let haven = terrain::haven(seed);
        let mut on_road = 0u32;
        for cz in 0..CELLS_PER_SIDE {
            for cx in 0..CELLS_PER_SIDE {
                let s = terrain::scatter(seed, &table, &haven, cx, cz);
                if s.occupant != Occupant::None
                    && s.occupant != Occupant::CrateSlot
                    && terrain::road_band(seed, s.x, s.z) == terrain::RoadBand::Carriageway
                {
                    on_road += 1;
                }
            }
        }
        assert_eq!(
            on_road, 0,
            "seed {seed}: {on_road} slots on the carriageway"
        );
    }
}
