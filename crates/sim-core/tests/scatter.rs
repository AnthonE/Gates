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

/// The seeds the density band is measured over — **wider than [`SEEDS`] on
/// purpose, and the widening is the point.**
///
/// `SEEDS` is also the basis of the measured constants in this file
/// (`CLUMP_NORM`, `FOREST_CLUMP_NORM`), so adding to it would move numbers
/// that were derived on it. The density band derives from nothing but
/// islands, so it can and should see more of them — and until 2026-09-16 it
/// saw four while `tests/haven.rs` and `tests/waystation.rs` swept a
/// DIFFERENT four (`SWEEP_SEEDS = [1, 42, 20_260_804, 0xDEAD_BEEF]`).
/// Nothing swept both, and two of theirs sat outside this band with no change
/// to the tree at all. A gate that holds a world-wide budget on a quarter of
/// the worlds the repo already generates is holding it on the wrong set.
const DENSITY_SEEDS: [u64; 8] = [0, 1, 7, 42, 12345, 20_260_731, 20_260_804, 0xDEAD_BEEF];

/// TERRAIN.md §6's live-slot band, the world's density budget. **(knob)**
///
/// **Re-derived 2026-09-16 from `13_000..=19_000`, over eight seeds instead
/// of four, and the re-derivation is a correction rather than a widening.**
/// The band was set at forest density v1 against a measurement of
/// 16,020 / 16,666 / 14,743 / 15,660. World structure v1 (2026-09-15) moved
/// the island under it — the Beach row went 300 → 145‰ over four times the
/// beach, the moisture field rescaled, the treeline transferred tree weight
/// to bush — and live slots fell 10–17% to:
///
/// | seed | live | | seed | live |
/// |---|---|---|---|---|
/// | 0 | 14,514 | | 42 | **12,913** |
/// | 1 | 13,802 | | 20260731 | 14,319 |
/// | 7 | 13,249 | | 20260804 | 13,592 |
/// | 12345 | 13,890 | | 0xDEADBEEF | **12,978** |
///
/// ⚠ **Two of those are under the old floor and the gate did not notice**,
/// because they are `tests/haven.rs`'s sweep seeds and not this file's. The
/// real margin on seed 7 was 249 slots — 1.9%, not the ~12% the old comment
/// implied. Nothing was red; the band had simply stopped describing the
/// islands this repo makes, which is `CLAUDE.md`'s prose-drift trap arriving
/// in a constant instead of a paragraph.
///
/// Set the way the old one was: ~12% under the measured minimum, ~14% over
/// the measured maximum, so it still reddens on a row change that moves
/// density (proven — halving the Forest tree row reads 9,824 on seed 0) and
/// no longer sits 1.9% off an island the repo already generates.
const LIVE_SLOTS_MIN: u32 = 11_400;
const LIVE_SLOTS_MAX: u32 = 16_500;

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
    counts: [u32; OCCUPANT_SLOTS],
}

/// Bucket count for an array indexed by `Occupant as usize` — the largest
/// discriminant plus one, which is NOT the number of variants (index 8 is
/// the client's stump and has no variant).
///
/// Derived from the shipped table rather than written as a literal, because
/// a literal here is the exact bug `terrain.rs`'s `occupant_volume` doc
/// records: `examples/terrain_stats.rs` carried `[0u32; 10]` through the
/// `HavenShelter = 10` commit and panicked on the first haven cell of every
/// seed. `CacheSlot = 11` would have done it again.
const OCCUPANT_SLOTS: usize = terrain::OCCUPANT_R_M.len();

