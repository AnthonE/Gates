//! The war effort's convoys (`convoy.rs`, `ARC.md` §5 M2): a column sets
//! out on the arc clock's timetable while its work takes deposits, walks a
//! town road from the ring end to the town's gate, and its load goes on the
//! quota; a carrier killed on the way leaves its load in its carcass, and a
//! column still out when the next is due is lost.
//!
//! Numbers come from `MobContent::probe_fixture` (the carrier carries 3 of
//! item 0 and 1 of item 1) and `WorksContent::probe_fixture`'s one work,
//! turned into an act-III work at the town with a quota no convoy fills.

use sim_core::backpack::BackpackContent;
use sim_core::convoy;
use sim_core::limits::{MAX_CONVOY, TICK_HZ};
use sim_core::mob::{self, MobContent, CARRIER_SLOT0, MOB_CARRIER};
use sim_core::movement::POS_XZ_Q;
use sim_core::terrain::SIDE_ROAD_POINTS;
use sim_core::works::{
    WorksContent, CONVOY_IN, CONVOY_OUT, WORK_COUNTDOWN, WORK_EV_CONVOY, WORK_OPEN,
};
use sim_core::world::{World, EV_WORK};

/// Ticks between columns: twenty minutes, longer than any town road takes
/// a carrier to walk.
const EVERY: u64 = 20 * 60 * TICK_HZ as u64;

/// A seed whose island has a town and a road to it.
fn seed() -> u64 {
    (1..64u64)
        .find(|&s| convoy::road_of(s, &sim_core::terrain::haven(s), 1).is_some())
        .expect("no seed in 1..64 has a town road")
}

/// A world whose one work takes deposits at the town and is fed by a
/// column of three every [`EVERY`] ticks.
fn armed(seed: u64) -> World {
    let mut w = World::new(seed);
    w.mob = MobContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    let mut wc = WorksContent::probe_fixture();
    wc.defs[0].act = 3;
    wc.defs[0].fallback_at = 100 * EVERY;
    wc.defs[0].inputs[0].need = 1_000;
    wc.defs[0].inputs[1].need = 1_000;
    wc.convoy_work = 0;
    wc.convoy_carriers = 3;
    wc.convoy_every = EVERY;
    w.works_def = wc;
    w.works.w[0].state = WORK_OPEN;
    w.works.refresh(&w.works_def);
    w
}

fn carriers(w: &World) -> usize {
    w.mobs.m[CARRIER_SLOT0..CARRIER_SLOT0 + MAX_CONVOY]
        .iter()
        .filter(|m| m.alive)
        .count()
}

fn convoy_events(w: &World, c: u32) -> usize {
    w.events
        .entries()
        .iter()
        .filter(|e| e.code == EV_WORK && e.b == WORK_EV_CONVOY && e.c == c)
        .count()
}

/// Jump the clock to column `n`'s hour and run that tick.
fn to_departure(w: &mut World, n: u64) {
    w.tick = n * EVERY;
    w.tick(&[]);
}

/// Walk column `n` in from its road's ring end until none is left on the
/// road (or the next column's hour), checking each carrier keeps to the
/// road; returns the road, the seconds it took and how often the last in
/// was heard.
fn walk_in(w: &mut World, seed: u64, n: u64) -> (sim_core::terrain::SideRoad, u64, usize) {
    to_departure(w, n);
    assert_eq!(carriers(w), 3, "seed {seed}: column {n} did not set out");
    assert_eq!(convoy_events(w, CONVOY_OUT), 1);
    let road = w.haven.roads[convoy::road_of(seed, &w.haven, n).unwrap()];
    for i in 0..3 {
        let m = &w.mobs.m[CARRIER_SLOT0 + i];
        let (x, z) = (m.body.qx as f32 * POS_XZ_Q, m.body.qz as f32 * POS_XZ_Q);
        let (sx, sz) = convoy::stand_at(&road, i);
        assert!((x - sx) * (x - sx) + (z - sz) * (z - sz) < 0.25);
        assert!(!m.homed, "a carrier is never enrolled");
    }
    let mut came_in = 0;
    let mut worst = 0.0f32;
    let start = w.tick;
    while carriers(w) > 0 && w.tick < start + EVERY - 2 {
        w.tick(&[]);
        came_in += convoy_events(w, CONVOY_IN);
        for m in w.mobs.m[CARRIER_SLOT0..CARRIER_SLOT0 + 3]
            .iter()
            .filter(|m| m.alive)
        {
            let (x, z) = (m.body.qx as f32 * POS_XZ_Q, m.body.qz as f32 * POS_XZ_Q);
            worst = worst.max(road.dist2(x, z).sqrt());
        }
    }
    let left: Vec<_> = w.mobs.m[CARRIER_SLOT0..CARRIER_SLOT0 + 3]
        .iter()
        .filter(|m| m.alive)
        .map(|m| {
            let (x, z) = (m.body.qx as f32 * POS_XZ_Q, m.body.qz as f32 * POS_XZ_Q);
            (m.leg, (x, z))
        })
        .collect();
    assert!(
        left.is_empty(),
        "seed {seed}: carriers still on the road after {} s, (leg, at): {left:?}",
        (w.tick - start) / TICK_HZ as u64
    );
    assert!(
        worst < 4.0 * convoy::GAP_M,
        "seed {seed}: a carrier wandered {worst:.1} m off its road"
    );
    (road, (w.tick - start) / TICK_HZ as u64, came_in)
}

