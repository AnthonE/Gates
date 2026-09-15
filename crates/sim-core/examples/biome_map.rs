//! Dev probe: dumps top-down maps of the island's biome, ground splat and
//! scatter occupancy as PPM, so the STRUCTURE of the world can be looked at
//! without a GPU.
//!
//! Not a gate. `reference/FORESTS.md` §8 is that every gate we have is
//! island-wide and an island-wide total is blind to per-biome structure; the
//! same blindness applies to a human reading a histogram. A forest that is
//! one 700 m blob and a forest that is twenty stands read identically in
//! `terrain_stats`' `forest 16321`, and differently here.
//!
//! Usage: biome_map <seed> <px> <out_prefix>

// Host-side probe: printing and file I/O are its job, not the sim's.
#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_types,
    clippy::disallowed_methods
)]

use sim_core::terrain::{
    self, Biome, Occupant, ScatterTable, CELLS_PER_SIDE, CELL_SIZE, ISLAND_SIZE,
};

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
    println!("wrote {path}");
}

fn main() {
    let seed: u64 = arg(1, 20260731);
    let px: usize = arg(2, 1024);
    let prefix: String = std::env::args().nth(3).unwrap_or_else(|| "bm".into());

    let haven = terrain::haven(seed);
    let table = ScatterTable::alpha_default();
    let step = ISLAND_SIZE / px as f32;

    let mut biome_img = vec![0u8; px * px * 3];
    let mut splat_img = vec![0u8; px * px * 3];
    let mut clump_img = vec![0u8; px * px * 3];

    for iz in 0..px {
        for ix in 0..px {
            let x = (ix as f32 + 0.5) * step;
            let z = (iz as f32 + 0.5) * step;
            let h = terrain::ground(seed, &haven, x, z);
            let i = (iz * px + ix) * 3;

            if h < terrain::LAND_MIN_H {
                let d = ((-h) / 40.0).clamp(0.0, 1.0);
                biome_img[i] = 10;
                biome_img[i + 1] = (60.0 - 40.0 * d) as u8;
                biome_img[i + 2] = (130.0 - 60.0 * d) as u8;
                splat_img[i..i + 3].copy_from_slice(&biome_img[i..i + 3]);
                clump_img[i..i + 3].copy_from_slice(&biome_img[i..i + 3]);
                continue;
            }

            let m = terrain::moisture(seed, x, z);
            let sl = terrain::ground_slope(seed, &haven, x, z);
            let c = match terrain::biome(h, m) {
                Biome::Beach => [230u8, 214, 150],
                Biome::Meadow => [124, 168, 82],
                Biome::Forest => [34, 84, 44],
                Biome::Highland => [150, 146, 140],
            };
            biome_img[i..i + 3].copy_from_slice(&c);

            // The splat as the ground actually mixes it: sand / grass /
            // litter / rock, each at its texture's rough albedo.
            let w = terrain::splat_from(h, m, sl);
            const CH: [[f32; 3]; 4] = [
                [222.0, 203.0, 156.0],
                [110.0, 142.0, 74.0],
                [78.0, 72.0, 48.0],
                [136.0, 132.0, 126.0],
            ];
            let mut acc = [0.0f32; 3];
            for k in 0..4 {
                let a = w[k] as f32 * (1.0 / 255.0);
                for ch in 0..3 {
                    acc[ch] += a * CH[k][ch];
                }
            }
            for ch in 0..3 {
                splat_img[i + ch] = acc[ch].clamp(0.0, 255.0) as u8;
            }

            // The grove field, as the forest keeps it.
            let g = terrain::clump(seed, x, z);
            let t = (g / 2.7).clamp(0.0, 1.0);
            clump_img[i] = (255.0 * t) as u8;
            clump_img[i + 1] = (255.0 * t * t) as u8;
            clump_img[i + 2] = 40;
        }
    }

    // Occupancy over the biome map: one dot per live slot.
    let mut occ_img = biome_img.clone();
    let mut counts = [0u32; 13];
    for cz in 0..CELLS_PER_SIDE {
        for cx in 0..CELLS_PER_SIDE {
            let s = terrain::scatter(seed, &table, &haven, cx, cz);
            if s.occupant == Occupant::None {
                continue;
            }
            counts[s.occupant as usize] += 1;
            let ix = (s.x / ISLAND_SIZE * px as f32) as usize;
            let iz = (s.z / ISLAND_SIZE * px as f32) as usize;
            if ix >= px || iz >= px {
                continue;
            }
            let c: [u8; 3] = match s.occupant {
                Occupant::Tree => [40, 255, 90],
                Occupant::Bush => [190, 230, 60],
                Occupant::Rock => [220, 220, 220],
                Occupant::StoneNode => [120, 160, 255],
                Occupant::MetalNode => [255, 170, 60],
                Occupant::SulfurNode => [255, 240, 60],
                _ => [255, 60, 200],
            };
            let r = if s.occupant == Occupant::Tree {
                0i32
            } else {
                1
            };
            for dz in -r..=r {
                for dx in -r..=r {
                    let (jx, jz) = (ix as i32 + dx, iz as i32 + dz);
                    if jx < 0 || jz < 0 || jx >= px as i32 || jz >= px as i32 {
                        continue;
                    }
                    let j = (jz as usize * px + jx as usize) * 3;
                    occ_img[j..j + 3].copy_from_slice(&c);
                }
            }
        }
    }

    // A species map: which kind of tree stands where, drawn as the realized
    // draw rather than as the field, so a painted region has to survive the
    // hash to show up here.
    let mut sp_img = biome_img.clone();
    for cz in 0..CELLS_PER_SIDE {
        for cx in 0..CELLS_PER_SIDE {
            let s = terrain::scatter(seed, &table, &haven, cx, cz);
            if s.occupant != Occupant::Tree {
                continue;
            }
            let ix = (s.x / ISLAND_SIZE * px as f32) as usize;
            let iz = (s.z / ISLAND_SIZE * px as f32) as usize;
            if ix >= px || iz >= px {
                continue;
            }
            let c: [u8; 3] = if s.species == 0 {
                [20, 110, 190]
            } else {
                [250, 210, 60]
            };
            let j = (iz * px + ix) * 3;
            sp_img[j..j + 3].copy_from_slice(&c);
        }
    }
    write_ppm(&format!("{prefix}-species.ppm"), px, &sp_img);
    write_ppm(&format!("{prefix}-biome.ppm"), px, &biome_img);
    write_ppm(&format!("{prefix}-splat.ppm"), px, &splat_img);
    write_ppm(&format!("{prefix}-clump.ppm"), px, &clump_img);
    write_ppm(&format!("{prefix}-occ.ppm"), px, &occ_img);

    // The structure a histogram cannot show: how many connected forest
    // patches there are, and how big the largest is.
    let n = (CELLS_PER_SIDE * CELLS_PER_SIDE) as usize;
    let mut is_forest = vec![false; n];
    let mut land = 0u32;
    let mut per_biome = [0u32; 4];
    for cz in 0..CELLS_PER_SIDE {
        for cx in 0..CELLS_PER_SIDE {
            let x = (cx as f32 + 0.5) * CELL_SIZE;
            let z = (cz as f32 + 0.5) * CELL_SIZE;
            let h = terrain::ground(seed, &haven, x, z);
            if h < terrain::LAND_MIN_H {
                continue;
            }
            land += 1;
            let b = terrain::biome(h, terrain::moisture(seed, x, z));
            per_biome[b as usize] += 1;
            if b == Biome::Forest {
                is_forest[(cz * CELLS_PER_SIDE + cx) as usize] = true;
            }
        }
    }
    let mut seen = vec![false; n];
    let mut patches: Vec<u32> = Vec::new();
    let mut stack: Vec<i32> = Vec::new();
    for start in 0..n {
        if !is_forest[start] || seen[start] {
            continue;
        }
        let mut size = 0u32;
        stack.push(start as i32);
        seen[start] = true;
        while let Some(p) = stack.pop() {
            size += 1;
            let (cx, cz) = (p % CELLS_PER_SIDE, p / CELLS_PER_SIDE);
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (jx, jz) = (cx + dx, cz + dz);
                if jx < 0 || jz < 0 || jx >= CELLS_PER_SIDE || jz >= CELLS_PER_SIDE {
                    continue;
                }
                let j = (jz * CELLS_PER_SIDE + jx) as usize;
                if is_forest[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j as i32);
                }
            }
        }
        patches.push(size);
    }
    patches.sort_unstable_by(|a, b| b.cmp(a));
    let ha = (CELL_SIZE * CELL_SIZE) / 10_000.0;
    println!(
        "seed {seed}: land {land} cells ({:.0} ha)",
        land as f32 * ha
    );
    println!(
        "  beach {} ({:.1}%)  meadow {} ({:.1}%)  forest {} ({:.1}%)  highland {} ({:.1}%)",
        per_biome[0],
        per_biome[0] as f32 * 100.0 / land as f32,
        per_biome[1],
        per_biome[1] as f32 * 100.0 / land as f32,
        per_biome[2],
        per_biome[2] as f32 * 100.0 / land as f32,
        per_biome[3],
        per_biome[3] as f32 * 100.0 / land as f32,
    );
    println!(
        "  forest patches {} — largest {} cells ({:.0} ha, {:.0}% of all forest), top 8 {:?}",
        patches.len(),
        patches.first().copied().unwrap_or(0),
        patches.first().copied().unwrap_or(0) as f32 * ha,
        patches.first().copied().unwrap_or(0) as f32 * 100.0 / per_biome[2].max(1) as f32,
        &patches[..patches.len().min(8)],
    );

    // ── Shore structure: the two numbers the picture is about ─────────────
    //
    // A beach is a WIDTH in metres, not a height band, and a coastline is
    // either crinkled or it is a circle. Neither is visible in a biome
    // histogram, so both are measured here.
    {
        let c = ISLAND_SIZE * 0.5;
        let mut widths2: Vec<f32> = Vec::new();
        let mut widths10: Vec<f32> = Vec::new();
        let mut shore_r: Vec<f32> = Vec::new();
        let bearings = 720usize;
        for b in 0..bearings {
            // Unit radial without trig: walk the unit square's rim and
            // normalize. Even spacing in angle is not needed — this is a
            // sample of the rim, not an integral over it.
            let t = b as f32 / bearings as f32 * 4.0;
            let (ux, uz) = match t as i32 {
                0 => (1.0, t - 0.5),
                1 => (1.5 - t, 0.5),
                2 => (-1.0, 2.5 - t),
                _ => (t - 3.5, -0.5),
            };
            let n = (ux * ux + uz * uz).sqrt();
            let (ux, uz) = (ux / n, uz / n);
            // Bisect the waterline along this radial.
            let (mut lo, mut hi) = (200.0f32, 1100.0f32);
            if terrain::height(seed, c + ux * lo, c + uz * lo) < 0.0 {
                continue;
            }
            if terrain::height(seed, c + ux * hi, c + uz * hi) > 0.0 {
                continue;
            }
            for _ in 0..40 {
                let mid = (lo + hi) * 0.5;
                if terrain::height(seed, c + ux * mid, c + uz * mid) > 0.0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let r0 = lo;
            shore_r.push(r0);
            // Walk inland until the ground clears 2 m, then 10 m.
            let mut w2 = f32::NAN;
            let mut step = 0.0f32;
            while step < 400.0 {
                let r = r0 - step;
                let h = terrain::height(seed, c + ux * r, c + uz * r);
                if w2.is_nan() && h > 2.0 {
                    w2 = step;
                }
                if h > 10.0 {
                    if !w2.is_nan() {
                        widths2.push(w2);
                        widths10.push(step);
                    }
                    break;
                }
                step += 0.5;
            }
        }
        let pct = |v: &mut Vec<f32>, q: f32| {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v[((v.len() - 1) as f32 * q) as usize]
        };
        let mut w2 = widths2.clone();
        let mut w10 = widths10.clone();
        println!(
            "  shore: waterline->2 m  p10 {:.1} p50 {:.1} p90 {:.1} m   waterline->10 m p50 {:.1} m  (n {})",
            pct(&mut w2, 0.1),
            pct(&mut w2, 0.5),
            pct(&mut w2, 0.9),
            pct(&mut w10, 0.5),
            w2.len()
        );
        // Coastline crinkle: the radius's own roughness, and the perimeter
        // of the land mask against the circle of equal area (1.0 = a disc).
        let mean_r = shore_r.iter().sum::<f32>() / shore_r.len() as f32;
        let var = shore_r
            .iter()
            .map(|r| (r - mean_r) * (r - mean_r))
            .sum::<f32>()
            / shore_r.len() as f32;
        let mut dsum = 0.0f32;
        for i in 0..shore_r.len() {
            let j = (i + 1) % shore_r.len();
            dsum += (shore_r[j] - shore_r[i]).abs();
        }
        let perim: f32 = {
            // 4-neighbour boundary length of the land mask on the 8 m grid.
            let mut e = 0u32;
            for cz in 0..CELLS_PER_SIDE {
                for cx in 0..CELLS_PER_SIDE {
                    let x = (cx as f32 + 0.5) * CELL_SIZE;
                    let z = (cz as f32 + 0.5) * CELL_SIZE;
                    if terrain::height(seed, x, z) < 0.0 {
                        continue;
                    }
                    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let (jx, jz) = (cx + dx, cz + dz);
                        let inside = (0..CELLS_PER_SIDE).contains(&jx)
                            && (0..CELLS_PER_SIDE).contains(&jz)
                            && terrain::height(
                                seed,
                                (jx as f32 + 0.5) * CELL_SIZE,
                                (jz as f32 + 0.5) * CELL_SIZE,
                            ) >= 0.0;
                        if !inside {
                            e += 1;
                        }
                    }
                }
            }
            e as f32 * CELL_SIZE
        };
        let area = land as f32 * CELL_SIZE * CELL_SIZE;
        let equal_disc = 2.0 * core::f32::consts::PI * (area / core::f32::consts::PI).sqrt();
        println!(
            "  coast: r mean {:.0} sd {:.1} m, |dr| per sample {:.2} m, perimeter {:.0} m = {:.2}x an equal-area disc",
            mean_r,
            var.sqrt(),
            dsum / shore_r.len() as f32,
            perim,
            perim / equal_disc
        );
    }
    // ── The treeline, and the species paint ───────────────────────────────
    //
    // Both are structure a total is blind to, which is this file's whole
    // reason for existing (`reference/FORESTS.md` §8).
    {
        let mut edge_cells = 0u32;
        let mut edge_tree = 0u32;
        let mut edge_bush = 0u32;
        let mut core_cells = 0u32;
        let mut core_tree = 0u32;
        let mut core_bush = 0u32;
        // Species, counted where the field is at each rail, so the question
        // asked is "is a region dominated" and not "was a coin flipped".
        let mut sp_lo = [0u32; 2];
        let mut sp_hi = [0u32; 2];
        let mut edge_width_n = 0u32;
        let mut edge_width_sum = 0.0f64;
        for cz in 0..CELLS_PER_SIDE {
            for cx in 0..CELLS_PER_SIDE {
                let x = (cx as f32 + 0.5) * CELL_SIZE;
                let z = (cz as f32 + 0.5) * CELL_SIZE;
                let h = terrain::ground(seed, &haven, x, z);
                if h < terrain::LAND_MIN_H {
                    continue;
                }
                let m = terrain::moisture(seed, x, z);
                let sl = terrain::ground_slope(seed, &haven, x, z);
                let w = terrain::splat_from(h, m, sl);
                let (a, b) = (w[1] as f32, w[2] as f32);
                let e = if a + b > 0.0 {
                    4.0 * a * b / ((a + b) * (a + b))
                } else {
                    0.0
                };
                let s = terrain::scatter(seed, &table, &haven, cx, cz);
                // "In the edge" = the transfer is at least half on.
                if e >= 0.5 && b > 0.0 {
                    edge_cells += 1;
                    match s.occupant {
                        Occupant::Tree => edge_tree += 1,
                        Occupant::Bush => edge_bush += 1,
                        _ => {}
                    }
                } else if b > a * 4.0 {
                    core_cells += 1;
                    match s.occupant {
                        Occupant::Tree => core_tree += 1,
                        Occupant::Bush => core_bush += 1,
                        _ => {}
                    }
                }
                if s.occupant == Occupant::Tree {
                    let share = terrain::species_share(seed, x, z);
                    if share < 0.15 {
                        sp_lo[s.species as usize] += 1;
                    } else if share > 0.85 {
                        sp_hi[s.species as usize] += 1;
                    }
                }
            }
        }
        // How wide the treeline band is in METRES, walked rather than assumed:
        // a transfer that only fires on a two-metre contour is a transfer
        // nobody sees.
        for b in 0..240usize {
            let t = b as f32 / 240.0 * 4.0;
            let (ux, uz) = match t as i32 {
                0 => (1.0, t - 0.5),
                1 => (1.5 - t, 0.5),
                2 => (-1.0, 2.5 - t),
                _ => (t - 3.5, -0.5),
            };
            let n = (ux * ux + uz * uz).sqrt();
            let (ux, uz) = (ux / n, uz / n);
            let c = ISLAND_SIZE * 0.5;
            let mut run = 0.0f32;
            let mut d = 60.0f32;
            while d < 860.0 {
                let (x, z) = (c + ux * d, c + uz * d);
                let hh = terrain::ground(seed, &haven, x, z);
                if hh >= terrain::LAND_MIN_H {
                    let w = terrain::splat_from(
                        hh,
                        terrain::moisture(seed, x, z),
                        terrain::ground_slope(seed, &haven, x, z),
                    );
                    let (a, bq) = (w[1] as f32, w[2] as f32);
                    let e = if a + bq > 0.0 {
                        4.0 * a * bq / ((a + bq) * (a + bq))
                    } else {
                        0.0
                    };
                    if e >= 0.5 && bq > 0.0 {
                        run += 2.0;
                    } else if run > 0.0 {
                        edge_width_sum += f64::from(run);
                        edge_width_n += 1;
                        run = 0.0;
                    }
                }
                d += 2.0;
            }
        }
        let per = |n: u32, d: u32| {
            if d == 0 {
                0.0
            } else {
                f64::from(n) / f64::from(d) * 156.25
            }
        };
        println!(
            "  treeline: {edge_cells} cells — tree {:.0}/ha bush {:.0}/ha | forest core {core_cells} cells — tree {:.0}/ha bush {:.0}/ha",
            per(edge_tree, edge_cells),
            per(edge_bush, edge_cells),
            per(core_tree, core_cells),
            per(core_bush, core_cells),
        );
        println!(
            "  treeline band crossed {edge_width_n} times, mean width {:.0} m",
            edge_width_sum / f64::from(edge_width_n.max(1))
        );
        println!(
            "  species: where share<0.15 -> {:?} ({:.0}% sp0); where share>0.85 -> {:?} ({:.0}% sp1)",
            sp_lo,
            f64::from(sp_lo[0]) * 100.0 / f64::from((sp_lo[0] + sp_lo[1]).max(1)),
            sp_hi,
            f64::from(sp_hi[1]) * 100.0 / f64::from((sp_hi[0] + sp_hi[1]).max(1)),
        );
    }
    println!("  slots {counts:?}");
}
