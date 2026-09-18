#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_types,
    clippy::disallowed_methods
)]
//! Why is the coast ring in pieces? Walks it bearing by bearing, classifies
//! every break by cause, and ablates each candidate fix.
//!
//! ⚠ **Uses real trig, deliberately.** `sim_core::yaw_dir` is a 256-entry LUT
//! (`yaw_lut.rs`), so a sweep built on it samples 256 bearings however many
//! steps it claims — 20.9 m of arc at r=850, against a 4 m carriageway. The
//! first draft of this file did exactly that and read the ring as ~9% in one
//! piece with cliffs as the cause; both numbers were the LUT. An example is
//! host code and may call `sin_cos`, and the ring predicate itself takes no
//! yaw — `ring_band` derives its radial from `(x, z)` — so the continuous
//! sweep is the honest instrument and the LUT was never part of the law.
//!
//! **Its first two sections measure `ring_probe`, the RAW predicate** — the
//! ring as it was before ring path v0, kept because that is what the
//! diagnosis was run against and what the traps below are about. The third
//! section walks the predicate's centre line and the solved `RingPath`'s side
//! by side, at one pitch, which is the only fair comparison of the two.
//!
//! `cargo run --release -p sim-core --example ring_breaks`
use sim_core::terrain::{
    self, RoadBand, CLIFF_SLOPE_RATIO, ISLAND_SIZE, LAND_MIN_H, RING_BEARINGS, ROAD_HALF_W,
    ROAD_R_MAX, ROAD_R_MIN,
};

/// Pitch the two centre lines are walked at in the comparison, metres.
const PITCH: f32 = 2.0;

const SEEDS: [u64; 8] = [
    20_260_731,
    42,
    0xDEAD_BEEF,
    1,
    555_555,
    31_337,
    8_675_309,
    20_260_804,
];
/// 0.77 m of arc at ROAD_R_MAX — a fifth of the carriageway, so two adjacent
/// samples overlap by construction when the radius holds.
const STEPS: usize = 8192;
const RSTEP: f32 = 0.5;

/// Carriageway runs along one radial, as (mid radius, walkable).
fn runs_on(seed: u64, ux: f32, uz: f32, out: &mut Vec<(f32, bool)>) {
    out.clear();
    let c = ISLAND_SIZE * 0.5;
    let mut r = ROAD_R_MIN;
    let mut start = -1.0f32;
    while r <= ROAD_R_MAX + RSTEP {
        let on = r <= ROAD_R_MAX
            && terrain::ring_probe(seed, c + ux * r, c + uz * r) == RoadBand::Carriageway;
        if on && start < 0.0 {
            start = r;
        } else if !on && start >= 0.0 {
            let mid = (start + r - RSTEP) * 0.5;
            let (mx, mz) = (c + ux * mid, c + uz * mid);
            out.push((
                mid,
                terrain::height(seed, mx, mz) >= LAND_MIN_H
                    && terrain::slope(seed, mx, mz) <= CLIFF_SLOPE_RATIO,
            ));
            start = -1.0;
        }
        r += RSTEP;
    }
}

/// Biggest unbroken arc, as a share, under (ignore_cliff, gap tolerance).
fn arc(all: &[Vec<(f32, bool)>], ignore_cliff: bool, tol: f32) -> f32 {
    let mut broken = vec![false; STEPS];
    for b in 0..STEPS {
        let n = (b + 1) % STEPS;
        let pick = |v: &Vec<(f32, bool)>| -> Vec<f32> {
            v.iter()
                .filter(|r| ignore_cliff || r.1)
                .map(|r| r.0)
                .collect()
        };
        let (a, c2) = (pick(&all[b]), pick(&all[n]));
        if a.is_empty() || c2.is_empty() {
            broken[b] = true;
            continue;
        }
        let mut gap = f32::MAX;
        for &r0 in &a {
            for &r1 in &c2 {
                gap = gap.min(((r0 - r1).abs() - ROAD_HALF_W * 2.0).max(0.0));
            }
        }
        if gap > tol {
            broken[b] = true;
        }
    }
    let (mut best, mut run) = (0usize, 0usize);
    for b in 0..STEPS * 2 {
        if broken[b % STEPS] {
            run = 0;
        } else {
            run += 1;
            best = best.max(run);
        }
    }
    100.0 * best.min(STEPS) as f32 / STEPS as f32
}

