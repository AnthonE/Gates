//! Socket inserts through the real placement, damage, lock and save paths.
use sim_core::build::{self, *};
use sim_core::collide::{self, ColIndex};
use sim_core::deploy::*;
use sim_core::gather::ItemStack;
use sim_core::movement::Body;
use sim_core::terrain;
use sim_core::world::{EventQueue, Player, World, EV_DEPLOY_REFUSED};
use sim_core::worldsave;

const SEED: u64 = 20260731;
const CX: u16 = 341;
const CZ: u16 = 341;
fn haven() -> &'static terrain::Haven {
    static H: std::sync::OnceLock<terrain::Haven> = std::sync::OnceLock::new();
    H.get_or_init(|| terrain::haven(SEED))
}

fn fixture(arch: u8, loc: u8, level: u8) -> (Box<World>, Player) {
    let mut w = Box::new(World::new(SEED));
    w.build = BuildContent::probe_fixture();
    for def in &mut w.build.pieces {
        def.n_costs = 0;
    }
    w.build.pieces[4] = PieceDef {
        shape: if arch != ARCH_GARAGE_DOOR {
            SHAPE_WINDOW
        } else {
            SHAPE_FRAME
        },
        material: MAT_TWIG,
        hp: 100,
        ..PieceDef::INERT
    };
    w.build.piece_count = w.build.piece_count.max(5);
    w.deploy = DeployContent::probe_fixture();
    w.deploy.defs[0] = DeployDef {
        arch,
        placement: if arch != ARCH_GARAGE_DOOR {
            PLACE_WINDOW
        } else {
            PLACE_FRAME
        },
        item: 10,
        hp: 500,
        ..DeployDef::INERT
    };
    w.deploy.defs[1] = DeployDef {
        arch: ARCH_LOCK,
        placement: PLACE_DOOR,
        item: 11,
        hp: 100,
        ..DeployDef::INERT
    };
    let (x, z) = cell_center(CX, CZ);
    let mut p = Player {
        id: 7,
        active: true,
        body: Body::at(SEED, haven(), x, z),
        ..Player::default()
    };
    p.inv[0] = ItemStack {
        item: 10,
        count: 10,
        cond: 0,
        skin: 0,
    };
    p.inv[1] = ItemStack {
        item: 11,
        count: 10,
        cond: 0,
        skin: 0,
    };
    let mut ev = EventQueue::default();
    for l in 0..=level {
        build::place(
            SEED,
            haven(),
            &w.build,
            &w.deploys,
            &mut w.pieces,
            &mut p,
            0,
            if l == 0 { 0 } else { 2 },
            CX,
            CZ,
            l,
            LOC_PLANE,
            false,
            0,
            &mut ev,
        );
        assert!(
            w.pieces.find(CX, CZ, l, LOC_PLANE).is_some(),
            "floor refused: {:?}",
            ev.entries()
        );
        build::place(
            SEED,
            haven(),
            &w.build,
            &w.deploys,
            &mut w.pieces,
            &mut p,
            0,
            if l == level { 4 } else { 1 },
            CX,
            CZ,
            l,
            loc,
            false,
            0,
            &mut ev,
        );
        assert!(
            w.pieces.find(CX, CZ, l, loc).is_some(),
            "edge refused: {:?}",
            ev.entries()
        );
    }
    (w, p)
}
fn put(w: &mut World, p: &mut Player, row: u16, loc: u8, level: u8) -> EventQueue {
    let mut ev = EventQueue::default();
    place_deploy(
        SEED,
        haven(),
        &w.deploy,
        &w.build,
        &mut w.pieces,
        &mut w.deploys,
        p,
        0,
        row,
        CX,
        CZ,
        level,
        loc,
        &mut ev,
    );
    ev
}
fn shot(
    cols: &ColIndex,
    loc: u8,
    level: u8,
    along: f32,
    height: f32,
) -> Option<(collide::PieceHit, bool)> {
    let base = column_floor_y(SEED, haven(), CX, CZ, cols.plate(CX, CZ).unwrap_or(0))
        + level as f32 * LEVEL_H_M;
    let x = CX as f32 * BUILD_CELL_M;
    let z = CZ as f32 * BUILD_CELL_M;
    let (sx, sz, ex, ez) = if loc == LOC_EDGE_XLO {
        (x - 0.4, z + along, x + 0.4, z + along)
    } else {
        (x + along, z - 0.4, x + along, z + 0.4)
    };
    collide::shot_hit(SEED, haven(), cols, sx, sz, ex, ez, base + height, 0.0)
}

