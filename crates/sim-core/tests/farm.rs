//! Farming v2 (`sim-core/farm.rs`): one seed is one plant in a planter's
//! bed. It grows by daylight under open sky on water, through its stages to
//! ripe; it stays ripe for a window, dies, and withers. A pick pays the crop
//! and kills the plant; a planter with plants does not come up; a planter
//! that is gone takes its plants with it.

use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_EDGE_XLO, LOC_PLANE};
use sim_core::deploy::{
    box_key, DeployContent, DeployDef, ARCH_PLANTER, MATTER_WOOD, PLACE_ANY, REFUSE_D_NOT_EMPTY,
    REFUSE_D_PLANTS_FULL, REFUSE_D_PLANT_ITEM, REFUSE_D_PLANT_SPOT, REFUSE_D_PLANT_STAGE,
};
use sim_core::farm::{
    FarmContent, PlantRec, FL_DRY, GONE_PICKED, GONE_UPROOTED, GONE_WITHERED, HOST_GROUND,
    STAGE_DYING, STAGE_RIPE, STAGE_SEED, TEND_PICK,
};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::limits::MAX_PLANTS;
use sim_core::oven::{CookContent, OVEN_PERIOD_TICKS};
use sim_core::world::{
    is_night, Command, World, EV_DEPLOY_REFUSED, EV_GATHER, EV_PLANT, EV_PLANT_GONE,
};

/// The solved authored sites for `seed`, memoized (`tests/oven.rs` says why).
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static sim_core::terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static sim_core::terrain::Haven = Box::leak(Box::new(sim_core::terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

const SEED: u64 = 0x0FEE_0FEE;
const PLAYER: u32 = 3;
const PLANTER: u16 = 20;
const SEED_ITEM: u16 = 21;
const CORN: u16 = 22;
const SKIN_FULL: u16 = 23;
const SKIN_EMPTY: u16 = 24;
/// Where the planter's item sits in the hand, and the seeds.
const PLANTER_SLOT: usize = 3;
const SEED_SLOT: usize = 6;
/// Whole-grow time at the fixture's 900 s corn.
const GROW: u64 = 900 * 30;

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

fn buildable_cell(seed: u64) -> (u16, u16) {
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (512 + dx).clamp(0, 2047) as u16;
                let cz = (512 + dz).clamp(0, 2047) as u16;
                let (x, z) = cell_center(cx, cz);
                if foundation_terrain_ok(seed, hv(seed), x, z)
                    && foundation_terrain_ok(seed, hv(seed), x + BUILD_CELL_M, z)
                {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell within 64 cells — the generator changed under this test");
}

/// A world with one player and the farm fixture on items 20..25, rain off
/// (every test that wants rain turns it on), standing in a buildable cell.
fn farm_world() -> (Box<World>, u16, u16) {
    let mut w = Box::new(World::new(SEED));
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.cook = CookContent::probe_fixture();
    for (item, cap) in [
        (PLANTER, 1),
        (SEED_ITEM, 50),
        (CORN, 50),
        (SKIN_FULL, 1),
        (SKIN_EMPTY, 1),
    ] {
        w.gather.stack_max[item as usize] = cap;
    }
    w.gather.item_count = w.gather.item_count.max(SKIN_EMPTY + 1);
    let row = w.deploy.def_count;
    w.deploy.defs[row as usize] = DeployDef {
        arch: ARCH_PLANTER,
        placement: PLACE_ANY,
        hp: 100,
        matter: MATTER_WOOD,
        item: PLANTER,
        n_costs: 0,
        costs: [(0, 0); 4],
    };
    w.deploy.def_count += 1;
    let mut fc = FarmContent::probe_fixture();
    fc.crops[0].seed = SEED_ITEM;
    fc.crops[0].yield_item = CORN;
    fc.water_full = SKIN_FULL;
    fc.water_empty = SKIN_EMPTY;
    fc.rain_wu_s = 0;
    w.cook.farm = fc;

    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: PLAYER }]);
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z);
    w.players[0].inv[SEED_SLOT] = ItemStack {
        item: SEED_ITEM,
        count: 50,
        cond: 0,
        skin: 0,
    };
    // The builder kit for the roofed tests: the fixture's wood and stone.
    for (slot, item) in [(0usize, 0u16), (1, 1)] {
        w.players[0].inv[slot] = ItemStack {
            item,
            count: 200,
            cond: 0,
            skin: 0,
        };
    }
    (w, cx, cz)
}

