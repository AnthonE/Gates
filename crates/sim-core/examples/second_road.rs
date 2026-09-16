#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_types,
    clippy::disallowed_methods
)]
//! Dev probe: what would a SECOND road be, and would anyone walk it?
//!
//! Not a gate. It exists because "can we add more roads" has an arithmetic
//! answer and two plausible ones are wrong.
//!
//! The coast ring works because the shoreline is a curve the terrain already
//! defines and `road_band` can invert locally (it never asks *where* the ring
//! is, only *am I on it*). A second road has to find its own reference curve,
//! and several can be written purely — a height contour, a radial ray, a
//! segment between two sites. What separates them is not expressibility. It
//! is whether the result is **walkable** and whether it **goes anywhere**.
//!
//! Measured on the shipped seed, 2 m grid, at the coast ring's own band
//! (`ROAD_SHOULDER_HALF_W`) so the comparison is like for like:
//!
//! | candidate | area | slope mean | unwalkable | walkable-connected |
//! |---|---|---|---|---|
//! | coast ring (today) | 5.4 ha | 0.450 | 2.9% | 79% in one piece |
//! | contour ring @ 25 m | 11.0 ha | 0.480 | 1.6% | 85% |
//! | 6 radial spokes | 5.3 ha | 0.247 | 1.6% | 80% |
//! | 2 site-to-site chords | 1.6 ha | 0.126 | 0.0% | 100% |
//!
//! ⚠ **Three findings, and two of them refuted the guess that preceded them.**
//!
//! 1. **Spokes do not climb cliffs.** A straight radial across generated
//!    terrain reads as a bad idea and is not one: 48% of this island's land
//!    sits between 10 and 20 m, so a line across the interior is shelf
//!    country. Spokes come out *flatter* than the shipped coast ring
//!    (0.247 against 0.450 mean slope) and no worse connected.
//! 2. **Chords between the authored sites are useless.** They are the
//!    flattest and most connected thing here — and they save **3–5%** of the
//!    walk, because all three sites sit on ONE ring and a chord subtending a
//!    small angle is very nearly its own arc. A parallel path, not a route.
//! 3. **The connectivity that matters is over walkable cells only.** Filling
//!    through cliff cells says every candidate is one piece; filling around
//!    them says the *shipped* coast ring is 79% in one piece, which is the
//!    bar — not 100%.
//!
//! And the number that says a second road is wanted at all, which is about
//! the island rather than about any candidate: **38% of walkable land is more
//! than 300 m of walking from any road** (p50 218 m, p90 570 m, max 782 m).
//! The interior has no route. Adding spokes to the ring and re-running the
//! same walk gives the dose-response the `--spokes` argument prints:
//!
//! | spokes | p50 | p90 | >300 m |
//! |---|---|---|---|
//! | 0 | 218 m | 570 m | 38% |
//! | 4 | 96 m | 254 m | 4% |
//! | 6 | 78 m | 208 m | **0%** |
//! | 12 | 46 m | 126 m | 0% |
//!
//! Six is the knee.
//!
//! Usage: `second_road [seed] [spokes-for-the-reach-measurement]`

use sim_core::terrain::{self, CLIFF_SLOPE_RATIO, ISLAND_SIZE};

/// The band a candidate is measured at — the same one `road_band` reports as
/// on-road, so the areas below are comparable to the ring's own.
const W: f32 = 5.0;
/// Sample step. Must be well under `W` or a straight line aliases into dashes
/// and every connectivity number is a property of the grid — which is exactly
/// what the first run of this file measured, at 4 m against a 2 m half-width.
const STEP: f32 = 2.0;
const N: usize = (ISLAND_SIZE as usize) / (STEP as usize);

