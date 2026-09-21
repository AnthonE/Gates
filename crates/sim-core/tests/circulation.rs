//! Real movement, placement and removal through every circulation footprint.
use sim_core::fmath::fabs;
use sim_core::{
    build::{self, *},
    circulation,
    collide::{self, ColIndex},
    input::InputFrame,
    movement::{self, Body, POS_XZ_Q, POS_Y_Q},
    occupy::Scratch,
    terrain,
};
const SEED: u64 = 20260731;
const CX: u16 = 341;
const CZ: u16 = 341;
fn world_point(loc: u8, x: f32, z: f32) -> (f32, f32) {
    match loc {
        LOC_RISER_XHI => (z, 3.0 - x),
        LOC_RISER_ZLO => (3.0 - x, 3.0 - z),
        LOC_RISER_XLO => (3.0 - z, x),
        _ => (x, z),
    }
}
fn route(shape: u8) -> Vec<(f32, f32)> {
    match shape {
        SHAPE_STAIRS_L => vec![(0.5, 0.1), (0.5, 2.5), (2.9, 2.5)],
        SHAPE_STAIRS_U => vec![(0.5, 0.1), (0.5, 2.5), (2.5, 2.5), (2.5, 0.1)],
        SHAPE_STAIRS_SPIRAL => vec![(0.5, 1.5), (0.5, 2.5), (2.5, 2.5), (2.5, 1.5)],
        SHAPE_STAIRS_TRI_SPIRAL => vec![(0.5, 2.45), (0.5, 0.5), (2.45, 0.5)],
        _ => vec![(1.5, 0.1), (1.5, 2.9)],
    }
}
#[test]
fn all_flights_walk_in_both_directions_after_every_quarter_turn() {
    let haven = terrain::haven(SEED);
    let base = column_floor_y(SEED, &haven, CX, CZ, 0);
    for shape in [
        SHAPE_RAMP,
        SHAPE_STAIRS_L,
        SHAPE_STAIRS_U,
        SHAPE_STAIRS_SPIRAL,
        SHAPE_STAIRS_TRI_SPIRAL,
    ] {
        for loc in STAIR_LOCS {
            for level in [0, 8, 2] {
                let mut cols = ColIndex::new();
                cols.add(CX, CZ, level, LOC_PLANE, SHAPE_FLOOR, 0);
                cols.add(CX, CZ, level, loc, shape, 0);
                let route = route(shape);
                let (x, z) = world_point(loc, route[0].0, route[0].1);
                let mut body = Body::at(SEED, &haven, CX as f32 * 3.0 + x, CZ as f32 * 3.0 + z);
                body.qy = movement::quant_y(
                    base + level_y(level) + circulation::surface(shape, loc, x, z).unwrap(),
                );
                let mut scratch = Scratch::barren();
                for &(x, z) in route.iter().skip(1).chain(route.iter().rev().skip(1)) {
                    let (x, z) = world_point(loc, x, z);
                    let target = (CX as f32 * 3.0 + x, CZ as f32 * 3.0 + z);
                    let mut arrived = false;
                    for _ in 0..160 {
                        let dx = target.0 - body.qx as f32 * POS_XZ_Q;
                        let dz = target.1 - body.qz as f32 * POS_XZ_Q;
                        if fabs(dx) < 0.055 && fabs(dz) < 0.055 {
                            arrived = true;
                            break;
                        }
                        let scale = fabs(dx).max(fabs(dz));
                        movement::step(
                            SEED,
                            &haven,
                            &cols,
                            &mut scratch.occupants(),
                            &mut body,
                            &InputFrame {
                                move_x: (dx / scale * 40.0) as i8,
                                move_z: (dz / scale * 40.0) as i8,
                                ..InputFrame::default()
                            },
                        );
                    }
                    assert!(
                        arrived,
                        "shape {shape}, loc {loc}, level {level}, target {target:?}, body {body:?}"
                    );
                    let expected =
                        base + level_y(level) + circulation::surface(shape, loc, x, z).unwrap();
                    assert!(
                        fabs(body.qy as f32 * POS_Y_Q - expected) < 0.15,
                        "shape {shape}, expected {expected}, body {body:?}"
                    );
                }
            }
        }
    }
}
#[test]
fn triangular_frame_keeps_an_open_centre_and_solid_rim() {
    let haven = terrain::haven(SEED);
    let base = column_floor_y(SEED, &haven, CX, CZ, 0);
    for loc in [
        LOC_TRI_XLO_ZLO,
        LOC_TRI_XLO_ZHI,
        LOC_TRI_XHI_ZHI,
        LOC_TRI_XHI_ZLO,
    ] {
        let mut cols = ColIndex::new();
        cols.add(CX, CZ, 1, loc, SHAPE_TRI_FLOOR_FRAME, 0);
        // Each orientation's right-angle corner is the rim; its centroid is clear.
        let (x, z) = match loc {
            LOC_TRI_XLO_ZLO => (1.0, 1.0),
            LOC_TRI_XLO_ZHI => (1.0, 2.0),
            LOC_TRI_XHI_ZHI => (2.0, 2.0),
            _ => (2.0, 1.0),
        };
        let origin = (CX as f32 * 3.0, CZ as f32 * 3.0);
        let ground =
            collide::piece_ground(SEED, &haven, &cols, origin.0 + x, origin.1 + z, base + 3.0);
        assert!(ground < base + 2.0, "open centre became a plane");
        assert!(collide::shot_stop(
            SEED,
            &haven,
            &cols,
            origin.0 + x,
            origin.1 + z,
            origin.0 + x + 0.01,
            origin.1 + z,
            base + 2.9,
            sim_core::ranged::ARROW_R_M
        )
        .is_none());
        let (x, z) = (
            if x < 1.5 { 0.1 } else { 2.9 },
            if z < 1.5 { 0.1 } else { 2.9 },
        );
        assert_eq!(
            collide::piece_ground(SEED, &haven, &cols, origin.0 + x, origin.1 + z, base + 3.0),
            base + 3.0
        );
        assert!(collide::shot_stop(
            SEED,
            &haven,
            &cols,
            origin.0 + x,
            origin.1 + z,
            origin.0 + x + 0.01,
            origin.1 + z,
            base + 2.9,
            sim_core::ranged::ARROW_R_M
        )
        .is_some());
    }
}

