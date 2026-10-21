//! Farming v2 (`sim-core/farm.rs`): one seed is one plant in a planter's
//! bed. It grows by daylight under open sky on water, through its stages to
//! ripe; it stays ripe for a window, dies, and withers. A pick pays the crop
//! and kills the plant; a planter with plants does not come up; a planter
//! that is gone takes its plants with it.

use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_EDGE_XLO, LOC_PLANE};
use sim_core::deploy::{
    box_key, DeployContent, DeployDef, ARCH_CATCHER, ARCH_PLANTER, MATTER_WOOD, PLACE_ANY,
    REFUSE_D_NOT_EMPTY, REFUSE_D_PLANTS_FULL, REFUSE_D_PLANT_ITEM, REFUSE_D_PLANT_SOIL,
    REFUSE_D_PLANT_SPOT, REFUSE_D_PLANT_STAGE, REFUSE_D_REACH,
};
use sim_core::farm::{
    ground_site, FarmContent, Ground, PlantRec, FL_DRY, GONE_PICKED, GONE_UPROOTED, GONE_WITHERED,
    HOST_GROUND, SPOT_GROUND, STAGE_DYING, STAGE_RIPE, STAGE_SEED, TEND_PICK,
};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::limits::MAX_PLANTS;
use sim_core::movement::quant_xz;
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
const HEMP_SEED: u16 = 26;
const HEMP_YIELD: u16 = 27;
const CATCHER: u16 = 28;
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
        (HEMP_SEED, 50),
        (HEMP_YIELD, 50),
        (CATCHER, 1),
    ] {
        w.gather.stack_max[item as usize] = cap;
    }
    w.gather.item_count = w.gather.item_count.max(CATCHER + 1);
    for seed in [SEED_ITEM, HEMP_SEED] {
        w.gather.cond_cap[seed as usize] = sim_core::farm::GENOME_VALUES as u16;
    }
    // The catcher's row, then the planter's: the planter stays the last row
    // (`place_planter`), the catcher the one before it (`place_catcher`).
    let row = w.deploy.def_count;
    w.deploy.defs[row as usize] = DeployDef {
        arch: ARCH_CATCHER,
        placement: PLACE_ANY,
        hp: 150,
        matter: MATTER_WOOD,
        item: CATCHER,
        n_costs: 0,
        costs: [(0, 0); 4],
    };
    w.deploy.def_count += 1;
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
    fc.ground_rain_wu_s = 0;
    fc.catcher_rain_wu_s = 0;
    // Blank seeds roll XXXXXX, the neutral genome, so every test that is
    // not about genes grows at the crop's own numbers. The gene tests plant
    // clones, whose genes they choose.
    fc.good_gene_pm = 0;
    fc.bad_w_pm = 0;
    // A second crop, for the crossbreed's "same crop" rule.
    fc.crops[1] = sim_core::farm::CropDef {
        look: sim_core::farm::LOOK_HEMP,
        seed: HEMP_SEED,
        yield_item: HEMP_YIELD,
        ..fc.crops[0]
    };
    fc.crop_count = 2;
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

/// **A bed plant roots in the soil of the planter as it is drawn**: on bare
/// ground the planter stands at `deploy::body_base_y` (the lowest terrain
/// under it), not the column's band, and the plant's point is that floor
/// plus the bed's soil rise (`client::ui::farm::soil_rise`, the planting
/// ghost's height), so the seed sits in the bed and not in the air above it.
#[test]
fn a_bed_plant_roots_in_the_soil_of_a_planter_on_bare_ground() {
    use sim_core::build::{band_floor_y, terrain_band};
    use sim_core::deploy::{body_base_y, DEPLOY_VOL};
    use sim_core::farm::BED_SOIL_DROP_M;
    use sim_core::movement::{quant_y, POS_Y_Q};
    let (mut w, cx, cz) = farm_world();
    // A cell beside the fixture's where the band and the ground disagree,
    // so the old floor (the column's band) would have floated the plant.
    let (cx, cz) = (0..8u16)
        .flat_map(|dx| (0..8u16).map(move |dz| (cx + dx, cz + dz)))
        .find(|&(x, z)| {
            let rec = sim_core::deploy::DeployRec {
                cx: x,
                cz: z,
                ..Default::default()
            };
            let drawn = body_base_y(SEED, hv(SEED), w.pieces.cols(), &rec, ARCH_PLANTER);
            sim_core::fmath::fabs(band_floor_y(terrain_band(SEED, hv(SEED), x, z), 0) - drawn) > 0.1
        })
        .expect("no cell near the fixture's where the band is off the ground");
    let (x, z) = cell_center(cx, cz);
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z - 1.5);
    let host = place_planter(&mut w, cx, cz, 0);
    plant(&mut w, host, 2);
    assert_eq!(refused(&w), None);
    let rec = *w.deploys.find(cx, cz, 0, 0).expect("the planter");
    let floor = body_base_y(SEED, hv(SEED), w.pieces.cols(), &rec, ARCH_PLANTER);
    let soil = floor + DEPLOY_VOL[ARCH_PLANTER as usize][1] - BED_SOIL_DROP_M;
    let p = w.farm.entries()[0];
    assert_eq!(
        p.qy,
        quant_y(soil),
        "the plant stands at {} and the bed's soil is at {soil}",
        p.qy as f32 * POS_Y_Q
    );
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

/// **A plant whose planter is gone cannot be tended through the box that
/// took its address.** A raid drops the planter, a box goes down in the same
/// cell before self-heal visits the plant, and a water, pick or clear on the
/// stale plant is refused: no water lands in the box (which the save decoder
/// refuses, so the shard would not boot), and the save still loads.
#[test]
fn a_stale_plant_is_not_tended_through_a_box_at_its_address() {
    use sim_core::deploy::ARCH_BOX;
    use sim_core::farm::{TEND_CLEAR, TEND_WATER};
    const BOX_ITEM: u16 = 25;
    const HAND: usize = 4;
    let (mut w, cx, cz) = farm_world();
    quick(&mut w);
    let host = place_planter(&mut w, cx, cz, 0);
    plant(&mut w, host, 0);
    let id = w.farm.entries()[0].id;
    // A box row after the planter's, in this world and the one it loads into.
    let add_box = |w: &mut World| {
        w.gather.stack_max[BOX_ITEM as usize] = 1;
        w.gather.item_count = w.gather.item_count.max(BOX_ITEM + 1);
        let row = w.deploy.def_count;
        w.deploy.defs[row as usize] = DeployDef {
            arch: ARCH_BOX,
            placement: PLACE_ANY,
            hp: 100,
            matter: MATTER_WOOD,
            item: BOX_ITEM,
            n_costs: 0,
            costs: [(0, 0); 4],
        };
        w.deploy.def_count += 1;
        row
    };
    let box_row = add_box(&mut w);
    // Ripe, so a pick would pay if it were let through.
    load_skin(&mut w, host);
    for _ in 0..2_000 {
        if w.farm.entries()[0].stage == STAGE_RIPE {
            break;
        }
        w.tick(&[]);
    }
    assert_eq!(w.farm.entries()[0].stage, STAGE_RIPE);
    // Plant 0 is visited on ticks where tick % 15 == 0; start just past one.
    while w.tick % OVEN_PERIOD_TICKS != 1 {
        w.tick(&[]);
    }

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
    w.players[0].inv[PLANTER_SLOT] = ItemStack {
        item: BOX_ITEM,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.players[0].frame.sel = PLANTER_SLOT as u8;
    w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row: box_row,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        pose: sim_core::footprint::Pose::CENTRE,
    }]);
    let bi = w.deploys.box_index(host).expect("the box took the address");
    assert_eq!(w.deploys.oven_states()[bi].arch, ARCH_BOX);
    assert_eq!(w.farm.len(), 1, "self-heal has not reached the plant yet");

    w.players[0].inv[HAND] = ItemStack {
        item: SKIN_FULL,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.players[0].frame.sel = HAND as u8;
    for act in [TEND_WATER, TEND_PICK, TEND_CLEAR] {
        w.tick(&[Command::Tend {
            id: PLAYER,
            plant: id,
            act,
        }]);
        assert_eq!(refused(&w), Some(REFUSE_D_REACH), "act {act} let through");
    }
    assert_eq!(w.players[0].inv[HAND].item, SKIN_FULL, "the skin was spent");
    assert!(!w.players[0]
        .inv
        .iter()
        .any(|s| s.item == CORN && s.count > 0));
    assert_eq!(w.deploys.oven_states()[bi].water, 0, "water in a box");

    idle(&mut w, OVEN_PERIOD_TICKS);
    assert!(
        w.farm.is_empty(),
        "self-heal did not uproot the stale plant"
    );
    w.tick(&[Command::Leave { id: PLAYER }]);
    let mut buf = vec![0u8; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut buf).expect("encodes");
    let (mut back, _, _) = farm_world();
    add_box(&mut back);
    back.load(&buf[..n]).expect("the save loads");
}

