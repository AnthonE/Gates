//! The Black Ziggurat: placed away from the town, on a carved floor, with
//! its route walkable — the front stairs to the summit, the ledges to the
//! blue and red readers — its doors shut until opened, and its crates on
//! their own rooms' floors.

// A test prints where it put things: host code, not sim code.
#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    clippy::disallowed_types
)]
use sim_core::{
    fmath::fabs,
    kit,
    monument::{self, Ziggurat},
    occupy::{Occupants, Pristine, SlotCache},
    terrain::{self, Haven, Occupant, ScatterTable},
};

const SEEDS: [u64; 12] = [
    1,
    2,
    7,
    42,
    99,
    1337,
    20_260_731,
    20_260_802,
    555_555,
    8_675_309,
    0xDEAD_BEEF,
    0x0BAD_C0DE,
];

fn occupants<'a>(
    h: &'a Haven,
    table: &'a ScatterTable,
    cache: &'a mut SlotCache,
    doors: u32,
) -> Occupants<'a> {
    Occupants {
        doors,
        table,
        haven: h,
        harvested: &Pristine,
        cache,
    }
}

#[test]
fn every_seed_places_a_ziggurat_far_from_the_town_on_a_flat_floor() {
    for seed in SEEDS {
        let h = terrain::haven(seed);
        let z = h.ziggurat;
        assert!(z.live, "seed {seed}: no ziggurat");
        let t = h.town;
        let d = ((z.x - t.x) * (z.x - t.x) + (z.z - t.z) * (z.z - t.z)).sqrt();
        println!(
            "seed {seed}: ziggurat ({:.0}, {:.0}) floor {:.1} rot {}  {d:.0} m from the town",
            z.x, z.z, z.floor_y, z.rot
        );
        assert!(
            d >= monument::ZIG_TOWN_CLEAR_M,
            "seed {seed}: {d} m from the town"
        );
        assert_eq!(z.x % monument::ZIG_SNAP_M, 0.0);
        assert_eq!(z.z % monument::ZIG_SNAP_M, 0.0);
        for gx in -19..=19 {
            for gz in -19..=19 {
                let (x, wz) = (z.x + gx as f32 * 2.0, z.z + gz as f32 * 2.0);
                let g = terrain::ground(seed, &h, x, wz);
                assert!(
                    fabs(g - z.floor_y) < 0.01,
                    "seed {seed}: {g} vs {}",
                    z.floor_y
                );
            }
        }
        assert!(!h
            .marks
            .iter()
            .any(|m| m.live && monument::covers(&z, m.x, m.z, 26.0)));
        assert!(terrain::build_reserved(&h, z.x, z.z, 0.0));
    }
}

/// Walk a body from local `from` to `to` in 0.1 m steps, stepping up onto
/// whatever is within a step and dropping onto whatever is below; returns
/// the feet at the end.
#[allow(clippy::too_many_arguments)]
fn walk(
    seed: u64,
    occ: &mut Occupants,
    z: &Ziggurat,
    from: (f32, f32),
    to: (f32, f32),
    mut feet: f32,
    what: &str,
) -> f32 {
    let p = z.placed();
    let n = (((to.0 - from.0).abs() + (to.1 - from.1).abs()) / 0.1) as i32;
    for i in 0..=n {
        let f = i as f32 / n.max(1) as f32;
        let (lx, lz) = (from.0 + (to.0 - from.0) * f, from.1 + (to.1 - from.1) * f);
        let (x, wz) = kit::to_world(&p, lx, lz);
        assert!(
            !occ.blocks(seed, x, wz, feet),
            "{what}: blocked at local ({lx:.1}, {lz:.1}) feet {:.2}",
            feet - z.floor_y
        );
        // Up a step, or down to whatever is below: a body falls.
        feet = occ.ground(seed, x, wz, feet).max(z.floor_y);
    }
    feet
}