#[test]
fn stacked_spirals_save_collapse_and_refuse_a_loaded_rotation() {
    use sim_core::{
        deploy::DeployContent,
        gather::GatherContent,
        limits::{INV_SLOTS, MAX_REMOVALS_PER_TICK},
        world::{EventQueue, Player, World},
        worldsave,
    };
    for shape in [SHAPE_STAIRS_SPIRAL, SHAPE_STAIRS_TRI_SPIRAL] {
        let mut w = Box::new(World::new(SEED));
        w.build.piece_count = 21;
        for row in 0..21 {
            w.build.pieces[row] = PieceDef {
                shape: row as u8,
                material: MAT_TWIG,
                hp: 100,
                ..PieceDef::INERT
            };
        }
        let mut p = Player {
            id: 7,
            active: true,
            body: Body::at(SEED, &w.haven, CX as f32 * 3.0 + 1.5, CZ as f32 * 3.0 + 1.5),
            ..Player::default()
        };
        for (row, level, loc) in [
            (SHAPE_FOUNDATION, 0, LOC_PLANE),
            (shape, 0, LOC_RISER),
            (shape, 8, LOC_RISER_ZLO),
            (shape, 1, LOC_RISER),
        ] {
            let before = w.pieces.len();
            build::place(
                SEED,
                &w.haven,
                &w.build,
                &w.deploys,
                &mut w.pieces,
                &mut p,
                0,
                row as u16,
                CX,
                CZ,
                level,
                loc,
                false,
                0,
                &mut EventQueue::default(),
            );
            assert_eq!(w.pieces.len(), before + 1, "shape {shape} level {level}");
        }
        let mut events = EventQueue::default();
        build::rotate(
            &w.build,
            &w.deploys,
            &mut w.pieces,
            &p,
            1,
            CX,
            CZ,
            0,
            LOC_RISER,
            &mut events,
        );
        assert!(w.pieces.find(CX, CZ, 0, LOC_RISER).is_some());
        assert!(w.pieces.find(CX, CZ, 0, LOC_RISER_XHI).is_none());
        let mut bytes = vec![0; worldsave::WORLD_SAVE_MAX_BYTES];
        let n = worldsave::encode(&w, &mut bytes).unwrap();
        let mut restored = World::new(SEED);
        restored.build = w.build;
        worldsave::decode_into(&mut restored, &bytes[..n]).unwrap();
        assert_eq!(
            restored.pieces.cols().get(CX, CZ),
            w.pieces.cols().get(CX, CZ)
        );
        let mut budget = MAX_REMOVALS_PER_TICK;
        build::demolish(
            &DeployContent::EMPTY,
            &w.build,
            &GatherContent::EMPTY,
            &mut w.pieces,
            &mut w.deploys,
            &mut p,
            1,
            CX,
            CZ,
            0,
            LOC_RISER,
            &mut budget,
            &mut events,
            &mut [Default::default(); INV_SLOTS],
        );
        assert_eq!(w.pieces.len(), 1, "only the foundation survives");
    }
}