/// Place a planter at the centre of `(cx, cz)` on `level`, then stand the
/// player a metre in front of it. Returns its box key.
fn place_planter(w: &mut World, cx: u16, cz: u16, level: u8) -> u32 {
    let row = w.deploy.def_count - 1;
    w.players[0].inv[PLANTER_SLOT] = ItemStack {
        item: PLANTER,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.players[0].frame.sel = PLANTER_SLOT as u8;
    let before = w.deploys.len();
    w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row,
        cx,
        cz,
        level,
        loc: LOC_PLANE,
        pose: sim_core::footprint::Pose::CENTRE,
    }]);
    assert_eq!(w.deploys.len(), before + 1, "the planter did not place");
    let (x, z) = cell_center(cx, cz);
    let qy = w.players[0].body.qy;
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z + 1.0);
    w.players[0].body.qy = w.players[0].body.qy.max(qy);
    box_key(cx, cz, level, 0)
}

fn plant(w: &mut World, host: u32, spot: u8) {
    w.tick(&[Command::Plant {
        id: PLAYER,
        slot: SEED_SLOT as u8,
        host,
        spot,
        qx: 0,
        qz: 0,
    }]);
}

fn refused(w: &World) -> Option<u32> {
    w.events
        .entries()
        .iter()
        .find(|e| e.code == EV_DEPLOY_REFUSED && e.a == PLAYER)
        .map(|e| e.b)
}

fn load_skin(w: &mut World, host: u32) {
    let i = w.deploys.box_index(host).expect("a planter is a container");
    w.deploys.set_box_slot(
        i,
        0,
        ItemStack {
            item: SKIN_FULL,
            count: 1,
            cond: 0,
            skin: 0,
        },
    );
}

fn tank(w: &World, host: u32) -> u32 {
    let i = w.deploys.box_index(host).expect("a planter is a container");
    w.deploys.oven_states()[i].water
}

fn idle(w: &mut World, ticks: u64) {
    for _ in 0..ticks {
        w.tick(&[]);
    }
}

fn gone(w: &World) -> Option<(u32, u32, u32)> {
    w.events
        .entries()
        .iter()
        .find(|e| e.code == EV_PLANT_GONE)
        .map(|e| (e.a, e.b, e.c))
}

/// A short crop, for the tests that are about what happens after ripe.
fn quick(w: &mut World) {
    w.cook.farm.crops[0].grow_ticks = 300;
    w.cook.farm.ripe_ticks = 300;
    w.cook.farm.dying_ticks = 150;
}

/// **One seed is one plant**: planting takes exactly one of a fifty-stack,
/// a spot holds one plant, and a planter has six spots.
#[test]
fn planting_takes_one_seed_and_a_spot_holds_one_plant() {
    let (mut w, cx, cz) = farm_world();
    let host = place_planter(&mut w, cx, cz, 0);
    plant(&mut w, host, 4);
    assert_eq!(refused(&w), None);
    assert_eq!(w.farm.len(), 1);
    assert_eq!(w.players[0].inv[SEED_SLOT].count, 49);
    let p = w.farm.entries()[0];
    assert_eq!(
        (p.host, p.spot, p.stage, p.owner),
        (host, 4, STAGE_SEED, PLAYER)
    );
    assert!(w
        .events
        .entries()
        .iter()
        .any(|e| e.code == EV_PLANT && e.a == p.id));

    plant(&mut w, host, 4);
    assert_eq!(refused(&w), Some(REFUSE_D_PLANT_SPOT), "a taken spot");
    plant(&mut w, host, 6);
    assert_eq!(
        refused(&w),
        Some(REFUSE_D_PLANT_SPOT),
        "spot 6 is past the bed"
    );
    assert_eq!(
        w.players[0].inv[SEED_SLOT].count, 49,
        "a refusal keeps the seed"
    );
    // Something that is not a seed.
    w.tick(&[Command::Plant {
        id: PLAYER,
        slot: 0,
        host,
        spot: 0,
        qx: 0,
        qz: 0,
    }]);
    assert_eq!(refused(&w), Some(REFUSE_D_PLANT_ITEM));
    for spot in [0, 1, 2, 3, 5] {
        plant(&mut w, host, spot);
        assert_eq!(refused(&w), None, "spot {spot}");
    }
    assert_eq!(w.farm.len(), 6, "six beds");
}

