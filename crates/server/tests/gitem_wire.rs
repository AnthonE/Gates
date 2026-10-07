//! The loose-stack lane, end to end through real bytes (ground items v0,
//! wire v65): a stack the sim scattered reaches both clients as a
//! `GItemSync` batch, a late joiner is repaired by the same walk, the
//! pickup action takes it, and **every client's set loses it** — which is
//! the half a sim test cannot see, because the store is silent on the
//! event lane and the walk is the only thing that can say a stack is gone.
//!
//! Every claim is made twice, `backpack_wire.rs`'s discipline: once
//! against the client mirror, once against `seen` — the decoded bytes the
//! server actually put on the lane. A client can agree with the world for
//! reasons the encoder never earned.

use client_core::core::{ClientCore, APPLIED2_GITEMS};
use protocol::{ActionMsg, EventMsg, ItemCatalog};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::backpack::BackpackContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::limits::INV_SLOTS;

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

const SEED: u64 = 20_260_802;
const SPAWN: (f32, f32) = (2048.0, 2048.0);
/// A fixture item with no weapon row: pure loot.
const FILLER: u16 = 7;

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
        let n = c.poll_input(&mut buf);
        if n > 0 {
            let dg = protocol::decode_input(&buf[..n]).expect("client encodes valid input");
            core.push_input(*slot, &dg);
        }
    }
    let mut snaps: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
    core.tick_bare(stats, |lane, slot, bytes| {
        match lane {
            Lane::Snapshot => snaps.push((slot, bytes.to_vec())),
            Lane::Event => events.push((slot, bytes.to_vec())),
        }
        true
    });
    let mut flags2 = [0u32; 4];
    for (slot, bytes) in events {
        seen.push((
            slot,
            protocol::decode_event(&bytes).expect("server events decode"),
        ));
        if let Some(c) = clients.iter_mut().find(|(s, _)| *s == slot).map(|(_, c)| c) {
            c.on_stream(&bytes).expect("server events decode");
            flags2[slot] |= c.applied2();
        }
    }
    for (slot, bytes) in snaps {
        if let Some(c) = clients.iter_mut().find(|(s, _)| *s == slot).map(|(_, c)| c) {
            c.on_datagram(&bytes);
        }
    }
    flags2
}

fn world_slot(core: &ShardCore, id: u32) -> usize {
    core.world
        .players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("player in world")
}

fn armed_core() -> Box<ShardCore> {
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.backpack = BackpackContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    core
}

/// Put one loose stack at the feet of the player in `slot`, the way a
/// barrel would — through the store's own verb, so the fixture cannot
/// scatter something the sim would refuse.
fn drop_one(core: &mut ShardCore, wslot: usize, item: u16, count: u16) -> u32 {
    let (qx, qz) = (
        core.world.players[wslot].body.qx,
        core.world.players[wslot].body.qz,
    );
    let mut items = [ItemStack::default(); INV_SLOTS];
    items[0] = ItemStack {
        item,
        count,
        cond: 0,
        skin: 0,
    };
    let (bc, seed, tick) = (core.world.backpack, core.world.seed, core.world.tick);
    let made = core
        .world
        .ground_items
        .scatter(&bc, seed, hv(seed), 0xB0_0B, qx, qz, &items, tick);
    assert_eq!(made, 1, "the fixture scattered nothing");
    core.world.ground_items.entries()[0].id
}

