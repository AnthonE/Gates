//! The authored kit, final terrain, road approaches and actual movement agree.
use sim_core::{
    build::{self, BuildContent, Pieces, BUILD_CELL_M, LOC_PLANE},
    collide::{ColIndex, CAPSULE_HEIGHT_M, CAPSULE_RADIUS_M},
    deploy::{self, DeployContent, Deploys},
    depot,
    gather::ItemStack,
    input::InputFrame,
    movement::{self, Body, POS_XZ_Q, POS_Y_Q},
    occupy::{Occupants, Pristine, SlotCache},
    terrain::{self, Haven, ScatterTable},
    world::{EventQueue, Player},
};

const SEEDS: [u64; 8] = [
    20_260_731,
    42,
    0xDEAD_BEEF,
    1,
    7,
    // These exhausted the first eight-candidate shortlist.
    16_200_471_036_536_512_599,
    982_049_951_981_097_978,
    9_468_199_478_243_335_150,
];

#[test]
fn complete_compounds_have_opposite_distinct_connections_and_flat_footings() {
    for seed in SEEDS {
        let h = terrain::haven(seed);
        assert!(terrain::sites_complete(&h), "seed {seed}");
        let ws = h.minor.iter().find(|s| depot::is_depot(s)).unwrap();
        let [a, b] = h.roads;
        assert_eq!(a.port.wrapping_add(128), b.port);
        assert_eq!(a.port, ws.phase);
        let junction_distance =
            ((a.rx - b.rx) * (a.rx - b.rx) + (a.rz - b.rz) * (a.rz - b.rz)).sqrt();
        assert!(junction_distance > terrain::ROAD_R_MIN);
        for (road, z) in [(a, depot::PORT_Z), (b, -depot::PORT_Z)] {
            let (x, z) = depot::to_world(ws, 0.0, z);
            assert!(
                sim_core::fmath::fabs(road.px - x) < 0.001
                    && sim_core::fmath::fabs(road.pz - z) < 0.001
            );
        }
        for lx in -20..=20 {
            for lz in -24..=24 {
                let (x, z) = depot::to_world(ws, lx as f32, lz as f32);
                assert!(
                    sim_core::fmath::fabs(terrain::ground(seed, &h, x, z) - ws.floor_y) < 0.001
                );
                assert!(terrain::in_waystation(&h, x, z));
            }
        }
        for part in depot::DEPOT_PARTS {
            let b = part.bounds;
            assert!(b[0] < b[3] && b[1] < b[4] && b[2] < b[5]);
            for x in [b[0], b[3]] {
                for z in [b[2], b[5]] {
                    assert!(x * x + z * z < depot::FOOTPRINT.stamp_m * depot::FOOTPRINT.stamp_m);
                }
            }
            if part.bounds[4] <= 0.0 {
                continue;
            }
            let (x, z) = depot::to_world(ws, (b[0] + b[3]) * 0.5, (b[2] + b[5]) * 0.5);
            let y = ws.floor_y + (b[1] + b[4]) * 0.5;
            assert!(depot::blocks(&h, x, z, y, 0.01, 0.01), "{part:?}");
            let support = depot::ground(&h, x, z, ws.floor_y + b[4]);
            assert!(support >= ws.floor_y + b[4] - 0.001);
            assert!(support <= ws.floor_y + b[4] + movement::STEP_UP);
        }
    }
}

fn occupants<'a>(h: &'a Haven, table: &'a ScatterTable, cache: &'a mut SlotCache) -> Occupants<'a> {
    Occupants {
        table,
        haven: h,
        harvested: &Pristine,
        cache,
    }
}

