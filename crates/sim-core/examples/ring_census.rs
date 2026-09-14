//! Dev probe for the client's tree budget: how many trees the prop
//! streamer's near ring holds and how many of them stand inside the LOD
//! swap, over eye positions sampled across an island. `client/tests/tree.rs`
//! and `client/tests/outer_ring.rs` record this probe's p90s as constants
//! and `tree::TREE_LOD_CAP` is sized against its maximum — this is the
//! command behind those numbers, so they are re-measured and never re-typed.
//!
//! The ring geometry is the client's (`terrain_mesh::CHUNK_M` = 64,
//! `NEAR_RADIUS` = 2 → a 5×5 chunk ring; `tree::TREE_LOD_SWAP_M` = 80) and
//! is taken on the command line so a moved knob is re-measured with it:
//!
//! `cargo run --release -p sim-core --example ring_census [seed] [swap_m] [chunk_m] [near_radius] [eyes]`
//!
//! Eyes are land cells drawn by a fixed stride over the whole 0..2048
//! square (the window `sim-core/tests/relief.rs` says cannot be aimed
//! wrong), so the sample is the island and not the spawn.

// Host-side tuning probe: printing is its job. The L5 wall bans
// format/print in SIM code; an example binary is not sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::terrain::{self, Occupant, ScatterTable, CELLS_PER_SIDE, CELL_SIZE, LAND_MIN_H};

fn main() {
    let mut args = std::env::args().skip(1);
    let seed: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(20260731);
    let swap_m: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(80.0);
    let chunk_m: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(64.0);
    let near_radius: i32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(2);
    let eyes_wanted: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(400);

    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);

    // Resolve the island once; the ring counts are then lookups.
    let n = (CELLS_PER_SIDE * CELLS_PER_SIDE) as usize;
    let mut tree_at = vec![None::<(f32, f32)>; n];
    let mut land = vec![false; n];
    for cz in 0..CELLS_PER_SIDE {
        for cx in 0..CELLS_PER_SIDE {
            let i = (cz * CELLS_PER_SIDE + cx) as usize;
            let s = terrain::scatter(seed, &table, &haven, cx, cz);
            if s.occupant == Occupant::Tree {
                tree_at[i] = Some((s.x, s.z));
            }
            let x = cx as f32 * CELL_SIZE + CELL_SIZE * 0.5;
            let z = cz as f32 * CELL_SIZE + CELL_SIZE * 0.5;
            land[i] = terrain::ground(seed, &haven, x, z) >= LAND_MIN_H;
        }
    }

    let cells_per_chunk = (chunk_m / CELL_SIZE) as i32;
    let ring_cells = (2 * near_radius + 1) * cells_per_chunk;
    // `f32::sqrt` is the one root the crate's walls allow, examples included.
    let stride = ((n as f32 / eyes_wanted as f32).sqrt() as i32).max(1);
    let (mut ring, mut near, mut fade) = (Vec::new(), Vec::new(), Vec::new());
    let mut densest = (0usize, 0.0f32, 0.0f32);
    let fade_m = swap_m + 15.0;
    for cz in (0..CELLS_PER_SIDE).step_by(stride as usize) {
        for cx in (0..CELLS_PER_SIDE).step_by(stride as usize) {
            if !land[(cz * CELLS_PER_SIDE + cx) as usize] {
                continue;
            }
            let ex = cx as f32 * CELL_SIZE + CELL_SIZE * 0.5;
            let ez = cz as f32 * CELL_SIZE + CELL_SIZE * 0.5;
            // The ring is chunk-aligned around the eye's own chunk, as
            // `props::stream` builds it.
            // The world is positive on both axes, so the truncating cast is
            // the floor (`floor` itself is walled, examples included).
            let ecx = (ex / chunk_m) as i32;
            let ecz = (ez / chunk_m) as i32;
            let c0x = (ecx - near_radius) * cells_per_chunk;
            let c0z = (ecz - near_radius) * cells_per_chunk;
            let (mut r, mut nr, mut fd) = (0usize, 0usize, 0usize);
            for oz in 0..ring_cells {
                for ox in 0..ring_cells {
                    let (tx, tz) = (c0x + ox, c0z + oz);
                    if !(0..CELLS_PER_SIDE).contains(&tx) || !(0..CELLS_PER_SIDE).contains(&tz) {
                        continue;
                    }
                    if let Some((x, z)) = tree_at[(tz * CELLS_PER_SIDE + tx) as usize] {
                        r += 1;
                        let d2 = (x - ex) * (x - ex) + (z - ez) * (z - ez);
                        nr += usize::from(d2 < swap_m * swap_m);
                        fd += usize::from(d2 < fade_m * fade_m);
                    }
                }
            }
            if nr > densest.0 {
                densest = (nr, ex, ez);
            }
            ring.push(r);
            near.push(nr);
            fade.push(fd);
        }
    }
    let pct = |v: &mut Vec<usize>, p: f64| {
        v.sort_unstable();
        v[((v.len() - 1) as f64 * p) as usize]
    };
    println!(
        "seed {seed}: {} land eyes (stride {stride} cells), ring {}x{} chunks of {chunk_m} m, swap {swap_m} m",
        ring.len(),
        2 * near_radius + 1,
        2 * near_radius + 1
    );
    println!(
        "  trees in ring:      p50 {:>4}  p90 {:>4}  p99 {:>4}  max {:>4}",
        pct(&mut ring, 0.5),
        pct(&mut ring, 0.9),
        pct(&mut ring, 0.99),
        pct(&mut ring, 1.0)
    );
    println!(
        "  inside swap:        p50 {:>4}  p90 {:>4}  p99 {:>4}  max {:>4}",
        pct(&mut near, 0.5),
        pct(&mut near, 0.9),
        pct(&mut near, 0.99),
        pct(&mut near, 1.0)
    );
    println!(
        "  inside swap+fade:   p50 {:>4}  p90 {:>4}  p99 {:>4}  max {:>4}   (fade 15 m: every tree drawing any near geometry)",
        pct(&mut fade, 0.5),
        pct(&mut fade, 0.9),
        pct(&mut fade, 0.99),
        pct(&mut fade, 1.0)
    );
    println!(
        "  densest eye: {} trees inside {swap_m} m at ({:.0}, {:.0})",
        densest.0, densest.1, densest.2
    );
}
