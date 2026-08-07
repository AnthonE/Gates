//! The backpack-on-the-wire gate (M1): `ShardCore` ↔ `ClientCore` through
//! real encoded bytes — a kill drops a bag both clients see, a late joiner
//! is handed the standing set by the sync walk, and the loot action moves
//! the loot into the killer's inventory and takes the bag off every
//! client's map. Deterministic, no sockets; asserts are structural and
//! exact (the deploy_wire shape).
//!
//! The composition this closes is the one the sim tests cannot: a bag
//! exists in the sim whether or not the wire says so, and a client can
//! agree with the world for reasons the *encoder* never earned. So every
//! claim here is made twice — once against the client mirror, once
//! against `seen`, the decoded bytes the server actually put on the lane.

use client_wasm::core::{ClientCore, APPLIED_BAGS};
use protocol::{ActionMsg, EventMsg, ItemCatalog};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::backpack::{BackpackContent, BAG_GONE_EMPTIED};
use sim_core::combat::CombatContent;
use sim_core::gather::{GatherContent, ItemStack, SWING_INTERVAL_TICKS};
use sim_core::input::BTN_PRIMARY;
use sim_core::movement::{Body, POS_XZ_Q};

const SEED: u64 = 20_260_802;
/// The canonical dev spawn point, guarded walkable in sim-core
/// `world::tests`.
const SPAWN: (f32, f32) = (1024.0, 1024.0);
/// The gather fixture's item 0 — also the combat fixture's 34-damage
/// weapon, so three hits kill.
const SPEAR: u16 = 0;
/// A fixture item with no weapon row: pure loot.
const JUNK: u16 = 7;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn pump(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    seen: &mut Vec<(usize, EventMsg)>,
) -> [u32; 4] {
    let mut buf = [0u8; 1100];
    for (slot, c) in clients.iter_mut() {
        c.advance(1000.0 / 30.0);
        c.predict.decay_error();
        let n = c.poll_input(&mut buf);
        if n > 0 {
            let dg = protocol::decode_input(&buf[..n]).expect("client encodes valid input");
            core.push_input(*slot, &dg);
        }
    }
    let mut snaps: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
    core.tick(stats, |lane, slot, bytes| {
        match lane {
            Lane::Snapshot => snaps.push((slot, bytes.to_vec())),
            Lane::Event => events.push((slot, bytes.to_vec())),
        }
        true
    });
    let mut flags = [0u32; 4];
    for (slot, bytes) in events {
        seen.push((
            slot,
            protocol::decode_event(&bytes).expect("server events decode"),
        ));
        if let Some(c) = clients.iter_mut().find(|(s, _)| *s == slot).map(|(_, c)| c) {
            flags[slot] |= c.on_stream(&bytes).expect("server events decode");
        }
    }
    for (slot, bytes) in snaps {
        if let Some(c) = clients.iter_mut().find(|(s, _)| *s == slot).map(|(_, c)| c) {
            c.on_datagram(&bytes);
        }
    }
    flags
}

fn world_slot(core: &ShardCore, id: u32) -> usize {
    core.world
        .players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("player in world")
}

fn armed_core() -> ShardCore {
    let mut core = ShardCore::new(SEED);
    core.world.gather = GatherContent::probe_fixture();
    core.world.combat = CombatContent::probe_fixture();
    core.world.backpack = BackpackContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    core
}

/// Hold slot 0's swing on slot 1 until the kill lands, standing them
/// coincident every tick so the aim cone has no bearing to fail on
/// (point-blank, the same arrangement `test_alloc_zero` uses). Returns
/// the events the wire carried while it happened.
fn fight_to_a_kill(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
) -> Vec<(usize, EventMsg)> {
    let w1 = world_slot(core, id_of(1));
    let deaths_before = core.world.players[w1].deaths;
    let mut seen = Vec::new();
    clients[0].1.set_input(BTN_PRIMARY, 0, 128, 0, 0, 0);
    clients[1].1.set_input(0, 0, 128, 0, 0, 0);
    for _ in 0..(SWING_INTERVAL_TICKS * 8) {
        // Stand the victim inside the attacker every tick: point-blank
        // has no bearing to test, so the aim cone cannot make the fight
        // flaky (`test_alloc_zero`'s arrangement).
        let (w0, w1) = (world_slot(core, id_of(0)), world_slot(core, id_of(1)));
        core.world.players[w1].body = core.world.players[w0].body;
        pump(core, stats, clients, &mut seen);
        let w1 = world_slot(core, id_of(1));
        if core.world.players[w1].deaths > deaths_before {
            clients[0].1.set_input(0, 0, 128, 0, 0, 0);
            return seen;
        }
    }
    panic!("three fixture spear hits must kill inside eight swing intervals");
}

