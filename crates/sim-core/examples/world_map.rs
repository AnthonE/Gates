//! A top-down picture of one seed's island: relief shading, water, the ring
//! road, the side roads, the landmark trails, the landmarks and the rock
//! formations. Writes a binary PPM.
//!
//!   cargo run --release -p sim-core --example world_map -- [seed] [out.ppm] [px]

// Host-side tool: printing, timing and writing a file are its job. The sim
// walls ban them in SIM code; an example binary is not sim code.
#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    clippy::disallowed_types
)]

use sim_core::boulder;
use sim_core::terrain::{self, RoadBand};
use std::io::Write;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seed: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20260731);
    let out = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "world_map.ppm".into());
    let px: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1024);

    let t = Instant::now();
    let haven = terrain::haven(seed);
    eprintln!("haven solved in {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    let marks = haven.marks.iter().filter(|m| m.live).count();
    let trails = haven.trails.iter().filter(|r| r.live).count();
    eprintln!("landmarks: {marks}, trails: {trails}");
    for m in haven.marks.iter().filter(|m| m.live) {
        eprintln!("  {:?} at ({:.0}, {:.0}) y {:.1}", m.kind, m.x, m.z, m.y);
    }

    let s = terrain::ISLAND_SIZE / px as f32;
    let mut img = vec![[0u8; 3]; px * px];
    let mut lat = terrain::Lattice::new();
    for j in 0..px {
        for i in 0..px {
            let (x, z) = ((i as f32 + 0.5) * s, (j as f32 + 0.5) * s);
            let h = terrain::height_memo(&mut lat, seed, x, z);
            let c = if h < terrain::SEA_LEVEL {
                [30, 60, 110]
            } else {
                let hx = terrain::height_memo(&mut lat, seed, x + s, z) - h;
                let hz = terrain::height_memo(&mut lat, seed, x, z + s) - h;
                let shade = (0.75 - (hx + hz) / s * 0.8).clamp(0.2, 1.2);
                let base = if h < terrain::BEACH_MAX_H {
                    [200.0, 190.0, 150.0]
                } else if h > terrain::TREELINE_H {
                    [160.0, 155.0, 150.0]
                } else {
                    [90.0 + h, 130.0 + h * 0.5, 70.0]
                };
                [
                    (base[0] * shade).min(255.0) as u8,
                    (base[1] * shade).min(255.0) as u8,
                    (base[2] * shade).min(255.0) as u8,
                ]
            };
            img[j * px + i] = c;
            if terrain::ring_band(&haven.ring, x, z) != RoadBand::Off {
                img[j * px + i] = [40, 40, 40];
            } else if terrain::side_band(&haven, x, z) != RoadBand::Off {
                img[j * px + i] = [140, 90, 50];
            }
        }
    }
    let mut blocks = 0usize;
    let n = boulder::cells_per_side();
    for bz in 0..n {
        for bx in 0..n {
            let f = boulder::formation(seed, &haven, bx, bz);
            for b in f.iter() {
                blocks += 1;
                let r = (b.radius() / s).max(1.0) as i32;
                let (ci, cj) = ((b.x / s) as i32, (b.z / s) as i32);
                for dj in -r..=r {
                    for di in -r..=r {
                        let (wx, wz) = ((ci + di) as f32 * s, (cj + dj) as f32 * s);
                        if boulder::surface(b, wx, wz, s * 0.5).is_none() {
                            continue;
                        }
                        let (ii, jj) = (ci + di, cj + dj);
                        if ii >= 0 && jj >= 0 && (ii as usize) < px && (jj as usize) < px {
                            img[jj as usize * px + ii as usize] = [120, 110, 100];
                        }
                    }
                }
            }
        }
    }
    eprintln!("rock blocks: {blocks}");
    for m in haven.marks.iter().filter(|m| m.live) {
        let r = (sim_core::landmark::LANDMARK_R_M / s).max(2.0) as i32;
        let (ci, cj) = ((m.x / s) as i32, (m.z / s) as i32);
        for dj in -r..=r {
            for di in -r..=r {
                let (ii, jj) = (ci + di, cj + dj);
                if ii >= 0 && jj >= 0 && (ii as usize) < px && (jj as usize) < px {
                    img[jj as usize * px + ii as usize] = [230, 40, 40];
                }
            }
        }
    }
    for site in std::iter::once((haven.x, haven.z))
        .chain(haven.minor.iter().filter(|w| w.live).map(|w| (w.x, w.z)))
    {
        let (ci, cj) = ((site.0 / s) as i32, (site.1 / s) as i32);
        for dj in -4..=4 {
            for di in -4..=4 {
                let (ii, jj) = (ci + di, cj + dj);
                if ii >= 0 && jj >= 0 && (ii as usize) < px && (jj as usize) < px {
                    img[jj as usize * px + ii as usize] = [250, 220, 40];
                }
            }
        }
    }
    let mut f = std::fs::File::create(&out).expect("create output");
    write!(f, "P6\n{px} {px}\n255\n").expect("header");
    for p in &img {
        f.write_all(p).expect("pixels");
    }
    eprintln!("wrote {out}");
}
