//! Dev probe: the island's slope distribution, from `terrain::slope` itself.
//! Not a gate — the thing worldgen shape changes have to be checked against,
//! because a detail term that reads well in a hillshade can still put ground
//! past the walkable threshold.

#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_types,
    clippy::disallowed_methods
)]

use sim_core::terrain::{self, CLIFF_SLOPE_RATIO, ISLAND_SIZE};

fn main() {
    let seeds: [u64; 4] = [20260731, 0x0047_4154_4553, 0x1, 0xDEAD_BEEF];
    println!(
        "{:>12} {:>7} {:>7} {:>7} {:>7} {:>7} {:>8} {:>8}",
        "seed", "p50", "p90", "p99", "p999", "max", "cliff‰", "hi_p99"
    );
    for seed in seeds {
        let mut all: Vec<f32> = Vec::new();
        let mut high: Vec<f32> = Vec::new();
        let step = 4.0f32;
        let mut z = step * 0.5;
        while z < ISLAND_SIZE {
            let mut x = step * 0.5;
            while x < ISLAND_SIZE {
                let h = terrain::height(seed, x, z);
                if h > 0.5 {
                    let s = terrain::slope(seed, x, z);
                    all.push(s);
                    if h > 44.0 {
                        high.push(s);
                    }
                }
                x += step;
            }
            z += step;
        }
        all.sort_by(|a, b| a.partial_cmp(b).unwrap());
        high.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let q = |v: &Vec<f32>, p: f64| v[((v.len() - 1) as f64 * p) as usize];
        let cliff = all.iter().filter(|&&s| s > CLIFF_SLOPE_RATIO).count();
        println!(
            "{seed:>12} {:>7.3} {:>7.3} {:>7.3} {:>7.3} {:>7.3} {:>8.1} {:>8.3}",
            q(&all, 0.50),
            q(&all, 0.90),
            q(&all, 0.99),
            q(&all, 0.999),
            all[all.len() - 1],
            1000.0 * cliff as f64 / all.len() as f64,
            if high.is_empty() { 0.0 } else { q(&high, 0.99) },
        );
    }
}