#[test]
fn a_scattered_stack_reaches_every_client_and_the_take_removes_it() {
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
    let w0 = world_slot(&core, id_of(0));
    let id = drop_one(&mut core, w0, FILLER, 9);

    let mut seen = Vec::new();
    let mut flags = [0u32; 4];
    for _ in 0..6 {
        let f = pump(&mut core, &stats, &mut clients, &mut seen);
        for (i, v) in f.iter().enumerate() {
            flags[i] |= v;
        }
    }

    // Both clients, because a loose stack is a world fact like a bag.
    for slot in [0usize, 1] {
        assert_ne!(
            flags[slot] & APPLIED2_GITEMS,
            0,
            "slot {slot} never heard about the stack"
        );
        let c = &clients.iter().find(|(s, _)| *s == slot).unwrap().1;
        assert_eq!(c.ground_items().len(), 1, "slot {slot} draws one stack");
        let g = c.ground_items()[0];
        assert_eq!(g.id, id, "slot {slot} has the wrong stack");
        assert_eq!(
            (g.item, g.count),
            (FILLER, 9),
            "slot {slot} does not know WHAT it is — the two fields a bag \
             does not carry are the whole point of this record"
        );
    }
    assert!(
        seen.iter().any(|(_, m)| matches!(
            m,
            EventMsg::GItemSync { count, recs, .. }
                if *count == 1 && recs[0].id == id && recs[0].count == 9
        )),
        "the bytes the server sent carry the stack, its id and its count"
    );

    // The take: payload-free, nearest-in-reach, and it is the arrow
    // recovery opcode (`Command::Pickup`) rather than a new one.
    let mut buf = [0u8; 64];
    let n = protocol::encode_action_pickup(&mut buf).expect("the pickup encodes");
    let act = protocol::decode_action(&buf[..n]).expect("and decodes");
    assert!(matches!(act, ActionMsg::Pickup));
    core.push_action(0, act);

    let mut after = Vec::new();
    for _ in 0..8 {
        pump(&mut core, &stats, &mut clients, &mut after);
    }

    assert!(
        core.world.ground_items.is_empty(),
        "the sim still has the stack"
    );
    let got: u16 = core.world.players[w0]
        .inv
        .iter()
        .filter(|s| s.item == FILLER)
        .map(|s| s.count)
        .sum();
    assert_eq!(got, 9, "the taker did not get it");

    // **And every client's set lost it**, which is what the walk exists
    // for: the store says nothing on the event lane when a stack leaves,
    // so a client that only ever heard inserts would draw a picture of a
    // stack somebody else is carrying.
    for slot in [0usize, 1] {
        let c = &clients.iter().find(|(s, _)| *s == slot).unwrap().1;
        assert!(
            c.ground_items().is_empty(),
            "slot {slot} still draws a stack that has been taken"
        );
    }
    assert!(
        after.iter().any(|(_, m)| matches!(
            m,
            EventMsg::GItemSync {
                reset: true,
                count: 0,
                ..
            }
        )),
        "the server never sent the empty reset that says the ground is clear"
    );
}

#[test]
fn a_late_joiner_is_handed_the_loose_stacks_by_the_walk() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    assert!(core.connect(0, id_of(0)));
    let mut clients = vec![(0usize, ClientCore::new(SEED, id_of(0), 0))];
    let mut warm = Vec::new();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut warm);
    }
    let w0 = world_slot(&core, id_of(0));
    let id = drop_one(&mut core, w0, FILLER, 4);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut warm);
    }

    // A second client arrives long after the barrel burst and hears no
    // broadcast about it — there was never one to hear.
    assert!(core.connect(1, id_of(1)));
    clients.push((1usize, ClientCore::new(SEED, id_of(1), 0)));
    let mut seen = Vec::new();
    let mut late = 0u32;
    for _ in 0..8 {
        late |= pump(&mut core, &stats, &mut clients, &mut seen)[1];
    }

    assert_ne!(
        late & APPLIED2_GITEMS,
        0,
        "the loose-stack walk never reached slot 1"
    );
    assert!(
        seen.iter()
            .any(|(slot, m)| *slot == 1
                && matches!(m, EventMsg::GItemSync { count, .. } if *count > 0)),
        "and it arrived as a sync batch"
    );
    let c1 = &clients.iter().find(|(s, _)| *s == 1).unwrap().1;
    assert_eq!(c1.ground_items().len(), 1);
    assert_eq!(c1.ground_items()[0].id, id);
    assert_eq!(c1.ground_items()[0].count, 4);
}