#[test]
fn the_stairs_climb_to_the_summit_and_the_ledges_reach_the_readers() {
    for seed in [20_260_731u64, 42, 0xDEAD_BEEF] {
        let h = terrain::haven(seed);
        let z = h.ziggurat;
        let table = ScatterTable::alpha_default();
        let mut cache = SlotCache::new();
        let mut occ = occupants(&h, &table, &mut cache, 0);
        let f0 = z.floor_y;
        // Up the front, all five flights.
        let top = walk(
            seed,
            &mut occ,
            &z,
            (0.0, -40.0),
            (0.0, -6.0),
            f0,
            "front stairs",
        );
        assert!(
            fabs(top - (f0 + 25.0)) < 0.01,
            "seed {seed}: summit at {}",
            top - f0
        );
        // From the first landing round the east ledge to the blue reader,
        // stepping off the stairs before the next flight.
        let mut feet = walk(
            seed,
            &mut occ,
            &z,
            (0.0, -40.0),
            (0.0, -31.5),
            f0,
            "flight 0",
        );
        assert!(fabs(feet - (f0 + 5.0)) < 0.01);
        feet = walk(
            seed,
            &mut occ,
            &z,
            (0.0, -31.5),
            (5.0, -31.5),
            feet,
            "the landing",
        );
        feet = walk(
            seed,
            &mut occ,
            &z,
            (5.0, -31.5),
            (29.0, -29.0),
            feet,
            "south ledge",
        );
        let (rx, ry, rz) = monument::DOORS[1].reader;
        feet = walk(
            seed,
            &mut occ,
            &z,
            (29.0, -29.0),
            (rx, rz),
            feet,
            "east ledge",
        );
        assert!(
            fabs(feet - (f0 + ry)) < 0.01,
            "seed {seed}: blue reader level {}",
            feet - f0
        );
        // From the third landing round the north ledge to the red reader.
        let mut feet = walk(
            seed,
            &mut occ,
            &z,
            (0.0, -40.0),
            (0.0, -19.5),
            f0,
            "flights 0-2",
        );
        assert!(fabs(feet - (f0 + 15.0)) < 0.01);
        feet = walk(
            seed,
            &mut occ,
            &z,
            (0.0, -19.5),
            (5.0, -19.5),
            feet,
            "the landing",
        );
        feet = walk(
            seed,
            &mut occ,
            &z,
            (5.0, -19.5),
            (5.0, -17.0),
            feet,
            "onto the ledge",
        );
        feet = walk(
            seed,
            &mut occ,
            &z,
            (5.0, -17.0),
            (17.0, -17.0),
            feet,
            "ledge 2 south",
        );
        feet = walk(
            seed,
            &mut occ,
            &z,
            (17.0, -17.0),
            (17.0, 17.0),
            feet,
            "ledge 2 east",
        );
        let (rx, ry, rz) = monument::DOORS[2].reader;
        feet = walk(
            seed,
            &mut occ,
            &z,
            (17.0, 17.0),
            (rx, rz),
            feet,
            "ledge 2 north",
        );
        assert!(
            fabs(feet - (f0 + ry)) < 0.01,
            "seed {seed}: red reader level"
        );
        // The portal into the entry hall, to the green reader.
        let (rx, _, rz) = monument::DOORS[0].reader;
        walk(seed, &mut occ, &z, (0.0, 40.0), (rx, rz), f0, "portal");
    }
}

#[test]
fn a_shut_door_stops_a_body_and_an_open_one_does_not() {
    let seed = 20_260_731;
    let h = terrain::haven(seed);
    let z = h.ziggurat;
    let table = ScatterTable::alpha_default();
    let rooms = [
        // (door, outside, inside, feet)
        (0usize, (0.0, 14.0), (0.0, 6.0), 0.0),
        (1, (27.0, 0.0), (8.0, 0.0), 5.0),
        (2, (-8.0, 16.0), (-8.0, 2.0), 15.0),
    ];
    for (d, out, inside, y) in rooms {
        let mut cache = SlotCache::new();
        let mut occ = occupants(&h, &table, &mut cache, 0);
        let (bx, bz) = {
            let b = monument::door_box(d).unwrap();
            ((b[0] + b[3]) * 0.5, (b[2] + b[5]) * 0.5)
        };
        let (x, wz) = kit::to_world(&z.placed(), bx, bz);
        assert!(
            occ.blocks(seed, x, wz, z.floor_y + y),
            "door {d} open while shut"
        );
        let mut cache = SlotCache::new();
        let mut occ = occupants(&h, &table, &mut cache, 1 << d);
        walk(
            seed,
            &mut occ,
            &z,
            out,
            inside,
            z.floor_y + y,
            "through the open door",
        );
        assert!(!monument::in_doorway(
            &z,
            d,
            x + 5.0,
            wz + 5.0,
            z.floor_y + y
        ));
        assert!(monument::in_doorway(&z, d, x, wz, z.floor_y + y));
    }
}

#[test]
fn the_crates_sit_on_their_rooms_floors() {
    let seed = 20_260_731;
    let h = terrain::haven(seed);
    let z = h.ziggurat;
    let table = ScatterTable::alpha_default();
    let mut tiers = [0; 3];
    for k in 0..monument::CRATES.len() {
        let (x, y, wz, _, occ) = monument::crate_world(&z, k).unwrap();
        let s = terrain::scatter(
            seed,
            &table,
            &h,
            (x / terrain::CELL_SIZE) as i32,
            (wz / terrain::CELL_SIZE) as i32,
        );
        assert_eq!(s.occupant, occ, "crate {k}");
        assert_eq!((s.x, s.y, s.z), (x, y, wz), "crate {k}");
        match occ {
            Occupant::GreenCrate => tiers[0] += 1,
            Occupant::BlueCrate => tiers[1] += 1,
            Occupant::EliteCrate => tiers[2] += 1,
            _ => {}
        }
    }
    assert_eq!(tiers, [1, 1, 2]);
}