fn build(seed: u64) -> Field {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);
    let n = (CELLS_PER_SIDE * CELLS_PER_SIDE) as usize;
    let mut f = Field {
        tree: vec![false; n],
        forest: vec![false; n],
        // Indexed by `Occupant as usize`, which skips 8: sized to the
        // largest discriminant + 1, not to the number of variants.
        counts: [0u32; OCCUPANT_SLOTS],
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
    for seed in DENSITY_SEEDS {
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
        // the shipped seed; this holds it on EIGHT, because a field with a
        // seed-dependent mean would pass there and fail in play — and
        // because four of these eight are the ones `tests/haven.rs` sweeps,
        // which this gate could not see until 2026-09-16. `LIVE_SLOTS_MIN`
        // carries the measurement and what moved it. The band is what
        // `limits::MAX_SLOT_LIVES` is sized past.
        assert!(
            (LIVE_SLOTS_MIN..=LIVE_SLOTS_MAX).contains(&live),
            "seed {seed}: {live} live slots is outside TERRAIN.md §6's \
             {LIVE_SLOTS_MIN}–{LIVE_SLOTS_MAX} — the clump field is meant to move \
             density around, not change how much there is (check `CLUMP_NORM` \
             and the capped biomes' `clump_cap_norm`), and a row change that \
             moves this moves `MAX_SLOT_LIVES`'s sizing with it."
        );
        assert!(
            live < sim_core::limits::MAX_SLOT_LIVES as u32,
            "seed {seed}: {live} live slots but the slot-life store holds \
             {} — a fully harvested island would not fit its own save.",
            sim_core::limits::MAX_SLOT_LIVES
        );
        assert!(
            f.counts[Occupant::Tree as usize] > 1_000,
            "seed {seed}: only {} trees",
            f.counts[Occupant::Tree as usize]
        );
    }
}

/// Every island's metal and sulfur land on the ore budget, whatever its rock.
///
/// Before the budget an island's ore was its rock area times the Highland
/// row, and the interior ranges made rock area the seed's: these eight
/// islands drew 93–209 metal and 54–147 sulfur, and raiding is priced in
/// sulfur. `haven` now scales the row per island (`ORE_TARGET`, `ORE_PM_*`);
/// what remains is the draw's own noise over independent cells, and the two
/// islands too bare of rock to reach the budget under `ORE_PM_MAX`. Measured
/// 2026-09-24: metal 131–170, sulfur 101–132.
const ORE_BAND: f32 = 0.25;

#[test]
fn test_ore_is_budgeted_per_island() {
    for seed in DENSITY_SEEDS {
        let f = build(seed);
        let pm = terrain::haven(seed).ore_pm;
        for (k, (name, occ)) in [
            ("metal", Occupant::MetalNode),
            ("sulfur", Occupant::SulfurNode),
        ]
        .into_iter()
        .enumerate()
        {
            let got = f.counts[occ as usize] as f32;
            let want = terrain::ORE_TARGET[k];
            println!(
                "seed {seed}: {name} {got} against {want} (scale {} per mille)",
                pm[k]
            );
            assert!(
                (want * (1.0 - ORE_BAND)..=want * (1.0 + ORE_BAND)).contains(&got),
                "seed {seed}: {got} {name} nodes against a budget of {want} — \
                 outside ±{:.0}%. The island's ore follows its rock area again \
                 (check `terrain::ore_budget` and `ore_budgeted`), or the draw \
                 moved under the estimate.",
                ORE_BAND * 100.0
            );
        }
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
/// The row is scaled by up to `max(clump)` — **as each biome keeps it**,
/// held to `ScatterTable::clump_cap[b]` and re-normalized — and the draw
/// compares a per-mille roll against the running total, so once a scaled
/// row reaches 1,000 the tail entries stop being reachable and the extra
/// weight evaporates — density would fall while every mean above still
/// read 1.0. Nothing else in the suite can see that, because it looks
/// exactly like a slightly thinner forest.
///
/// Two halves. The pure bound: each biome's row total at the field's
/// measured peak through that biome's cap. The blend is a convex mix of
/// those four scaled rows (`scatter_draw_row` scales before it blends), so
/// the largest of them bounds every cell — up to per-entry rounding, which
/// is the second half: the shipped seeds are swept cell by cell and the row
/// each land cell actually draws against is held under the rail too.
///
/// **The Forest row is 700‰ against a field that peaks at 2.7**, which
/// would be 1,890 uncapped; the cap is what makes it 936. A cap that
/// stopped binding (`FOREST_CLUMP_CAP` back at the peak) fails the first
/// half on this row; a normalizer typed too high fails both.
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
    // The ore budget scales the Highland row's metal and sulfur by up to
    // `ORE_PM_MAX`, so the pure rows are held at that ceiling.
    let mut ceiling = terrain::haven(SEEDS[0]);
    ceiling.ore_pm = [terrain::ORE_PM_MAX; 2];
    for (b, row) in table.weights.iter().enumerate() {
        let total: u32 = terrain::ore_budgeted(*row, &ceiling)
            .iter()
            .map(|&w| u32::from(w))
            .sum();
        let factor = hi.min(table.clump_cap[b]) * table.clump_cap_norm[b];
        let scaled = total as f32 * factor;
        println!(
            "biome {b}: row total {total} per-mille, field peak x{hi:.3} as kept x{factor:.3} = {scaled:.0}"
        );
        assert!(
            scaled < 1_000.0,
            "biome {b}: its weight row totals {total} per-mille and the clump \
             field peaks at {hi:.3} — {factor:.3} through this biome's cap — so \
             a grove cell asks for {scaled:.0} of 1,000. Past the rail the last \
             entries in the row become unreachable and density falls with \
             nothing reporting it — lower the cap, `CLUMP_NORM`'s ceiling or \
             the row, do not raise the rail."
        );
    }

    // On the ground: the row a cell draws against, every land cell of every
    // gate seed, at the field's value there. The convexity argument above is
    // exact before rounding and this is what holds the rounding.
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let mut worst = 0u32;
        for gz in 0..CELLS_PER_SIDE {
            for gx in 0..CELLS_PER_SIDE {
                let x = gx as f32 * CELL_SIZE + CELL_SIZE * 0.5;
                let z = gz as f32 * CELL_SIZE + CELL_SIZE * 0.5;
                let h = terrain::ground(seed, &haven, x, z);
                if h < LAND_MIN_H {
                    continue;
                }
                let row = terrain::ore_budgeted(
                    terrain::scatter_draw_row(
                        &table,
                        h,
                        terrain::moisture(seed, x, z),
                        terrain::ground_slope(seed, &haven, x, z),
                        terrain::clump(seed, x, z),
                    ),
                    &haven,
                );
                worst = worst.max(row.iter().map(|&w| u32::from(w)).sum());
            }
        }
        println!("seed {seed}: the densest drawn row asks for {worst} of 1,000");
        assert!(
            worst < 1_000,
            "seed {seed}: a cell draws against a row totalling {worst} per-mille \
             — past the rail, so its tail entries are unreachable there."
        );
    }
}

