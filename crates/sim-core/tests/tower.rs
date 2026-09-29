//! **The owners build up.** A trailer's bases came out one storey high: every
//! owner re-laid a foundation and two walls wherever its walk had got to. A
//! tower owner (`RaidPlan::with_storeys`) walks `tower_piece` bottom up on one
//! plot instead. This drives that profile straight into `World::tick` and
//! holds the whole tower, cap included, and that it goes back up after it is
//! brought down.

#![allow(clippy::disallowed_macros)]

use sim_core::bots::{raid_step, tower_len, tower_piece, RaidPlan, RaidRows, TowerPart};
use sim_core::bots::{RAID_CYCLE, TOWER_STOREYS};
use sim_core::build::{anchor, foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::movement::Body;
use sim_core::rng::Pcg32;
use sim_core::world::{Command, World};

const SEED: u64 = 0x70_3E4;
const ID: u32 = 1;

/// `probe_fixture`'s twig foundation, wall and floor. The deploy rows point
/// at an empty table, so the box and lock steps refuse and leave the tower
/// as the only thing this owner can build.
fn rows() -> RaidRows {
    RaidRows {
        foundation: 0,
        wall: 1,
        floor: 2,
        container: 0,
        lock: 0,
        charge_slot: 0,
        goods_slot: 2,
        code: 4242,
    }
}

/// Would `place` take a foundation here: terrain, the depot and landmarks.
fn buildable(w: &World, cx: u16, cz: u16) -> bool {
    let (ax, az) = anchor(cx, cz, LOC_PLANE);
    let pad = BUILD_CELL_M * 1.5;
    foundation_terrain_ok(SEED, &w.haven, ax, az)
        && !sim_core::depot::reserves(&w.haven, ax, az, pad)
        && !sim_core::landmark::covers(&w.haven.marks, ax, az, pad)
}

/// The cell nearest the middle of the island whose east and south
/// neighbours are buildable too, so the walls on their low edges take a
/// plate.
fn plot(w: &World) -> (u16, u16) {
    let c = (sim_core::terrain::ISLAND_SIZE * 0.5 / BUILD_CELL_M) as i32;
    for r in 0..256i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let (cx, cz) = ((c + dx) as u16, (c + dz) as u16);
                if [(cx, cz), (cx + 1, cz), (cx, cz + 1)]
                    .iter()
                    .all(|&(x, z)| buildable(w, x, z))
                {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable plot near the middle of seed {SEED:#x}");
}

/// Enough of both fixture materials for every piece, every pass.
fn restock(w: &mut World, slot: usize) {
    for (i, item) in [0u16, 1].into_iter().enumerate() {
        w.players[slot].inv[i] = ItemStack {
            item,
            count: 500,
            cond: 0,
            skin: 0,
        };
    }
}

/// Which tower pieces stand, in blueprint order, checked by shape.
fn standing(w: &World, cx: u16, cz: u16) -> Vec<bool> {
    (0..tower_len(TOWER_STOREYS))
        .map(|i| {
            let (part, x, z, level, loc) = tower_piece(cx, cz, TOWER_STOREYS, i);
            let row = match part {
                TowerPart::Foundation => 0,
                TowerPart::Wall => 1,
                TowerPart::Floor => 2,
            };
            w.pieces
                .find(x, z, level, loc)
                .is_some_and(|p| p.row as u16 == row)
        })
        .collect()
}

/// Run the owner's cycle, one step a tick (the wire's one action per client
/// per tick), for `ticks`.
fn run(w: &mut World, slot: usize, plan: &mut RaidPlan, rng: &mut Pcg32, ticks: u64) {
    for t in 0..ticks {
        if t % RAID_CYCLE as u64 == 0 {
            restock(w, slot);
        }
        let cmd = raid_step(plan, rng, rows());
        w.tick(&[cmd]);
    }
}

#[test]
fn a_tower_owner_builds_every_storey_and_rebuilds_after_a_collapse() {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    let (cx, cz) = plot(&w);
    let (x, z) = anchor(cx, cz, LOC_PLANE);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: ID }]);
    let slot = w
        .players
        .iter()
        .position(|p| p.active && p.id == ID)
        .expect("the owner seated");
    w.players[slot].body = Body::at(SEED, &w.haven, x, z);

    let mut plan = RaidPlan::new(ID, cx, cz, false).with_storeys(TOWER_STOREYS);
    let mut rng = Pcg32::new(SEED ^ 0x5A1D_C0DE, 0);
    let len = tower_len(TOWER_STOREYS) as u64;
    // Three pieces a cycle, so one pass is `len / 3` cycles; two passes is
    // room for any piece that came up before what holds it.
    let pass = RAID_CYCLE as u64 * len.div_ceil(3);
    run(&mut w, slot, &mut plan, &mut rng, 2 * pass);

    let up = standing(&w, cx, cz);
    assert!(
        up.iter().all(|&s| s),
        "the tower is missing pieces {:?} of {len}",
        up.iter()
            .enumerate()
            .filter(|(_, &s)| !s)
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
    );
    // The cap: a floor on the top storey, the thing a one-storey base lacks.
    assert!(
        w.pieces.find(cx, cz, TOWER_STOREYS, LOC_PLANE).is_some(),
        "no cap on storey {TOWER_STOREYS}"
    );

    // Bring it down from the bottom: the cascade takes everything above.
    w.tick(&[Command::Demolish {
        id: ID,
        deploy: false,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    let down = standing(&w, cx, cz);
    assert!(
        down.iter().all(|&s| !s),
        "a demolished foundation left {} of the tower standing",
        down.iter().filter(|&&s| s).count()
    );

    // The cursor wraps, so the owner puts it all back.
    run(&mut w, slot, &mut plan, &mut rng, 2 * pass);
    assert!(
        standing(&w, cx, cz).iter().all(|&s| s),
        "the owner did not rebuild its tower after the collapse"
    );
}

/// The blueprint's own shape: each storey is a plane and four walls on that
/// storey, and the last piece caps the top.
#[test]
fn the_tower_blueprint_goes_up_a_storey_at_a_time() {
    for storeys in 1..=sim_core::bots::MAX_TOWER_STOREYS {
        let len = tower_len(storeys);
        let mut walls = 0;
        let mut last = 0;
        for i in 0..len {
            let (part, _, _, level, _) = tower_piece(100, 100, storeys, i);
            assert!(level >= last, "piece {i} of {storeys} went down a storey");
            last = level;
            match part {
                TowerPart::Foundation => assert_eq!((i, level), (0, 0)),
                TowerPart::Floor => assert!(i % 5 == 0 && level == (i / 5) as u8),
                TowerPart::Wall => walls += 1,
            }
        }
        assert_eq!(walls, 4 * storeys as u16);
        let (part, _, _, level, loc) = tower_piece(100, 100, storeys, len - 1);
        assert_eq!((part, level, loc), (TowerPart::Floor, storeys, LOC_PLANE));
    }
}
