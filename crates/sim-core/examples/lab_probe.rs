//! Where each seed's lab is cut (`lab.rs`), and what its host is: the kinds
//! the island holds, the keep chosen, its depth and its crates.
//! `cargo run --release -p sim-core --example lab_probe [seed…]`.
#![allow(clippy::disallowed_macros, clippy::disallowed_types)]

fn main() {
    let seeds: Vec<u64> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let seeds = if seeds.is_empty() {
        vec![20260731, 1, 7, 11, 42, 1234]
    } else {
        seeds
    };
    let table = sim_core::terrain::ScatterTable::alpha_default();
    for seed in seeds {
        let h = sim_core::terrain::haven(seed);
        let keeps = h
            .marks
            .iter()
            .filter(|m| m.live && m.kind == sim_core::lab::HOST_KIND)
            .count();
        let Some(m) = sim_core::lab::host(&h.marks) else {
            println!("seed {seed}: no lab ({keeps} keeps)");
            continue;
        };
        let floor = sim_core::terrain::ground(seed, &h, m.x, m.z);
        println!(
            "seed {seed}: lab under the keep at ({:.0}, {:.0}), base {:.1}, pit {:.1} ({keeps} keeps)",
            m.x, m.z, m.y, floor
        );
        for k in 0..sim_core::lab::CRATES.len() {
            let (x, y, z, _, occ) = sim_core::lab::crate_world(m, k).unwrap();
            let cx = (x / sim_core::terrain::CELL_SIZE) as i32;
            let cz = (z / sim_core::terrain::CELL_SIZE) as i32;
            let s = sim_core::terrain::scatter(seed, &table, &h, cx, cz);
            println!(
                "  crate {k}: {occ:?} at ({x:.1}, {y:.1}, {z:.1}); the cell holds {:?} at y {:.1}",
                s.occupant, s.y
            );
        }
    }
}