// ---------------------------------------------------------------- genes

/// A genome from its six letters, slot 0 first.
fn g(letters: &[u8; 6]) -> u16 {
    letters.iter().enumerate().fold(0u16, |acc, (k, &c)| {
        let l = sim_core::farm::GENE_CHARS
            .iter()
            .position(|&x| x == c)
            .expect("a gene letter") as u8;
        sim_core::farm::with_gene(acc, k, l)
    })
}

/// Where the clone stacks sit in the pack for planting.
const CLONE_SLOT: usize = 7;

/// Plant a clone of `genes` (`item`'s seed carrying `cond = genes + 1`) in
/// `spot`, and return the new plant's id.
fn plant_clone(w: &mut World, host: u32, spot: u8, item: u16, genes: u16) -> u32 {
    w.players[0].inv[CLONE_SLOT] = ItemStack {
        item,
        count: 1,
        cond: genes + 1,
        skin: 0,
    };
    let next = w.farm.next_id();
    w.tick(&[Command::Plant {
        id: PLAYER,
        slot: CLONE_SLOT as u8,
        host,
        spot,
        qx: 0,
        qz: 0,
    }]);
    assert_eq!(refused(w), None, "the clone at spot {spot} did not plant");
    next
}

fn plant_of(w: &World, id: u32) -> PlantRec {
    let i = w.farm.index_of_id(id).expect("the plant stands");
    w.farm.entries()[i]
}

/// Tick until plant `id` reaches `stage`, or panic after `max` ticks.
fn grow_to(w: &mut World, id: u32, stage: u8, max: u64) {
    for _ in 0..max {
        if plant_of(w, id).stage >= stage {
            return;
        }
        w.tick(&[]);
    }
    panic!(
        "plant {id} stuck at stage {} short of {stage}",
        plant_of(w, id).stage
    );
}

mod crossbreed_unit {
    use super::g;
    use sim_core::farm::crossbreed;

    /// (a) Two W neighbours outweigh an own G: 20 > 6.
    #[test]
    fn two_w_replace_an_own_g() {
        let own = g(b"GXXXXX");
        let n = [(1, g(b"WXXXXX")), (2, g(b"WXXXXX"))];
        assert_eq!(crossbreed(own, &n), g(b"WXXXXX"));
    }

    /// (b) One X outweighs an own G: 10 > 6.
    #[test]
    fn one_x_replaces_an_own_g() {
        assert_eq!(crossbreed(g(b"GGGGGG"), &[(4, g(b"XXXXXX"))]), g(b"XXXXXX"));
    }

    /// (c) One Y does not outweigh an own G: 6 is not more than 6.
    #[test]
    fn one_y_does_not_replace_an_own_g() {
        assert_eq!(crossbreed(g(b"GGGGGG"), &[(4, g(b"YYYYYY"))]), g(b"GGGGGG"));
    }

    /// (d) Two Y outweigh an own W: 12 > 10.
    #[test]
    fn two_y_replace_an_own_w() {
        let n = [(1, g(b"YYYYYY")), (2, g(b"YYYYYY"))];
        assert_eq!(crossbreed(g(b"WWWWWW"), &n), g(b"YYYYYY"));
    }

    /// (e) One Y does not outweigh an own W: 6 < 10.
    #[test]
    fn one_y_does_not_replace_an_own_w() {
        assert_eq!(crossbreed(g(b"WWWWWW"), &[(1, g(b"YYYYYY"))]), g(b"WWWWWW"));
    }

    /// (f) A tie between letters goes to the one planted first, and a tie
    /// that does not outweigh the own gene changes nothing.
    #[test]
    fn a_tie_goes_to_the_first_planted_and_must_still_outweigh() {
        // G (id 5) and Y (id 3) tie at 6, under the own X's 10.
        let n = [(3, g(b"YXXXXX")), (5, g(b"GXXXXX"))];
        assert_eq!(crossbreed(g(b"XXXXXX"), &n), g(b"XXXXXX"));
        // Two G (7, 9) and two Y (3, 8): 12 each, over 10; Y was first.
        let n = [
            (3, g(b"YXXXXX")),
            (7, g(b"GXXXXX")),
            (8, g(b"YXXXXX")),
            (9, g(b"GXXXXX")),
        ];
        assert_eq!(
            sim_core::farm::gene(crossbreed(g(b"XXXXXX"), &n), 0),
            sim_core::farm::GENE_Y
        );
        // The order the neighbours arrive in does not matter.
        let mut r = n;
        r.reverse();
        assert_eq!(crossbreed(g(b"XXXXXX"), &r), crossbreed(g(b"XXXXXX"), &n));
    }

    /// (g) The own gene never votes: an own G and one G neighbour (6) lose
    /// to two Y (12 > 6).
    #[test]
    fn the_own_gene_does_not_vote() {
        let n = [(1, g(b"GXXXXX")), (2, g(b"YXXXXX")), (3, g(b"YXXXXX"))];
        assert_eq!(
            sim_core::farm::gene(crossbreed(g(b"GXXXXX"), &n), 0),
            sim_core::farm::GENE_Y
        );
    }

    /// (h) Every slot is decided on its own.
    #[test]
    fn slots_are_independent() {
        let n = [(1, g(b"WYGHXW")), (2, g(b"WYGHXW"))];
        // Slot 0: W 20 > G 6. Slot 1: Y 12 > W 10. Slot 2: G 12 > X 10.
        // Slot 3: H 12 > G 6. Slot 4: X 20 > W 10. Slot 5: W 20 > G 6.
        assert_eq!(crossbreed(g(b"GWXGWG"), &n), g(b"WYGHXW"));
        // One donor: a green 6 moves nothing (not past a red 10 nor
        // strictly past a green 6), a red 10 moves a green one only.
        assert_eq!(crossbreed(g(b"GWXGWG"), &n[..1]), g(b"WWXGWW"));
    }

    /// (i) No neighbours, no change.
    #[test]
    fn no_neighbours_no_change() {
        assert_eq!(crossbreed(g(b"GGYHWX"), &[]), g(b"GGYHWX"));
    }
}

