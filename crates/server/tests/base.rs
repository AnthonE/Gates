//! **A bot's base is a base a player can use.** The owner profile
//! (`sim_core::bots::STARTER`) driven straight into `World::tick` on the
//! shipped content — the rows, prices and rules a shard runs — and then used
//! the way a player uses a base: the shut front door stops a body, the code
//! opens it, the airlock leads to the second door and the cupboard, and the
//! stairs climb to the floor over it. Nobody else may build inside its claim.

use sim_core::bots::{
    base_step, BaseOp, BasePlan, BaseRows, Part, STARTER, STARTER_FOOTPRINT, STARTER_STAND_M,
};
use sim_core::build::{
    anchor, column_floor_y, foundation_terrain_ok, terrain_band, BUILD_CELL_M, LEVEL_H_M,
    LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE, MAT_STONE, REFUSE_B_CLAIM, SHAPE_DOORWAY, SHAPE_FLOOR,
    SHAPE_FOUNDATION, SHAPE_ROOF, SHAPE_STAIRS_L, SHAPE_TRI_FOUNDATION, SHAPE_TRI_ROOF, SHAPE_WALL,
};
use sim_core::deploy::{ACCESS_OP_ENTER, ARCH_BOX, ARCH_DOOR, ARCH_HEARTH};
use sim_core::gather::ItemStack;
use sim_core::input::InputFrame;
use sim_core::movement::{self, Body, POS_XZ_Q, POS_Y_Q};
use sim_core::occupy::{Barren, Scratch};
use sim_core::world::{Command, World, EV_BUILD_REFUSED};

const SEED: u64 = 20260731;
const OWNER: u32 = 1;
const VISITOR: u32 = 2;
const STRANGER: u32 = 3;
const CODE: u16 = 4242;

struct Fixture {
    w: World,
    content: content::Content,
    rows: BaseRows,
}

/// A world on the shipped tables, installed the way `net.rs` installs them.
fn fixture() -> Fixture {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let content = content::Content::load_dir(&dir).expect("shipped content loads");
    let t = server::net::bake_all(&content).expect("shipped content bakes");
    let mut w = World::new(SEED);
    w.gather = t.gather;
    w.craft = t.craft;
    w.build = t.build;
    w.deploy = t.deploy;
    w.combat = t.combat;
    w.backpack = t.backpack;
    w.survival = t.survival;
    w.cook = t.cook;
    w.loot = t.loot;
    w.mob = t.mobs;
    w.research = t.research;
    let mut rows = server::population::base_rows(&content).expect("shipped base rows");
    rows.code = CODE;
    Fixture { w, content, rows }
}

/// Would a foundation go here: terrain, the depot and the landmarks.
fn buildable(w: &World, cx: u16, cz: u16) -> bool {
    let (ax, az) = anchor(cx, cz, LOC_PLANE);
    let pad = BUILD_CELL_M * 1.5;
    foundation_terrain_ok(SEED, &w.haven, ax, az)
        && !sim_core::terrain::build_reserved(&w.haven, ax, az, pad)
}

/// The plot nearest the island's middle where the whole base stands on even
/// ground: its footprint buildable, and every column the base touches within
/// a band of the core's, so no wall is refused for its plate.
fn plot(w: &World) -> (u16, u16) {
    let c = (sim_core::terrain::ISLAND_SIZE * 0.5 / BUILD_CELL_M) as i32;
    for r in 0..256i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let (cx, cz) = ((c + dx) as u16, (c + dz) as u16);
                let band = terrain_band(SEED, &w.haven, cx, cz);
                let even = [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1)]
                    .iter()
                    .all(|&(ox, oz)| {
                        (terrain_band(SEED, &w.haven, cx + ox, cz + oz) - band).abs() <= 1
                    });
                let plan = BasePlan::new(0, cx, cz);
                if even
                    && STARTER_FOOTPRINT.iter().all(|&(dx, dz)| {
                        let (x, z) = plan.cell(dx, dz);
                        buildable(w, x, z)
                    })
                {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no even plot near the middle of seed {SEED}");
}

fn slot_of(w: &World, id: u32) -> usize {
    w.players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("seated")
}