#[test]
fn bars_leave_shooting_gaps_and_die_separately_from_the_socket() {
    for loc in [LOC_EDGE_XLO, LOC_EDGE_ZLO] {
        for level in [0, 2] {
            let (mut w, mut p) = fixture(ARCH_WINDOW_BARS, loc, level);
            assert!(shot(w.pieces.cols(), loc, level, 1.5, 1.6).is_none());
            let ev = put(&mut w, &mut p, 0, loc, level);
            assert!(!ev.entries().iter().any(|e| e.code == EV_DEPLOY_REFUSED));
            assert_eq!(p.inv[0].count, 9);
            assert!(shot(w.pieces.cols(), loc, level, 1.5, 1.6).unwrap().1);
            assert!(shot(w.pieces.cols(), loc, level, 1.35, 1.6).is_none());
            assert!(!shot(w.pieces.cols(), loc, level, 0.5, 1.6).unwrap().1);
            let duplicate = put(&mut w, &mut p, 0, loc, level);
            assert_eq!(duplicate.entries()[0].b, REFUSE_D_SPOT);
            assert_eq!(p.inv[0].count, 9);
            let mut ev = EventQueue::default();
            assert!(use_door(
                &w.deploy,
                &mut w.pieces,
                &mut w.deploys,
                &mut p,
                CX,
                CZ,
                level,
                loc,
                &mut ev
            )
            .is_none());
            assert!(damage_deploy(
                &w.deploy,
                &mut w.pieces,
                &mut w.deploys,
                0,
                500,
                &mut ev
            ));
            assert!(w.pieces.find(CX, CZ, level, loc).is_some());
            assert!(shot(w.pieces.cols(), loc, level, 1.5, 1.6).is_none());
        }
    }
}

#[test]
fn garage_door_works_with_the_existing_lock_and_removal_paths() {
    for loc in [LOC_EDGE_XLO, LOC_EDGE_ZLO] {
        for level in [0, 2] {
            let (mut w, mut p) = fixture(ARCH_GARAGE_DOOR, loc, level);
            assert!(shot(w.pieces.cols(), loc, level, 1.5, 1.6).is_none());
            let _ = put(&mut w, &mut p, 0, loc, level);
            assert!(shot(w.pieces.cols(), loc, level, 1.5, 1.6).unwrap().1);
            let mut ev = EventQueue::default();
            assert!(use_door(
                &w.deploy,
                &mut w.pieces,
                &mut w.deploys,
                &mut p,
                CX,
                CZ,
                level,
                loc,
                &mut ev
            )
            .is_some());
            assert!(shot(w.pieces.cols(), loc, level, 1.5, 1.6).is_none());
            assert!(use_door(
                &w.deploy,
                &mut w.pieces,
                &mut w.deploys,
                &mut p,
                CX,
                CZ,
                level,
                loc,
                &mut ev
            )
            .is_some());
            let ev = put(&mut w, &mut p, 1, loc, level);
            assert!(!ev.entries().iter().any(|e| e.code == EV_DEPLOY_REFUSED));
            assert!(w.deploys.find(CX, CZ, level, loc).unwrap().has_lock);
            // Destroy the socket; its insert and attached lock must leave too.
            let i = w
                .pieces
                .entries()
                .iter()
                .position(|r| r.level == level && r.loc == loc)
                .unwrap();
            let mut ev = EventQueue::default();
            let mut budget = sim_core::limits::MAX_REMOVALS_PER_TICK;
            assert!(damage_piece(
                &w.deploy,
                &w.build,
                &mut w.pieces,
                &mut w.deploys,
                i,
                1000,
                &mut budget,
                &mut ev
            ));
            assert!(w.deploys.find(CX, CZ, level, loc).is_none());
            assert_eq!(w.deploys.locks().len(), 0);
            assert!(shot(w.pieces.cols(), loc, level, 1.5, 1.6).is_none());
        }
    }
}

#[test]
fn wrong_socket_refuses_without_spending_the_item() {
    for arch in [ARCH_WINDOW_BARS, ARCH_GARAGE_DOOR] {
        let (mut w, mut p) = fixture(arch, LOC_EDGE_XLO, 0);
        w.deploy.defs[0].placement = if arch != ARCH_GARAGE_DOOR {
            PLACE_FRAME
        } else {
            PLACE_WINDOW
        };
        let ev = put(&mut w, &mut p, 0, LOC_EDGE_XLO, 0);
        assert_eq!(ev.entries()[0].b, REFUSE_D_SUPPORT);
        assert_eq!(p.inv[0].count, 10);
        assert!(w.deploys.entries().is_empty());
    }
}

