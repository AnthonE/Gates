//! Roadside junk (Rust's roadside spawns): oil barrels, road signs, food
//! boxes and junk piles stand along the roads and only there, and a junk
//! pile's wrecked car never sits on a carriageway.

use sim_core::terrain::{
    self, Occupant, RoadBand, ScatterTable, CAR_WRECK_BOXES, CELLS_PER_SIDE, JUNK_REACH_M,
};

const SEEDS: [u64; 2] = [0x600D_C0DE, 7];

/// Distance from (x, z) to the nearest road centre line, ring or side road.
fn road_dist(h: &terrain::Haven, x: f32, z: f32) -> f32 {
    let mut d2 = h.ring.dist2(x, z);
    for r in h.roads.iter().filter(|r| r.live) {
        d2 = d2.min(r.dist2(x, z));
    }
    d2.sqrt()
}

#[test]
fn roadside_junk_stands_by_the_road_and_off_it() {
    let table = ScatterTable::alpha_default();
    for seed in SEEDS {
        let h = terrain::haven(seed);
        let mut counts = [0u32; terrain::OCCUPANT_R_M.len()];
        for cz in 0..CELLS_PER_SIDE {
            for cx in 0..CELLS_PER_SIDE {
                let s = terrain::scatter(seed, &table, &h, cx, cz);
                if !matches!(
                    s.occupant,
                    Occupant::OilBarrel
                        | Occupant::RoadSign
                        | Occupant::FoodCrate
                        | Occupant::CarWreck
                        | Occupant::TireStack
                ) {
                    continue;
                }
                counts[s.occupant as usize] += 1;
                let d = road_dist(&h, s.x, s.z);
                assert!(
                    d <= JUNK_REACH_M + 0.01,
                    "seed {seed:#x}: {:?} at ({}, {}) stands {d} m from any road",
                    s.occupant,
                    s.x,
                    s.z
                );
                assert_ne!(
                    terrain::road_band(seed, &h, s.x, s.z),
                    RoadBand::Carriageway,
                    "seed {seed:#x}: {:?} on the carriageway",
                    s.occupant
                );
                if s.occupant == Occupant::CarWreck {
                    // Every box corner, in the slot's frame, off the road.
                    let (sn, cs) = sim_core::yaw_dir((s.yaw as u16) << 8);
                    for b in CAR_WRECK_BOXES {
                        for (fx, fz) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
                            let lx = (b[0] + fx * b[3] * 0.5) * s.scale;
                            let lz = (b[2] + fz * b[5] * 0.5) * s.scale;
                            // Local +Z is (sin, cos); local +X is (cos, −sin).
                            let wx = s.x + lx * cs + lz * sn;
                            let wz = s.z - lx * sn + lz * cs;
                            assert_ne!(
                                terrain::road_band(seed, &h, wx, wz),
                                RoadBand::Carriageway,
                                "seed {seed:#x}: the car at ({}, {}) reaches the carriageway",
                                s.x,
                                s.z
                            );
                        }
                    }
                }
            }
        }
        for o in [
            Occupant::OilBarrel,
            Occupant::RoadSign,
            Occupant::FoodCrate,
            Occupant::CarWreck,
            Occupant::TireStack,
        ] {
            assert!(
                counts[o as usize] >= 5,
                "seed {seed:#x}: only {} {o:?} on the island",
                counts[o as usize]
            );
        }
    }
}
