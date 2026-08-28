//! Dev probe: what share of the island's build cells will hold a foundation,
//! and how big the contiguous buildable patches are.
//!
//! The share alone is not the question a builder cares about — one cell holds
//! one foundation and a base is several — so this also reports the share of
//! passing cells whose whole 3x3 neighbourhood passes, which is what a second
//! and third piece need.

#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_types,
    clippy::disallowed_methods
)]

use sim_core::build::{build_cell_of, foundation_terrain_ok, BUILD_CELL_M};
use sim_core::terrain::{self, ISLAND_SIZE};

fn main() {
    println!(
        "{:>12} {:>9} {:>9} {:>9} {:>9}",
        "seed", "land", "ok", "ok‰", "3x3‰"
    );
    for seed in [
        0x0047_4154_4553u64,
        0x0047_4154_4552,
        20260731,
        0x1,
        0xDEAD_BEEF,
    ] {
        let hv = terrain::haven(seed);
        let n = build_cell_of(ISLAND_SIZE);
        let ok = |cx: i32, cz: i32| {
            let ax = (cx as f32 + 0.5) * BUILD_CELL_M;
            let az = (cz as f32 + 0.5) * BUILD_CELL_M;
            terrain::height(seed, ax, az) > 0.5 && foundation_terrain_ok(seed, &hv, ax, az)
        };
        let mut grid = vec![false; (n * n) as usize];
        let mut land = 0u32;
        for cz in 0..n {
            for cx in 0..n {
                let ax = (cx as f32 + 0.5) * BUILD_CELL_M;
                let az = (cz as f32 + 0.5) * BUILD_CELL_M;
                if terrain::height(seed, ax, az) > 0.5 {
                    land += 1;
                    grid[(cz * n + cx) as usize] = ok(cx, cz);
                }
            }
        }
        let good = grid.iter().filter(|&&b| b).count() as u32;
        let mut solid9 = 0u32;
        for cz in 1..n - 1 {
            for cx in 1..n - 1 {
                if (-1..=1).all(|dz| (-1..=1).all(|dx| grid[((cz + dz) * n + cx + dx) as usize])) {
                    solid9 += 1;
                }
            }
        }
        println!(
            "{seed:>12} {land:>9} {good:>9} {:>9.1} {:>9.1}",
            1000.0 * good as f64 / land as f64,
            1000.0 * solid9 as f64 / land as f64
        );
    }
}