fn stack(c: &content::Content, id: &str, count: u16) -> ItemStack {
    ItemStack {
        item: c.item_index(id).unwrap_or_else(|| panic!("shipped {id}")),
        count,
        cond: 0,
        skin: 0,
    }
}

fn seat(w: &mut World, id: u32, x: f32, z: f32) -> usize {
    w.tick(&[Command::Join { id }]);
    let slot = slot_of(w, id);
    w.players[slot].body = Body::at(SEED, &w.haven, x, z);
    slot
}

/// The owner, standing where the profile stands it, with enough to build
/// and grade the whole base: every piece, every deployable, two doors and a
/// lock for each.
fn build(f: &mut Fixture, cx: u16, cz: u16) -> BasePlan {
    let x = cx as f32 * BUILD_CELL_M + STARTER_STAND_M.0;
    let z = cz as f32 * BUILD_CELL_M + STARTER_STAND_M.1;
    let slot = seat(&mut f.w, OWNER, x, z);
    let kit = [
        stack(&f.content, "item.wood", 1000),
        stack(&f.content, "item.wood", 1000),
        stack(&f.content, "item.wood", 1000),
        stack(&f.content, "item.stone", 1000),
        stack(&f.content, "item.stone", 1000),
        stack(&f.content, "item.stone", 1000),
        stack(&f.content, "item.stone", 1000),
        stack(&f.content, "item.stone", 1000),
        stack(&f.content, "item.hearth", 1),
        stack(&f.content, "item.door_wood", 1),
        stack(&f.content, "item.door_wood", 1),
        stack(&f.content, "item.lock_code", 1),
        stack(&f.content, "item.lock_code", 1),
        stack(&f.content, "item.box_small", 1),
    ];
    for (i, s) in kit.into_iter().enumerate() {
        f.w.players[slot].inv[i] = s;
    }
    let mut plan = BasePlan::new(OWNER, cx, cz);
    // Two passes: the second is what a raid would get, refused wherever the
    // first already built. One command a tick, the wire's own rate.
    for _ in 0..2 * STARTER.len() {
        let cmd = base_step(&mut plan, f.rows, STARTER);
        f.w.tick(&[cmd]);
    }
    plan
}

/// The shape a blueprint piece has in the shipped table.
fn shape(part: Part) -> u8 {
    match part {
        Part::Foundation => SHAPE_FOUNDATION,
        Part::TriFoundation => SHAPE_TRI_FOUNDATION,
        Part::Wall => SHAPE_WALL,
        Part::Doorway => SHAPE_DOORWAY,
        Part::Floor => SHAPE_FLOOR,
        Part::Stairs => SHAPE_STAIRS_L,
        Part::Roof => SHAPE_ROOF,
        Part::TriRoof => SHAPE_TRI_ROOF,
    }
}