/// **A blank seed's genes are a deterministic roll**: equal inputs give
/// equal genes, and over many rolls half the slots are good (G, Y or H in
/// equal parts) and half the rest are W.
#[test]
fn a_seed_rolls_its_genes_deterministically_at_the_content_share() {
    use sim_core::farm::{count, roll_genes, GENE_G, GENE_H, GENE_W, GENE_Y};
    let fc = FarmContent::probe_fixture();
    assert_eq!((fc.good_gene_pm, fc.bad_w_pm), (500, 500));
    assert_eq!(
        roll_genes(&fc, SEED, 40, 41, 1234, 7),
        roll_genes(&fc, SEED, 40, 41, 1234, 7)
    );
    let mut tally = [0u32; 5];
    let mut distinct = std::collections::BTreeSet::new();
    for id in 1..=2000u32 {
        let genes = roll_genes(&fc, SEED, 40, 41, 1234, id);
        assert!((genes as u32) < sim_core::farm::GENOME_VALUES);
        distinct.insert(genes);
        for l in 0..5u8 {
            tally[l as usize] += count(genes, l);
        }
    }
    let slots = 2000 * 6;
    let good = tally[GENE_G as usize] + tally[GENE_Y as usize] + tally[GENE_H as usize];
    assert!(
        (slots * 47 / 100..=slots * 53 / 100).contains(&good),
        "good share {good} of {slots}"
    );
    let w = tally[GENE_W as usize];
    let bad = slots - good;
    assert!(
        (bad * 45 / 100..=bad * 55 / 100).contains(&w),
        "W share {w} of {bad}"
    );
    for l in [GENE_G, GENE_Y, GENE_H] {
        let n = tally[l as usize];
        assert!(n * 3 > good * 9 / 10 && n * 3 < good * 11 / 10, "{l}: {n}");
    }
    assert!(distinct.len() > 1500, "the id does not reach the roll");
    // The tick and the cell reach it too.
    assert_ne!(
        (1..50)
            .map(|t| roll_genes(&fc, SEED, 40, 41, t, 7))
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        1
    );
    // Off: every slot X.
    assert_eq!(roll_genes(&FarmContent::EMPTY, SEED, 40, 41, 1234, 7), 0);
}

/// **A blank seed plants a rolled genome, a clone plants its own.**
#[test]
fn a_clone_plants_its_genes_and_a_blank_seed_rolls() {
    let (mut w, cx, cz) = farm_world();
    w.cook.farm.good_gene_pm = 500;
    w.cook.farm.bad_w_pm = 500;
    let host = place_planter(&mut w, cx, cz, 0);
    let want = g(b"GGYHWX");
    let id = plant_clone(&mut w, host, 3, SEED_ITEM, want);
    assert_eq!(plant_of(&w, id).genes, want);
    assert_eq!(w.players[0].inv[CLONE_SLOT], ItemStack::default());
    let next = w.farm.next_id();
    let tick = w.tick;
    plant(&mut w, host, 0);
    let rolled = plant_of(&w, next).genes;
    assert_eq!(
        rolled,
        sim_core::farm::roll_genes(&w.cook.farm, SEED, cx, cz, tick, next),
        "the roll is the plant's cell, its tick and its id"
    );
    // A `cond` past the genome count is no clone: it rolls.
    w.players[0].inv[CLONE_SLOT] = ItemStack {
        item: SEED_ITEM,
        count: 1,
        cond: sim_core::farm::GENOME_VALUES as u16 + 1,
        skin: 0,
    };
    let next = w.farm.next_id();
    let tick = w.tick;
    w.tick(&[Command::Plant {
        id: PLAYER,
        slot: CLONE_SLOT as u8,
        host,
        spot: 5,
        qx: 0,
        qz: 0,
    }]);
    assert_eq!(
        plant_of(&w, next).genes,
        sim_core::farm::roll_genes(&w.cook.farm, SEED, cx, cz, tick, next)
    );
}

/// **Crossbreeding counts only the touching plants of its own crop in its
/// own bed, alive**: hemp beside corn, a corn two columns over, a corn in
/// another planter and a dying corn all leave it alone, and a touching
/// corn takes it over. It runs once, entering CROSSBREED; a neighbour
/// planted after it changes nothing.
///
/// Two touching plants that enter CROSSBREED in the same visit round cross
/// in store order: the later one reads the earlier one's new genes. That is
/// deterministic but not symmetric, so this test staggers its plantings.
#[test]
fn crossbreed_counts_touching_same_crop_live_neighbours_once() {
    use sim_core::farm::{FL_CROSSED, STAGE_CROSSBREED, STAGE_DYING, STAGE_MATURE};
    let (mut w, cx, cz) = farm_world();
    quick(&mut w);
    // Long enough a death that the dying donor stands through the cross.
    w.cook.farm.dying_ticks = 3_000;
    let gs = g(b"GGGGGG");
    let ws = g(b"WWWWWW");
    let host = place_planter(&mut w, cx, cz, 0);
    load_skin(&mut w, host);
    // Spot 3 touches spot 0: grow a W corn there to DYING first.
    let dying = plant_clone(&mut w, host, 3, SEED_ITEM, ws);
    grow_to(&mut w, dying, STAGE_DYING, 2_000);
    // Then the target at 0, beside a W hemp (1), a W corn two columns away
    // (2) and a W corn not touching (5). A Y corn at 4 touches but cannot
    // outweigh G (6 is not > 6).
    let target = plant_clone(&mut w, host, 0, SEED_ITEM, gs);
    plant_clone(&mut w, host, 1, HEMP_SEED, ws);
    plant_clone(&mut w, host, 2, SEED_ITEM, ws);
    plant_clone(&mut w, host, 5, SEED_ITEM, ws);
    plant_clone(&mut w, host, 4, SEED_ITEM, g(b"YYYYYY"));
    // A W corn at spot 1 of a second planter: the same spot number,
    // another bed.
    let (x, z) = cell_center(cx + 1, cz);
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z);
    let other = place_planter(&mut w, cx + 1, cz, 0);
    plant_clone(&mut w, other, 1, SEED_ITEM, ws);
    grow_to(&mut w, target, STAGE_CROSSBREED, 2_000);
    assert_eq!(plant_of(&w, dying).stage, STAGE_DYING, "the donor withered");
    let p = plant_of(&w, target);
    assert_ne!(p.flags & FL_CROSSED, 0, "the cross did not run");
    assert_eq!(p.genes, gs, "a neighbour that does not count changed it");
    assert!(w
        .events
        .entries()
        .iter()
        .any(|e| e.code == EV_PLANT && e.a == target && e.b >> 8 == STAGE_CROSSBREED as u32));

    // The same bed with a touching W corn (spot 3, alive) takes it over.
    let (mut w, cx, cz) = farm_world();
    quick(&mut w);
    let host = place_planter(&mut w, cx, cz, 0);
    load_skin(&mut w, host);
    let target = plant_clone(&mut w, host, 0, SEED_ITEM, gs);
    plant_clone(&mut w, host, 3, SEED_ITEM, ws);
    grow_to(&mut w, target, STAGE_CROSSBREED, 2_000);
    assert_eq!(
        plant_of(&w, target).genes,
        ws,
        "a touching W corn did not count"
    );
    // Once: a G neighbour planted beside it now, and a whole grow on,
    // leave the crossed genes alone.
    plant_clone(&mut w, host, 1, SEED_ITEM, gs);
    plant_clone(&mut w, host, 4, SEED_ITEM, gs);
    grow_to(&mut w, target, STAGE_MATURE, 2_000);
    idle(&mut w, 200);
    assert_eq!(plant_of(&w, target).genes, ws, "it crossed twice");
}

/// **Six G ripen a plant in 1/1.6 of the crop's time.**
#[test]
fn six_g_ripen_in_one_part_in_one_point_six_of_the_time() {
    let (mut w, cx, cz) = farm_world();
    let host = place_planter(&mut w, cx, cz, 0);
    load_skin(&mut w, host);
    let start = w.tick;
    let id = plant_clone(&mut w, host, 0, SEED_ITEM, g(b"GGGGGG"));
    grow_to(&mut w, id, STAGE_RIPE, GROW);
    let took = w.tick - start;
    let want = GROW * 10 / 16;
    assert!(
        took.abs_diff(want) <= 2 * OVEN_PERIOD_TICKS,
        "6G ripened in {took} ticks, wanted {want}"
    );
}

/// **Six Y yield floor(4 × 1.6) = 6 corn.**
#[test]
fn six_y_pick_six_corn() {
    let (mut w, cx, cz) = farm_world();
    quick(&mut w);
    let host = place_planter(&mut w, cx, cz, 0);
    load_skin(&mut w, host);
    let id = plant_clone(&mut w, host, 0, SEED_ITEM, g(b"YYYYYY"));
    grow_to(&mut w, id, STAGE_RIPE, 2_000);
    w.tick(&[Command::Tend {
        id: PLAYER,
        plant: id,
        act: TEND_PICK,
    }]);
    assert_eq!(refused(&w), None);
    let corn: u32 = w.players[0]
        .inv
        .iter()
        .filter(|s| s.item == CORN)
        .map(|s| s.count as u32)
        .sum();
    assert_eq!(corn, 6);
}