/// **On its hour, along its road, onto the quota.** Nothing stands up
/// before the first departure; on it the column stands at the road's ring
/// end and the island hears it set out; each carrier walks the road (never
/// more than a few metres off it) to the town's gate, and the three loads
/// go on the quota with the island hearing the last come in.
#[test]
fn a_column_sets_out_on_its_hour_and_walks_the_town_road_onto_the_quota() {
    let seed = seed();
    let mut w = armed(seed);
    assert_eq!(mob::kind_of(CARRIER_SLOT0), MOB_CARRIER);
    assert_eq!(mob::capacity(MOB_CARRIER), MAX_CONVOY);
    for _ in 0..600 {
        w.tick(&[]);
        assert_eq!(carriers(&w), 0, "a carrier before its column's hour");
    }
    let (road, took, came_in) = walk_in(&mut w, seed, 1);
    assert_eq!(&w.works.w[0].got[..2], &[9, 3], "three loads on the quota");
    assert_eq!(came_in, 1, "the last in is heard once");
    let len = road.path_len();
    assert!(
        took as f32 > len / 3.0,
        "a {len:.0} m road walked in {took} s"
    );
}

/// **Every town road gets walked.** On a handful of seeds, both of the
/// town's roads (whichever columns draw them) bring a whole column in.
#[test]
fn both_town_roads_bring_a_column_in_on_a_handful_of_seeds() {
    let seeds: Vec<u64> = (1..64u64)
        .filter(|&s| convoy::road_of(s, &sim_core::terrain::haven(s), 1).is_some())
        .take(4)
        .collect();
    assert_eq!(seeds.len(), 4);
    for seed in seeds {
        let h = sim_core::terrain::haven(seed);
        let mut walked = Vec::new();
        for n in 1..16u64 {
            let r = convoy::road_of(seed, &h, n).unwrap();
            if walked.contains(&r) {
                continue;
            }
            let mut w = armed(seed);
            walk_in(&mut w, seed, n);
            walked.push(r);
        }
        let live = h.roads[sim_core::terrain::DEPOT_ROADS..]
            .iter()
            .filter(|r| r.live)
            .count();
        assert_eq!(walked.len(), live, "seed {seed}: a road no column drew");
    }
}

/// **A raid takes the load.** One carrier killed on the road leaves a
/// carcass holding exactly its load, and only the other two loads reach
/// the quota.
#[test]
fn a_carrier_killed_on_the_road_leaves_its_load_in_its_carcass() {
    let mut w = armed(seed());
    to_departure(&mut w, 1);
    for _ in 0..90 {
        w.tick(&[]);
    }
    let slot = CARRIER_SLOT0 + 1;
    let hp = w.mobs.m[slot].hp;
    let mc = w.mob_now();
    assert!(mob::hurt_slot(
        &w.backpack,
        &mc,
        w.tick,
        None,
        &w.players,
        &mut w.mobs,
        &mut w.backpacks,
        &mut w.events,
        slot,
        hp,
    ));
    assert!(!w.mobs.m[slot].alive);
    let bag = w
        .backpacks
        .entries()
        .iter()
        .find(|b| b.owner == mob::mob_id(slot))
        .expect("no carcass");
    let held = |item: u16| {
        bag.items
            .iter()
            .filter(|s| s.item == item)
            .map(|s| s.count as u32)
            .sum::<u32>()
    };
    assert_eq!((held(0), held(1)), (3, 1), "the carcass is the load");
    let start = w.tick;
    while carriers(&w) > 0 && w.tick < start + EVERY - 200 {
        w.tick(&[]);
    }
    assert_eq!(carriers(&w), 0);
    assert_eq!(&w.works.w[0].got[..2], &[6, 2], "two loads came in");
}

/// **Only while it takes deposits, and one column at a time.** A work
/// counting down sends nobody; a column still out when the next is due is
/// gone, and the next stands up fresh at the ring end, its load never
/// counted.
#[test]
fn no_column_for_a_shut_work_and_a_late_one_is_lost() {
    let seed = seed();
    let mut w = armed(seed);
    w.works.w[0].state = WORK_COUNTDOWN;
    w.works.w[0].lit_at = 1_000 * EVERY;
    to_departure(&mut w, 1);
    assert_eq!(carriers(&w), 0, "a column for a work that takes nothing");
    assert_eq!(convoy_events(&w, CONVOY_OUT), 0);

    let mut w = armed(seed);
    w.works_def.convoy_every = 30 * TICK_HZ as u64;
    let every = w.works_def.convoy_every;
    w.tick = every;
    w.tick(&[]);
    assert_eq!(carriers(&w), 3);
    for _ in 0..every - 1 {
        w.tick(&[]);
    }
    assert_eq!(carriers(&w), 3, "thirty seconds is not the whole road");
    w.tick(&[]);
    assert_eq!(
        convoy_events(&w, CONVOY_OUT),
        1,
        "the second column set out"
    );
    let road = w.haven.roads[convoy::road_of(seed, &w.haven, 2).unwrap()];
    for i in 0..3 {
        let m = &w.mobs.m[CARRIER_SLOT0 + i];
        let (x, z) = (m.body.qx as f32 * POS_XZ_Q, m.body.qz as f32 * POS_XZ_Q);
        let (sx, sz) = convoy::stand_at(&road, i);
        assert!((x - sx) * (x - sx) + (z - sz) * (z - sz) < 0.25);
        assert_eq!(m.leg as usize, SIDE_ROAD_POINTS - 2);
    }
    assert_eq!(&w.works.w[0].got[..2], &[0, 0], "the lost column counted");
}