#[test]
fn carriageways_clear_real_capsules_over_final_ground_including_both_edges() {
    for seed in SEEDS {
        let h = terrain::haven(seed);
        let table = ScatterTable::alpha_default();
        let mut cache = SlotCache::new();
        let mut occ = occupants(&h, &table, &mut cache);
        for road in h.roads {
            let dx = road.rx - road.px;
            let dz = road.rz - road.pz;
            let len = (dx * dx + dz * dz).sqrt();
            let n = (len * 2.0) as usize + 1;
            for i in 0..=n {
                let t = i as f32 / n as f32;
                for cross in -2..=2 {
                    // Body centres remain inside the carriageway by their radius.
                    let off = cross as f32 * (terrain::ROAD_HALF_W - CAPSULE_RADIUS_M) * 0.5;
                    let (x, z) = (
                        road.px + dx * t + dz / len * off,
                        road.pz + dz * t - dx / len * off,
                    );
                    let y = terrain::ground(seed, &h, x, z);
                    assert!(y >= terrain::LAND_MIN_H, "seed {seed} water {x},{z}");
                    assert!(
                        terrain::ground_slope(seed, &h, x, z) <= terrain::CLIFF_SLOPE_RATIO,
                        "seed {seed} slope {x},{z}"
                    );
                    assert!(
                        !occ.blocks(seed, x, z, y),
                        "seed {seed} blocked road {x},{z}"
                    );
                }
            }
        }
    }
}

fn walk(seed: u64, h: &Haven, start: (f32, f32), yaw: u8, distance: f32) {
    let table = ScatterTable::alpha_default();
    let mut cache = SlotCache::new();
    let mut occ = occupants(h, &table, &mut cache);
    let cols = ColIndex::new();
    let mut body = Body::at(seed, h, start.0, start.1);
    let frame = InputFrame {
        yaw: (yaw as u16) << 8,
        move_z: 127,
        ..InputFrame::default()
    };
    let (dx, dz) = sim_core::yaw_dir(frame.yaw);
    for _ in 0..2000 {
        movement::step(seed, h, &cols, &mut occ, &mut body, &frame);
        let x = body.qx as f32 * POS_XZ_Q;
        let z = body.qz as f32 * POS_XZ_Q;
        if (x - start.0) * dx + (z - start.1) * dz >= distance - 0.1 {
            return;
        }
        let y = body.qy as f32 * POS_Y_Q;
        assert!(y >= terrain::ground(seed, h, x, z) - 0.02);
    }
    panic!("seed {seed} capsule stalled walking {distance}m from {start:?} bearing {yaw}");
}

#[test]
fn movement_crosses_both_gates_and_the_warehouse_door_in_both_directions() {
    for seed in SEEDS {
        let h = terrain::haven(seed);
        let ws = h.minor.iter().find(|s| depot::is_depot(s)).unwrap();
        for sign in [-1.0, 1.0] {
            let start = depot::to_world(ws, 0.0, sign * (depot::PORT_Z + 8.0));
            let yaw = if sign < 0.0 {
                ws.phase
            } else {
                ws.phase.wrapping_add(128)
            };
            walk(seed, &h, start, yaw, 2.0 * (depot::PORT_Z + 8.0));
        }
        walk(
            seed,
            &h,
            depot::to_world(ws, 0.0, 0.0),
            ws.phase.wrapping_sub(64),
            12.0,
        );
        walk(
            seed,
            &h,
            depot::to_world(ws, -12.0, 0.0),
            ws.phase.wrapping_add(64),
            12.0,
        );
        let (x, z) = depot::to_world(ws, -4.2, 6.0);
        assert!(depot::blocks(
            &h,
            x,
            z,
            ws.floor_y,
            CAPSULE_RADIUS_M,
            CAPSULE_HEIGHT_M
        ));
        let (x, z) = depot::to_world(ws, -4.2, 0.0);
        assert!(!depot::blocks(
            &h,
            x,
            z,
            ws.floor_y,
            CAPSULE_RADIUS_M,
            CAPSULE_HEIGHT_M
        ));
        assert!(depot::blocks(&h, x, z, ws.floor_y + 4.1, 0.01, 0.01));
    }
}