/// **Six W drink 2.5× the water** (37 wu a period against 15: the draw is
/// whole water units).
#[test]
fn six_w_drain_a_planter_two_and_a_half_times_faster() {
    let drained = |genes: u16| {
        let (mut w, cx, cz) = farm_world();
        let host = place_planter(&mut w, cx, cz, 0);
        load_skin(&mut w, host);
        plant_clone(&mut w, host, 0, SEED_ITEM, genes);
        idle(&mut w, OVEN_PERIOD_TICKS);
        let full = tank(&w, host);
        idle(&mut w, 100 * OVEN_PERIOD_TICKS);
        full - tank(&w, host)
    };
    let x = drained(g(b"XXXXXX"));
    let w6 = drained(g(b"WWWWWW"));
    assert_eq!(x, 100 * 15);
    assert_eq!(w6, 100 * 37);
    assert!(w6 * 10 >= x * 24 && w6 * 10 <= x * 25, "{w6} against {x}");
}

/// **A clone is cut from a sapling to a mature plant**: it pays the crop's
/// clones (one more per two Y) as its seed carrying its genes, the plant is
/// gone, and the clones never merge into a blank seed stack. A seedling or
/// a fruiting plant refuses.
#[test]
fn a_clone_pays_seeds_carrying_genes_and_kills_the_plant() {
    use sim_core::farm::{GONE_CLONED, STAGE_FRUITING, STAGE_SAPLING, STAGE_SEEDLING, TEND_CLONE};
    let clone = |w: &mut World, id: u32| {
        w.tick(&[Command::Tend {
            id: PLAYER,
            plant: id,
            act: TEND_CLONE,
        }])
    };
    let (mut w, cx, cz) = farm_world();
    quick(&mut w);
    // A blank stack with room, so a merge would have somewhere to go.
    w.players[0].inv[SEED_SLOT].count = 10;
    let host = place_planter(&mut w, cx, cz, 0);
    load_skin(&mut w, host);
    let genes = g(b"GGYHWX");
    let id = plant_clone(&mut w, host, 0, SEED_ITEM, genes);
    grow_to(&mut w, id, STAGE_SEEDLING, 2_000);
    assert_eq!(plant_of(&w, id).stage, STAGE_SEEDLING);
    clone(&mut w, id);
    assert_eq!(refused(&w), Some(REFUSE_D_PLANT_STAGE), "a seedling cloned");
    grow_to(&mut w, id, STAGE_SAPLING, 2_000);
    clone(&mut w, id);
    assert_eq!(refused(&w), None);
    let ev = w
        .events
        .entries()
        .iter()
        .find(|e| e.code == EV_GATHER && e.a == PLAYER)
        .copied()
        .expect("a clone announces its pay");
    assert_eq!(ev.b, (SEED_ITEM as u32) << 16 | 2, "two clones");
    assert_eq!(gone(&w), Some((id, GONE_CLONED as u32, PLAYER)));
    assert!(w.farm.index_of_id(id).is_none());
    assert_eq!(
        w.players[0].inv[SEED_SLOT],
        ItemStack {
            item: SEED_ITEM,
            count: 10,
            cond: 0,
            skin: 0
        },
        "a clone merged into the blank seeds"
    );
    let clones: Vec<ItemStack> = w.players[0]
        .inv
        .iter()
        .copied()
        .filter(|s| s.item == SEED_ITEM && s.cond != 0)
        .collect();
    assert_eq!(
        clones,
        vec![ItemStack {
            item: SEED_ITEM,
            count: 2,
            cond: genes + 1,
            skin: 0
        }]
    );

    // Two Y pay one more; past MATURE it refuses.
    let yy = g(b"YYXXXX");
    let id = plant_clone(&mut w, host, 1, SEED_ITEM, yy);
    grow_to(&mut w, id, STAGE_SAPLING, 2_000);
    clone(&mut w, id);
    assert!(w
        .events
        .entries()
        .iter()
        .any(|e| e.code == EV_GATHER && e.b == (SEED_ITEM as u32) << 16 | 3));
    let id = plant_clone(&mut w, host, 2, SEED_ITEM, yy);
    grow_to(&mut w, id, STAGE_FRUITING, 2_000);
    clone(&mut w, id);
    assert_eq!(
        refused(&w),
        Some(REFUSE_D_PLANT_STAGE),
        "a fruiting plant cloned"
    );
    // A clone of the clone grows the genes it was cut with.
    let i = w.players[0]
        .inv
        .iter()
        .position(|s| s.item == SEED_ITEM && s.cond == genes + 1)
        .unwrap();
    let next = w.farm.next_id();
    w.tick(&[Command::Plant {
        id: PLAYER,
        slot: i as u8,
        host,
        spot: 3,
        qx: 0,
        qz: 0,
    }]);
    assert_eq!(refused(&w), None);
    assert_eq!(plant_of(&w, next).genes, genes);
    assert_eq!(w.players[0].inv[i].count, 1);
}

// ---------------------------------------------------------------- ground

/// Whether the open ground at `(x, z)` would take a seed as far as the
/// terrain and the build say, and what biome it is.
fn site(w: &World, x: f32, z: f32) -> Result<(f32, sim_core::terrain::Biome), Ground> {
    let y = ground_site(SEED, hv(SEED), w.pieces.cols(), x, z, || false)?;
    Ok((y, sim_core::terrain::biome_at(SEED, y, x, z)))
}

/// The first point, scanned outward in rings of `step` metres from the
/// fixture's cell, that `want` takes — the generator decides where things
/// are, so a fixture point is found, never typed.
fn scan(w: &World, step: f32, rings: i32, want: impl Fn(f32, f32) -> bool) -> (f32, f32) {
    let (cx, cz) = buildable_cell(SEED);
    let (x0, z0) = cell_center(cx, cz);
    for r in 0..rings {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let (x, z) = (x0 + dx as f32 * step, z0 + dz as f32 * step);
                if want(x, z) {
                    return (x, z);
                }
            }
        }
    }
    let _ = w;
    panic!("no fixture point within {rings} rings of {step} m — the generator moved");
}

/// Open meadow at `(x, z)` and at `(x + 0.9, z)`, so two ground plants fit
/// a hand's reach apart.
fn meadow(w: &World) -> (f32, f32) {
    use sim_core::terrain::Biome;
    scan(w, 1.5, 200, |x, z| {
        [(x, z), (x + 0.9, z), (x + 0.45, z + 1.0)]
            .iter()
            .all(|&(x, z)| matches!(site(w, x, z), Ok((_, Biome::Meadow))))
    })
}

/// Stand the player a metre from `(x, z)`.
fn stand_by(w: &mut World, x: f32, z: f32) {
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z + 1.0);
}

fn sow(w: &mut World, slot: usize, x: f32, z: f32) {
    w.tick(&[Command::Plant {
        id: PLAYER,
        slot: slot as u8,
        host: HOST_GROUND,
        spot: 0,
        qx: quant_xz(x),
        qz: quant_xz(z),
    }]);
}

/// Sow a clone of `genes` at `(x, z)` and return its id.
fn sow_clone(w: &mut World, x: f32, z: f32, genes: u16) -> u32 {
    w.players[0].inv[CLONE_SLOT] = ItemStack {
        item: SEED_ITEM,
        count: 1,
        cond: genes + 1,
        skin: 0,
    };
    let next = w.farm.next_id();
    sow(w, CLONE_SLOT, x, z);
    assert_eq!(refused(w), None, "the clone at ({x}, {z}) did not sow");
    next
}

/// The hotbar slot a skin is poured from.
const HAND: usize = 5;

