//! Dev probe: prints height distribution, biome mix, and live-slot counts
//! for a seed. Used to keep the generator inside TERRAIN.md §6's numbers
//! when shape params change. Not a gate — the gates pin goldens.

// Host-side tuning probe: printing is its job. The L5 wall bans
// format/print in SIM code; an example binary is not sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::terrain::{self, Occupant, ScatterTable, CELLS_PER_SIDE};

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x0047_4154_4553);

    let mut hist = [0u32; 12]; // <0, 0-10, 10-20, ... 90-100, >100
    let mut biomes = [0u32; 4];
    for gz in 0..256i32 {
        for gx in 0..256i32 {
            let x = gx as f32 * 8.0 + 4.0;
            let z = gz as f32 * 8.0 + 4.0;
            let h = terrain::height(seed, x, z);
            let bucket = if h < 0.0 {
                0
            } else {
                (((h / 10.0) as i32) + 1).min(11) as usize
            };
            hist[bucket] += 1;
            if h >= 0.0 {
                biomes[terrain::biome(h, terrain::moisture(seed, x, z)) as usize] += 1;
            }
        }
    }
    println!("height histogram (65,536 samples on the 8 m grid):");
    let labels = [
        "  sea", " 0-10", "10-20", "20-30", "30-40", "40-50", "50-60", "60-70", "70-80", "80-90",
        "90-100", " 100+",
    ];
    for (l, c) in labels.iter().zip(hist.iter()) {
        println!("  {l}: {c}");
    }
    println!(
        "biomes: beach {} meadow {} forest {} highland {}",
        biomes[0], biomes[1], biomes[2], biomes[3]
    );

    // The grove field itself. Its island mean is exactly what `CLUMP_NORM`
    // has to invert, so this is where that constant's value comes from.
    let (mut csum, mut csumsq) = (0f64, 0f64);
    let mut chist = [0u32; 8];
    for gz in 0..256i32 {
        for gx in 0..256i32 {
            let g = f64::from(terrain::clump(
                seed,
                gx as f32 * 8.0 + 4.0,
                gz as f32 * 8.0 + 4.0,
            ));
            csum += g;
            csumsq += g * g;
            chist[((g * 2.0) as usize).min(7)] += 1;
        }
    }
    let cmean = csum / 65_536.0;
    println!(
        "clump: mean {cmean:.4} sd {:.4} hist(0.5-wide buckets) {chist:?}",
        // `f64::sqrt` is walled, tests and examples included; `f32::sqrt` is
        // the one root the wall allows (`crates/sim-core/clippy.toml`).
        ((csumsq / 65_536.0 - cmean * cmean) as f32).sqrt()
    );

    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);
    // Indexed by `Occupant as usize`, which skips 8 (see the enum): sized to
    // the largest discriminant + 1, not to the number of variants.
    let mut counts = [0u32; 10];
    let mut trees = vec![0u8; (CELLS_PER_SIDE * CELLS_PER_SIDE) as usize];
    let mut land = vec![0u8; (CELLS_PER_SIDE * CELLS_PER_SIDE) as usize];
    for cz in 0..CELLS_PER_SIDE {
        for cx in 0..CELLS_PER_SIDE {
            let slot = terrain::scatter(seed, &table, &haven, cx, cz);
            counts[slot.occupant as usize] += 1;
            let i = (cz * CELLS_PER_SIDE + cx) as usize;
            trees[i] = u8::from(slot.occupant == Occupant::Tree);
            let (x, z) = (cx as f32 * 8.0 + 4.0, cz as f32 * 8.0 + 4.0);
            let h = terrain::height(seed, x, z);
            land[i] = u8::from(
                h >= terrain::LAND_MIN_H
                    && terrain::biome(h, terrain::moisture(seed, x, z)) == terrain::Biome::Forest,
            );
        }
    }
    let live: u32 = counts[1..].iter().sum();
    println!(
        "slots: live {live} (tree {} stone {} metal {} sulfur {} bush {} rock {} barrel {} crate {})",
        counts[Occupant::Tree as usize],
        counts[Occupant::StoneNode as usize],
        counts[Occupant::MetalNode as usize],
        counts[Occupant::SulfurNode as usize],
        counts[Occupant::Bush as usize],
        counts[Occupant::Rock as usize],
        counts[Occupant::BarrelSlot as usize],
        counts[Occupant::CrateSlot as usize],
    );

    // Texture of the tree field, the numbers `tests/scatter.rs` gates. A
    // per-cell independent draw puts a near-deterministic count in every
    // window; groves and clearings are what makes the distribution wide.
    const R: i32 = 2; // 5x5 cells = a 40 m window
    let n = ((2 * R + 1) * (2 * R + 1)) as f64;
    let (mut windows, mut sum, mut sumsq, mut empty) = (0u32, 0f64, 0f64, 0u32);
    let mut hist = [0u32; 9];
    for cz in R..CELLS_PER_SIDE - R {
        for cx in R..CELLS_PER_SIDE - R {
            let (mut c, mut all_forest) = (0u32, true);
            for wz in cz - R..=cz + R {
                for wx in cx - R..=cx + R {
                    let j = (wz * CELLS_PER_SIDE + wx) as usize;
                    all_forest &= land[j] != 0;
                    c += u32::from(trees[j]);
                }
            }
            if !all_forest {
                continue;
            }
            windows += 1;
            sum += f64::from(c);
            sumsq += f64::from(c) * f64::from(c);
            empty += u32::from(c == 0);
            hist[(c as usize).min(8)] += 1;
        }
    }
    let w = f64::from(windows);
    let mean = sum / w;
    let var = sumsq / w - mean * mean;
    // The independent-draw null, in closed form: a window of `n` cells each
    // holding a tree with probability mean/n is binomial, so its variance is
    // n·p·(1−p) and its empty share is (1−p)^n. No control run needed —
    // the null is arithmetic (SPAWN.md §9.3).
    let p = mean / n;
    let null_var = n * p * (1.0 - p);
    // (1−p)^n by repeated multiply: `powf` is walled here too.
    let mut null_empty = 1.0f64;
    for _ in 0..(2 * R + 1) * (2 * R + 1) {
        null_empty *= 1.0 - p;
    }
    println!(
        "tree windows (40 m, all-forest): n {windows} mean {mean:.3} var {var:.3} \
         dispersion {:.3} (null 1.000)",
        var / null_var
    );
    println!(
        "  empty {:.4} (null {null_empty:.4})  histogram 0..=8+ {hist:?}",
        f64::from(empty) / w
    );
}
