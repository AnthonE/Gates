//! Farming v2 on the wire (v135), through the shard and `ClientCore`: a
//! seed planted by its action bytes reaches a near client as `SUB_PLANT`,
//! and a client that joins afterwards gets it from the plant walk
//! (`SUB_PLANT_SYNC`). Run with `RUST_MIN_STACK=16777216`.

use client_core::core::ClientCore;
use protocol::{EventMsg, ItemCatalog, PlantTarget};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::deploy::{box_key, DeployContent, DeployDef, ARCH_PLANTER, MATTER_WOOD, PLACE_ANY};
use sim_core::farm::{FarmContent, GONE_UPROOTED, STAGE_SEED};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::movement::Body;
use sim_core::oven::CookContent;
use sim_core::world::Command;

const SEED: u64 = 20_260_804;
const PLANTER: u16 = 20;
const SEED_ITEM: u16 = 21;
const CORN: u16 = 22;
const SEED_SLOT: u8 = 6;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn world_slot(core: &ShardCore, id: u32) -> usize {
    core.world
        .players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("player in world")
}

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

fn buildable_cell(core: &ShardCore) -> (u16, u16) {
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let (cx, cz) = ((512 + dx) as u16, (512 + dz) as u16);
                if (-1..=1).all(|k| {
                    let (x, z) = cell_center((cx as i32 + k) as u16, cz);
                    foundation_terrain_ok(SEED, &core.world.haven, x, z)
                }) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no flat run of cells near the middle");
}

/// One server tick, the event bytes decoded per slot and fed to the clients.
fn pump(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    seen: &mut Vec<(usize, EventMsg)>,
) {
    let mut buf = [0u8; 1100];
    for (slot, c) in clients.iter_mut() {
        c.advance(1000.0 / 30.0);
        let n = c.poll_input(&mut buf);
        if n > 0 {
            let dg = protocol::decode_input(&buf[..n]).expect("client encodes valid input");
            core.push_input(*slot, &dg);
        }
    }
    let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
    core.tick_bare(stats, |lane, slot, bytes| {
        if matches!(lane, Lane::Event) {
            events.push((slot, bytes.to_vec()));
        }
        true
    });
    for (slot, bytes) in events {
        seen.push((slot, protocol::decode_event(&bytes).expect("events decode")));
        if let Some((_, c)) = clients.iter_mut().find(|(s, _)| *s == slot) {
            c.on_stream(&bytes).expect("events decode");
        }
    }
}

fn pump_n(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    n: usize,
) -> Vec<(usize, EventMsg)> {
    let mut seen = Vec::new();
    for _ in 0..n {
        pump(core, stats, clients, &mut seen);
    }
    seen
}