/// Pour the full skin put in hand on plant `id`.
fn pour(w: &mut World, id: u32) -> Option<u32> {
    w.players[0].inv[HAND] = ItemStack {
        item: SKIN_FULL,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.players[0].frame.sel = HAND as u8;
    w.tick(&[Command::Tend {
        id: PLAYER,
        plant: id,
        act: sim_core::farm::TEND_WATER,
    }]);
    refused(w)
}

/// **A seed sows soft open ground**: one unit, a ground plant at the soil
/// point (no host, spot 15), paced by its biome's soil, holding no water
/// yet — and a second one closer than the spacing refuses.
#[test]
fn a_seed_sows_open_meadow_and_a_neighbour_too_close_refuses() {
    let (mut w, _, _) = farm_world();
    let (x, z) = meadow(&w);
    stand_by(&mut w, x, z);
    sow(&mut w, SEED_SLOT, x, z);
    assert_eq!(refused(&w), None);
    assert_eq!(w.players[0].inv[SEED_SLOT].count, 49);
    let p = w.farm.entries()[0];
    assert_eq!(
        (p.host, p.spot, p.owner, p.soil_pm, p.water),
        (HOST_GROUND, SPOT_GROUND, PLAYER, 550, 0)
    );
    let y = sim_core::terrain::ground(SEED, hv(SEED), x, z);
    assert_eq!(
        (p.qx, p.qy, p.qz),
        (quant_xz(x), sim_core::movement::quant_y(y), quant_xz(z))
    );
    assert!(w
        .events
        .entries()
        .iter()
        .any(|e| e.code == EV_PLANT && e.a == p.id));
    // 0.3 m off: inside the 0.6 m spacing.
    sow(&mut w, SEED_SLOT, x + 0.3, z);
    assert_eq!(refused(&w), Some(REFUSE_D_PLANT_SPOT));
    // 0.9 m off: clear of it.
    sow(&mut w, SEED_SLOT, x + 0.9, z);
    assert_eq!(refused(&w), None);
    assert_eq!(w.farm.ground_count(), 2);
    // Past a hand's reach: refused before the ground is asked.
    sow(&mut w, SEED_SLOT, x + 6.0, z);
    assert_eq!(refused(&w), Some(REFUSE_D_REACH));
}

/// **The road, the town, the sea's edge, a roof, a foundation, a foreign
/// claim, the desert and the Verdigris refuse a seed**, each for its own
/// reason, at points the generator put there.
#[test]
fn ground_that_is_not_soft_open_soil_refuses_a_seed() {
    use sim_core::terrain::{biome_at, road_band, Biome, RoadBand};
    let (mut w, cx, cz) = farm_world();
    let refuse = |w: &mut World, x: f32, z: f32| {
        stand_by(w, x, z);
        sow(w, SEED_SLOT, x, z);
        refused(w)
    };
    let land = |x: f32, z: f32| {
        sim_core::terrain::ground(SEED, hv(SEED), x, z) >= sim_core::terrain::LAND_MIN_H
    };

    // The carriageway.
    let (x, z) = scan(&w, 4.0, 800, |x, z| {
        land(x, z) && road_band(SEED, hv(SEED), x, z) == RoadBand::Carriageway
    });
    assert_eq!(site(&w, x, z).err(), Some(Ground::Road));
    assert_eq!(refuse(&mut w, x, z), Some(REFUSE_D_PLANT_SOIL), "the road");

    // The town's reserve, off its roads.
    let town = &hv(SEED).town;
    assert!(town.live, "the fixture's island has no town");
    let (x, z) = (0..400)
        .map(|k| {
            (
                town.x + (k % 20) as f32 * 2.0 - 20.0,
                town.z + (k / 20) as f32 * 2.0 - 20.0,
            )
        })
        .find(|&(x, z)| {
            land(x, z)
                && road_band(SEED, hv(SEED), x, z) == RoadBand::Off
                && sim_core::terrain::build_reserved(hv(SEED), x, z, 0.5)
        })
        .expect("no unpaved point inside the town's reserve");
    assert_eq!(site(&w, x, z).err(), Some(Ground::Reserved));
    assert_eq!(refuse(&mut w, x, z), Some(REFUSE_D_PLANT_SOIL), "the town");

    // The desert and the Verdigris: open land the soil table leaves out.
    for (biome, what) in [(Biome::Desert, "the desert"), (Biome::Fel, "the Verdigris")] {
        let (x, z) = scan(&w, 24.0, 160, |x, z| {
            land(x, z)
                && biome_at(SEED, sim_core::terrain::ground(SEED, hv(SEED), x, z), x, z) == biome
                && site(&w, x, z).is_ok()
        });
        assert_eq!(refuse(&mut w, x, z), Some(REFUSE_D_PLANT_SOIL), "{what}");
    }

    // A foundation, then a wall and a floor over it: built over, then roofed.
    for (row, level, loc) in [
        (0u16, 0u8, LOC_PLANE),
        (1, 0, LOC_EDGE_XLO),
        (2, 1, LOC_PLANE),
    ] {
        let (x, z) = cell_center(cx, cz);
        w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z + 2.5);
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
        if row == 0 {
            assert_eq!(site(&w, x, z).err(), Some(Ground::Built));
        }
    }
    let (x, z) = cell_center(cx, cz);
    assert_eq!(site(&w, x, z).err(), Some(Ground::Roofed));
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x + 0.5, z);
    sow(&mut w, SEED_SLOT, x, z);
    assert_eq!(refused(&w), Some(REFUSE_D_PLANT_SOIL), "under a roof");
    assert!(w.farm.is_empty(), "something sowed");
}

/// **A foreign base's claim refuses a seed beside it**, and its own crew
/// sows there.
#[test]
fn a_foreign_claim_refuses_a_seed_and_the_crew_sows() {
    use sim_core::deploy::REFUSE_D_CLAIM;
    let (mut w, cx, cz) = farm_world();
    let (x, z) = cell_center(cx, cz);
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z + 2.5);
    w.tick(&[Command::Place {
        id: PLAYER,
        row: 0,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        freehand: false,
        plate: 0,
    }]);
    assert_eq!(w.pieces.len(), 1, "the foundation did not place");
    w.players[0].inv[PLANTER_SLOT] = ItemStack {
        item: 2,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.players[0].frame.sel = PLANTER_SLOT as u8;
    w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row: 0,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        pose: sim_core::footprint::Pose::CENTRE,
    }]);
    assert_eq!(w.deploys.hearths().len(), 1, "the hearth did not place");
    // Two metres off the foundation's edge, inside the claim's cushion.
    let (px, pz) = (x + BUILD_CELL_M * 0.5 + 2.0, z);
    assert!(
        site(&w, px, pz).is_ok(),
        "the ground beside the base is not soft"
    );

    const STRANGER: u32 = 4;
    w.tick(&[Command::Join { id: STRANGER }]);
    let s = w
        .players
        .iter()
        .position(|p| p.id == STRANGER)
        .expect("joined");
    w.players[s].body = sim_core::movement::Body::at(SEED, hv(SEED), px + 0.5, pz + 1.0);
    w.players[s].inv[SEED_SLOT] = ItemStack {
        item: SEED_ITEM,
        count: 5,
        cond: 0,
        skin: 0,
    };
    w.tick(&[Command::Plant {
        id: STRANGER,
        slot: SEED_SLOT as u8,
        host: HOST_GROUND,
        spot: 0,
        qx: quant_xz(px),
        qz: quant_xz(pz),
    }]);
    let why = w
        .events
        .entries()
        .iter()
        .find(|e| e.code == EV_DEPLOY_REFUSED && e.a == STRANGER)
        .map(|e| e.b);
    assert_eq!(why, Some(REFUSE_D_CLAIM));
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), px + 0.5, pz + 1.0);
    sow(&mut w, SEED_SLOT, px, pz);
    assert_eq!(refused(&w), None, "the crew's own ground");
}

/// Ground plant `k` of `owner`, far off along the island's edge: a cap's
/// worth of them, put straight in the store.
fn far(k: usize, owner: u32) -> PlantRec {
    PlantRec {
        owner,
        host: HOST_GROUND,
        spot: SPOT_GROUND,
        qx: 10 + k as i32 * 100,
        qz: 10,
        ..PlantRec::default()
    }
}

