//! Dev probe: the Verdigris over a wipe (`blight.rs`), top down, as PPM —
//! one map per (hour, tier) and the share of the land it holds, so the
//! spread's pacing is looked at rather than guessed.
//!
//! Usage: blight_map <seed> <salt> <px> <out_prefix>
// Host-side probe: printing and file I/O are its job, not the sim's.
#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_types,
    clippy::disallowed_methods
)]

use sim_core::blight::{self, Plan};
use sim_core::terrain::{self, ISLAND_SIZE};

fn arg<T: std::str::FromStr>(i: usize, d: T) -> T {
    std::env::args()
        .nth(i)
        .and_then(|s| s.parse().ok())
        .unwrap_or(d)
}

fn write_ppm(path: &str, px: usize, rgb: &[u8]) {
    use std::io::Write;
    let mut f = std::fs::File::create(path).expect("create");
    write!(f, "P6\n{px} {px}\n255\n").expect("hdr");
    f.write_all(rgb).expect("body");
}

/// The moments a wipe is looked at: hour, fel tier.
const MOMENTS: [(u64, u8); 6] = [(0, 0), (24, 1), (48, 1), (96, 1), (168, 1), (168, 2)];

fn main() {
    let seed: u64 = arg(1, 20260731);
    let salt: u32 = arg(2, 1);
    let px: usize = arg(3, 512);
    let prefix: String = std::env::args().nth(4).unwrap_or_else(|| "blight".into());

    let haven = terrain::haven(seed);
    let plan = Plan::new(seed, salt, &haven);
    let step = ISLAND_SIZE / px as f32;
    println!("seed {seed} salt {salt}: {} outbreaks", plan.n);
    for s in plan.sites.iter().take(plan.n as usize) {
        println!(
            "  outbreak at ({:.0}, {:.0}) from hour {:.1}, radius {:.0} m",
            s.x, s.z, s.start_h, s.r
        );
    }

    // The terrain once: height and the heart's wander noise per texel.
    let mut ground = vec![0f32; px * px];
    let mut noise = vec![0f32; px * px];
    for iz in 0..px {
        for ix in 0..px {
            let x = (ix as f32 + 0.5) * step;
            let z = (iz as f32 + 0.5) * step;
            ground[iz * px + ix] = terrain::ground(seed, &haven, x, z);
            noise[iz * px + ix] = terrain::fel_wander(seed, x, z);
        }
    }

    for (h, tier) in MOMENTS {
        let now = blight::now(&plan, &haven, h * blight::TICKS_PER_HOUR, tier);
        let mut img = vec![0u8; px * px * 3];
        let (mut land, mut half, mut quarter, mut hot) = (0u32, 0u32, 0u32, 0u32);
        for iz in 0..px {
            for ix in 0..px {
                let k = iz * px + ix;
                let x = (ix as f32 + 0.5) * step;
                let z = (iz as f32 + 0.5) * step;
                let g = ground[k];
                // North up: the map's row 0 is the island's far z.
                let i = ((px - 1 - iz) * px + ix) * 3;
                if g < terrain::LAND_MIN_H {
                    img[i..i + 3].copy_from_slice(&[20, 40, 70]);
                    continue;
                }
                land += 1;
                let d = blight::depth_n(&now, x, z, g, noise[k]);
                if d.fel >= 0.5 {
                    half += 1;
                }
                if d.fel >= 0.25 {
                    quarter += 1;
                }
                if d.hot > 0.25 {
                    hot += 1;
                }
                let shade = 90.0 + (g / 180.0).clamp(0.0, 1.0) * 110.0;
                let base = [shade * 0.8, shade, shade * 0.75];
                let ash = [25.0f32, 120.0, 35.0];
                let burn = [210.0f32, 230.0, 40.0];
                for (j, out) in img[i..i + 3].iter_mut().enumerate() {
                    let mut c = base[j];
                    c += (ash[j] - c) * d.fel.min(1.0);
                    c += (burn[j] - c) * d.hot.min(1.0);
                    *out = c as u8;
                }
            }
        }
        // THE GATE's ward, a ring.
        let (tx, tz) = blight::ward_centre(&haven);
        for a in 0..720 {
            let t = a as f32 * (std::f32::consts::TAU / 720.0);
            for r in [blight::WARD_IN_M, blight::WARD_OUT_M] {
                let (x, z) = (tx + r * t.cos(), tz + r * t.sin());
                let (ix, iz) = ((x / step) as i64, (z / step) as i64);
                if ix >= 0 && iz >= 0 && (ix as usize) < px && (iz as usize) < px {
                    let i = ((px - 1 - iz as usize) * px + ix as usize) * 3;
                    img[i..i + 3].copy_from_slice(&[255, 255, 255]);
                }
            }
        }
        let path = format!("{prefix}_h{h:03}_t{tier}.ppm");
        write_ppm(&path, px, &img);
        let pc = |n: u32| 100.0 * n as f32 / land.max(1) as f32;
        println!(
            "hour {h:3} tier {tier}: front {:.3}, {:.1}% of land ≥ 0.5, {:.1}% ≥ 0.25, hot {:.2}% — {path}",
            now.front,
            pc(half),
            pc(quarter),
            pc(hot)
        );
    }
}
