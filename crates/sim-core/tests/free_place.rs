//! Free placement: a body deployable stands where it was placed inside its
//! cell, facing where it was turned, and the sim reads it there — placement
//! clearance, support, the support cascade, collision and the save file.

use sim_core::build::{
    foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_EDGE_XLO, LOC_PLANE, REFUSE_B_SPOT,
};
use sim_core::collide;
use sim_core::deploy::{
    body_base_y, box_key, DeployContent, DeployDef, ARCH_BOX, ARCH_FIRE, ARCH_FURNACE, PLACE_ANY,
    PLACE_FOUNDATION, REFUSE_D_SPOT,
};
use sim_core::footprint::Pose;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::movement::Body;
use sim_core::terrain;
use sim_core::world::{Command, World, EV_BUILD_REFUSED, EV_DEPLOY_REFUSED};
use sim_core::worldsave::WORLD_SAVE_MAX_BYTES;

fn hv(seed: u64) -> &'static terrain::Haven {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static terrain::Haven = Box::leak(Box::new(terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

const SEED: u64 = 0x50_11D0;
const PLAYER: u32 = 3;

const BOX_ROW: u16 = 0;
const FURNACE_ROW: u16 = 1;
const FIRE_ROW: u16 = 2;
const FOUNDATION_ROW: u16 = 0;
const WALL_ROW: u16 = 1;

fn content() -> DeployContent {
    let mut d = DeployContent::EMPTY;
    d.defs[0] = DeployDef {
        arch: ARCH_BOX,
        placement: PLACE_ANY,
        hp: 100,
        item: 6,
        ..DeployDef::INERT
    };
    d.defs[1] = DeployDef {
        arch: ARCH_FURNACE,
        placement: PLACE_FOUNDATION,
        hp: 100,
        item: 7,
        ..DeployDef::INERT
    };
    d.defs[2] = DeployDef {
        arch: ARCH_FIRE,
        placement: PLACE_ANY,
        hp: 50,
        item: 8,
        ..DeployDef::INERT
    };
    d.def_count = 3;
    d
}

fn centre(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

/// Four buildable cells in a row along x, near the island's middle.
fn run_of_four(seed: u64) -> (u16, u16) {
    for r in 0..96i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let a = (682 + dx).clamp(0, 2040) as u16;
                let cz = (682 + dz).clamp(0, 2040) as u16;
                if (0..4).all(|k| {
                    let (x, z) = centre(a + k, cz);
                    foundation_terrain_ok(seed, hv(seed), x, z)
                        && terrain::ground_slope(seed, hv(seed), x, z) < 0.2
                }) {
                    return (a, cz);
                }
            }
        }
    }
    panic!("no flat run of four buildable cells near the centre");
}

/// A world with foundations on cells `a+1` and `a+2` (one plate), cell `a`
/// left bare, and the player standing on the seam between the two
/// foundations holding boxes, furnaces and fires.
fn world() -> (World, u16, u16) {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = content();
    let (a, cz) = run_of_four(SEED);
    let x = (a + 2) as f32 * BUILD_CELL_M;
    let z = centre(a, cz).1 - 1.0;
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: PLAYER }]);
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
    for (slot, item) in [(0usize, 0u16), (1, 6), (2, 7), (3, 8)] {
        w.players[0].inv[slot] = ItemStack {
            item,
            count: 40,
            cond: 0,
            skin: 0,
        };
    }
    for cx in [a + 1, a + 2] {
        w.tick(&[Command::Place {
            id: PLAYER,
            row: FOUNDATION_ROW,
            cx,
            cz,
            level: 0,
            loc: LOC_PLANE,
            freehand: false,
            plate: 0,
        }]);
    }
    assert_eq!(w.pieces.len(), 2, "the fixture needs both foundations");
    let floor = |cx: u16| {
        body_base_y(
            SEED,
            hv(SEED),
            w.pieces.cols(),
            &sim_core::deploy::DeployRec {
                cx,
                cz,
                ..Default::default()
            },
            ARCH_BOX,
        )
    };
    assert_eq!(
        floor(a + 1),
        floor(a + 2),
        "the fixture's floors are one plate"
    );
    (w, a, cz)
}

/// Place a deployable through the real command path: `Ok` or the refusal.
fn place(w: &mut World, row: u16, cx: u16, cz: u16, pose: Pose) -> Result<(), u32> {
    w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        pose,
    }]);
    match w
        .events
        .entries()
        .iter()
        .find(|e| e.code == EV_DEPLOY_REFUSED)
    {
        Some(e) => Err(e.b),
        None => Ok(()),
    }
}

fn pose(ox: i8, oz: i8, yaw: u8) -> Pose {
    Pose { ox, oz, yaw }
}

#[test]
fn two_deployables_share_a_foundation_and_a_third_between_them_refuses() {
    let (mut w, a, cz) = world();
    let cx = a + 1;
    assert_eq!(place(&mut w, BOX_ROW, cx, cz, pose(-60, 0, 0)), Ok(()));
    assert_eq!(place(&mut w, FURNACE_ROW, cx, cz, pose(60, 0, 0)), Ok(()));
    assert_eq!(w.deploys.len(), 2);
    let (b, f) = (w.deploys.entries()[0], w.deploys.entries()[1]);
    assert_ne!(b.loc, f.loc, "two records cannot share an address");
    assert!(w
        .deploys
        .box_index(box_key(b.cx, b.cz, b.level, b.loc))
        .is_some());
    assert!(w
        .deploys
        .oven_index(box_key(f.cx, f.cz, f.level, f.loc))
        .is_some());
    assert_eq!(
        place(&mut w, BOX_ROW, cx, cz, Pose::CENTRE),
        Err(REFUSE_D_SPOT),
        "a box between the two overlaps both"
    );
    assert_eq!(w.deploys.len(), 2);
}