#[test]
fn a_planted_seed_reaches_a_near_client_and_a_joiner() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    core.world.cook = CookContent::probe_fixture();
    for (item, cap) in [(PLANTER, 1), (SEED_ITEM, 50), (CORN, 50)] {
        core.world.gather.stack_max[item as usize] = cap;
    }
    core.world.gather.item_count = core.world.gather.item_count.max(CORN + 1);
    let mut dc = DeployContent::probe_fixture();
    let row = dc.def_count;
    dc.defs[row as usize] = DeployDef {
        arch: ARCH_PLANTER,
        placement: PLACE_ANY,
        hp: 100,
        matter: MATTER_WOOD,
        item: PLANTER,
        ..DeployDef::INERT
    };
    dc.def_count += 1;
    core.world.deploy = dc;
    let mut fc = FarmContent::probe_fixture();
    fc.crops[0].seed = SEED_ITEM;
    fc.crops[0].yield_item = CORN;
    core.world.cook.farm = fc;
    core.catalog = ItemCatalog::EMPTY;
    let (cx, cz) = buildable_cell(&core);
    core.world.dev_spawn = Some(cell_center(cx, cz));

    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    pump_n(&mut core, &stats, &mut clients, 8);

    let (w0, w1) = (world_slot(&core, id_of(0)), world_slot(&core, id_of(1)));
    let (x, z) = cell_center(cx, cz);
    let (nx, nz) = cell_center(cx + 1, cz);
    core.world.players[w1].body = Body::at(SEED, &core.world.haven, nx, nz);
    core.world.players[w0].inv[3] = ItemStack {
        item: PLANTER,
        count: 1,
        cond: 0,
        skin: 0,
        ..ItemStack::EMPTY
    };
    core.world.players[w0].inv[SEED_SLOT as usize] = ItemStack {
        item: SEED_ITEM,
        count: 50,
        cond: 0,
        skin: 0,
        ..ItemStack::EMPTY
    };
    core.world.tick(&[Command::PlaceDeploy {
        id: id_of(0),
        row,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        pose: sim_core::footprint::Pose::CENTRE,
    }]);
    let key = box_key(cx, cz, 0, LOC_PLANE);
    assert!(
        core.world.deploys.box_index(key).is_some(),
        "the planter stands"
    );
    let qy = core.world.players[w0].body.qy;
    core.world.players[w0].body = Body::at(SEED, &core.world.haven, x, z + 1.0);
    core.world.players[w0].body.qy = core.world.players[w0].body.qy.max(qy);
    pump_n(&mut core, &stats, &mut clients, 4);

    // 1 · the planter's action bytes plant a seed: both clients hear it.
    let mut buf = [0u8; 64];
    let n =
        protocol::encode_action_plant(SEED_SLOT, PlantTarget::Bed { host: key, spot: 4 }, &mut buf)
            .unwrap();
    core.push_action(0, protocol::decode_action(&buf[..n]).unwrap());
    let seen = pump_n(&mut core, &stats, &mut clients, 4);
    assert_eq!(core.world.farm.len(), 1, "the seed is in the soil");
    let id = core.world.farm.entries()[0].id;
    for slot in [0usize, 1] {
        assert!(
            seen.iter().any(|(s, m)| *s == slot
                && matches!(m, EventMsg::Plant(p) if p.id == id && p.stage == STAGE_SEED && p.spot == 4)),
            "client {slot} heard no SUB_PLANT: {seen:?}"
        );
        let p = clients[slot].1.plants.find(id).expect("mirrored");
        assert_eq!((p.spot, p.stage), (4, STAGE_SEED));
    }
    assert_eq!(core.world.players[w0].inv[SEED_SLOT as usize].count, 49);

    // 2 · a client joining afterwards gets it from the plant walk.
    assert!(core.connect(2, id_of(2)));
    clients.push((2usize, ClientCore::new(SEED, id_of(2), 0)));
    let seen = pump_n(&mut core, &stats, &mut clients, 60);
    assert!(
        seen.iter().any(|(s, m)| *s == 2
            && matches!(m, EventMsg::PlantSync { recs, count, .. }
                if recs[..*count as usize].iter().any(|p| p.id == id))),
        "the joiner's plant walk never carried the plant"
    );
    assert!(
        clients[2].1.plants.find(id).is_some(),
        "the joiner mirrors it"
    );
    // Its walk is counted as the plant walk's, not the deployables'.
    assert!(
        ShardStats::get(&stats.plant_walk_completes) >= 3,
        "three clients walked the farm to its end"
    );

    // 3 · the planter is broken: self-heal uproots its plant within a farm
    //   period, and the removal is told to every client.
    let di = (0..core.world.deploys.len())
        .find(|&i| core.world.deploys.entries()[i].cx == cx)
        .unwrap();
    let mut ev = sim_core::world::EventQueue::default();
    let w = &mut core.world;
    assert!(sim_core::deploy::damage_deploy(
        &w.deploy,
        &mut w.pieces,
        &mut w.deploys,
        di,
        9_999,
        &mut ev,
    ));
    let seen = pump_n(&mut core, &stats, &mut clients, 20);
    assert!(core.world.farm.is_empty());
    for (slot, c) in &clients {
        assert!(
            seen.iter().any(|(s, m)| s == slot
                && *m
                    == EventMsg::PlantGone {
                        id,
                        reason: GONE_UPROOTED
                    }),
            "client {slot} was not told"
        );
        assert!(c.plants.find(id).is_none(), "client {slot} still draws it");
    }
}