fn main() {
    compare();

    println!(
        "{:>12} {:>6} {:>7} {:>6} {:>7} {:>8} {:>9} {:>9} {:>9}",
        "seed", "forks", "absent", "cliff", "anggap", "breaks", "today", "nocliff", "gap<=25m"
    );
    let mut acc = [0.0f32; 3];
    for seed in SEEDS {
        let mut all = vec![Vec::new(); STEPS];
        let mut scratch = Vec::new();
        let (mut forks, mut absent, mut cliff, mut anggap) = (0usize, 0usize, 0usize, 0usize);
        for (b, slot) in all.iter_mut().enumerate() {
            let a = b as f32 / STEPS as f32 * core::f32::consts::TAU;
            let (uz, ux) = a.sin_cos();
            runs_on(seed, ux, uz, &mut scratch);
            if scratch.len() > 1 {
                forks += 1;
            }
            *slot = scratch.clone();
        }
        let mut hist: Vec<f32> = Vec::new();
        for b in 0..STEPS {
            let n = (b + 1) % STEPS;
            if all[b].is_empty() || all[n].is_empty() {
                absent += 1;
                continue;
            }
            let pick = |v: &Vec<(f32, bool)>| -> Vec<f32> {
                v.iter().filter(|r| r.1).map(|r| r.0).collect()
            };
            let (a, c2) = (pick(&all[b]), pick(&all[n]));
            if a.is_empty() || c2.is_empty() {
                cliff += 1;
                continue;
            }
            let mut gap = f32::MAX;
            for &r0 in &a {
                for &r1 in &c2 {
                    gap = gap.min(((r0 - r1).abs() - ROAD_HALF_W * 2.0).max(0.0));
                }
            }
            hist.push(gap);
            if gap > ROAD_HALF_W * 2.0 {
                anggap += 1;
            }
        }
        hist.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let v = [
            arc(&all, false, ROAD_HALF_W * 2.0),
            arc(&all, true, ROAD_HALF_W * 2.0),
            arc(&all, false, 25.0),
        ];
        for (i, x) in v.iter().enumerate() {
            acc[i] += x;
        }
        println!(
            "{seed:>12} {forks:>6} {absent:>7} {cliff:>6} {anggap:>7} {:>8} {:>8.1}% {:>8.1}% {:>8.1}%",
            absent + cliff + anggap, v[0], v[1], v[2]
        );
        let q = |f: f32| hist[((hist.len() - 1) as f32 * f) as usize];
        println!(
            "             gap p50 {:.1} p90 {:.1} p99 {:.1} max {:.1} m",
            q(0.5),
            q(0.9),
            q(0.99),
            hist[hist.len() - 1]
        );
    }
    let n = SEEDS.len() as f32;
    println!(
        "{:>12} {:>47} {:>8.1}% {:>8.1}% {:>8.1}%  <- mean",
        "",
        "",
        acc[0] / n,
        acc[1] / n,
        acc[2] / n
    );
    grid_trap();
}

/// **Trap 2: a component count over ring cells measures the GRID.**
///
/// Same islands, same 8-neighbour flood, two grids. The `biggest comp` column
/// disagrees between them, so no piece count from either is a measurement —
/// and the `share` column, which agrees to a tenth of a point, is what
/// `tests/road.rs::the_ring_is_ground_a_player_can_stand_on` gates instead.
fn grid_trap() {
    println!("\ncomponent count vs grid, and the share that survives both:");
    println!(
        "{:>12} {:>8} {:>10} {:>12} {:>10}",
        "grid", "cells", "standable", "biggest comp", "share"
    );
    for grid in [2.0f32, 4.0] {
        let side = (ISLAND_SIZE / grid) as usize;
        let (mut cell_t, mut walk_t, mut big_t) = (0u64, 0u64, 0u64);
        for seed in SEEDS {
            let mut ok = vec![false; side * side];
            let (mut cells, mut walk) = (0u64, 0u64);
            for iz in 0..side {
                for ix in 0..side {
                    let (x, z) = (ix as f32 * grid, iz as f32 * grid);
                    if terrain::ring_probe(seed, x, z) != RoadBand::Carriageway {
                        continue;
                    }
                    cells += 1;
                    if terrain::height(seed, x, z) >= LAND_MIN_H
                        && terrain::slope(seed, x, z) <= CLIFF_SLOPE_RATIO
                    {
                        ok[iz * side + ix] = true;
                        walk += 1;
                    }
                }
            }
            let mut seen = vec![false; side * side];
            let mut biggest = 0u64;
            let mut stack: Vec<usize> = Vec::new();
            for start in 0..side * side {
                if !ok[start] || seen[start] {
                    continue;
                }
                seen[start] = true;
                stack.clear();
                stack.push(start);
                let mut sz = 0u64;
                while let Some(p) = stack.pop() {
                    sz += 1;
                    let (px, pz) = ((p % side) as i32, (p / side) as i32);
                    for dz in -1..=1i32 {
                        for dx in -1..=1i32 {
                            let (jx, jz) = (px + dx, pz + dz);
                            if (dx == 0 && dz == 0)
                                || jx < 0
                                || jz < 0
                                || jx >= side as i32
                                || jz >= side as i32
                            {
                                continue;
                            }
                            let j = jz as usize * side + jx as usize;
                            if ok[j] && !seen[j] {
                                seen[j] = true;
                                stack.push(j);
                            }
                        }
                    }
                }
                biggest = biggest.max(sz);
            }
            cell_t += cells;
            walk_t += walk;
            big_t += biggest;
        }
        println!(
            "{:>11}m {:>8} {:>10} {:>11.1}% {:>9.1}%",
            grid,
            cell_t,
            walk_t,
            100.0 * big_t as f32 / walk_t as f32,
            100.0 * walk_t as f32 / cell_t as f32
        );
    }
}