/// `ScatterTable::clump_cap_norm` is re-derived here rather than trusted, on
/// the grid the constant's doc comment claims it was measured on — the same
/// gate `test_clump_normalizer_holds` is for `CLUMP_NORM`, one level down.
///
/// A capped biome draws against `min(clump, cap) × norm`, and `norm` is the
/// reciprocal of the island mean of `min(clump, cap)`: a number that is
/// right when written and wrong the moment the field's shaping or the cap
/// moves, silently taking that biome's density with it. So the gate does
/// the division again, per seed, with the field's own wobble as tolerance.
///
/// And the cap has to BIND, or it is a comment: the share of cells the field
/// reads at or above it is the share of the biome standing at its scaled
/// ceiling, which is the whole reason a cap exists. Measured 43.3% on the
/// four seeds at the shipped cap; a cap at the field's peak reads 1.7% and
/// fails here, while passing the mean check above trivially (norm 1.0).
#[test]
fn test_clump_cap_normalizer_holds() {
    let table = ScatterTable::alpha_default();
    let n = f64::from(CELLS_PER_SIDE) * f64::from(CELLS_PER_SIDE);
    for b in 0..4 {
        let cap = table.clump_cap[b];
        let norm = table.clump_cap_norm[b];
        if !cap.is_finite() {
            assert!(
                norm == 1.0,
                "biome {b}: no cap, so the field's own mean (1.0 by `CLUMP_NORM`) \
                 is the mean and the normalizer must be exactly 1.0, not {norm}"
            );
            continue;
        }
        for seed in SEEDS {
            let (mut sum, mut at) = (0f64, 0u32);
            for gz in 0..CELLS_PER_SIDE {
                for gx in 0..CELLS_PER_SIDE {
                    let g = terrain::clump(
                        seed,
                        gx as f32 * CELL_SIZE + CELL_SIZE * 0.5,
                        gz as f32 * CELL_SIZE + CELL_SIZE * 0.5,
                    );
                    sum += f64::from(g.min(cap));
                    at += u32::from(g >= cap);
                }
            }
            let mean = sum / n * f64::from(norm);
            let share = f64::from(at) / n;
            println!(
                "biome {b} seed {seed}: min(clump, {cap}) x {norm} means {mean:.4}, \
                 at the cap on {:.1}% of cells",
                share * 100.0
            );
            assert!(
                (0.97..=1.03).contains(&mean),
                "biome {b} seed {seed}: the capped field means {mean:.4} after its \
                 normalizer, so it multiplies this biome's density by that. \
                 `clump_cap_norm[{b}]` is the reciprocal of the mean of \
                 `min(clump, cap)` and has drifted — re-derive it \
                 (`examples/clump_cap.rs`)."
            );
            assert!(
                share >= 0.25,
                "biome {b} seed {seed}: the field is at or above its cap on only \
                 {:.1}% of cells — the cap is not binding, so this biome has no \
                 stands at its ceiling and the cap is a comment.",
                share * 100.0
            );
        }
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
        // Both authored tiers, counted SEPARATELY, because each draws its own
        // container kind and therefore its own loot table: the pad's five
        // `CrateSlot` and the lesser tier's pairs of `CacheSlot` are placed
        // because a site is there, and a grove may thin neither. Counting
        // them together — which this did while both tiers placed `CrateSlot`
        // — cannot see a crate appearing where a cache should, which is the
        // whole tier gradient landing on the wrong table.
        let minor = terrain::haven(seed).minor;
        let live = minor.iter().filter(|w| w.live).count() as u32;
        // Summed over the live sites' TIERS — the inland one owes no cache
        // (`terrain::INLAND_CRATES`), so a flat `live * WAYSTATION_CRATES`
        // would demand a container the ladder refuses it.
        let caches: u32 = minor
            .iter()
            .filter(|w| w.live)
            .map(|w| terrain::site_crates(w.kind) as u32)
            .sum();
        assert_eq!(
            f.counts[Occupant::CrateSlot as usize],
            HAVEN_CRATES as u32,
            "seed {seed}: the pad's container ring lost a crate — the clump \
             field is meant to sit below the haven branch in `scatter`."
        );
        assert_eq!(
            f.counts[Occupant::CacheSlot as usize],
            caches,
            "seed {seed}: the lesser tier's {live} site(s) do not carry the \
             {caches} cache(s) their tiers owe — same rule one tier down."
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
                    && terrain::road_band(seed, &haven, s.x, s.z) == terrain::RoadBand::Carriageway
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

// ── The mix, not the amount (TERRAIN.md §1 stage 9's stated residual) ──────
//
// `clump` above gates how MUCH stands somewhere. These gate WHAT stands
// there. Stage 9 shipped its clumping with one line left open — "`biome()`
// is still a hard classifier, so a biome boundary is still a step in
// *composition* even though density now ramps across it" — and
// `terrain::scatter_row` closes it by drawing the mix from `splat_from`,
// the same four weights the ground material and the clutter population
// already use. The three checks below are, in order: the blend is faithful
// where the splat is one-hot, it ramps where the splat is not, and it is
// convex everywhere (which is what keeps `test_no_biome_row_saturates`
// above a valid bound on the blended row without knowing it exists).

/// Height and slope chosen so only the moisture channel is in play: above
/// `SPLAT_BEACH_BAND`, below `SPLAT_ALPINE_BAND`, flat. The splat there is
/// `[0, 1-wood, wood, 0]`, so the sweep is a pure Meadow→Forest transition
/// and the tree weight is the whole story.
const MIX_H: f32 = 20.0;
const MIX_SLOPE: f32 = 0.0;

/// The transition the two classifiers disagree about, swept fine enough
/// that the smoothstep's own curvature is resolved.
const MIX_LO: f32 = -0.05;
const MIX_HI: f32 = 0.15;
const MIX_STEPS: i32 = 200;

fn mix_at(table: &ScatterTable, moist: f32) -> [u16; terrain::OCCUPANT_KINDS] {
    terrain::scatter_row(table, MIX_H, moist, MIX_SLOPE)
}

/// The hard classifier, re-derived here rather than remembered — this is the
/// control the ramp is measured against, and it is the code that shipped
/// before this change.
fn hard_row_at(table: &ScatterTable, moist: f32) -> [u16; terrain::OCCUPANT_KINDS] {
    table.weights[terrain::biome(MIX_H, moist) as usize]
}

/// Where the splat is one-hot the blend must be the identity, or this is not
/// a softening of the boundary but a re-authoring of the whole island.
///
/// It is the property that lets `test_scatter_density_preserved` above stay
/// a statement about `CLUMP_NORM`: interiors are untouched, so any density
/// the blend moves, it moves in the transition bands alone.
#[test]
fn test_scatter_mix_is_identity_in_the_interior() {
    let table = ScatterTable::alpha_default();
    for (label, moist) in [("meadow", MIX_LO), ("forest", MIX_HI)] {
        let blended = mix_at(&table, moist);
        let pure = hard_row_at(&table, moist);
        println!("{label} interior (moist {moist}): blended {blended:?} pure {pure:?}");
        assert_eq!(
            blended, pure,
            "deep in the {label} the splat is one-hot, so the blended row must \
             equal the row the hard classifier picked — it does not, so \
             `scatter_row` is not a boundary fix, it is a new table."
        );
    }
}

/// The claim itself: composition crosses the boundary as a ramp, where it
/// used to cross as a step.
///
/// Both classifiers are walked over the same sweep and the largest
/// single-sample change in the tree weight is taken from each. The hard
/// one's is the entire Meadow→Forest difference in one sample, by
/// construction — that is what a classifier is. The blend's is bounded by
/// the smoothstep's slope, so the ratio is the measurement, and no number
/// from a previous build is involved on either side.
#[test]
fn test_scatter_mix_ramps_where_it_used_to_step() {
    let table = ScatterTable::alpha_default();
    let tree_span = {
        let m = table.weights[Biome::Meadow as usize][0] as i32;
        let f = table.weights[Biome::Forest as usize][0] as i32;
        (f - m).max(m - f)
    };

    let mut worst_blend = 0i32;
    let mut worst_hard = 0i32;
    let mut prev: Option<(
        [u16; terrain::OCCUPANT_KINDS],
        [u16; terrain::OCCUPANT_KINDS],
    )> = None;
    for i in 0..=MIX_STEPS {
        let moist = MIX_LO + (MIX_HI - MIX_LO) * (i as f32 / MIX_STEPS as f32);
        let b = mix_at(&table, moist);
        let h = hard_row_at(&table, moist);
        if let Some((pb, ph)) = prev {
            for k in 0..terrain::OCCUPANT_KINDS {
                let db = b[k] as i32 - pb[k] as i32;
                worst_blend = worst_blend.max(db).max(-db);
                let dh = h[k] as i32 - ph[k] as i32;
                worst_hard = worst_hard.max(dh).max(-dh);
            }
        }
        prev = Some((b, h));
    }

    println!(
        "moisture sweep {MIX_LO}..{MIX_HI} in {MIX_STEPS} steps: worst per-sample \
         jump — blended {worst_blend} per-mille, hard classifier {worst_hard}, \
         tree span {tree_span}"
    );

    // The control really is a cliff: the classifier moves the whole span at
    // one sample. If this ever fails, the sweep stopped crossing the edge
    // and the comparison below would be measuring nothing.
    assert_eq!(
        worst_hard, tree_span,
        "the hard classifier should jump the full Meadow→Forest tree span \
         ({tree_span}) at one sample — it jumped {worst_hard}, so this sweep \
         no longer crosses the boundary and the ramp assert below is vacuous."
    );

    // And the blend is not. An order of magnitude is a floor, not the
    // measurement: at this step count the smoothstep's own slope puts it
    // near fifty times gentler, so a regression that half-reverted the blend
    // would still fall through this.
    assert!(
        worst_blend * 10 < worst_hard,
        "composition still steps: the blended row's worst per-sample jump is \
         {worst_blend} per-mille against the classifier's {worst_hard}. Stage \
         9's residual is that a biome edge is a step in composition; a blend \
         that jumps nearly as hard as the classifier has not closed it."
    );
    assert!(
        worst_blend > 0,
        "the blended row never changed across the whole sweep — the mix is \
         not tracking moisture at all."
    );
}

/// Convexity, on the real island rather than the synthetic sweep.
///
/// Every blended entry must lie inside the range its four pure rows span.
/// This is what makes `test_no_biome_row_saturates` above still a bound on
/// what `scatter` actually draws: that test rails the four authored rows,
/// and a convex blend of them cannot exceed the largest. It also catches the
/// failure a normalization bug would produce — a row that sums past its
/// inputs, thinning the tail entries with every mean still reading right.
///
/// Sampled on the same cell centers the field above is built on, so the
/// points are the ones the world is actually drawn at.
#[test]
fn test_scatter_mix_is_convex_and_the_island_uses_it() {
    let table = ScatterTable::alpha_default();
    let mut lo = [u16::MAX; terrain::OCCUPANT_KINDS];
    let mut hi = [0u16; terrain::OCCUPANT_KINDS];
    for row in table.weights.iter() {
        for k in 0..terrain::OCCUPANT_KINDS {
            lo[k] = lo[k].min(row[k]);
            hi[k] = hi[k].max(row[k]);
        }
    }
    // The span of the one pair the treeline moves weight inside of.
    let mut pair_lo = u16::MAX;
    let mut pair_hi = 0u16;
    for row in table.weights.iter() {
        let p = row[terrain::ROW_TREE] + row[terrain::ROW_BUSH];
        pair_lo = pair_lo.min(p);
        pair_hi = pair_hi.max(p);
    }

    for seed in SEEDS {
        let mut land = 0u32;
        let mut blended = 0u32;
        for gz in 0..CELLS_PER_SIDE {
            for gx in 0..CELLS_PER_SIDE {
                let x = gx as f32 * CELL_SIZE + CELL_SIZE * 0.5;
                let z = gz as f32 * CELL_SIZE + CELL_SIZE * 0.5;
                let h = terrain::height(seed, x, z);
                if h < LAND_MIN_H {
                    continue;
                }
                land += 1;
                let sl = terrain::slope(seed, x, z);
                let w = terrain::splat_from(h, terrain::moisture(seed, x, z), sl);
                let row = terrain::scatter_row(&table, h, terrain::moisture(seed, x, z), sl);
                // Every entry the treeline does not touch is still strictly
                // convex, and so is the PAIR it moves weight between — which
                // is the claim that actually protects the saturation bound,
                // because `test_no_biome_row_saturates` is about totals and a
                // transfer inside a row moves none.
                //
                // ⚠ **`ROW_TREE` and `ROW_BUSH` are exempt individually and
                // that is the feature, not a loosened gate** (world structure
                // v1). `EDGE_TREE_TO_BUSH` hands tree weight to bush on the
                // treeline, so a border cell legitimately carries more bush
                // than any authored row does — 75 against the [10, 70] the
                // four rows span, measured. What must still hold, and is
                // asserted below, is that the transfer went where it said:
                // the pair's SUM stays inside the pure rows' span, so nothing
                // leaked into stone, sulfur or a barrel, and the row total is
                // untouched. Dropping the pair from the sweep entirely would
                // have been the loosening; this is a different exact claim.
                for k in 0..terrain::OCCUPANT_KINDS {
                    if k == terrain::ROW_TREE || k == terrain::ROW_BUSH {
                        continue;
                    }
                    assert!(
                        row[k] >= lo[k] && row[k] <= hi[k],
                        "seed {seed} cell ({gx},{gz}): blended entry {k} is {}, \
                         outside the [{}, {}] its four authored rows span. A \
                         convex blend cannot do that — the normalization is \
                         wrong, and `test_no_biome_row_saturates` is no longer \
                         a bound on what scatter draws.",
                        row[k],
                        lo[k],
                        hi[k]
                    );
                }
                let pair = row[terrain::ROW_TREE] + row[terrain::ROW_BUSH];
                assert!(
                    pair >= pair_lo && pair <= pair_hi,
                    "seed {seed} cell ({gx},{gz}): tree+bush is {pair}, outside \
                     the [{pair_lo}, {pair_hi}] its four authored rows span. The \
                     treeline is a TRANSFER between those two entries, so their \
                     sum is convex even where neither is — a sum outside the \
                     span means weight was created or leaked in from elsewhere."
                );
                // "In a transition" = no single ground identity owns the
                // cell outright. This is the share of the island the change
                // can reach at all, and it is reported because a fix that
                // touched almost nothing would otherwise read as a fix.
                if w.iter().all(|&c| c < 230) {
                    blended += 1;
                }
            }
        }
        let share = blended as f32 / land as f32;
        println!(
            "seed {seed}: {blended}/{land} land cells sit in a transition \
             ({:.1}% of the island's land)",
            share * 100.0
        );
        assert!(
            blended * 20 > land,
            "seed {seed}: only {blended} of {land} land cells are in a \
             transition band. Below a twentieth of the island the composition \
             blend is not worth the arithmetic — say so rather than shipping \
             it as a boundary fix."
        );
    }
}
