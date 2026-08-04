//! Dev probe: prints height distribution, biome mix, and live-slot counts
//! for a seed. Used to keep the generator inside TERRAIN.md §6's numbers
//! when shape params change. Not a gate — the gates pin goldens.

// Host-side tuning probe: printing is its job. The L5 wall bans
// format/print in SIM code; an example binary is not sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::terrain::{self, Occupant, ScatterTable, CELLS_PER_SIDE};

fn main() {
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x0047_4154_4553);

    let mut hist = [0u32; 12]; // <0, 0-10, 10-20, ... 90-100, >100
    let mut biomes = [0u32; 4];
    for gz in 0..256i32 {
        for gx in 0..256i32 {
            let x = gx as f32 * 8.0 + 4.0;
            let z = gz as f32 * 8.0 + 4.0;
            let h = terrain::height(seed, x, z);
            let bucket = if h < 0.0 {
                0
            } else {
                (((h / 10.0) as i32) + 1).min(11) as usize
            };
            hist[bucket] += 1;
            if h >= 0.0 {
                biomes[terrain::biome(h, terrain::moisture(seed, x, z)) as usize] += 1;
            }
        }
    }
    println!("height histogram (65,536 samples on the 8 m grid):");
    let labels = [
        "  sea", " 0-10", "10-20", "20-30", "30-40", "40-50", "50-60", "60-70", "70-80", "80-90",
        "90-100", " 100+",
    ];
    for (l, c) in labels.iter().zip(hist.iter()) {
        println!("  {l}: {c}");
    }
    println!(
        "biomes: beach {} meadow {} forest {} highland {}",
        biomes[0], biomes[1], biomes[2], biomes[3]
    );

    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);
    let mut counts = [0u32; 8];
    for cz in 0..CELLS_PER_SIDE {
        for cx in 0..CELLS_PER_SIDE {
            counts[terrain::scatter(seed, &table, &haven, cx, cz).occupant as usize] += 1;
        }
    }
    let live: u32 = counts[1..].iter().sum();
    println!(
        "slots: live {live} (tree {} stone {} metal {} sulfur {} bush {} rock {} barrel {})",
        counts[Occupant::Tree as usize],
        counts[Occupant::StoneNode as usize],
        counts[Occupant::MetalNode as usize],
        counts[Occupant::SulfurNode as usize],
        counts[Occupant::Bush as usize],
        counts[Occupant::Rock as usize],
        counts[Occupant::BarrelSlot as usize],
    );
}
