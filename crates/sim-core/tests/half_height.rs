//! Half-storey addresses must carry real support, collision and save identity.
use sim_core::build::{self, *};
use sim_core::collide::{self, ColIndex};
use sim_core::deploy::DeployContent;
use sim_core::gather::GatherContent;
use sim_core::limits::{INV_SLOTS, MAX_BUILD_SOCKETS, MAX_REMOVALS_PER_TICK};
use sim_core::movement::Body;
use sim_core::terrain;
use sim_core::world::{EventQueue, Player, World};
use sim_core::worldsave;
const SEED: u64 = 20260731;
const CX: u16 = 341;
const CZ: u16 = 341;
fn fixture() -> (Box<World>, Player) {
    let mut w = Box::new(World::new(SEED));
    w.build = BuildContent::probe_fixture();
    w.build.piece_count = 14;
    for (row, shape) in [
        (0, SHAPE_FOUNDATION),
        (1, SHAPE_WALL),
        (2, SHAPE_FLOOR),
        (12, SHAPE_HALF_WALL),
        (13, SHAPE_LOW_WALL),
    ] {
        w.build.pieces[row] = PieceDef {
            shape,
            material: MAT_TWIG,
            hp: 100,
            ..PieceDef::INERT
        };
    }
    let p = Player {
        id: 7,
        active: true,
        body: Body::at(
            SEED,
            &w.haven,
            CX as f32 * BUILD_CELL_M + 1.5,
            CZ as f32 * BUILD_CELL_M + 1.5,
        ),
        ..Player::default()
    };
    (w, p)
}
fn place(w: &mut World, p: &mut Player, row: u16, level: u8, loc: u8) -> bool {
    let n = w.pieces.len();
    build::place(
        SEED,
        &w.haven,
        &w.build,
        &w.deploys,
        &mut w.pieces,
        p,
        0,
        row,
        CX,
        CZ,
        level,
        loc,
        false,
        0,
        &mut EventQueue::default(),
    );
    w.pieces.len() > n
}
#[test]
fn half_walls_bear_half_storeys_and_collapse_their_actual_dependents() {
    for loc in [LOC_EDGE_XLO, LOC_EDGE_ZLO] {
        let (mut w, mut p) = fixture();
        assert!(place(&mut w, &mut p, 0, 0, LOC_PLANE));
        assert!(place(&mut w, &mut p, 12, 0, loc));
        assert!(
            !place(&mut w, &mut p, 2, 1, LOC_PLANE),
            "half a wall cannot support a full-height ceiling"
        );
        assert!(place(&mut w, &mut p, 2, 8, LOC_PLANE));
        assert!(place(&mut w, &mut p, 12, 8, loc));
        assert!(place(&mut w, &mut p, 2, 1, LOC_PLANE));
        let mut blob = vec![0; worldsave::WORLD_SAVE_MAX_BYTES];
        let n = worldsave::encode(&w, &mut blob).unwrap();
        let mut restored = World::new(SEED);
        restored.build = w.build;
        worldsave::decode_into(&mut restored, &blob[..n]).unwrap();
        assert_eq!(
            restored.pieces.cols().get(CX, CZ),
            w.pieces.cols().get(CX, CZ)
        );
        assert!(restored.pieces.find(CX, CZ, 8, LOC_PLANE).is_some());
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
            loc,
            &mut budget,
            &mut EventQueue::default(),
            &mut [Default::default(); INV_SLOTS],
        );
        assert_eq!(w.pieces.len(), 1, "the ground foundation alone survives");
    }
}
#[test]
fn low_cover_bears_no_floor_and_offset_edges_cannot_interpenetrate() {
    let (mut w, mut p) = fixture();
    assert!(place(&mut w, &mut p, 0, 0, LOC_PLANE));
    assert!(place(&mut w, &mut p, 13, 0, LOC_EDGE_XLO));
    for level in [8, 1] {
        assert!(!place(&mut w, &mut p, 2, level, LOC_PLANE));
    }
    assert!(place(&mut w, &mut p, 1, 0, LOC_EDGE_ZLO));
    assert!(!place(&mut w, &mut p, 12, 8, LOC_EDGE_ZLO));
}
#[test]
fn partial_edges_block_only_their_drawn_height_on_both_axes_and_diagonals() {
    let haven = terrain::haven(SEED);
    for shape in [SHAPE_HALF_WALL, SHAPE_LOW_WALL] {
        for level in [0, 8, 2, 10] {
            for loc in [LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_DIAG_A, LOC_DIAG_B] {
                let mut cols = ColIndex::new();
                cols.add(CX, CZ, level, loc, shape, 0);
                let base = column_floor_y(SEED, &haven, CX, CZ, 0) + level_y(level);
                let x = CX as f32 * BUILD_CELL_M;
                let z = CZ as f32 * BUILD_CELL_M;
                let (a, b) = match loc {
                    LOC_EDGE_XLO => ((x - 0.5, z + 1.5), (x + 0.5, z + 1.5)),
                    LOC_EDGE_ZLO => ((x + 1.5, z - 0.5), (x + 1.5, z + 0.5)),
                    _ => ((x + 1.0, z + 1.5), (x + 2.0, z + 1.5)),
                };
                for (y, hit) in [
                    (base + wall_height(shape) - 0.1, true),
                    (base + wall_height(shape) + 0.1, false),
                ] {
                    assert_eq!(
                        collide::shot_stop(SEED, &haven, &cols, a.0, a.1, b.0, b.1, y, 0.0)
                            .is_some(),
                        hit
                    );
                }
            }
        }
    }
}
#[test]
fn socket_steps_preserve_old_addresses_and_stop_at_the_height_cap() {
    for level in 0..8 {
        assert_eq!(level_y(level), level as f32 * LEVEL_H_M);
    }
    for level in 0..MAX_BUILD_SOCKETS as u8 {
        if let Some(next) = level_step(level, 1) {
            assert_eq!(level_y(next), level_y(level) + LEVEL_H_M * 0.5);
            assert_eq!(level_step(next, -1), Some(level));
        }
    }
    assert_eq!(level_step(0, -1), None);
    assert_eq!(level_step(15, 1), None);
}