#[test]
fn save_load_rebuilds_insert_collision_at_both_storeys() {
    for arch in [ARCH_WINDOW_BARS, ARCH_GARAGE_DOOR] {
        for level in [0, 2] {
            let (mut w, mut p) = fixture(arch, LOC_EDGE_ZLO, level);
            let _ = put(&mut w, &mut p, 0, LOC_EDGE_ZLO, level);
            let mut bytes = vec![0; worldsave::WORLD_SAVE_MAX_BYTES];
            let n = worldsave::encode(&w, &mut bytes).unwrap();
            let mut back = Box::new(World::new(SEED));
            back.build = w.build;
            back.deploy = w.deploy;
            worldsave::decode_into(&mut back, &bytes[..n]).unwrap();
            assert!(
                shot(back.pieces.cols(), LOC_EDGE_ZLO, level, 1.5, 1.6)
                    .unwrap()
                    .1
            );
            assert_eq!(back.deploys.entries(), w.deploys.entries());
            if arch != ARCH_GARAGE_DOOR {
                assert!(shot(back.pieces.cols(), LOC_EDGE_ZLO, level, 1.35, 1.6).is_none());
            }
        }
    }
}

#[test]
fn a_lower_adjacent_floor_seals_the_wall_foot_without_moving_its_head() {
    for loc in [LOC_EDGE_XLO, LOC_EDGE_ZLO] {
        for level in [0, 2] {
            let mut cols = ColIndex::new();
            let (ox, oz) = if loc == LOC_EDGE_XLO {
                (CX - 1, CZ)
            } else {
                (CX, CZ - 1)
            };
            let band = terrain_band(SEED, haven(), CX, CZ);
            let other_plate = (band - terrain_band(SEED, haven(), ox, oz)) as i8;
            cols.add(CX, CZ, level, loc, SHAPE_WALL, 1);
            assert!(shot(&cols, loc, level, 1.5, -0.25).is_none());
            cols.add(ox, oz, level, LOC_PLANE, SHAPE_FLOOR, other_plate);
            assert_eq!(
                collide::edge_foot_drop(SEED, haven(), &cols, CX, CZ, level, loc, 1),
                BUILD_BASE_Q_M
            );
            assert!(!shot(&cols, loc, level, 1.5, -0.25).unwrap().1);
            assert!(!shot(&cols, loc, level, 1.5, LEVEL_H_M - 0.1).unwrap().1);
            assert!(shot(&cols, loc, level, 1.5, -BUILD_BASE_Q_M - 0.1).is_none());
            cols.del(ox, oz, level, LOC_PLANE, SHAPE_FLOOR);
            assert!(shot(&cols, loc, level, 1.5, -0.25).is_none());
        }
    }
}

#[test]
fn glass_and_shutters_seal_bar_gaps_and_only_shutters_open() {
    for arch in [ARCH_WINDOW_GLASS, ARCH_WINDOW_SHUTTER] {
        for loc in [LOC_EDGE_XLO, LOC_EDGE_ZLO] {
            for level in [0, 2] {
                let (mut w, mut p) = fixture(arch, loc, level);
                let _ = put(&mut w, &mut p, 0, loc, level);
                assert!(shot(w.pieces.cols(), loc, level, 1.35, 1.6).unwrap().1);
                let ev = put(&mut w, &mut p, 1, loc, level);
                assert!(ev.entries().iter().any(|e| e.code == EV_DEPLOY_REFUSED));
                assert!(!w.deploys.entries()[0].has_lock);
                let mut ev = EventQueue::default();
                let toggled = use_door(
                    &w.deploy,
                    &mut w.pieces,
                    &mut w.deploys,
                    &mut p,
                    CX,
                    CZ,
                    level,
                    loc,
                    &mut ev,
                );
                if arch == ARCH_WINDOW_SHUTTER {
                    assert!(toggled.is_some());
                    assert!(shot(w.pieces.cols(), loc, level, 1.35, 1.6).is_none());
                    use_door(
                        &w.deploy,
                        &mut w.pieces,
                        &mut w.deploys,
                        &mut p,
                        CX,
                        CZ,
                        level,
                        loc,
                        &mut ev,
                    );
                } else {
                    assert!(toggled.is_none());
                }
                assert!(shot(w.pieces.cols(), loc, level, 1.35, 1.6).unwrap().1);
                let mut bytes = vec![0; worldsave::WORLD_SAVE_MAX_BYTES];
                let n = worldsave::encode(&w, &mut bytes).unwrap();
                let mut back = Box::new(World::new(SEED));
                back.build = w.build;
                back.deploy = w.deploy;
                worldsave::decode_into(&mut back, &bytes[..n]).unwrap();
                assert!(shot(back.pieces.cols(), loc, level, 1.35, 1.6).unwrap().1);
                assert!(damage_deploy(
                    &w.deploy,
                    &mut w.pieces,
                    &mut w.deploys,
                    0,
                    500,
                    &mut ev
                ));
                assert!(shot(w.pieces.cols(), loc, level, 1.35, 1.6).is_none());
            }
        }
    }
}