/// **Watered, open and by day, a plant passes every stage at its share of
/// the grow and is ripe at the crop's seconds** — 900 s, one period either
/// way (a plant steps on its 15-tick turn).
#[test]
fn a_watered_plant_passes_its_stages_and_ripens_on_time() {
    let (mut w, cx, cz) = farm_world();
    let host = place_planter(&mut w, cx, cz, 0);
    load_skin(&mut w, host);
    plant(&mut w, host, 0);
    let start = w.tick;
    let ends = w.cook.farm.stage_end_pct;
    let mut reached = [0u64; 7];
    let mut stage = STAGE_SEED;
    while stage < STAGE_RIPE && w.tick < start + GROW + 100 {
        w.tick(&[]);
        assert!(!is_night(w.tick), "the grow must run by day");
        let now = w.farm.entries()[0].stage;
        if now != stage {
            assert_eq!(now, stage + 1, "a stage was skipped");
            stage = now;
            reached[stage as usize] = w.tick - start;
        }
    }
    assert_eq!(stage, STAGE_RIPE, "never ripened");
    for k in 1..=STAGE_RIPE as usize {
        let want = ends[k - 1] as u64 * GROW / 100;
        assert!(
            reached[k].abs_diff(want) <= OVEN_PERIOD_TICKS,
            "stage {k} at {} ticks, wanted {want}",
            reached[k]
        );
    }
    let p = w.farm.entries()[0];
    assert_eq!(p.flags & FL_DRY, 0);
    assert_eq!(p.avg_pm(), 1000, "full health all the way");
    // Set inside the tick, whose own number is one behind the clock after it.
    assert_eq!(
        p.deadline,
        start + reached[STAGE_RIPE as usize] - 1 + w.cook.farm.ripe_ticks as u64
    );
    // One plant drank one skin's worth less the grow.
    let left = tank(&w, host);
    assert!(
        left.abs_diff(w.cook.farm.skin_wu - GROW as u32) <= 30,
        "tank {left}"
    );
}