// ── Both rings, one pitch (ring path v0's before and after) ──────────────

fn standable(seed: u64, x: f32, z: f32) -> bool {
    terrain::height(seed, x, z) >= LAND_MIN_H && terrain::slope(seed, x, z) <= CLIFF_SLOPE_RATIO
}

/// (standable share, longest unbroken run in metres, total length in metres)
fn walk(pts: &[(f32, f32)], seed: u64) -> (f32, f32, f32) {
    let mut ok: Vec<bool> = Vec::new();
    let mut seg: Vec<f32> = Vec::new();
    for i in 0..pts.len() {
        let (ax, az) = pts[i];
        let (bx, bz) = pts[(i + 1) % pts.len()];
        let len = ((bx - ax).powi(2) + (bz - az).powi(2)).sqrt();
        let n = (len / PITCH) as usize + 1;
        for k in 0..n {
            let t = k as f32 / n as f32;
            ok.push(standable(seed, ax + (bx - ax) * t, az + (bz - az) * t));
            seg.push(len / n as f32);
        }
    }
    let total: f32 = seg.iter().sum();
    let good: f32 = seg
        .iter()
        .zip(&ok)
        .filter(|(_, o)| **o)
        .map(|(s, _)| *s)
        .sum();
    let (mut best, mut run) = (0.0f32, 0.0f32);
    for i in 0..ok.len() * 2 {
        let j = i % ok.len();
        if ok[j] {
            run += seg[j];
            best = best.max(run)
        } else {
            run = 0.0
        }
    }
    (100.0 * good / total, best.min(total), total)
}

/// The predicate ring's centre line: per bearing, the mid radius of its
/// innermost carriageway run.
fn probe_line(seed: u64, steps: usize) -> Vec<(f32, f32)> {
    let c = ISLAND_SIZE * 0.5;
    let mut out = Vec::new();
    for b in 0..steps {
        let a = b as f32 / steps as f32 * core::f32::consts::TAU;
        let (uz, ux) = a.sin_cos();
        let (mut r, mut start) = (ROAD_R_MIN, -1.0f32);
        while r <= ROAD_R_MAX + 0.5 {
            let on = r <= ROAD_R_MAX
                && terrain::ring_probe(seed, c + ux * r, c + uz * r) == RoadBand::Carriageway;
            if on && start < 0.0 {
                start = r;
            } else if !on && start >= 0.0 {
                let mid = (start + r - 0.5) * 0.5;
                out.push((c + ux * mid, c + uz * mid));
                break;
            }
            r += 0.5;
        }
    }
    out
}

fn compare() {
    println!(
        "{:>12} {:>22} {:>24}",
        "seed", "PREDICATE (today)", "SOLVED PATH"
    );
    println!(
        "{:>12} {:>8} {:>7} {:>6} {:>8} {:>7} {:>6}",
        "", "stand%", "run m", "len m", "stand%", "run m", "len m"
    );
    let (mut a0, mut b0, mut a1, mut b1) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for seed in SEEDS {
        let h = terrain::haven(seed);
        let (ps, pr, pl) = walk(&probe_line(seed, RING_BEARINGS * 8), seed);
        let solved: Vec<(f32, f32)> = (0..RING_BEARINGS).map(|i| h.ring.node(i as i32)).collect();
        let (ss, sr, sl) = walk(&solved, seed);
        a0 += ps;
        b0 += 100.0 * pr / pl;
        a1 += ss;
        b1 += 100.0 * sr / sl;
        println!("{seed:>12} {ps:>7.1}% {pr:>6.0} {pl:>6.0} {ss:>7.1}% {sr:>6.0} {sl:>6.0}");
    }
    let k = SEEDS.len() as f32;
    println!(
        "{:>12} {:>7.1}% {:>6.1}%        {:>7.1}% {:>6.1}%   <- mean (run as % of ring)",
        "",
        a0 / k,
        b0 / k,
        a1 / k,
        b1 / k
    );
}