/// Stacks that land are appended to the walk, never a restart — not the
/// burst, and not the one that lands while the walk is still sending it:
/// every earlier record is where it was, so the walk carries on and the
/// client ends with all of them without a reset. A fight lands arrows tick
/// after tick (`spent.rs`), and a restart per landing would never reach
/// the newest. A removal still restarts it (the take above).
#[test]
fn a_stack_that_lands_mid_walk_is_appended_not_restarted() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    assert!(core.connect(0, id_of(0)));
    let mut clients = vec![(0usize, ClientCore::new(SEED, id_of(0), 0))];
    let mut warm = Vec::new();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut warm);
    }
    let w0 = world_slot(&core, id_of(0));
    let (qx, qy, qz) = {
        let b = core.world.players[w0].body;
        (b.qx, b.qy, b.qz)
    };
    let (bc, tick) = (core.world.backpack, core.world.tick);
    let stack = ItemStack {
        item: FILLER,
        count: 1,
        cond: 0,
        skin: 0,
    };
    // More than one batch, so the walk is mid-way after one pump.
    let batch = protocol::event::GITEM_SYNC_BATCH;
    for k in 0..batch + 4 {
        core.world
            .ground_items
            .drop_one(&bc, (qx + k as i32, qy, qz), stack, tick, [0; 3]);
    }
    let mut seen = Vec::new();
    pump(&mut core, &stats, &mut clients, &mut seen);
    core.world
        .ground_items
        .drop_one(&bc, (qx - 5, qy, qz), stack, tick, [0; 3]);
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }

    let resets = seen
        .iter()
        .filter(|(_, m)| matches!(m, EventMsg::GItemSync { reset: true, .. }))
        .count();
    assert_eq!(resets, 0, "a landing restarted the walk");
    let c = &clients[0].1;
    assert_eq!(
        c.ground_items().len(),
        batch + 5,
        "the client has every stack, the late one included"
    );
}

/// An arrow standing in a body reaches every client as the body it is in
/// and where in it and which way (`SUB_LODGED_SYNC`, wire v94), and when it
/// comes out every client's set loses it — the same two halves as the loose
/// stacks above, through real bytes.
#[test]
fn an_arrow_in_a_body_reaches_every_client_and_comes_out_of_every_set() {
    use protocol::WireLodged;
    use sim_core::spent::SpentRec;
    let stats = ShardStats::default();
    let mut core = armed_core();
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    let mut seen = Vec::new();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    let w1 = world_slot(&core, id_of(1));
    let host = id_of(1);
    let life = u64::from(core.world.players[w1].deaths);
    let tick = core.world.tick;
    core.world.spent.lodge(SpentRec {
        host,
        life,
        round: FILLER,
        ready_at: tick + 10_000,
        off: [-12, 131, 25],
        dir: [3, -20, -127],
        ..SpentRec::default()
    });
    let want = WireLodged {
        host,
        item: FILLER,
        off: [-12, 131, 25],
        dir: [3, -20, -127],
    };
    seen.clear();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    for (slot, c) in &clients {
        assert_eq!(
            c.lodged(),
            &[want][..],
            "client {slot} draws it in the body"
        );
        assert!(
            seen.iter().any(|(s, m)| *s == *slot
                && matches!(m, EventMsg::LodgedSync { recs, count: 1, .. } if recs[0] == want)),
            "and the server put it on client {slot}'s lane"
        );
    }

    // It rides the body without a byte: following its host moves nothing
    // the walk sends.
    seen.clear();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    assert!(
        !seen
            .iter()
            .any(|(_, m)| matches!(m, EventMsg::LodgedSync { .. })),
        "an arrow riding still costs nothing on the lane"
    );

    // Out it comes: every client's set loses it.
    assert!(core.world.spent.take_at(0).is_some());
    seen.clear();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    for (slot, c) in &clients {
        assert!(c.lodged().is_empty(), "client {slot} stops drawing it");
        assert!(
            seen.iter().any(|(s, m)| *s == *slot
                && matches!(
                    m,
                    EventMsg::LodgedSync {
                        reset: true,
                        count: 0,
                        ..
                    }
                )),
            "told by an empty reset on client {slot}'s lane"
        );
    }
}