#[test]
fn stepped_foundations_bear_only_the_high_edge_and_keep_plate_limits() {
    let haven = terrain::haven(SEED);
    assert_eq!(
        build::plate_for(
            &ColIndex::new(),
            SEED,
            &haven,
            CX,
            CZ,
            true,
            (PLATE_RISE_MAX_BANDS + 1) as i8
        ),
        Err(REFUSE_B_PLATE_HIGH)
    );
    use sim_core::world::{EventQueue, Player, World};
    for loc in STAIR_LOCS {
        let mut w = Box::new(World::new(SEED));
        w.build.piece_count = 21;
        for row in 0..21 {
            w.build.pieces[row] = PieceDef {
                shape: row as u8,
                material: MAT_TWIG,
                hp: 100,
                ..PieceDef::INERT
            };
        }
        let mut p = Player {
            id: 7,
            active: true,
            body: Body::at(SEED, &w.haven, CX as f32 * 3.0 + 1.5, CZ as f32 * 3.0 + 1.5),
            ..Player::default()
        };
        build::place(
            SEED,
            &w.haven,
            &w.build,
            &w.deploys,
            &mut w.pieces,
            &mut p,
            0,
            SHAPE_FOUNDATION_STEPS as u16,
            CX,
            CZ,
            0,
            loc,
            true,
            1,
            &mut EventQueue::default(),
        );
        assert_eq!(w.pieces.len(), 1);
        let top = column_floor_y(SEED, &w.haven, CX, CZ, w.pieces.entries()[0].plate);
        let (low_x, low_z) = world_point(loc, 1.5, 0.1);
        let (high_x, high_z) = world_point(loc, 1.5, 2.9);
        for (x, z, expected) in [(low_x, low_z, top - 1.45), (high_x, high_z, top - 0.05)] {
            assert!(
                fabs(
                    collide::piece_ground(
                        SEED,
                        &w.haven,
                        w.pieces.cols(),
                        CX as f32 * 3.0 + x,
                        CZ as f32 * 3.0 + z,
                        top
                    ) - expected
                ) < 0.001
            );
        }
        let bearing = match loc {
            LOC_RISER => (CX, CZ + 1, LOC_EDGE_ZLO),
            LOC_RISER_XHI => (CX + 1, CZ, LOC_EDGE_XLO),
            LOC_RISER_ZLO => (CX, CZ, LOC_EDGE_ZLO),
            _ => (CX, CZ, LOC_EDGE_XLO),
        };
        build::place(
            SEED,
            &w.haven,
            &w.build,
            &w.deploys,
            &mut w.pieces,
            &mut p,
            0,
            SHAPE_WALL as u16,
            bearing.0,
            bearing.1,
            0,
            bearing.2,
            false,
            0,
            &mut EventQueue::default(),
        );
        assert_eq!(w.pieces.len(), 2, "high edge carries wall, loc {loc}");
        build::rotate(
            &w.build,
            &w.deploys,
            &mut w.pieces,
            &p,
            1,
            CX,
            CZ,
            0,
            loc,
            &mut EventQueue::default(),
        );
        assert!(
            w.pieces.find(CX, CZ, 0, loc).is_some(),
            "cannot turn under a loaded edge"
        );
        // A step cannot secretly become a full slab or overlap one.
        build::place(
            SEED,
            &w.haven,
            &w.build,
            &w.deploys,
            &mut w.pieces,
            &mut p,
            0,
            SHAPE_FOUNDATION as u16,
            CX,
            CZ,
            0,
            LOC_PLANE,
            false,
            0,
            &mut EventQueue::default(),
        );
        assert_eq!(w.pieces.len(), 2);
    }
}