#[test]
fn construction_and_ground_deploys_refuse_the_reserved_yard_without_payment() {
    let seed = 20_260_731;
    let h = terrain::haven(seed);
    let ws = h.minor.iter().find(|s| depot::is_depot(s)).unwrap();
    let bc = BuildContent::probe_fixture();
    let dc = DeployContent::probe_fixture();
    for lx in [0.0, -4.2, 12.0] {
        let (x, z) = depot::to_world(ws, lx, 0.0);
        let (cx, cz) = ((x / BUILD_CELL_M) as u16, (z / BUILD_CELL_M) as u16);
        let mut p = Player {
            id: 1,
            active: true,
            body: Body::at(seed, &h, x, z),
            ..Player::default()
        };
        p.inv[0] = ItemStack {
            item: 0,
            count: 100,
            cond: 0,
        };
        p.inv[1] = ItemStack {
            item: 5,
            count: 1,
            cond: 0,
        };
        let before = p.inv;
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut events = EventQueue::default();
        build::place(
            seed,
            &h,
            &bc,
            &deploys,
            &mut pieces,
            &mut p,
            0,
            0,
            cx,
            cz,
            0,
            LOC_PLANE,
            false,
            0,
            &mut events,
        );
        assert_eq!(events.entries()[0].a, build::REFUSE_B_SPOT);
        assert_eq!(p.inv, before);
        assert_eq!(pieces.len(), 0);
        events = EventQueue::default();
        deploy::place_deploy(
            seed,
            &h,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            3,
            cx,
            cz,
            0,
            LOC_PLANE,
            &mut events,
        );
        assert_eq!(events.entries()[0].a, deploy::REFUSE_D_SPOT);
        assert_eq!(p.inv, before);
        assert_eq!(deploys.len(), 0);
    }
}

/// Exercise the real weapon sampler and damage arbitration, not the depot
/// predicate alone. The open doorway is the positive control: a shot that
/// never fired or missed its target cannot satisfy both cases.
#[test]
fn warehouse_cover_stops_a_bullet_but_its_doorway_allows_the_hit() {
    use sim_core::{
        combat::CombatContent,
        input::BTN_PRIMARY,
        limits::{MAX_ARROWS, MAX_PLAYERS},
        ranged::{self, Chip, Kill, ARROW_EYE_MM, SURF_WORLD},
        rewind::Rewind,
        world::EV_IMPACT,
    };

    // The shared combat fixture arms item 6 with round 7 in magazine 0.
    let cc = CombatContent::probe_fixture();
    let cols = ColIndex::new();
    for seed in [20_260_731, 42] {
        let h = terrain::haven(seed);
        let site = h.minor.iter().find(|s| depot::is_depot(s)).unwrap();
        for (local_z, covered) in [(6.0, true), (0.0, false)] {
            let table = ScatterTable::alpha_default();
            let mut cache = SlotCache::new();
            let mut occ = occupants(&h, &table, &mut cache);
            let mut players = Box::new([Player::default(); MAX_PLAYERS]);
            for (index, local_x) in [0.0, -8.0].into_iter().enumerate() {
                let (x, z) = depot::to_world(site, local_x, local_z);
                players[index] = Player {
                    id: index as u32 + 1,
                    active: true,
                    hp: 100,
                    hp_max: 100,
                    body: Body::at(seed, &h, x, z),
                    ..Player::default()
                };
            }
            // As in tests/gun.rs, put the target's chest on the ray so this
            // checks cover rather than the head/body damage multiplier.
            players[1].body.qy =
                ((site.floor_y + ARROW_EYE_MM as f32 / 1000.0 - 1.2) / POS_Y_Q) as i32;
            players[0].inv[0] = ItemStack {
                item: 6,
                count: 1,
                cond: 0,
            };
            players[0].mag[0] = 1;
            players[0].mag_round[0] = 7;
            players[0].frame = InputFrame {
                buttons: BTN_PRIMARY,
                yaw: (site.phase.wrapping_sub(64) as u16) << 8,
                pitch: 128,
                ..InputFrame::default()
            };
            let mut events = EventQueue::default();
            ranged::hitscan(
                seed,
                &h,
                &cols,
                &mut occ,
                0,
                &Rewind::new(),
                &[0; MAX_PLAYERS],
                &cc,
                &mut players,
                &mut events,
                &mut [Kill::default(); MAX_ARROWS],
                &mut [Chip::default(); MAX_ARROWS],
            );
            assert_eq!(players[0].mag[0], 0, "both cases must fire a round");
            if covered {
                assert_eq!(players[1].hp, 100, "seed {seed}: wall let the shot through");
                let impact = events
                    .entries()
                    .iter()
                    .find(|e| e.code == EV_IMPACT)
                    .expect("the stopped shot must mark its impact");
                assert_eq!((impact.a >> 24) as u8, SURF_WORLD);
            } else {
                assert_eq!(
                    players[1].hp,
                    100 - cc.ranged[6].damage,
                    "seed {seed}: the doorway control must land a chest hit"
                );
            }
        }
    }
}