/// **One planter sows at most 48 ground plants, the island 512.**
#[test]
fn the_49th_ground_plant_of_one_owner_refuses() {
    use sim_core::limits::{MAX_GROUND_PLANTS, MAX_GROUND_PLANTS_PER_OWNER};
    let (mut w, _, _) = farm_world();
    let (x, z) = meadow(&w);
    for k in 0..MAX_GROUND_PLANTS_PER_OWNER - 1 {
        w.farm.insert(far(k, PLAYER)).expect("room");
    }
    stand_by(&mut w, x + 0.45, z);
    sow(&mut w, SEED_SLOT, x, z);
    assert_eq!(refused(&w), None, "the 48th");
    sow(&mut w, SEED_SLOT, x + 0.9, z);
    assert_eq!(refused(&w), Some(REFUSE_D_PLANTS_FULL), "the 49th");
    assert_eq!(w.players[0].inv[SEED_SLOT].count, 49);

    // Someone else's fill the island.
    let (mut w, _, _) = farm_world();
    for k in 0..MAX_GROUND_PLANTS {
        w.farm.insert(far(k, 100 + k as u32)).expect("room");
    }
    stand_by(&mut w, x, z);
    sow(&mut w, SEED_SLOT, x, z);
    assert_eq!(refused(&w), Some(REFUSE_D_PLANTS_FULL), "the 513th");
}

/// **A relog keeps the sower's ground plants on its cap**: a player id is
/// a connection's, and the sleeper woken under a new one still owns its 48
/// (`World::remap_player_ids`, which the restart's id floor reads too).
#[test]
fn a_relog_keeps_the_sowers_ground_plants_on_its_cap() {
    use sim_core::limits::MAX_GROUND_PLANTS_PER_OWNER;
    let (mut w, _, _) = farm_world();
    let (x, z) = meadow(&w);
    for k in 0..MAX_GROUND_PLANTS_PER_OWNER {
        w.farm.insert(far(k, PLAYER)).expect("room");
    }
    let mut seen = 0;
    w.remap_player_ids(&mut |id| {
        seen += (id == PLAYER) as usize;
        id
    });
    assert!(
        seen > MAX_GROUND_PLANTS_PER_OWNER,
        "the id floor misses a sower"
    );
    w.tick(&[Command::Leave { id: PLAYER }]);
    const NEW: u32 = 0x0100 | PLAYER;
    w.tick(&[Command::Wake {
        id: NEW,
        sleeper: PLAYER,
    }]);
    assert_eq!(w.players[0].id, NEW, "the sleeper did not wake");
    assert_eq!(
        (w.farm.ground_count_of(PLAYER), w.farm.ground_count_of(NEW)),
        (0, MAX_GROUND_PLANTS_PER_OWNER)
    );
    stand_by(&mut w, x, z);
    w.tick(&[Command::Plant {
        id: NEW,
        slot: SEED_SLOT as u8,
        host: HOST_GROUND,
        spot: 0,
        qx: quant_xz(x),
        qz: quant_xz(z),
    }]);
    let why = w
        .events
        .entries()
        .iter()
        .find(|e| e.code == EV_DEPLOY_REFUSED && e.a == NEW)
        .map(|e| e.b);
    assert_eq!(why, Some(REFUSE_D_PLANTS_FULL), "a relog reset the cap");
}

/// A cell whose centre is soft soil a foundation may stand on, with a
/// buildable neighbour at +x: the fixture for building over a plant.
fn soft_cell(w: &World) -> (u16, u16) {
    let (cx0, cz0) = buildable_cell(SEED);
    for r in 0..24i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let (cx, cz) = ((cx0 as i32 + dx) as u16, (cz0 as i32 + dz) as u16);
                let (x, z) = cell_center(cx, cz);
                let soft = site(w, x, z).is_ok_and(|(_, b)| {
                    w.cook.farm.soil_pm.get(b as usize).is_some_and(|&s| s > 0)
                });
                if soft
                    && foundation_terrain_ok(SEED, hv(SEED), x, z)
                    && foundation_terrain_ok(SEED, hv(SEED), x + BUILD_CELL_M, z)
                {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no soft buildable cell near the fixture — the generator moved");
}

/// Place the fixture's workbench (row 1, item 3) at the centre of
/// `(cx, cz)`, standing outside its footprint.
fn place_bench(w: &mut World, cx: u16, cz: u16) {
    let (x, z) = cell_center(cx, cz);
    w.players[0].inv[PLANTER_SLOT] = ItemStack {
        item: 3,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.players[0].frame.sel = PLANTER_SLOT as u8;
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z + 1.5);
    let before = w.deploys.len();
    w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row: 1,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        pose: sim_core::footprint::Pose::CENTRE,
    }]);
    assert_eq!(w.deploys.len(), before + 1, "the workbench did not place");
}

/// Place build row `row` at `(cx, cz, level, loc)`, standing off the cell.
fn place_piece(w: &mut World, row: u16, cx: u16, cz: u16, level: u8, loc: u8) {
    let (x, z) = cell_center(cx, cz);
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z + 2.5);
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

/// Tick until plant `id` is uprooted, at most `ticks`; whether it was.
fn uprooted_within(w: &mut World, id: u32, ticks: u64) -> bool {
    for _ in 0..ticks {
        if gone(w) == Some((id, GONE_UPROOTED as u32, 0)) {
            return true;
        }
        w.tick(&[]);
    }
    gone(w) == Some((id, GONE_UPROOTED as u32, 0))
}

/// **A seed does not sow inside a standing deploy**: a workbench is taller
/// than a step, and its top is still built over the soil.
#[test]
fn a_seed_does_not_sow_inside_a_workbench() {
    let (mut w, _, _) = farm_world();
    let (cx, cz) = soft_cell(&w);
    place_bench(&mut w, cx, cz);
    let (x, z) = cell_center(cx, cz);
    assert_eq!(site(&w, x, z).err(), Some(Ground::Built));
    sow(&mut w, SEED_SLOT, x, z);
    assert_eq!(refused(&w), Some(REFUSE_D_PLANT_SOIL));
    assert!(w.farm.is_empty(), "a seed sowed inside the bench");
}

/// **Building over a ground plant uproots it** within a period: a
/// foundation laid over its soil, a deploy set on it, or a floor roofed
/// over it. Left there it could never ripen, be cut or be cleared, and it
/// would hold its owner's cap and the island's for good.
#[test]
fn building_over_a_ground_plant_uproots_it() {
    let sown = |w: &mut World, x: f32, z: f32| {
        stand_by(w, x, z);
        sow_clone(w, x, z, g(b"XXXXXX"))
    };

    // A foundation.
    let (mut w, _, _) = farm_world();
    let (cx, cz) = soft_cell(&w);
    let (x, z) = cell_center(cx, cz);
    let id = sown(&mut w, x, z);
    place_piece(&mut w, 0, cx, cz, 0, LOC_PLANE);
    assert!(
        uprooted_within(&mut w, id, OVEN_PERIOD_TICKS + 1),
        "under a foundation"
    );
    assert_eq!(w.farm.ground_count_of(PLAYER), 0);

    // A workbench.
    let (mut w, _, _) = farm_world();
    let id = sown(&mut w, x, z);
    place_bench(&mut w, cx, cz);
    assert!(
        uprooted_within(&mut w, id, OVEN_PERIOD_TICKS + 1),
        "inside a bench"
    );

    // A floor over the cell, held up by the wall of the base beside it:
    // roofed, with nothing built on the soil itself.
    let (mut w, _, _) = farm_world();
    let id = sown(&mut w, x, z);
    place_piece(&mut w, 0, cx + 1, cz, 0, LOC_PLANE);
    place_piece(&mut w, 1, cx + 1, cz, 0, LOC_EDGE_XLO);
    idle(&mut w, OVEN_PERIOD_TICKS + 1);
    assert!(
        w.farm.index_of_id(id).is_some(),
        "the base beside it uprooted it"
    );
    place_piece(&mut w, 2, cx, cz, 1, LOC_PLANE);
    assert_eq!(site(&w, x, z).err(), Some(Ground::Roofed));
    assert!(
        uprooted_within(&mut w, id, OVEN_PERIOD_TICKS + 1),
        "under a floor"
    );
}