/// **The dark holds a plant, and so does a roof**: by night, or under a
/// floor, it neither grows nor drinks.
#[test]
fn night_and_a_roof_hold_a_plant_and_it_drinks_nothing() {
    // Night.
    let (mut w, cx, cz) = farm_world();
    let host = place_planter(&mut w, cx, cz, 0);
    load_skin(&mut w, host);
    plant(&mut w, host, 0);
    idle(&mut w, 60);
    let grown = w.farm.entries()[0].grown;
    let water = tank(&w, host);
    assert!(grown > 0 && water > 0, "it grew and drank by day");
    let day = sim_core::limits::DAY_TICKS;
    let dusk = (sim_core::limits::DAY_PORTION as f64 * day as f64) as u64;
    let phase = (w.tick + sim_core::limits::DAY_PHASE_TICKS) % day;
    w.tick += (dusk + day - phase) % day + 1;
    assert!(is_night(w.tick));
    idle(&mut w, 120);
    assert!(is_night(w.tick));
    assert_eq!(w.farm.entries()[0].grown, grown, "it grew in the dark");
    assert_eq!(tank(&w, host), water, "it drank in the dark");

    // A roof: a foundation, a wall and a floor over the planter's cell.
    let (mut w, cx, cz) = farm_world();
    for (row, level, loc) in [
        (0u16, 0u8, LOC_PLANE),
        (1, 0, LOC_EDGE_XLO),
        (2, 1, LOC_PLANE),
    ] {
        let before = w.pieces.len();
        w.tick(&[Command::Place {
            id: PLAYER,
            row,
            cx,
            cz,
            level,
            loc,
            freehand: false,
            plate: 0,
        }]);
        assert_eq!(w.pieces.len(), before + 1, "piece {row} did not place");
    }
    // Stand on the foundation.
    let plate = w.pieces.cols().plate(cx, cz).unwrap_or(0);
    let floor = sim_core::build::column_floor_y(SEED, hv(SEED), cx, cz, plate);
    w.players[0].body.qy = sim_core::movement::quant_y(floor);
    let host = place_planter(&mut w, cx, cz, 0);
    w.players[0].body.qy = sim_core::movement::quant_y(floor);
    load_skin(&mut w, host);
    plant(&mut w, host, 0);
    assert_eq!(refused(&w), None, "the roofed planter takes a seed");
    idle(&mut w, 120);
    assert_eq!(w.farm.entries()[0].grown, 0, "it grew under a roof");
    assert_eq!(tank(&w, host), 0, "it drank under a roof");
    let i = w.deploys.box_index(host).unwrap();
    assert_eq!(
        w.deploys.box_slot(i, 0).item,
        SKIN_FULL,
        "the skin was poured"
    );
}

/// **A dry planter holds its plants**, and a full skin in its water slot
/// is poured only once a plant thirsts.
#[test]
fn a_dry_planter_holds_and_a_skin_is_poured_only_for_a_thirsty_plant() {
    let (mut w, cx, cz) = farm_world();
    let host = place_planter(&mut w, cx, cz, 0);
    plant(&mut w, host, 2);
    idle(&mut w, 60);
    let p = w.farm.entries()[0];
    assert_eq!(p.grown, 0, "a dry plant grew");
    assert_ne!(p.flags & FL_DRY, 0, "and it is marked dry");

    let (mut w, cx, cz) = farm_world();
    let host = place_planter(&mut w, cx, cz, 0);
    load_skin(&mut w, host);
    idle(&mut w, 60);
    let i = w.deploys.box_index(host).unwrap();
    assert_eq!(
        w.deploys.box_slot(i, 0).item,
        SKIN_FULL,
        "an idle planter drank"
    );
    assert_eq!(tank(&w, host), 0);
    plant(&mut w, host, 2);
    idle(&mut w, OVEN_PERIOD_TICKS * 2);
    assert_eq!(
        w.deploys.box_slot(i, 0).item,
        SKIN_EMPTY,
        "the skin was not poured"
    );
    assert!(tank(&w, host) > 0);
    assert!(w.farm.entries()[0].grown > 0);
}

/// **Ripe is a window, then the plant dies, then it withers away** — at the
/// content's ticks, announced as it goes.
#[test]
fn a_ripe_plant_dies_and_withers_on_its_clock() {
    let (mut w, cx, cz) = farm_world();
    quick(&mut w);
    let host = place_planter(&mut w, cx, cz, 0);
    load_skin(&mut w, host);
    plant(&mut w, host, 0);
    let id = w.farm.entries()[0].id;
    let mut ripe_at = 0;
    for _ in 0..400 {
        w.tick(&[]);
        if w.farm.entries()[0].stage == STAGE_RIPE {
            ripe_at = w.tick;
            break;
        }
    }
    assert!(ripe_at > 0, "never ripened");
    let mut dying_at = 0;
    for _ in 0..400 {
        w.tick(&[]);
        if w.farm.entries()[0].stage == STAGE_DYING {
            dying_at = w.tick;
            assert!(w
                .events
                .entries()
                .iter()
                .any(|e| e.code == EV_PLANT && e.a == id && e.b >> 8 == STAGE_DYING as u32));
            break;
        }
    }
    assert!(
        (dying_at - ripe_at).abs_diff(300) <= OVEN_PERIOD_TICKS,
        "ripe for {} ticks",
        dying_at - ripe_at
    );
    let mut withered_at = 0;
    for _ in 0..400 {
        w.tick(&[]);
        if let Some(g) = gone(&w) {
            assert_eq!(g, (id, GONE_WITHERED as u32, 0));
            withered_at = w.tick;
            break;
        }
    }
    assert!(w.farm.is_empty(), "the dead plant stands");
    assert!(
        (withered_at - dying_at).abs_diff(150) <= OVEN_PERIOD_TICKS,
        "dying for {} ticks",
        withered_at - dying_at
    );
}