#[test]
fn a_kill_puts_a_bag_on_every_client_and_the_loot_takes_it_off() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    // Let the join drips settle so nothing below is racing a catalog.
    let mut warm = Vec::new();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut warm);
    }
    for (_, c) in &clients {
        assert!(c.bags.is_empty(), "nobody has died yet");
    }

    // Arm both, and give the victim something worth taking.
    let (w0, w1) = (world_slot(&core, id_of(0)), world_slot(&core, id_of(1)));
    core.world.players[w0].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
    };
    core.world.players[w1].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
    };
    core.world.players[w1].inv[1] = ItemStack {
        item: JUNK,
        count: 42,
    };

    let seen = fight_to_a_kill(&mut core, &stats, &mut clients);
    assert_eq!(core.world.backpacks.len(), 1, "the death dropped one bag");
    let bag = core.world.backpacks.entries()[0];

    // The wire itself said so — to both clients, at the position the sim
    // holds, and not merely "a client that agrees".
    let dropped: Vec<usize> = seen
        .iter()
        .filter_map(|(slot, m)| match m {
            EventMsg::BagDropped { id, qx, qy, qz } if *id == bag.id => {
                assert_eq!(
                    (*qx, *qy, *qz),
                    (bag.qx, bag.qy, bag.qz),
                    "bag moved on the wire"
                );
                Some(*slot)
            }
            _ => None,
        })
        .collect();
    assert_eq!(dropped, vec![0, 1], "a bag is a broadcast, like a death");
    assert!(
        seen.iter()
            .any(|(_, m)| matches!(m, EventMsg::Death { .. })),
        "the death rides the same lane"
    );
    for (_, c) in &clients {
        assert_eq!(c.bags.len(), 1, "both clients hold the bag");
        let mirrored = c.bags.entries()[0];
        assert_eq!(mirrored.id, bag.id);
        assert_eq!(
            (mirrored.qx, mirrored.qy, mirrored.qz),
            (bag.qx, bag.qy, bag.qz)
        );
    }

    // The killer reaches for it. They never moved, so it is at their feet.
    assert!(core.wants_action(0), "hand should be open");
    core.push_action(0, ActionMsg::Loot);
    let mut seen = Vec::new();
    pump(&mut core, &stats, &mut clients, &mut seen);

    assert!(core.world.backpacks.is_empty(), "an emptied bag leaves");
    let w0 = world_slot(&core, id_of(0));
    let held: Vec<(u16, u16)> = core.world.players[w0]
        .inv
        .iter()
        .filter(|s| s.count > 0)
        .map(|s| (s.item, s.count))
        .collect();
    assert_eq!(
        held,
        vec![(SPEAR, 2), (JUNK, 42)],
        "the kill paid: the victim's kit is in the killer's pockets"
    );
    let removed: Vec<usize> = seen
        .iter()
        .filter_map(|(slot, m)| match m {
            EventMsg::BagRemoved { id, why } if *id == bag.id => {
                assert_eq!(*why as u32, BAG_GONE_EMPTIED, "the reason is on the wire");
                Some(*slot)
            }
            _ => None,
        })
        .collect();
    assert_eq!(removed, vec![0, 1], "the removal is a broadcast too");
    for (_, c) in &clients {
        assert!(c.bags.is_empty(), "and no client is still drawing it");
    }
}

#[test]
fn a_late_joiner_is_handed_the_standing_bags_by_the_sync_walk() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    let mut warm = Vec::new();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut warm);
    }
    let (w0, w1) = (world_slot(&core, id_of(0)), world_slot(&core, id_of(1)));
    core.world.players[w0].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
    };
    core.world.players[w1].inv[0] = ItemStack {
        item: JUNK,
        count: 9,
    };
    let seen = fight_to_a_kill(&mut core, &stats, &mut clients);
    assert!(
        seen.iter()
            .any(|(_, m)| matches!(m, EventMsg::Death { .. })),
        "somebody died"
    );
    assert_eq!(core.world.backpacks.len(), 1);
    let bag = core.world.backpacks.entries()[0];

    // A third client arrives after the fact and hears nothing about it —
    // the broadcast is long gone. The walk is what must repair them.
    assert!(core.connect(2, id_of(2)));
    clients.push((2usize, ClientCore::new(SEED, id_of(2), 0)));
    let mut late = 0u32;
    let mut seen = Vec::new();
    for _ in 0..8 {
        late |= pump(&mut core, &stats, &mut clients, &mut seen)[2];
    }
    assert_ne!(late & APPLIED_BAGS, 0, "the bag walk never reached slot 2");
    assert!(
        seen.iter()
            .any(|(slot, m)| *slot == 2
                && matches!(m, EventMsg::BagSync { count, .. } if *count > 0)),
        "and it arrived as a sync batch, not as a replayed broadcast"
    );
    let c2 = &clients.iter().find(|(s, _)| *s == 2).unwrap().1;
    assert_eq!(c2.bags.len(), 1, "the late joiner sees the standing bag");
    assert_eq!(c2.bags.entries()[0].id, bag.id);
}

#[test]
fn a_bag_out_of_reach_is_refused_by_distance_alone() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    let mut warm = Vec::new();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut warm);
    }
    let (w0, w1) = (world_slot(&core, id_of(0)), world_slot(&core, id_of(1)));
    core.world.players[w0].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
    };
    core.world.players[w1].inv[1] = ItemStack {
        item: JUNK,
        count: 42,
    };
    fight_to_a_kill(&mut core, &stats, &mut clients);
    let bag = core.world.backpacks.entries()[0];

    // Walk the killer 20 m off and press loot. The action lane carries no
    // target, so there is nothing here to forge — distance is the whole
    // check, and it must hold.
    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].body = Body::at(
        SEED,
        bag.qx as f32 * POS_XZ_Q + 20.0,
        bag.qz as f32 * POS_XZ_Q,
    );
    core.push_action(0, ActionMsg::Loot);
    let mut seen = Vec::new();
    pump(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(core.world.backpacks.len(), 1, "the bag is untouched");
    let w0 = world_slot(&core, id_of(0));
    assert!(
        core.world.players[w0].inv.iter().all(|s| s.item != JUNK),
        "and nothing crossed twenty metres into a pocket"
    );
    assert!(
        !seen
            .iter()
            .any(|(_, m)| matches!(m, EventMsg::BagRemoved { .. })),
        "nor did the wire announce a removal that never happened"
    );
}