/// **Meadow corn on full water ripens in its grow over the soil's 0.55 and
/// picks floor(4 × 0.55) = 2.**
#[test]
fn watered_meadow_corn_ripens_at_the_soils_pace_and_picks_two() {
    let (mut w, _, _) = farm_world();
    let (x, z) = meadow(&w);
    stand_by(&mut w, x, z);
    let id = sow_clone(&mut w, x, z, g(b"XXXXXX"));
    assert_eq!(pour(&mut w, id), None);
    assert_eq!(plant_of(&w, id).water, w.cook.farm.ground_cap_wu);
    let start = w.tick;
    let want = GROW * 1000 / 550;
    while plant_of(&w, id).stage < STAGE_RIPE && w.tick < start + want + 600 {
        w.tick(&[]);
        assert!(!is_night(w.tick), "the grow must run by day");
        // Fed and watered, so the long grow does not starve the farmer.
        w.players[0].food = w.survival.max_food;
        w.players[0].water = w.survival.max_water;
    }
    let took = w.tick - start;
    let p = plant_of(&w, id);
    assert_eq!(p.stage, STAGE_RIPE, "never ripened");
    assert!(
        took.abs_diff(want) <= 2 * OVEN_PERIOD_TICKS,
        "ripe after {took} ticks, wanted {want}"
    );
    assert_eq!(p.avg_pm(), 550);
    assert_eq!(p.flags & (FL_DRY | sim_core::farm::FL_CROSSED), 0);
    w.tick(&[Command::Tend {
        id: PLAYER,
        plant: id,
        act: TEND_PICK,
    }]);
    assert_eq!(refused(&w), None);
    let ev = w
        .events
        .entries()
        .iter()
        .find(|e| e.code == EV_GATHER && e.a == PLAYER)
        .copied()
        .expect("a pick announces its pay");
    assert_eq!(ev.b, (CORN as u32) << 16 | 2, "two corn");
    assert_eq!(gone(&w), Some((id, GONE_PICKED as u32, PLAYER)));
}

/// **A dry ground plant holds; the rain fills it, and so does a skin**,
/// which empties in the hand; a full one refuses another.
#[test]
fn a_dry_ground_plant_holds_until_rain_or_a_skin_waters_it() {
    use sim_core::deploy::REFUSE_D_PLANT_WATER;
    use sim_core::weather::{FORCE_FADE_TICKS, KEEP_TIME, STORM};
    let (mut w, _, _) = farm_world();
    let (x, z) = meadow(&w);
    stand_by(&mut w, x + 0.45, z);
    let a = sow_clone(&mut w, x, z, g(b"XXXXXX"));
    let b = sow_clone(&mut w, x + 0.9, z, g(b"XXXXXX"));
    idle(&mut w, 60);
    for id in [a, b] {
        let p = plant_of(&w, id);
        assert_eq!((p.grown, p.water), (0, 0), "a dry plant grew");
        assert_ne!(p.flags & FL_DRY, 0, "a dry plant is not marked dry");
    }

    // A skin: poured whole, the plant takes what it holds.
    assert_eq!(pour(&mut w, a), None);
    assert_eq!(
        w.players[0].inv[HAND].item, SKIN_EMPTY,
        "the skin did not empty"
    );
    assert_eq!(plant_of(&w, a).water, w.cook.farm.ground_cap_wu);
    assert_eq!(pour(&mut w, a), Some(REFUSE_D_PLANT_WATER), "a full plant");
    assert_eq!(
        w.players[0].inv[HAND].item, SKIN_FULL,
        "a refused pour spent the skin"
    );
    idle(&mut w, 60);
    let p = plant_of(&w, a);
    assert!(p.grown > 0 && p.flags & FL_DRY == 0, "a watered plant held");

    // The rain.
    w.cook.farm.ground_rain_wu_s = 3 * 30;
    w.tick(&[Command::AdminEnv {
        weather: STORM,
        time_pm: KEEP_TIME,
    }]);
    idle(&mut w, FORCE_FADE_TICKS + 60);
    assert!(
        plant_of(&w, b).water > 0,
        "a storm did not water an open plant"
    );
    idle(&mut w, 60);
    assert!(plant_of(&w, b).grown > 0, "the rain-fed plant held");
}

/// **A ground plant never crossbreeds**, however close a neighbour of its
/// crop stands.
#[test]
fn a_ground_plant_beside_its_own_crop_never_crossbreeds() {
    use sim_core::farm::{FL_CROSSED, STAGE_MATURE};
    let (mut w, _, _) = farm_world();
    quick(&mut w);
    let (x, z) = meadow(&w);
    stand_by(&mut w, x + 0.45, z);
    let mine = g(b"GGGGGG");
    let a = sow_clone(&mut w, x, z, mine);
    let b = sow_clone(&mut w, x + 0.9, z, g(b"WWWWWW"));
    for id in [a, b] {
        assert_eq!(pour(&mut w, id), None);
    }
    grow_to(&mut w, a, STAGE_MATURE, 4_000);
    let p = plant_of(&w, a);
    assert_eq!(p.genes, mine, "a ground plant took a neighbour's genes");
    assert_eq!(p.flags & FL_CROSSED, 0);
}

/// **A save keeps a ground plant**, water and all, to the same hash.
#[test]
fn a_save_round_trips_a_ground_plant() {
    let (mut w, _, _) = farm_world();
    let (x, z) = meadow(&w);
    stand_by(&mut w, x, z);
    let id = sow_clone(&mut w, x, z, g(b"GYHXWX"));
    assert_eq!(pour(&mut w, id), None);
    idle(&mut w, 40);
    w.tick(&[Command::Leave { id: PLAYER }]);
    let mut buf = vec![0u8; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut buf).expect("encodes");
    let (mut back, _, _) = farm_world();
    back.load(&buf[..n]).expect("its own bytes load");
    assert_eq!(back.farm.entries(), w.farm.entries());
    assert_eq!(back.farm.entries()[0].host, HOST_GROUND);
    assert!(back.farm.entries()[0].water > 0);
    assert_eq!(back.state_hash(), w.state_hash());
}

// ---------------------------------------------------------------- catcher

/// Place a water catcher at `(cx, cz)` + `pose` on `level`, standing the
/// player on the spot first. Returns its box key, or `None` if it did not
/// place.
fn place_catcher(
    w: &mut World,
    cx: u16,
    cz: u16,
    level: u8,
    pose: sim_core::footprint::Pose,
) -> Option<u32> {
    let row = w.deploy.def_count - 2;
    assert_eq!(w.deploy.defs[row as usize].arch, ARCH_CATCHER);
    w.players[0].inv[PLANTER_SLOT] = ItemStack {
        item: CATCHER,
        count: 1,
        cond: 0,
        skin: 0,
    };
    w.players[0].frame.sel = PLANTER_SLOT as u8;
    let (x, z) = sim_core::footprint::centre(cx, cz, pose);
    let qy = w.players[0].body.qy;
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z + 1.0);
    w.players[0].body.qy = w.players[0].body.qy.max(qy);
    let before = w.deploys.len();
    w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row,
        cx,
        cz,
        level,
        loc: LOC_PLANE,
        pose,
    }]);
    (w.deploys.len() == before + 1).then(|| box_key(cx, cz, level, 0))
}

fn set_tank(w: &mut World, host: u32, water: u32) {
    let i = w.deploys.box_index(host).expect("a tank is a container");
    w.deploys.set_box_water(i, water);
    assert_eq!(tank(w, host), water, "the box has no tank");
}

fn slot0(w: &World, host: u32) -> ItemStack {
    let i = w.deploys.box_index(host).expect("a catcher is a container");
    w.deploys.box_slot(i, 0)
}

