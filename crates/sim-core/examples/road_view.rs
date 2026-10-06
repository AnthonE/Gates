//! Top-down picture of the road and what stands on it, to scale.
//!
//!   cargo run --release -p sim-core --example road_view -- <seed-hex> <out.ppm> [junk|open|sites] [n]
//!
//! Finds `n` (default 4) windows on the coast ring — around junk-pile wrecks,
//! on open road, or at the pad and its waystations — and tiles them into one
//! PPM: carriageway dark, shoulder tan, every occupant drawn as its sim
//! footprint (box tables rotated by the slot's yaw). 384 px windows: 48 m,
//! or 112 m for the sites.

// An example prints; the sim walls on `print!` are for sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::terrain::{self, Occupant, RoadBand, ScatterTable, CELLS_PER_SIDE, CELL_SIZE};
use std::io::Write;

const WIN_PX: usize = 384;

fn colour(o: Occupant) -> [u8; 3] {
    match o {
        Occupant::Tree => [30, 80, 35],
        Occupant::Bush => [90, 140, 60],
        Occupant::Rock | Occupant::StoneNode => [120, 120, 115],
        Occupant::MetalNode => [150, 120, 100],
        Occupant::SulfurNode => [190, 180, 60],
        Occupant::BarrelSlot => [40, 90, 200],
        Occupant::OilBarrel => [220, 30, 25],
        Occupant::RoadSign => [255, 210, 0],
        Occupant::FoodCrate => [230, 130, 40],
        Occupant::CarWreck => [150, 70, 30],
        Occupant::TireStack => [10, 10, 10],
        _ => [255, 0, 255],
    }
}

fn boxes(o: Occupant) -> &'static [[f32; 6]] {
    match o {
        Occupant::RoadSign => &terrain::ROAD_SIGN_BOXES,
        Occupant::CarWreck => &terrain::CAR_WRECK_BOXES,
        _ => &[],
    }
}

/// Does slot `s` cover the point (x, z)?
fn covers(s: &terrain::Slot, x: f32, z: f32) -> bool {
    let (r, _) = terrain::occupant_volume(s.occupant);
    let (dx, dz) = (x - s.x, z - s.z);
    let table = boxes(s.occupant);
    if table.is_empty() {
        // The bush blocks nothing; draw it at a token size so it shows.
        let r = if r > 0.0 { r } else { 0.5 } * s.scale;
        return dx * dx + dz * dz <= r * r;
    }
    let (sn, cs) = sim_core::yaw_dir((s.yaw as u16) << 8);
    let lx = dx * cs - dz * sn;
    let lz = dx * sn + dz * cs;
    table.iter().any(|b| {
        (lx - b[0] * s.scale).abs() <= b[3] * 0.5 * s.scale
            && (lz - b[2] * s.scale).abs() <= b[5] * 0.5 * s.scale
    })
}

fn render(
    seed: u64,
    h: &terrain::Haven,
    table: &ScatterTable,
    (cx, cz): (f32, f32),
    win_m: f32,
) -> Vec<[u8; 3]> {
    let px_m = win_m / WIN_PX as f32;
    let x0 = cx - win_m * 0.5;
    let z0 = cz - win_m * 0.5;
    let mut slots = Vec::new();
    let c0 = ((x0 / CELL_SIZE) as i32 - 1).max(0);
    let c1 = (((x0 + win_m) / CELL_SIZE) as i32 + 1).min(CELLS_PER_SIDE - 1);
    let r0 = ((z0 / CELL_SIZE) as i32 - 1).max(0);
    let r1 = (((z0 + win_m) / CELL_SIZE) as i32 + 1).min(CELLS_PER_SIDE - 1);
    for gz in r0..=r1 {
        for gx in c0..=c1 {
            let s = terrain::scatter(seed, table, h, gx, gz);
            if s.occupant != Occupant::None {
                slots.push(s);
            }
        }
    }
    let mut img = vec![[0u8; 3]; WIN_PX * WIN_PX];
    for py in 0..WIN_PX {
        for px in 0..WIN_PX {
            let x = x0 + (px as f32 + 0.5) * px_m;
            // North up: image row 0 is the window's far +z edge.
            let z = z0 + win_m - (py as f32 + 0.5) * px_m;
            let g = terrain::ground(seed, h, x, z);
            let mut c = if g < terrain::LAND_MIN_H {
                [40, 70, 110]
            } else {
                match terrain::road_band(seed, h, x, z) {
                    RoadBand::Carriageway => [70, 70, 72],
                    RoadBand::Shoulder => [175, 160, 120],
                    RoadBand::Off => [120, 150, 90],
                }
            };
            for s in &slots {
                if covers(s, x, z) {
                    c = colour(s.occupant);
                }
            }
            // A 4 m grid, faintly, for scale.
            if (x.rem_euclid(4.0) < px_m) || (z.rem_euclid(4.0) < px_m) {
                c = [
                    c[0].saturating_sub(18),
                    c[1].saturating_sub(18),
                    c[2].saturating_sub(18),
                ];
            }
            img[py * WIN_PX + px] = c;
        }
    }
    img
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seed = args
        .get(1)
        .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0x600D_C0DE);
    let out = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "road_view.ppm".into());
    let mode = args.get(3).map_or("junk", |m| m.as_str());
    let n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(4);
    let table = ScatterTable::alpha_default();
    let h = terrain::haven(seed);

    // Window centres: wrecks for junk piles, open-shoulder road signs, or
    // the sites on the ring.
    let want = if mode == "open" {
        Occupant::RoadSign
    } else {
        Occupant::CarWreck
    };
    let win_m = if mode == "sites" { 112.0 } else { 48.0 };
    let mut centres: Vec<(f32, f32)> = Vec::new();
    if mode == "sites" {
        centres.push((h.x, h.z));
        for ws in h.minor.iter() {
            if ws.live && ws.kind == terrain::SiteKind::Waystation {
                centres.push((ws.x, ws.z));
            }
        }
    }
    'scan: for gz in 0..CELLS_PER_SIDE {
        if mode == "sites" {
            break;
        }
        for gx in 0..CELLS_PER_SIDE {
            let s = terrain::scatter(seed, &table, &h, gx, gz);
            if s.occupant == want
                && centres
                    .iter()
                    .all(|(x, z)| (x - s.x).abs() > 500.0 || (z - s.z).abs() > 500.0)
            {
                centres.push((s.x, s.z));
                if centres.len() == n {
                    break 'scan;
                }
            }
        }
    }
    let cols = centres.len().clamp(1, 2);
    let rows = centres.len().div_ceil(cols).max(1);
    let (w, hgt) = (
        cols * WIN_PX + (cols - 1) * 4,
        rows * WIN_PX + (rows - 1) * 4,
    );
    let mut sheet = vec![[255u8; 3]; w * hgt];
    for (i, (x, z)) in centres.iter().enumerate() {
        println!("window {i}: centre ({x:.0}, {z:.0})");
        let img = render(seed, &h, &table, (*x, *z), win_m);
        let (ox, oy) = ((i % cols) * (WIN_PX + 4), (i / cols) * (WIN_PX + 4));
        for py in 0..WIN_PX {
            for px in 0..WIN_PX {
                sheet[(oy + py) * w + ox + px] = img[py * WIN_PX + px];
            }
        }
    }
    let mut f = std::fs::File::create(&out).expect("create output");
    write!(f, "P6\n{w} {hgt}\n255\n").unwrap();
    for p in &sheet {
        f.write_all(p).unwrap();
    }
    println!("wrote {out} ({w}x{hgt})");
}
