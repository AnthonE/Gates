#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_types,
    clippy::disallowed_methods
)]
//! What the side road came out as, and what it bought.
//!
//! `reference/ROADS.md` §8 gates 1–3 in report form: per seed, the road's
//! length and port, then the island-wide reach — the walk from any walkable
//! land cell to the nearest road — with the ring alone and with the ring plus
//! the side road. §7.2 measured the ring alone at p50 218 m / p90 570 m and
//! 38% of land over 300 m away; this is what the second tier moves.
//!
//! `cargo run --release -p sim-core --example side_road`

use sim_core::terrain::{self, Haven, RoadBand, CLIFF_SLOPE_RATIO, ISLAND_SIZE, LAND_MIN_H};

const SEEDS: [u64; 8] = [
    1,
    42,
    20_260_731,
    20_260_804,
    555_555,
    31_337,
    0xDEAD_BEEF,
    0x0BAD_C0DE,
];

/// 4 m, so a cell is under the carriageway's own width.
const STEP: f32 = 4.0;
const N: usize = (ISLAND_SIZE / STEP) as usize;

/// Walk distances in cells from every road cell, over WALKABLE land only.
fn reach(seed: u64, h: &Haven, with_side: bool) -> (Vec<u32>, usize) {
    let mut ok = vec![false; N * N];
    let mut dist = vec![u32::MAX; N * N];
    let mut q: Vec<usize> = Vec::new();
    for iz in 0..N {
        for ix in 0..N {
            let i = iz * N + ix;
            let (x, z) = (ix as f32 * STEP, iz as f32 * STEP);
            if terrain::height(seed, x, z) < LAND_MIN_H
                || terrain::slope(seed, x, z) > CLIFF_SLOPE_RATIO
            {
                continue;
            }
            ok[i] = true;
            let on = terrain::ring_band(seed, x, z) != RoadBand::Off
                || (with_side && terrain::side_band(h, x, z) != RoadBand::Off);
            if on {
                dist[i] = 0;
                q.push(i);
            }
        }
    }
    let mut head = 0usize;
    while head < q.len() {
        let p = q[head];
        head += 1;
        let (px, pz) = (p % N, p / N);
        for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
            let (jx, jz) = (px as i32 + dx, pz as i32 + dz);
            if jx < 0 || jz < 0 || jx >= N as i32 || jz >= N as i32 {
                continue;
            }
            let j = jz as usize * N + jx as usize;
            if ok[j] && dist[j] == u32::MAX {
                dist[j] = dist[p] + 1;
                q.push(j);
            }
        }
    }
    let mut d: Vec<u32> = (0..N * N)
        .filter(|&i| ok[i] && dist[i] != u32::MAX)
        .map(|i| dist[i])
        .collect();
    let stranded = (0..N * N).filter(|&i| ok[i] && dist[i] == u32::MAX).count();
    d.sort_unstable();
    (d, stranded)
}

fn stats(d: &[u32]) -> (f32, f32, f32) {
    let at = |f: f32| d[((d.len() - 1) as f32 * f) as usize] as f32 * STEP;
    let over =
        d.iter().filter(|&&v| v as f32 * STEP > 300.0).count() as f32 * 100.0 / d.len() as f32;
    (at(0.5), at(0.9), over)
}

fn main() {
    println!(
        "{:>12} {:>7} {:>5} {:>28} {:>28}",
        "seed", "len", "port", "ring only (p50/p90/>300m)", "ring + side"
    );
    let (mut sum_before, mut sum_after) = (0.0f32, 0.0f32);
    for &seed in SEEDS.iter() {
        let h = terrain::haven(seed);
        let r = h.roads[0];
        // The road, not its chord — they stopped being the same number at
        // side road bend v0 and this column says how far you actually walk.
        let len = r.path_len();
        let (d0, s0) = reach(seed, &h, false);
        let (d1, s1) = reach(seed, &h, true);
        let (a50, a90, aov) = stats(&d0);
        let (b50, b90, bov) = stats(&d1);
        sum_before += aov;
        sum_after += bov;
        println!(
            "{seed:>12} {:>6.0}m {:>5} {:>8.0}m {:>8.0}m {:>7.1}% {:>8.0}m {:>8.0}m {:>7.1}%  \
             stranded {s0}->{s1}",
            len, r.port, a50, a90, aov, b50, b90, bov
        );
    }
    println!(
        "\nmean share of walkable land over 300 m from a road: {:.1}% -> {:.1}%",
        sum_before / SEEDS.len() as f32,
        sum_after / SEEDS.len() as f32
    );
}