/// One sprinkle's pour into each target: 12 plant-seconds a second for the
/// 5 s between sprinkles, at 30 ticks a second.
const SHARE: u32 = 12 * 30 * 5;
/// Long enough for every catcher to have sprinkled once.
const SPRINKLE: u64 = sim_core::farm::CATCHER_SPRINKLE_PERIODS * OVEN_PERIOD_TICKS + 1;

/// **A catcher banks rain only under open sky**, and stops at its cap.
#[test]
fn a_catcher_banks_rain_under_open_sky_up_to_its_cap() {
    use sim_core::footprint::Pose;
    use sim_core::weather::{FORCE_FADE_TICKS, KEEP_TIME, STORM};
    let (mut w, cx, cz) = farm_world();
    w.cook.farm.catcher_rain_wu_s = 20 * 30;
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
    let roofed = place_catcher(&mut w, cx, cz, 0, Pose::CENTRE).expect("the roofed catcher");
    w.players[0].body.qy = 0;
    let open = place_catcher(&mut w, cx + 1, cz, 0, Pose::CENTRE).expect("the open catcher");
    w.tick(&[Command::AdminEnv {
        weather: STORM,
        time_pm: KEEP_TIME,
    }]);
    idle(&mut w, FORCE_FADE_TICKS + 60);
    assert!(tank(&w, open) > 0, "a storm did not fill an open catcher");
    assert_eq!(tank(&w, roofed), 0, "a storm filled a roofed catcher");

    // Nearly full, it tops out at its cap and holds there.
    let cap = w.cook.farm.catcher_cap_wu;
    set_tank(&mut w, open, cap - 10);
    idle(&mut w, 4 * OVEN_PERIOD_TICKS);
    assert_eq!(tank(&w, open), cap, "the barrel ran past its cap");
}

/// **A catcher fills an empty skin once it holds a skin's worth**, not a
/// drop before, and the fill costs exactly what pouring a skin pays.
#[test]
fn a_catcher_fills_an_empty_skin_for_exactly_a_skins_worth() {
    use sim_core::footprint::Pose;
    let (mut w, cx, cz) = farm_world();
    let c = place_catcher(&mut w, cx, cz, 0, Pose::CENTRE).expect("the catcher");
    let skin = w.cook.farm.skin_wu;
    let i = w.deploys.box_index(c).unwrap();
    w.deploys.set_box_slot(
        i,
        0,
        ItemStack {
            item: SKIN_EMPTY,
            count: 1,
            cond: 0,
            skin: 0,
        },
    );
    set_tank(&mut w, c, skin - 1);
    idle(&mut w, 2 * OVEN_PERIOD_TICKS);
    assert_eq!(slot0(&w, c).item, SKIN_EMPTY, "filled short of a skin");
    assert_eq!(
        tank(&w, c),
        skin - 1,
        "the barrel moved with no skin filled"
    );

    set_tank(&mut w, c, skin + 500);
    idle(&mut w, 2 * OVEN_PERIOD_TICKS);
    assert_eq!(
        slot0(&w, c).item,
        SKIN_FULL,
        "a skin's worth filled nothing"
    );
    assert_eq!(slot0(&w, c).count, 1);
    assert_eq!(tank(&w, c), 500, "a fill cost other than one skin");
    // And a full skin is left alone.
    idle(&mut w, 2 * OVEN_PERIOD_TICKS);
    assert_eq!(tank(&w, c), 500);
}

/// Water from a full catcher at `(cx, cz)` reaches a planter `dx_q` pose
/// steps west of the next cell's centre (3 m less `dx_q` × 11.7 mm away),
/// whose tank starts at `start`: what that planter holds a sprinkle later,
/// and what the catcher lost.
fn sprinkle_into_planter(dx_q: i8, start: u32) -> (u32, u32, u32) {
    use sim_core::footprint::Pose;
    let (mut w, cx, cz) = farm_world();
    let p = {
        let (x, z) = sim_core::footprint::centre(cx + 1, cz, Pose::CENTRE);
        w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z);
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
            cx: cx + 1,
            cz,
            level: 0,
            loc: LOC_PLANE,
            pose: Pose {
                ox: -dx_q,
                oz: 0,
                yaw: 0,
            },
        }]);
        assert_eq!(w.deploys.len(), before + 1, "the planter did not place");
        box_key(cx + 1, cz, 0, 0)
    };
    let c = place_catcher(&mut w, cx, cz, 0, Pose::CENTRE).expect("the catcher");
    let full = w.cook.farm.catcher_cap_wu;
    set_tank(&mut w, c, full);
    set_tank(&mut w, p, start);
    idle(&mut w, SPRINKLE);
    (tank(&w, p), full - tank(&w, c), w.cook.farm.planter_cap_wu)
}

/// **A catcher waters a planter 2.9 m away and not one 3.1 m away**, a
/// sprinkle's share at a time.
#[test]
fn a_catcher_waters_a_planter_within_three_metres_only() {
    // 9 pose steps is 0.105 m: 2.895 m and 3.105 m.
    let (near, spent, _) = sprinkle_into_planter(9, 0);
    assert_eq!(near, SHARE, "a planter 2.9 m away took {near}, not a share");
    assert_eq!(spent, SHARE, "the catcher paid other than what it poured");
    let (far, spent, _) = sprinkle_into_planter(-9, 0);
    assert_eq!(far, 0, "a planter 3.1 m away was watered");
    assert_eq!(spent, 0);
}

/// **A catcher never fills a planter past its cap**: it pours only the room.
#[test]
fn a_catcher_never_overfills_a_planter() {
    let cap = FarmContent::probe_fixture().planter_cap_wu;
    let (got, spent, cap2) = sprinkle_into_planter(9, cap - 100);
    assert_eq!(cap, cap2);
    assert_eq!(got, cap, "the planter ran past its cap");
    assert_eq!(spent, 100, "the catcher paid for water that did not fit");
}

/// **A catcher feeds a dry ground plant in its reach.**
#[test]
fn a_catcher_feeds_a_dry_ground_plant_in_range() {
    let (mut w, _, _) = farm_world();
    let (x, z) = meadow(&w);
    stand_by(&mut w, x, z);
    let id = sow_clone(&mut w, x, z, g(b"XXXXXX"));
    assert_eq!(plant_of(&w, id).water, 0);
    // A catcher a couple of metres off, wherever the ground takes one.
    let c = [
        (1.6f32, 0.0f32),
        (-1.6, 0.0),
        (0.0, 1.8),
        (0.0, -1.8),
        (1.4, 1.4),
    ]
    .iter()
    .find_map(|&(dx, dz)| {
        let (cx, cz, ox, oz) = sim_core::footprint::quantize(x + dx, z + dz)?;
        let pose = sim_core::footprint::Pose { ox, oz, yaw: 0 };
        place_catcher(&mut w, cx, cz, 0, pose)
    })
    .expect("no catcher placed beside the plant");
    assert!(w.farm.index_of_id(id).is_some(), "the catcher uprooted it");
    let full = w.cook.farm.catcher_cap_wu;
    set_tank(&mut w, c, full);
    idle(&mut w, SPRINKLE);
    let p = plant_of(&w, id);
    assert!(p.water > 0, "a dry ground plant in reach was not fed");
    assert!(p.water <= SHARE);
    assert_eq!(
        full - tank(&w, c),
        SHARE,
        "the catcher paid other than a share"
    );
}

/// **A save keeps a catcher's barrel** to the same hash (the decoder takes
/// water on a catcher as it does on a planter).
#[test]
fn a_save_round_trips_a_catchers_barrel() {
    let (mut w, cx, cz) = farm_world();
    let c =
        place_catcher(&mut w, cx, cz, 0, sim_core::footprint::Pose::CENTRE).expect("the catcher");
    set_tank(&mut w, c, 4321);
    w.tick(&[Command::Leave { id: PLAYER }]);
    let mut buf = vec![0u8; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut buf).expect("encodes");
    let (mut back, _, _) = farm_world();
    back.load(&buf[..n]).expect("its own bytes load");
    assert_eq!(tank(&back, c), 4321);
    assert_eq!(back.state_hash(), w.state_hash());
}