#[test]
fn a_bot_builds_the_whole_starter_base() {
    let mut f = fixture();
    let (cx, cz) = plot(&f.w);
    let plan = build(&mut f, cx, cz);
    let w = &f.w;

    // Every piece of the blueprint stands, as the shape it was meant to be.
    for op in STARTER {
        if let BaseOp::Place(part, dx, dz, level, loc) = *op {
            let (x, z) = plan.cell(dx, dz);
            let rec = w
                .pieces
                .find(x, z, level, loc)
                .unwrap_or_else(|| panic!("{part:?} at {dx},{dz} L{level} loc {loc} never stood"));
            assert_eq!(
                w.build.pieces[rec.row as usize].shape,
                shape(part),
                "{part:?} at {dx},{dz} L{level}"
            );
        }
    }
    // Two storeys and a roof: the storey a trailer caught was the only one.
    assert!(w.pieces.find(cx, cz, 2, LOC_PLANE).is_some(), "no roof");

    // The cupboard in the core, its owner on the crew.
    let hearth = w
        .deploys
        .hearths()
        .iter()
        .find(|h| (h.cx, h.cz, h.level) == (cx, cz, 0))
        .expect("the cupboard stands in the core");
    assert!(
        hearth.crew.contains(OWNER),
        "the owner is not on its own crew"
    );
    assert!(
        hearth.stock.iter().any(|&s| s > 0),
        "the cupboard was never stocked for upkeep"
    );
    let arch = |x: u16, z: u16, level: u8, loc: u8| {
        w.deploys
            .find(x, z, level, loc)
            .map(|d| w.deploy.defs[d.row as usize].arch)
    };
    assert_eq!(arch(cx, cz, 0, LOC_PLANE), Some(ARCH_HEARTH));
    // Two doors between the outside and the cupboard: the front one on the
    // airlock triangle's west side, the inner one on its north side.
    let front = (cx + 1, cz + 1, 0, LOC_EDGE_XLO);
    let inner = (cx + 1, cz + 1, 0, LOC_EDGE_ZLO);
    for (name, (x, z, level, loc)) in [("front", front), ("inner", inner)] {
        assert_eq!(arch(x, z, level, loc), Some(ARCH_DOOR), "{name} door");
        let lock = w
            .deploys
            .locks()
            .iter()
            .find(|l| (l.cx, l.cz, l.level, l.loc) == (x, z, level, loc))
            .unwrap_or_else(|| panic!("no lock on the {name} door"));
        assert_eq!(lock.code, CODE, "the {name} door's lock was never armed");
    }
    // Loot upstairs over the cupboard.
    assert_eq!(
        arch(cx, cz, 1, LOC_PLANE),
        Some(ARCH_BOX),
        "the upstairs box"
    );

    // Graded: the core's walls are stone, the thing that stands between a
    // raider and the cupboard.
    for (x, z, loc) in [
        (cx, cz, LOC_EDGE_XLO),
        (cx, cz, LOC_EDGE_ZLO),
        (cx, cz + 1, LOC_EDGE_ZLO),
    ] {
        let rec = w.pieces.find(x, z, 0, loc).expect("core wall");
        assert_eq!(
            w.build.pieces[rec.row as usize].material, MAT_STONE,
            "core wall at {x},{z} loc {loc} is not stone"
        );
    }
}

#[test]
fn nobody_else_builds_inside_its_claim() {
    let mut f = fixture();
    let (cx, cz) = plot(&f.w);
    build(&mut f, cx, cz);
    // A stranger two cells south of the front door, with a foundation's
    // wood in hand.
    let (x, z) = (
        (cx + 1) as f32 * BUILD_CELL_M + 1.5,
        (cz + 3) as f32 * BUILD_CELL_M + 1.5,
    );
    let slot = seat(&mut f.w, STRANGER, x, z);
    f.w.players[slot].inv[0] = stack(&f.content, "item.wood", 1000);
    f.w.tick(&[Command::Place {
        id: STRANGER,
        row: f.rows.foundation,
        cx: cx + 1,
        cz: cz + 3,
        level: 0,
        loc: LOC_PLANE,
        freehand: false,
        plate: 0,
    }]);
    assert!(
        f.w.events
            .entries()
            .iter()
            .any(|e| e.code == EV_BUILD_REFUSED && e.a == STRANGER && e.b == REFUSE_B_CLAIM),
        "a stranger built inside the base's claim"
    );
    assert!(f.w.pieces.find(cx + 1, cz + 3, 0, LOC_PLANE).is_none());
}

/// A body walking the base, on the world's own collision.
struct Walker {
    b: Body,
    sc: Scratch<Barren>,
}

impl Walker {
    fn at(w: &World, x: f32, z: f32, y: Option<f32>) -> Self {
        let mut b = Body::at(SEED, &w.haven, x, z);
        if let Some(y) = y {
            b.qy = movement::quant_y(y);
        }
        Self {
            b,
            sc: Scratch::barren(),
        }
    }

    fn x(&self) -> f32 {
        self.b.qx as f32 * POS_XZ_Q
    }

    fn z(&self) -> f32 {
        self.b.qz as f32 * POS_XZ_Q
    }

    fn y(&self) -> f32 {
        self.b.qy as f32 * POS_Y_Q
    }