/// Connected components of `on`, 4-neighbour, optionally refusing to cross
/// cells a body cannot walk.
fn components(on: &[bool], slope: &[f32], walkable_only: bool) -> Vec<u32> {
    let mut seen = vec![false; on.len()];
    let mut out: Vec<u32> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let ok = |i: usize| on[i] && (!walkable_only || slope[i] <= CLIFF_SLOPE_RATIO);
    for s in 0..on.len() {
        if !ok(s) || seen[s] {
            continue;
        }
        let mut size = 0u32;
        seen[s] = true;
        stack.push(s);
        while let Some(p) = stack.pop() {
            size += 1;
            let (px, pz) = (p % N, p / N);
            for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let (jx, jz) = (px as i32 + dx, pz as i32 + dz);
                if jx < 0 || jz < 0 || jx >= N as i32 || jz >= N as i32 {
                    continue;
                }
                let j = jz as usize * N + jx as usize;
                if ok(j) && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        out.push(size);
    }
    out.sort_unstable_by(|a, b| b.cmp(a));
    out
}

fn report(name: &str, on: &[bool], slope: &[f32]) {
    let live: Vec<usize> = (0..on.len()).filter(|&i| on[i]).collect();
    if live.is_empty() {
        println!("  {name:<20} EMPTY");
        return;
    }
    let n = live.len() as f64;
    let sum: f64 = live.iter().map(|&i| f64::from(slope[i])).sum();
    let worst = live.iter().fold(0.0f32, |a, &i| a.max(slope[i]));
    let steep = live
        .iter()
        .filter(|&&i| slope[i] > CLIFF_SLOPE_RATIO)
        .count();
    let all = components(on, slope, false);
    let walk = components(on, slope, true);
    let wtotal: u32 = walk.iter().sum();
    println!(
        "  {name:<20} {:>6} cells ({:>5.1} ha)  slope mean {:.3} max {:.2}  unwalkable {:.1}%",
        live.len(),
        n * f64::from(STEP * STEP) / 10_000.0,
        sum / n,
        worst,
        steep as f64 * 100.0 / n,
    );
    println!(
        "  {:<20} pieces {} (largest {:.0}%)   WALKABLE-ONLY pieces {} (largest {:.0}%)",
        "",
        all.len(),
        f64::from(all[0]) * 100.0 / n,
        walk.len(),
        f64::from(walk.first().copied().unwrap_or(0)) * 100.0 / f64::from(wtotal.max(1)),
    );
}

/// Perpendicular distance from `(dx, dz)` to the ray from the origin along
/// unit `(sx, sz)`, or `None` behind the ray's start.
///
/// **Trig-free by construction** — a cross product against a `yaw_lut` unit
/// vector, which is wall 1's only angle source. This is the whole of what a
/// spoke costs: no height tap, no memo, no state.
fn ray_offset(dx: f32, dz: f32, sx: f32, sz: f32) -> Option<f32> {
    if dx * sx + dz * sz <= 0.0 {
        return None;
    }
    let perp = dx * sz - dz * sx;
    Some(if perp < 0.0 { -perp } else { perp })
}

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20260731);
    let reach_spokes: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(6);
    let haven = terrain::haven(seed);
    let c = ISLAND_SIZE * 0.5;

    let mut h = vec![0.0f32; N * N];
    let mut sl = vec![0.0f32; N * N];
    let mut land = vec![false; N * N];
    for iz in 0..N {
        for ix in 0..N {
            let (x, z) = (ix as f32 * STEP, iz as f32 * STEP);
            let i = iz * N + ix;
            h[i] = terrain::ground(seed, &haven, x, z);
            sl[i] = terrain::ground_slope(seed, &haven, x, z);
            land[i] = h[i] >= terrain::LAND_MIN_H;
        }
    }
    println!("seed {seed}: second-road candidates, half-width {W} m on a {STEP} m grid");

    let mut on_ring = vec![false; N * N];
    for iz in 0..N {
        for ix in 0..N {
            let i = iz * N + ix;
            on_ring[i] = land[i]
                && terrain::ring_band(seed, ix as f32 * STEP, iz as f32 * STEP)
                    != terrain::RoadBand::Off;
        }
    }
    report("coast ring", &on_ring, &sl);

    // A height contour, as the perpendicular distance to a level set: to
    // first order that is |h - H| / |grad h|, and `scatter` already resolves
    // both terms on every cell it draws.
    for hh in [16.0f32, 20.0, 25.0, 32.0] {
        let mut on = vec![false; N * N];
        for i in 0..N * N {
            let d = h[i] - hh;
            on[i] = land[i] && (if d < 0.0 { -d } else { d }) < W * sl[i];
        }
        report(&format!("contour {hh:.0} m"), &on, &sl);
    }

    for n in [4usize, 6, 8] {
        let mut on = vec![false; N * N];
        for iz in 0..N {
            for ix in 0..N {
                let i = iz * N + ix;
                if !land[i] {
                    continue;
                }
                let (dx, dz) = (ix as f32 * STEP - c, iz as f32 * STEP - c);
                on[i] = (0..n).any(|k| {
                    let (sx, sz) = sim_core::yaw_dir(((k * 256 / n) as u16) << 8);
                    ray_offset(dx, dz, sx, sz).is_some_and(|p| p < W)
                });
            }
        }
        report(&format!("{n} spokes"), &on, &sl);
    }

    // Chords: the pad to each live waystation, point-to-segment.
    let segs: Vec<(f32, f32, f32, f32)> = haven
        .minor
        .iter()
        .filter(|w| w.live)
        .map(|ws| (haven.x, haven.z, ws.x, ws.z))
        .collect();
    {
        let mut on = vec![false; N * N];
        for iz in 0..N {
            for ix in 0..N {
                let i = iz * N + ix;
                if !land[i] {
                    continue;
                }
                let (px, pz) = (ix as f32 * STEP, iz as f32 * STEP);
                on[i] = segs.iter().any(|&(ax, az, bx, bz)| {
                    let (vx, vz) = (bx - ax, bz - az);
                    let t =
                        (((px - ax) * vx + (pz - az) * vz) / (vx * vx + vz * vz)).clamp(0.0, 1.0);
                    let (ex, ez) = (px - (ax + vx * t), pz - (az + vz * t));
                    (ex * ex + ez * ez).sqrt() < W
                });
            }
        }
        report(&format!("{} chords", segs.len()), &on, &sl);
    }

    // What a chord would actually save. The answer is why they are not the
    // road this island wants: three sites on one ring means a chord subtends
    // a small angle, and a small chord is very nearly its own arc.
    for (k, &(ax, az, bx, bz)) in segs.iter().enumerate() {
        let straight = ((bx - ax) * (bx - ax) + (bz - az) * (bz - az)).sqrt();
        let rr = (((ax - c) * (ax - c) + (az - c) * (az - c)).sqrt()
            + ((bx - c) * (bx - c) + (bz - c) * (bz - c)).sqrt())
            * 0.5;
        // arc = 2r·asin(s/2r), by series — a ratio quoted to one decimal does
        // not need libm, and wall 1 does not have it.
        let u = (straight / (2.0 * rr)).clamp(0.0, 1.0);
        let arc = 2.0 * rr * (u + u * u * u / 6.0 + 3.0 * u * u * u * u * u / 40.0);
        let steps = (straight / STEP) as i32;
        let wet = (0..=steps)
            .filter(|&t| {
                let f = t as f32 / steps as f32;
                terrain::ground(seed, &haven, ax + (bx - ax) * f, az + (bz - az) * f)
                    < terrain::LAND_MIN_H
            })
            .count();
        println!(
            "  chord {k}: {straight:.0} m against a ~{arc:.0} m arc — saves {:.0}%; {:.1}% of it over water",
            (1.0 - straight / arc) * 100.0,
            wet as f64 * 100.0 / f64::from(steps + 1),
        );
    }

    // ── Is any of the island out of reach of the road it already has? ─────
    //
    // A multi-source walk from every road cell, refusing cliffs, so the
    // answer is metres of WALKING rather than metres of straight line.
    let mut dist = vec![u32::MAX; N * N];
    let mut q: Vec<usize> = Vec::new();
    for iz in 0..N {
        for ix in 0..N {
            let i = iz * N + ix;
            if !land[i] {
                continue;
            }
            let (dx, dz) = (ix as f32 * STEP - c, iz as f32 * STEP - c);
            let on_spoke = reach_spokes > 0
                && (0..reach_spokes).any(|k| {
                    let (sx, sz) = sim_core::yaw_dir(((k * 256 / reach_spokes) as u16) << 8);
                    ray_offset(dx, dz, sx, sz).is_some_and(|p| p < W)
                });
            if on_ring[i] || on_spoke {
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
            if land[j] && sl[j] <= CLIFF_SLOPE_RATIO && dist[j] == u32::MAX {
                dist[j] = dist[p] + 1;
                q.push(j);
            }
        }
    }
    let mut d: Vec<u32> = (0..N * N)
        .filter(|&i| land[i] && sl[i] <= CLIFF_SLOPE_RATIO && dist[i] != u32::MAX)
        .map(|i| dist[i])
        .collect();
    let stranded = (0..N * N)
        .filter(|&i| land[i] && sl[i] <= CLIFF_SLOPE_RATIO && dist[i] == u32::MAX)
        .count();
    d.sort_unstable();
    let at = |f: f32| d[((d.len() - 1) as f32 * f) as usize] as f32 * STEP;
    println!(
        "\n  reach with the ring + {reach_spokes} spokes: walk to the nearest road \
         p50 {:.0} m  p90 {:.0} m  p99 {:.0} m  max {:.0} m",
        at(0.5),
        at(0.9),
        at(0.99),
        at(1.0)
    );
    println!(
        "  {:.0}% of walkable land is over 300 m of walking from a road; {stranded} cells reach none",
        d.iter().filter(|&&v| v as f32 * STEP > 300.0).count() as f64 * 100.0 / d.len() as f64,
    );
    println!(
        "  island centre: ground {:.2} m, slope {:.3}, biome {:?}",
        terrain::ground(seed, &haven, c, c),
        terrain::ground_slope(seed, &haven, c, c),
        terrain::biome(
            terrain::ground(seed, &haven, c, c),
            terrain::moisture(seed, c, c)
        )
    );
}
