#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_types,
    clippy::disallowed_methods
)]
//! Measure: is there anywhere inland a third site may stand?
//!
//! `reference/ROADS.md` §9.2.1 asks for one authored place off the road ring.
//! The roster's separation floor was sized for the ring — `WAYSTATION_MIN_SEP_M
//! = ROAD_R_MIN = 600 m`, spread along a circumference of 4–6 km — and the
//! interior is a disc of radius `INLAND_R_MAX` ≈ 580 m. Demanding 600 m from
//! three sites that ring it is a different geometric question, so this asks it
//! rather than arguing about it.
//!
//! Prints, per seed: the pad and both waystations' radii, then for a ladder of
//! candidate floors the share of the inland disc that clears every taken site
//! AND is land off the carriageway, measured on a fine grid; and what the
//! shipped coarse lattice (`INLAND_CANDIDATES` × `INLAND_RADII`) would find.
//!
//! `cargo run --release -p sim-core --example inland_scan`

use sim_core::terrain::{
    self, Haven, RoadBand, SiteKind, INLAND_CANDIDATES, INLAND_RADII, ISLAND_SIZE, LAND_MIN_H,
};

/// Swept rather than read, so the bracket can be chosen on a measurement.
fn r_max() -> f32 {
    std::env::var("INLAND_R_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(terrain::INLAND_R_MAX)
}

const SEEDS: [u64; 16] = [
    1,
    2,
    7,
    42,
    99,
    1337,
    20_260_731,
    20_260_804,
    555_555,
    8_675_309,
    31_337,
    4_294_967_291,
    123_456_789,
    999_999_937,
    0xDEAD_BEEF,
    0x0BAD_C0DE,
];

const FLOORS: [f32; 6] = [600.0, 500.0, 400.0, 350.0, 300.0, 250.0];

/// Every site already on the roster, as (x, z).
fn taken(pad: &Haven) -> Vec<(f32, f32)> {
    let mut v = vec![(pad.x, pad.z)];
    // The ring tier ONLY. The inland site is what this is measuring room
    // for, so counting it would be asking whether a SECOND one fits — which
    // is the shape the first draft of this had, reading 11% where the
    // question's answer was 46%.
    for ws in pad.minor.iter() {
        if ws.live && ws.kind != SiteKind::Inland {
            v.push((ws.x, ws.z));
        }
    }
    v
}

fn clears(sites: &[(f32, f32)], x: f32, z: f32, sep: f32) -> bool {
    sites.iter().all(|&(sx, sz)| {
        let (dx, dz) = (x - sx, z - sz);
        dx * dx + dz * dz >= sep * sep
    })
}

fn usable(seed: u64, x: f32, z: f32) -> bool {
    terrain::height(seed, x, z) >= LAND_MIN_H
        && terrain::road_band(seed, x, z) != RoadBand::Carriageway
}

fn main() {
    let c = ISLAND_SIZE * 0.5;
    let r_max = r_max();
    println!("inland disc: r <= {r_max:.2} m about the island centre");
    println!("lattice: {INLAND_CANDIDATES} bearings x {INLAND_RADII} radii\n");

    // Fine grid over the inland disc, 8 m — dense enough that a 15 m site
    // radius cannot hide between samples.
    const STEP: f32 = 8.0;
    let n = (r_max / STEP) as i32;

    let mut fine_tot = [0u32; FLOORS.len()];
    let mut fine_hit = [0u32; FLOORS.len()];
    let mut lat_hit = [0u32; FLOORS.len()];
    let mut seeds_with = [0u32; FLOORS.len()];

    for &seed in SEEDS.iter() {
        let pad = terrain::haven(seed);
        let sites = taken(&pad);
        let radii: Vec<f32> = sites
            .iter()
            .map(|&(x, z)| ((x - c) * (x - c) + (z - c) * (z - c)).sqrt())
            .collect();
        print!("seed {seed:>12}: {} sites at r =", sites.len());
        for r in radii.iter() {
            print!(" {r:.0}");
        }

        let mut line = String::new();
        for (fi, &sep) in FLOORS.iter().enumerate() {
            // Fine grid.
            let mut tot = 0u32;
            let mut hit = 0u32;
            for iz in -n..=n {
                for ix in -n..=n {
                    let (x, z) = (c + ix as f32 * STEP, c + iz as f32 * STEP);
                    let (dx, dz) = (x - c, z - c);
                    if dx * dx + dz * dz > r_max * r_max {
                        continue;
                    }
                    tot += 1;
                    if usable(seed, x, z) && clears(&sites, x, z, sep) {
                        hit += 1;
                    }
                }
            }
            fine_tot[fi] += tot;
            fine_hit[fi] += hit;

            // The shipped coarse lattice.
            let mut lat = 0u32;
            for b in 0..INLAND_CANDIDATES {
                let (dx, dz) =
                    sim_core::yaw_dir((b as u16 * (256 / INLAND_CANDIDATES) as u16) << 8);
                for k in 1..=INLAND_RADII {
                    let r = r_max * (k as f32 / INLAND_RADII as f32);
                    let (x, z) = (c + dx * r, c + dz * r);
                    if usable(seed, x, z) && clears(&sites, x, z, sep) {
                        lat += 1;
                    }
                }
            }
            lat_hit[fi] += lat;
            if lat > 0 {
                seeds_with[fi] += 1;
            }
            let pct = 100.0 * hit as f32 / tot.max(1) as f32;
            line.push_str(&format!("  {sep:.0}m: {pct:5.1}% / lat {lat:>3}"));
        }
        println!("{line}");
    }

    println!(
        "\n{:>7} {:>10} {:>12} {:>16}",
        "floor", "area", "lattice hits", "seeds with >=1"
    );
    for (fi, &sep) in FLOORS.iter().enumerate() {
        println!(
            "{:>6.0}m {:>9.2}% {:>12} {:>13}/{}",
            sep,
            100.0 * fine_hit[fi] as f32 / fine_tot[fi].max(1) as f32,
            lat_hit[fi],
            seeds_with[fi],
            SEEDS.len()
        );
    }
    // And where the shipped bracket actually puts one. This is the half the
    // sweep above cannot show: a feasible disc says a site COULD stand, and
    // this says where the lattice chose.
    println!("\nwhere the shipped scan lands one:");
    for &seed in SEEDS.iter() {
        let pad = terrain::haven(seed);
        for ws in pad
            .minor
            .iter()
            .filter(|w| w.live && w.kind == SiteKind::Inland)
        {
            let r = ((ws.x - c) * (ws.x - c) + (ws.z - c) * (ws.z - c)).sqrt();
            let mut nearest = f32::MAX;
            for (sx, sz) in taken(&pad) {
                let (dx, dz) = (ws.x - sx, ws.z - sz);
                nearest = nearest.min((dx * dx + dz * dz).sqrt());
            }
            println!(
                "  seed {seed:>12}: r {r:>5.0} m, {:>4.0} m inside the ring's \
                 innermost radius, nearest site {nearest:.0} m, y {:.1} m",
                terrain::ROAD_R_MIN - r,
                ws.y
            );
        }
    }
    println!(
        "\n(the `usable` mask alone — land, off the carriageway — is the {:.1}% ceiling)",
        {
            let mut tot = 0u32;
            let mut hit = 0u32;
            for &seed in SEEDS.iter() {
                for iz in -n..=n {
                    for ix in -n..=n {
                        let (x, z) = (c + ix as f32 * STEP, c + iz as f32 * STEP);
                        let (dx, dz) = (x - c, z - c);
                        if dx * dx + dz * dz > r_max * r_max {
                            continue;
                        }
                        tot += 1;
                        if usable(seed, x, z) {
                            hit += 1;
                        }
                    }
                }
            }
            100.0 * hit as f32 / tot.max(1) as f32
        }
    );
}
