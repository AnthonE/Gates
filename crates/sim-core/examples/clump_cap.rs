//! Dev probe for `ScatterTable::clump_cap`: for a sweep of caps, the island
//! mean of `min(clump, cap)` (whose reciprocal is the biome's
//! `clump_cap_norm`), the share of cells at or above the cap (the share of a
//! biome standing at its scaled ceiling), and the row total each cap
//! admits under the saturation rail. Not a gate — `tests/scatter.rs`
//! re-derives the shipped normalizer and holds the rail.
//!
//! `cargo run --release -p sim-core --example clump_cap [cap ...]`

// Host-side tuning probe: printing is its job. The L5 wall bans
// format/print in SIM code; an example binary is not sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::terrain::{self, CELLS_PER_SIDE, CELL_SIZE};

const SEEDS: [u64; 4] = [0, 1, 7, 12345];

fn main() {
    let caps: Vec<f32> = {
        let args: Vec<f32> = std::env::args()
            .skip(1)
            .filter_map(|s| s.parse().ok())
            .collect();
        if args.is_empty() {
            vec![1.0, 1.1, 1.2, 1.25, 1.3, 1.4, 1.5, 1.6, 1.8, 2.0, 2.7]
        } else {
            args
        }
    };
    let n = (CELLS_PER_SIDE * CELLS_PER_SIDE) as f64;
    println!("cap    mean(min(g,cap))  norm   at-cap   row rail (‰ = 1000 / (cap × norm))");
    for cap in caps {
        let (mut sum, mut at) = (0f64, 0u64);
        for seed in SEEDS {
            for gz in 0..CELLS_PER_SIDE {
                for gx in 0..CELLS_PER_SIDE {
                    let g = terrain::clump(
                        seed,
                        gx as f32 * CELL_SIZE + CELL_SIZE * 0.5,
                        gz as f32 * CELL_SIZE + CELL_SIZE * 0.5,
                    );
                    sum += f64::from(g.min(cap));
                    at += u64::from(g >= cap);
                }
            }
        }
        let mean = sum / (n * SEEDS.len() as f64);
        let norm = 1.0 / mean;
        println!(
            "{cap:<5.2}  {mean:>9.4}        {norm:>6.4}  {:>5.1}%   {:>5.0}",
            at as f64 / (n * SEEDS.len() as f64) * 100.0,
            1000.0 / (f64::from(cap) * norm)
        );
    }
}
