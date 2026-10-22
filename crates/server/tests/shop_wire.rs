//! The compound shop on the wire (v118), through the shard and `ClientCore`
//! the way an agent sees it: opening a vending machine tells the opener its
//! offers (nobody else), a change re-tells whoever has it open, and every
//! press is answered to the presser.

use client_core::core::ClientCore;
use protocol::{EventMsg, ItemCatalog, ShopAct, ShopRow};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::deploy::{box_key, DeployContent, DeployDef, ARCH_VENDING, PLACE_ANY};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::inventory::CONT_BOX;
use sim_core::movement::Body;
use sim_core::shop::{REFUSE_SH_ALLOWED, REFUSE_SH_OFFER, SHOP_OP_BUY, SHOP_OP_SET};
use sim_core::world::Command;

const SEED: u64 = 20_260_804;
/// Probe-fixture items: the machine stacks to one, the other two to 100.
const MACHINE: u16 = 11;
const JUNK: u16 = 10;
const CORN: u16 = 8;

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

/// Feed an action in as real bytes, the way the action lane would.
fn act(core: &mut ShardCore, slot: usize, bytes: impl Fn(&mut [u8]) -> usize) {
    let mut buf = [0u8; 64];
    let n = bytes(&mut buf);
    core.push_action(
        slot,
        protocol::decode_action(&buf[..n]).expect("round trips"),
    );
}

fn shop(core: &mut ShardCore, slot: usize, key: u32, a: ShopAct) {
    act(core, slot, |buf| {
        protocol::encode_action_shop(key, a, buf).unwrap()
    });
}

fn views(seen: &[(usize, EventMsg)], slot: usize) -> Vec<protocol::ShopView> {
    seen.iter()
        .filter_map(|(s, m)| match m {
            EventMsg::Shop(v) if *s == slot => Some(*v),
            _ => None,
        })
        .collect()
}

#[test]
fn the_opener_is_told_the_offers_and_every_press_is_answered() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    let mut dc = DeployContent::probe_fixture();
    let row = dc.def_count;
    dc.defs[row as usize] = DeployDef {
        arch: ARCH_VENDING,
        placement: PLACE_ANY,
        hp: 500,
        item: MACHINE,
        ..DeployDef::INERT
    };
    dc.def_count += 1;
    dc.shop_coin = JUNK;
    core.world.deploy = dc;
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
    let (ox, oz) = cell_center(cx - 1, cz);
    let (sx, sz) = cell_center(cx + 1, cz);
    core.world.players[w0].body = Body::at(SEED, &core.world.haven, ox, oz);
    core.world.players[w1].body = Body::at(SEED, &core.world.haven, sx, sz);
    core.world.players[w0].inv[1] = ItemStack {
        item: MACHINE,
        count: 1,
        cond: 0,
        skin: 0,
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
    let i = core
        .world
        .deploys
        .vending_index(key)
        .expect("the machine stands");
    core.world.deploys.set_box_slot(
        i,
        0,
        ItemStack {
            item: CORN,
            count: 10,
            cond: 0,
            skin: 0,
        },
    );

    // 1 · the stranger opens it: told the machine, with no offers yet and no
    //     right to edit; the owner, who opened nothing, is told nothing.
    act(&mut core, 1, |buf| {
        protocol::encode_action_container(CONT_BOX, key, buf).unwrap()
    });
    let seen = pump_n(&mut core, &stats, &mut clients, 6);
    let v = views(&seen, 1);
    assert_eq!(v.len(), 1, "the opener is told the machine once: {v:?}");
    assert_eq!((v[0].key, v[0].coin, v[0].may_edit), (key, JUNK, false));
    assert!(v[0].rows.iter().all(|r| !r.live()));
    assert!(views(&seen, 0).is_empty(), "only the opener is told");
    assert!(clients[1].1.shop_offers(key).is_some());

    // 2 · the owner sets an offer: answered, and the viewer re-told.
    let set = ShopAct::Set {
        row: 0,
        item: CORN,
        lot: 2,
        price: 3,
    };
    shop(&mut core, 0, key, set);
    let seen = pump_n(&mut core, &stats, &mut clients, 6);
    let (n, r) = clients[0].1.shop_last_result();
    let r = r.expect("the owner's press is answered");
    assert_eq!(n, 1);
    assert_eq!((r.key, r.op, r.row, r.code), (key, SHOP_OP_SET, 0, 0));
    assert_eq!(views(&seen, 1).len(), 1, "the viewer is re-told once");
    let want = ShopRow {
        item: CORN,
        lot: 2,
        price: 3,
        have: 10,
    };
    assert_eq!(clients[1].1.shop_offers(key).unwrap().rows[0], want);
    assert_eq!(want.lots(), 5);

    // 3 · the stranger buys two lots: answered, and the stock re-told.
    core.world.players[w1].inv[0] = ItemStack {
        item: JUNK,
        count: 10,
        cond: 0,
        skin: 0,
    };
    let seen_buy = ShopAct::Buy {
        row: 0,
        times: 2,
        terms: want.terms(),
    };
    shop(&mut core, 1, key, seen_buy);
    pump_n(&mut core, &stats, &mut clients, 6);
    let (_, r) = clients[1].1.shop_last_result();
    let r = r.expect("the buy is answered");
    assert_eq!((r.op, r.row, r.times, r.code), (SHOP_OP_BUY, 0, 2, 0));
    assert_eq!(clients[1].1.shop_offers(key).unwrap().rows[0].have, 6);
    assert_eq!(
        sim_core::craft::inv_count(&core.world.players[w1].inv, CORN),
        4
    );

    // 3b · bait and switch: the owner reprices the row under the buyer's
    //      view (a lot of one at three times the price). The buy names the
    //      offer it saw, so it is refused and nothing moves.
    let switch = ShopAct::Set {
        row: 0,
        item: CORN,
        lot: 1,
        price: 9,
    };
    shop(&mut core, 0, key, switch);
    shop(&mut core, 1, key, seen_buy);
    pump_n(&mut core, &stats, &mut clients, 6);
    let (_, r) = clients[1].1.shop_last_result();
    assert_eq!(r.unwrap().code as u32, REFUSE_SH_OFFER);
    assert_eq!(
        sim_core::craft::inv_count(&core.world.players[w1].inv, JUNK),
        4,
        "the switched buy took no junk"
    );

    // 4 · the stranger may not set an offer, and is told why.
    shop(&mut core, 1, key, set);
    pump_n(&mut core, &stats, &mut clients, 6);
    let (_, r) = clients[1].1.shop_last_result();
    assert_eq!(r.unwrap().code as u32, REFUSE_SH_ALLOWED);

    // 5 · the owner opens it and is told it may edit.
    act(&mut core, 0, |buf| {
        protocol::encode_action_container(CONT_BOX, key, buf).unwrap()
    });
    pump_n(&mut core, &stats, &mut clients, 6);
    assert!(clients[0].1.shop_offers(key).unwrap().may_edit);
}
