//! Dev probe: what the interior ranges (`terrain` stage 4d) are, per seed —
//! how much ground they cover, how steep it is, and how high they reach.
//! Not a gate: `tests/massif.rs` holds the bands this measures, and this is
//! what a change to a range's shape has to be re-read with.
//!
//! The footprint is every 4 m sample where a range lifts the ground by more
//! than 5 m; slope is `terrain::slope`, the sim's own.

#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_types,
    clippy::disallowed_methods
)]

use sim_core::terrain::{self, CLIFF_SLOPE_RATIO, ISLAND_SIZE};

fn main() {
    let seeds: [u64; 4] = [20260731, 0x0047_4154_4553, 0x1, 0xDEAD_BEEF];
    println!(
        "{:>12} {:>9} {:>6} {:>6} {:>6} {:>6} {:>7} {:>7}",
        "seed", "area km²", "p50", "p90", "p99", "max", "cliff%", "peak m"
    );
    for seed in seeds {
        let mut s = Vec::new();
        let mut peak = 0.0f32;
        let mut z = 2.0;
        while z < ISLAND_SIZE {
            let mut x = 2.0;
            while x < ISLAND_SIZE {
                if terrain::massif_lift_at(seed, x, z) > 5.0 {
                    s.push(terrain::slope(seed, x, z));
                    peak = peak.max(terrain::height(seed, x, z));
                }
                x += 4.0;
            }
            z += 4.0;
        }
        s.sort_by(|a, b| a.total_cmp(b));
        let p = |q: f32| s[((s.len() as f32 - 1.0) * q) as usize];
        let cliff = s.iter().filter(|v| **v > CLIFF_SLOPE_RATIO).count() as f32 / s.len() as f32;
        println!(
            "{seed:>12} {:>9.3} {:>6.2} {:>6.2} {:>6.2} {:>6.2} {:>7.1} {:>7.0}",
            s.len() as f32 * 16.0 / 1e6,
            p(0.5),
            p(0.9),
            p(0.99),
            p(1.0),
            cliff * 100.0,
            peak
        );
    }
}