/// **A pick before ripe refuses; at ripe it pays the crop and kills the
/// plant.**
#[test]
fn a_ripe_pick_pays_four_corn_and_kills_the_plant() {
    let (mut w, cx, cz) = farm_world();
    quick(&mut w);
    let host = place_planter(&mut w, cx, cz, 0);
    load_skin(&mut w, host);
    plant(&mut w, host, 1);
    let id = w.farm.entries()[0].id;
    let pick = |w: &mut World| {
        w.tick(&[Command::Tend {
            id: PLAYER,
            plant: id,
            act: TEND_PICK,
        }])
    };
    pick(&mut w);
    assert_eq!(refused(&w), Some(REFUSE_D_PLANT_STAGE), "picked unripe");
    for _ in 0..400 {
        w.tick(&[]);
        if w.farm.entries()[0].stage == STAGE_RIPE {
            break;
        }
    }
    assert_eq!(w.farm.entries()[0].stage, STAGE_RIPE);
    pick(&mut w);
    assert_eq!(refused(&w), None);
    let g = w
        .events
        .entries()
        .iter()
        .find(|e| e.code == EV_GATHER && e.a == PLAYER)
        .copied()
        .expect("a pick announces its pay");
    assert_eq!(g.b, (CORN as u32) << 16 | 4, "four corn");
    assert_eq!(g.c, 0, "nothing spilled");
    assert_eq!(gone(&w), Some((id, GONE_PICKED as u32, PLAYER)));
    assert!(w.farm.is_empty());
    let corn: u32 = w.players[0]
        .inv
        .iter()
        .filter(|s| s.item == CORN)
        .map(|s| s.count as u32)
        .sum();
    assert_eq!(corn, 4);
}