    /// Walk along `(move_x, move_z)` until `done` holds or `ticks` run out.
    fn walk(
        &mut self,
        w: &World,
        mx: i8,
        mz: i8,
        ticks: u32,
        done: impl Fn(&Self) -> bool,
    ) -> bool {
        for _ in 0..ticks {
            if done(self) {
                return true;
            }
            movement::step(
                SEED,
                &w.haven,
                w.pieces.cols(),
                &mut self.sc.occupants(),
                &mut self.b,
                &InputFrame {
                    move_x: mx,
                    move_z: mz,
                    ..InputFrame::default()
                },
            );
        }
        done(self)
    }
}

#[test]
fn a_player_gets_in_through_both_doors_and_up_the_stairs() {
    let mut f = fixture();
    let (cx, cz) = plot(&f.w);
    build(&mut f, cx, cz);
    let x0 = cx as f32 * BUILD_CELL_M;
    let z0 = cz as f32 * BUILD_CELL_M;
    let floor = column_floor_y(
        SEED,
        &f.w.haven,
        cx,
        cz,
        f.w.pieces.cols().plate(cx, cz).expect("the core is built"),
    );

    // The airlock triangle's cell: the front door on its west side, the
    // inner door on its north side, the long side a diagonal wall.
    let (ax, az) = (x0 + BUILD_CELL_M, z0 + BUILD_CELL_M);
    let front = (cx + 1, cz + 1, 0, LOC_EDGE_XLO);
    let inner = (cx + 1, cz + 1, 0, LOC_EDGE_ZLO);
    let door = |w: &World, (x, z, level, loc): (u16, u16, u8, u8)| {
        w.deploys.find(x, z, level, loc).is_some_and(|d| d.open)
    };

    // Outside the front door, facing it: a shut door is a wall.
    let mut v = Walker::at(&f.w, ax - 1.2, az + 1.5, None);
    assert!(
        !v.walk(&f.w, 127, 0, 60, |v| v.x() > ax + 0.5),
        "walked through the shut front door"
    );

    // The visitor knows the code: it enters it and opens the door.
    seat(&mut f.w, VISITOR, ax - 1.0, az + 1.5);
    let open = |w: &mut World, (x, z, level, loc): (u16, u16, u8, u8)| {
        w.tick(&[Command::Access {
            id: VISITOR,
            cx: x,
            cz: z,
            level,
            loc,
            op: ACCESS_OP_ENTER,
            code: CODE,
        }]);
        w.tick(&[Command::Use {
            id: VISITOR,
            cx: x,
            cz: z,
            level,
            loc,
        }]);
    };
    open(&mut f.w, front);
    assert!(door(&f.w, front), "the code did not open the front door");

    // Into the airlock — and no further: the inner door is shut, so the
    // cupboard is still a door away.
    assert!(
        v.walk(&f.w, 127, 0, 60, |v| v.x() > ax + 0.6),
        "could not walk in through the open front door: at ({:.2}, {:.2}) of the airlock",
        v.x() - ax,
        v.z() - az
    );
    assert!(
        v.walk(&f.w, 0, -127, 60, |v| v.z() < az + 0.6),
        "could not cross the airlock: at ({:.2}, {:.2})",
        v.x() - ax,
        v.z() - az
    );
    assert!(
        v.walk(&f.w, 127, 0, 60, |v| v.x() > ax + 1.45),
        "could not line up on the inner door: at ({:.2}, {:.2})",
        v.x() - ax,
        v.z() - az
    );
    assert!(
        !v.walk(&f.w, 0, -127, 60, |v| v.z() < az - 0.5),
        "walked through the shut inner door"
    );
    // The visitor walks round to the inner door's reach and opens it too.
    open(&mut f.w, inner);
    assert!(door(&f.w, inner), "the code did not open the inner door");
    assert!(
        v.walk(&f.w, 0, -127, 60, |v| v.z() < az - 1.0),
        "could not walk from the airlock into the room: at ({:.2}, {:.2})",
        v.x() - ax,
        v.z() - az
    );
    // One room: the stair cell opens into the core with the cupboard.
    assert!(
        v.walk(&f.w, -127, 0, 60, |v| v.x() < ax - 0.3),
        "the stair cell and the core are not one room: at ({:.2}, {:.2})",
        v.x() - ax,
        v.z() - az
    );

    // The stairs: from the foot of the flight in the airlock's south-east
    // corner, north up the first run, west along the second, and out onto
    // the floor over the cupboard.
    let sx = x0 + BUILD_CELL_M;
    let mut c = Walker::at(&f.w, sx + 2.5, z0 + 2.9, Some(floor));
    assert!(
        c.walk(&f.w, 0, -127, 80, |c| c.z() < z0 + 0.8),
        "stuck on the first run at ({:.2}, {:.2}, {:.2})",
        c.x(),
        c.y(),
        c.z()
    );
    assert!(
        c.walk(&f.w, -127, 0, 80, |c| c.x() < sx - 0.8),
        "stuck on the second run at ({:.2}, {:.2}, {:.2})",
        c.x(),
        c.y(),
        c.z()
    );
    assert!(
        (c.y() - (floor + LEVEL_H_M)).abs() <= 0.05,
        "the flight did not land on the upper floor: y {:.2} against {:.2}",
        c.y(),
        floor + LEVEL_H_M
    );
    assert!(c.x() < sx && c.x() > x0, "not over the core");
}