#[test]
fn a_deployable_never_crosses_a_wall() {
    let (mut w, a, cz) = world();
    // The wall on the seam between the two foundations.
    w.tick(&[Command::Place {
        id: PLAYER,
        row: WALL_ROW,
        cx: a + 2,
        cz,
        level: 0,
        loc: LOC_EDGE_XLO,
        freehand: false,
        plate: 0,
    }]);
    assert_eq!(w.pieces.len(), 3, "the wall must stand");
    // Centred 0.1 m past the seam: across the wall.
    assert_eq!(
        place(&mut w, BOX_ROW, a + 2, cz, pose(-119, 0, 0)),
        Err(REFUSE_D_SPOT)
    );
    // Pushed up against it from one side, a centimetre clear: fine.
    assert_eq!(place(&mut w, BOX_ROW, a + 2, cz, pose(-66, 0, 0)), Ok(()));
}

#[test]
fn a_deployable_spanning_two_foundations_falls_with_either() {
    let (mut w, a, cz) = world();
    // Across the seam between a+1 and a+2, and one wholly on a+2.
    assert_eq!(place(&mut w, BOX_ROW, a + 2, cz, pose(-119, 0, 0)), Ok(()));
    assert_eq!(place(&mut w, BOX_ROW, a + 2, cz, pose(70, 0, 0)), Ok(()));
    assert_eq!(w.deploys.len(), 2);
    w.tick(&[Command::Demolish {
        id: PLAYER,
        deploy: false,
        cx: a + 1,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.pieces.len(), 1, "the foundation came down");
    assert_eq!(
        w.deploys.len(),
        1,
        "the box across the seam lost its footing and the other did not"
    );
    assert_eq!(w.deploys.entries()[0].pose.ox, 70);
}

#[test]
fn a_foundation_is_refused_over_a_deployable_on_the_ground() {
    let (mut w, a, cz) = world();
    assert_eq!(place(&mut w, FIRE_ROW, a, cz, Pose::CENTRE), Ok(()));
    w.tick(&[Command::Place {
        id: PLAYER,
        row: FOUNDATION_ROW,
        cx: a,
        cz,
        level: 0,
        loc: LOC_PLANE,
        freehand: false,
        plate: 0,
    }]);
    assert!(w
        .events
        .entries()
        .iter()
        .any(|e| e.code == EV_BUILD_REFUSED && e.b == REFUSE_B_SPOT));
    assert_eq!(w.pieces.len(), 2, "no foundation was built over the fire");
}

#[test]
fn a_ground_box_stands_on_the_lowest_ground_under_it() {
    let (mut w, a, cz) = world();
    assert_eq!(place(&mut w, BOX_ROW, a, cz, pose(30, -40, 20)), Ok(()));
    let rec = w.deploys.entries()[0];
    let rect = rec.rect(ARCH_BOX).unwrap();
    let base = body_base_y(SEED, hv(SEED), w.pieces.cols(), &rec, ARCH_BOX);
    let lowest = rect
        .samples()
        .iter()
        .map(|&(x, z)| terrain::ground(SEED, hv(SEED), x, z))
        .fold(f32::INFINITY, f32::min);
    assert_eq!(base, lowest, "it must not float over any corner");
    // The collision stands it at the same height: its top is a surface.
    let top = collide::piece_ground(SEED, hv(SEED), w.pieces.cols(), rect.x, rect.z, base + 2.0);
    assert_eq!(top, base + sim_core::deploy::solid_vol(ARCH_BOX).unwrap().1);
}

#[test]
fn a_turned_furnace_blocks_where_it_stands_and_survives_a_save() {
    let (mut w, a, cz) = world();
    assert_eq!(
        place(&mut w, FURNACE_ROW, a + 1, cz, pose(0, 0, 64)),
        Ok(())
    );
    let check = |w: &World| {
        let (x, z) = centre(a + 1, cz);
        let rec = w.deploys.entries()[0];
        let feet = body_base_y(SEED, hv(SEED), w.pieces.cols(), &rec, ARCH_FURNACE);
        // A quarter turn puts the furnace's 1.3 m width along z.
        assert!(collide::deploy_blocked(
            SEED,
            hv(SEED),
            w.pieces.cols(),
            x,
            z + 0.95,
            feet
        ));
        assert!(!collide::deploy_blocked(
            SEED,
            hv(SEED),
            w.pieces.cols(),
            x + 0.95,
            z,
            feet
        ));
    };
    check(&w);

    // Every body in a loaded world is asleep, so the round trip is taken
    // from a world whose player already left.
    w.tick(&[Command::Leave { id: PLAYER }]);
    let mut buf = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut buf).expect("save");
    let mut w2 = World::new(SEED);
    w2.gather = GatherContent::probe_fixture();
    w2.build = BuildContent::probe_fixture();
    w2.deploy = content();
    w2.load(&buf[..n]).expect("load");
    assert_eq!(w2.deploys.entries()[0].pose, pose(0, 0, 64));
    assert_eq!(w2.deploys.entries(), w.deploys.entries());
    assert_eq!(w2.deploys.boxes(), w.deploys.boxes());
    assert_eq!(w2.deploys.hearths(), w.deploys.hearths());
    assert_eq!(w2.pieces.entries(), w.pieces.entries());
    assert_eq!(w2.state_hash(), w.state_hash());
    check(&w2);
}