/// **A planter with plants does not come up; one that is broken takes its
/// plants with it**, within one farm period.
#[test]
fn a_planted_planter_refuses_pickup_and_a_broken_one_uproots() {
    let (mut w, cx, cz) = farm_world();
    let host = place_planter(&mut w, cx, cz, 0);
    plant(&mut w, host, 0);
    plant(&mut w, host, 5);
    w.tick(&[Command::Demolish {
        id: PLAYER,
        deploy: true,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(refused(&w), Some(REFUSE_D_NOT_EMPTY));
    assert_eq!(w.deploys.len(), 1, "the planted planter came up");

    let di = (0..w.deploys.len())
        .find(|&i| w.deploys.entries()[i].cx == cx && w.deploys.entries()[i].cz == cz)
        .unwrap();
    let mut ev = sim_core::world::EventQueue::default();
    assert!(sim_core::deploy::damage_deploy(
        &w.deploy,
        &mut w.pieces,
        &mut w.deploys,
        di,
        9_999,
        &mut ev,
    ));
    assert!(w.deploys.box_index(host).is_none());
    let mut uprooted = 0;
    for _ in 0..OVEN_PERIOD_TICKS {
        w.tick(&[]);
        uprooted += w
            .events
            .entries()
            .iter()
            .filter(|e| e.code == EV_PLANT_GONE && e.b == GONE_UPROOTED as u32)
            .count();
    }
    assert_eq!(uprooted, 2);
    assert!(w.farm.is_empty());
}

/// **A full island refuses a seed** and keeps it in the hand.
#[test]
fn a_full_farm_refuses_a_seed() {
    let (mut w, cx, cz) = farm_world();
    let host = place_planter(&mut w, cx, cz, 0);
    for _ in 0..MAX_PLANTS {
        w.farm
            .insert(PlantRec {
                host: HOST_GROUND,
                spot: sim_core::farm::SPOT_GROUND,
                ..PlantRec::default()
            })
            .expect("room");
    }
    plant(&mut w, host, 0);
    assert_eq!(refused(&w), Some(REFUSE_D_PLANTS_FULL));
    assert_eq!(w.players[0].inv[SEED_SLOT].count, 50);
}

/// **A save keeps the farm**: three plants round-trip to the same
/// `state_hash`, and a plant with an impossible stage is refused.
#[test]
fn a_save_round_trips_the_farm_and_refuses_a_bad_stage() {
    let (mut w, cx, cz) = farm_world();
    let host = place_planter(&mut w, cx, cz, 0);
    load_skin(&mut w, host);
    for spot in [0, 3, 4] {
        plant(&mut w, host, spot);
    }
    idle(&mut w, 40);
    // Asleep, so the save is lossless for the body too (`tests/worldsave.rs`).
    w.tick(&[Command::Leave { id: PLAYER }]);
    assert_eq!(w.farm.len(), 3);
    assert!(tank(&w, host) > 0);
    let mut buf = vec![0u8; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut buf).expect("encodes");
    let (mut back, _, _) = farm_world();
    back.load(&buf[..n]).expect("its own bytes load");
    assert_eq!(back.farm.entries(), w.farm.entries());
    assert_eq!(back.farm.next_id(), w.farm.next_id());
    assert_eq!(back.state_hash(), w.state_hash());

    // The last plant's stage byte: the farm section ends the blob.
    let at = n - sim_core::farm::PLANT_BYTES + 14;
    assert_eq!(buf[at], w.farm.entries()[2].stage);
    buf[at] = 9;
    let (mut bad, _, _) = farm_world();
    assert_eq!(
        bad.load(&buf[..n]),
        Err(sim_core::worldsave::WorldSaveError::BadPlant)
    );
}

/// **Rain fills an open planter's tank, and not a roofed one's.**
#[test]
fn rain_fills_an_open_planter_and_not_a_roofed_one() {
    use sim_core::weather::{FORCE_FADE_TICKS, KEEP_TIME, STORM};
    let (mut w, cx, cz) = farm_world();
    w.cook.farm.rain_wu_s = 6 * 30;
    // A roofed cell beside an open one.
    for (row, level, loc) in [
        (0u16, 0u8, LOC_PLANE),
        (1, 0, LOC_EDGE_XLO),
        (2, 1, LOC_PLANE),
    ] {
        let before = w.pieces.len();
        w.tick(&[Command::Place {
            id: PLAYER,
            row,
            cx,
            cz,
            level,
            loc,
            freehand: false,
            plate: 0,
        }]);
        assert_eq!(w.pieces.len(), before + 1, "piece {row} did not place");
    }
    let plate = w.pieces.cols().plate(cx, cz).unwrap_or(0);
    let floor = sim_core::build::column_floor_y(SEED, hv(SEED), cx, cz, plate);
    w.players[0].body.qy = sim_core::movement::quant_y(floor);
    let roofed = place_planter(&mut w, cx, cz, 0);
    let (x, z) = cell_center(cx + 1, cz);
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z);
    let open = place_planter(&mut w, cx + 1, cz, 0);
    w.tick(&[Command::AdminEnv {
        weather: STORM,
        time_pm: KEEP_TIME,
    }]);
    idle(&mut w, FORCE_FADE_TICKS + 60);
    assert!(tank(&w, open) > 0, "a storm did not fill an open planter");
    assert_eq!(tank(&w, roofed), 0, "a storm filled a roofed planter");
}
