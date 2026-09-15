#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_types,
    clippy::disallowed_methods
)]
//! How many (seed, bearing) pairs have NO carriageway — i.e. how open the
//! road ring is. `tests/road.rs::road_ring_is_closed_on_every_bearing`
//! asserts this is zero; this is the same march, reported as a count so a
//! worldgen knob can be swept against it instead of bisected by hand.
use sim_core::terrain::{self, RoadBand, ISLAND_SIZE, ROAD_R_MAX, ROAD_R_MIN};
use sim_core::yaw_dir;
fn main() {
    let c = ISLAND_SIZE * 0.5;
    let seeds = [
        0x0047_4154_4553u64,
        0x1,
        0xDEAD_BEEF,
        0x5EED,
        20260731,
        0,
        7,
        12345,
    ];
    let (mut open, mut total) = (0u32, 0u32);
    let (mut r_lo, mut r_hi) = (f32::MAX, 0.0f32);
    let mut thin = usize::MAX;
    for seed in seeds {
        let mut seed_open = 0;
        for b in 0..64u16 {
            let (ux, uz) = yaw_dir((b * 4) << 8);
            let mut hits = 0usize;
            let mut d = ROAD_R_MIN;
            while d <= ROAD_R_MAX {
                if terrain::road_band(seed, c + ux * d, c + uz * d) == RoadBand::Carriageway {
                    hits += 1;
                    r_lo = r_lo.min(d);
                    r_hi = r_hi.max(d);
                }
                d += 1.0;
            }
            total += 1;
            if hits == 0 {
                open += 1;
                seed_open += 1;
            } else {
                thin = thin.min(hits);
            }
        }
        if seed_open > 0 {
            println!("   seed {seed:#x}: {seed_open} open bearings");
        }
    }
    println!("   ring: {open}/{total} bearings OPEN, thinnest crossing {thin} m, radius {r_lo:.0}-{r_hi:.0} m");
}
