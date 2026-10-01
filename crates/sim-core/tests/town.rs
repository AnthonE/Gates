//! The town ("THE GATE"): placed on every seed, on its lattice, on a flat
//! carved floor, reachable by road, and walkable from its gates to the gate.

// A test prints its timing: host code, not sim code.
#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    clippy::disallowed_types
)]
use sim_core::{
    collide::CAPSULE_RADIUS_M,
    fmath::fabs,
    kit,
    occupy::{Occupants, Pristine, SlotCache},
    terrain::{self, Haven, Occupant, ScatterTable},
    town,
};

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

fn occupants<'a>(h: &'a Haven, table: &'a ScatterTable, cache: &'a mut SlotCache) -> Occupants<'a> {
    Occupants {
        table,
        haven: h,
        harvested: &Pristine,
        cache,
    }
}

#[test]
fn every_seed_places_a_flat_reachable_town_in_the_middle() {
    for seed in SEEDS {
        let t0 = std::time::Instant::now();
        let h = terrain::haven(seed);
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        let t = h.town;
        assert!(t.live, "seed {seed}: no town");
        assert!(
            terrain::sites_complete(&h),
            "seed {seed}: incomplete island: roads {:?} minor {:?}",
            h.roads.iter().map(|r| r.live).collect::<Vec<_>>(),
            h.minor.iter().map(|m| m.live).collect::<Vec<_>>()
        );
        println!(
            "seed {seed}: town ({:.0}, {:.0}) floor {:.1} rot {} relief {:.1}  haven() {ms:.0} ms",
            t.x, t.z, t.floor_y, t.rot, t.relief
        );
        // On the lattice, the floor on its half metre.
        assert_eq!(t.x % town::TOWN_SNAP_M, 0.0);
        assert_eq!(t.z % town::TOWN_SNAP_M, 0.0);
        assert_eq!(t.floor_y * 2.0, ((t.floor_y * 2.0) as i32) as f32);
        let c = terrain::ISLAND_SIZE * 0.5;
        let r = ((t.x - c) * (t.x - c) + (t.z - c) * (t.z - c)).sqrt();
        assert!(
            r <= town::TOWN_R_MAX + town::TOWN_SNAP_M,
            "seed {seed}: {r} m out"
        );
        // The whole compound stands on its carved floor.
        for gx in -21..=21 {
            for gz in -21..=21 {
                let (x, z) = (t.x + gx as f32 * 2.0, t.z + gz as f32 * 2.0);
                let g = terrain::ground(seed, &h, x, z);
                assert!(
                    fabs(g - t.floor_y) < 0.01,
                    "seed {seed}: {g} vs {} at {x},{z}",
                    t.floor_y
                );
            }
        }
        // Its main road starts at the north gate's port.
        let road = h.roads[terrain::DEPOT_ROADS];
        assert!(road.live);
        let (px, pz) = kit::to_world(&t.placed(), 0.0, town::PORT_R);
        assert!(
            fabs(road.px - px) < 0.01 && fabs(road.pz - pz) < 0.01,
            "seed {seed}"
        );
        // Nothing grows inside and no landmark stands in it.
        assert!(!h
            .marks
            .iter()
            .any(|m| m.live && town::covers(&t, m.x, m.z, 26.0)));
    }
}

#[test]
fn the_compound_blocks_at_its_walls_and_opens_at_its_gates() {
    let seed = 20_260_731;
    let h = terrain::haven(seed);
    let t = h.town;
    let table = ScatterTable::alpha_default();
    let mut cache = SlotCache::new();
    let mut occ = occupants(&h, &table, &mut cache);
    let p = t.placed();
    let feet = t.floor_y;
    // Walk the market street from the north gate's port to the south's.
    let mut lz = town::PORT_R;
    while lz >= -town::PORT_R {
        let (x, z) = kit::to_world(&p, 0.0, lz);
        assert!(
            !occ.blocks(seed, x, z, feet),
            "street blocked at local z {lz}"
        );
        lz -= 0.5;
    }
    // And the east-west road through the plaza.
    let mut lx = -town::PORT_R;
    while lx <= town::PORT_R {
        let (x, z) = kit::to_world(&p, lx, 0.0);
        if fabs(lx) > 14.5 || fabs(lx) < 5.0 {
            assert!(
                !occ.blocks(seed, x, z, feet),
                "cross street blocked at local x {lx}"
            );
        }
        lx += 0.5;
    }
    // The walls stop a body.
    for (wx, wz) in [(-20.0, 40.8), (20.0, -40.8), (40.8, 20.0), (-40.8, -20.0)] {
        let (x, z) = kit::to_world(&p, wx, wz);
        assert!(occ.blocks(seed, x, z, feet), "wall open at {wx},{wz}");
    }
    // A pylon too, and the dais is a step.
    let (x, z) = kit::to_world(&p, 9.5, 0.0);
    assert!(occ.blocks(seed, x, z, feet + 0.5));
    let (x, z) = kit::to_world(&p, 6.0 + CAPSULE_RADIUS_M, 5.0);
    assert!(!occ.blocks(seed, x, z, feet));
    assert!(occ.ground(seed, x, z, feet) >= feet + 0.5 - 1e-3);
}

#[test]
fn stations_and_kiosks_land_on_their_cells_and_inside_the_safe_zone() {
    for seed in [20_260_731u64, 42, 0xDEAD_BEEF] {
        let h = terrain::haven(seed);
        let t = h.town;
        for k in 0..town::STATIONS.len() {
            let (_, x, z) = town::station_world(&t, k).unwrap();
            let cx = (x - 1.5) / 3.0;
            let cz = (z - 1.5) / 3.0;
            assert_eq!(
                cx,
                (cx as i32) as f32,
                "seed {seed} station {k} off its cell"
            );
            assert_eq!(
                cz,
                (cz as i32) as f32,
                "seed {seed} station {k} off its cell"
            );
            assert!(town::safe(&t, x, z));
        }
        for k in 0..town::KIOSKS.len() {
            let (x, z) = town::kiosk_world(&t, k).unwrap();
            assert!(town::safe(&t, x, z));
        }
        // Scatter leaves the town alone.
        let table = ScatterTable::alpha_default();
        let c0x = ((t.x - 60.0) / terrain::CELL_SIZE) as i32;
        let c0z = ((t.z - 60.0) / terrain::CELL_SIZE) as i32;
        for cx in c0x..c0x + 15 {
            for cz in c0z..c0z + 15 {
                let s = terrain::scatter(seed, &table, &h, cx, cz);
                if s.occupant != Occupant::None {
                    assert!(
                        !town::safe(&t, s.x, s.z),
                        "seed {seed}: {:?} inside",
                        s.occupant
                    );
                }
            }
        }
    }
}