/// **And a bot does it over the wire**, the way a player's client would: an
/// owner (`run_bot`, stream 0) spawned on an even plot with the starter kit
/// `ci/film.sh` hands a population settles once, stands in its core and puts
/// up its cupboard, its doors and the storey above them through the real
/// handshake, action lane and pace.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_owner_bot_builds_its_base_over_the_wire() {
    use server::botclient::{bot_endpoint, run_bot};
    use std::time::Duration;

    let f = fixture();
    let (cx, cz) = plot(&f.w);
    let mut tables = server::net::bake_all(&f.content).expect("shipped content bakes");
    let mut kit = sim_core::inventory::SpawnKit::EMPTY;
    for (i, (id, n)) in [
        ("item.wood", 1000),
        ("item.stone", 1000),
        ("item.hearth", 1),
        ("item.door_metal", 1),
        ("item.door_wood", 1),
        ("item.box_small", 1),
        ("item.lock_code", 1),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(kit.set(i, stack(&f.content, id, n)), "kit slot {i}");
    }
    tables.spawn_kit = kit;
    let mut cfg = server::config::ShardConfig::ephemeral(SEED);
    cfg.dev_spawn = Some((
        cx as f32 * BUILD_CELL_M + STARTER_STAND_M.0,
        cz as f32 * BUILD_CELL_M + STARTER_STAND_M.1,
    ));
    let handle = server::net::spawn_shard(
        cfg,
        tables,
        server::store::Saves::off(),
        server::worldfile::WorldBoot::off(),
    )
    .await
    .expect("shard boots");
    let rows = server::population::raid_rows(&f.content).expect("shipped raid rows");
    assert!(rows.base.is_some(), "the shipped content builds no base");
    let endpoint = bot_endpoint().expect("client endpoint");
    let r = run_bot(
        &endpoint,
        handle.local_addr,
        0,
        Duration::from_secs(8),
        Some(rows),
    )
    .await
    .expect("the owner ran");
    handle
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);

    let seen = format!(
        "{} ticks, {} steps, {} actions, placed {} pieces / {} deploys, top storey {}, \
         refused b{} d{}",
        r.ticks_walked,
        r.raid_steps,
        r.actions_sent,
        r.pieces_placed,
        r.deploys_placed,
        r.top_storey,
        r.build_refused,
        r.deploy_refused
    );
    assert_eq!(r.raid_cycles, 1, "settled more or less than once: {seen}");
    assert_eq!(r.last_plot, Some((cx, cz)), "settled off its spawn: {seen}");
    assert_eq!(
        r.actions_unencodable, 0,
        "a base op has no wire form: {seen}"
    );
    // The cupboard and both doors (a lock is a record on a door, not a
    // deployable of its own).
    assert!(r.deploys_placed >= 3, "cupboard and doors: {seen}");
    // The shell and the storey above it.
    assert!(
        r.top_storey >= 1,
        "nothing stood above the ground floor: {seen}"
    );
}
