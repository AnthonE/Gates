//! The gather-on-the-wire gate (M1): `ShardCore` ↔ `ClientCore` through
//! real encoded bytes on both lanes — a swing's payout reaches the
//! swinger's inventory mirror and toast ring, the slot's harvest reaches
//! every client's harvested set, a late joiner is synced by the reset
//! walk, the respawn releases everyone, and an event-ring overflow heals
//! through the same resync path. Deterministic, no sockets; asserts are
//! structural and exact.

use client_wasm::core::{ClientCore, APPLIED_RESET};
use protocol::ItemCatalog;
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::gather::{cell_key, GatherContent, SWING_INTERVAL_TICKS};
use sim_core::input::BTN_PRIMARY;
use sim_core::terrain::{self, Occupant, ScatterTable, CELL_SIZE};
use sim_core::yaw_dir;

const SEED: u64 = 20_260_731;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

/// A gatherable tree, the walkable stand point beside it, and the yaw
/// facing it — the same scan `sim-core/tests/gather.rs` uses. Panics if
/// the seed offers none: a setup failure, never a skip.
fn find_isolated_tree() -> ((f32, f32), u16, (u16, u16)) {
    let table = ScatterTable::alpha_default();
    for cz in 40..216i32 {
        for cx in 40..216i32 {
            let s = terrain::scatter(SEED, &table, cx, cz);
            if s.occupant != Occupant::Tree {
                continue;
            }
            let (px, pz) = (s.x - 1.2, s.z);
            let py = terrain::height(SEED, px, pz);
            if (s.y - py).max(py - s.y) > 1.0 || py < 1.0 {
                continue;
            }
            let pcx = (px / CELL_SIZE) as i32;
            let pcz = (pz / CELL_SIZE) as i32;
            let mut rivals = 0;
            for dz in -1..=1i32 {
                for dx in -1..=1i32 {
                    let n = terrain::scatter(SEED, &table, pcx + dx, pcz + dz);
                    if sim_core::gather::node_index(n.occupant).is_some() {
                        let d2 = (n.x - px) * (n.x - px) + (n.z - pz) * (n.z - pz);
                        if d2 <= 6.25 && (n.x != s.x || n.z != s.z) {
                            rivals += 1;
                        }
                    }
                }
            }
            if rivals > 0 {
                continue;
            }
            let (dx, dz) = (s.x - px, s.z - pz);
            let mut best_yaw = 0u16;
            let mut best_dot = f32::MIN;
            for hi in 0..=255u16 {
                let yaw = hi << 8;
                let (fx, fz) = yaw_dir(yaw);
                let dot = fx * dx + fz * dz;
                if dot > best_dot {
                    best_dot = dot;
                    best_yaw = yaw;
                }
            }
            return ((px, pz), best_yaw, (cx as u16, cz as u16));
        }
    }
    panic!("seed {SEED:#x} offered no isolated tree in the scanned block");
}

/// A synthetic catalog matching the probe fixture's 8 items.
fn probe_catalog() -> ItemCatalog {
    let mut cat = ItemCatalog::EMPTY;
    cat.count = 8;
    for i in 0..8usize {
        let name = [b'P', b'0' + i as u8];
        cat.set(i, &name).unwrap();
    }
    cat
}

/// One lockstep pump. Events for connection slots in `drop_events_for`
/// are refused at the ring (the overflow case); everything else flows.
/// Returns the union of `APPLIED_*` flags each connection slot saw.
fn pump(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    drop_events_for: &[usize],
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
    core.tick(stats, |lane, slot, bytes| match lane {
        Lane::Snapshot => {
            snaps.push((slot, bytes.to_vec()));
            true
        }
        Lane::Event => {
            if drop_events_for.contains(&slot) {
                return false;
            }
            events.push((slot, bytes.to_vec()));
            true
        }
    });
    let mut flags = [0u32; 4];
    for (slot, bytes) in events {
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

#[test]
fn gather_rides_the_wire() {
    let (pos, yaw, (cx, cz)) = find_isolated_tree();
    let key = cell_key(cx, cz);
    let fixture = GatherContent::probe_fixture();
    let tree = fixture.nodes[0];

    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    core.world.gather = fixture;
    core.world.dev_spawn = Some(pos);
    core.catalog = probe_catalog();
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];

    // Swing until the tree exhausts: client 0 holds primary facing it,
    // client 1 stands idle beside it.
    let ticks = SWING_INTERVAL_TICKS * (tree.hits as u64 + 2);
    for _ in 0..ticks {
        clients[0].1.set_input(BTN_PRIMARY, yaw, 0, 0, 0);
        clients[1].1.set_input(0, yaw, 0, 0, 0);
        pump(&mut core, &stats, &mut clients, &[]);
    }

    // The swinger's inventory mirror matches the world exactly.
    let world_inv = core
        .world
        .players
        .iter()
        .find(|p| p.active && p.id == id_of(0))
        .expect("swinger in world")
        .inv;
    assert_eq!(clients[0].1.inv, world_inv, "inventory mirror drifted");
    assert_eq!(world_inv[0].item, tree.output, "tree pays its output");
    assert!(world_inv[0].count > 0, "swings paid nothing");
    // Toasts arrived for the swinger, none for the bystander.
    let (t_item, t_added) = clients[0].1.pop_toast().expect("a toast landed");
    assert_eq!(t_item, tree.output);
    assert!(t_added > 0);
    assert_eq!(clients[1].1.pop_toast(), None, "bystander got a toast");
    let bystander_inv = clients[1].1.inv;
    assert!(
        bystander_inv.iter().all(|s| s.count == 0),
        "bystander inventory should be empty"
    );

    // The harvest broadcast reached both clients, and matches the server.
    assert!(core.world.slot_lives.is_harvested(cx, cz), "server state");
    assert!(clients[0].1.harvested.contains(key), "swinger's set");
    assert!(clients[1].1.harvested.contains(key), "bystander's set");

    // Both catalogs filled from the drip.
    for (slot, c) in &clients {
        assert_eq!(c.catalog.count, 8, "client {slot} catalog count");
        assert_eq!(c.catalog.name(0), b"P0", "client {slot} catalog name");
    }

    // A late joiner is synced by the reset walk: the harvested cell
    // arrives without having seen any harvest event.
    assert!(core.connect(2, id_of(2)));
    clients.push((2usize, ClientCore::new(SEED, id_of(2), 0)));
    let mut late_flags = 0u32;
    for _ in 0..8 {
        for (_, c) in clients.iter_mut() {
            c.set_input(0, yaw, 0, 0, 0);
        }
        late_flags |= pump(&mut core, &stats, &mut clients, &[])[2];
    }
    assert!(late_flags & APPLIED_RESET != 0, "join sync never reset");
    assert!(clients[2].1.harvested.contains(key), "late joiner synced");

    // Respawn: jump the world clock to the slot's release tick; the
    // broadcast clears every client's set. (The clock jump costs the
    // clients their snapshot baselines — recovery is the zero-state path,
    // which is exactly what it exists for.)
    let respawn_at = core
        .world
        .slot_lives
        .entries()
        .iter()
        .find(|e| (e.cx, e.cz) == (cx, cz))
        .expect("harvested entry exists")
        .respawn_at;
    assert!(respawn_at > core.world.tick, "respawn is in the future");
    core.world.tick = respawn_at;
    for _ in 0..4 {
        for (_, c) in clients.iter_mut() {
            c.set_input(0, yaw, 0, 0, 0);
        }
        pump(&mut core, &stats, &mut clients, &[]);
    }
    assert!(!core.world.slot_lives.is_harvested(cx, cz));
    for (slot, c) in &clients {
        assert!(
            !c.harvested.contains(key),
            "client {slot} still shows the node harvested after respawn"
        );
    }
    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}

/// The overflow policy is a wall (limits.rs): a refused event push must
/// heal through `ev_resync` — the client that missed a broadcast ends up
/// correct anyway, via the reset walk.
#[test]
fn event_ring_overflow_heals_by_resync() {
    let (pos, yaw, (cx, cz)) = find_isolated_tree();
    let key = cell_key(cx, cz);
    let fixture = GatherContent::probe_fixture();
    let tree = fixture.nodes[0];

    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    core.world.gather = fixture;
    core.world.dev_spawn = Some(pos);
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];

    // Client 1's event ring refuses everything while the tree is felled.
    let ticks = SWING_INTERVAL_TICKS * (tree.hits as u64 + 2);
    for _ in 0..ticks {
        clients[0].1.set_input(BTN_PRIMARY, yaw, 0, 0, 0);
        clients[1].1.set_input(0, yaw, 0, 0, 0);
        pump(&mut core, &stats, &mut clients, &[1]);
    }
    assert!(core.world.slot_lives.is_harvested(cx, cz));
    assert!(!clients[1].1.harvested.contains(key), "events were dropped");
    assert!(
        ShardStats::get(&stats.ev_resyncs) > 0,
        "refused pushes must be counted as resyncs"
    );

    // The ring drains; the resync walk restores the truth.
    for _ in 0..8 {
        clients[0].1.set_input(0, yaw, 0, 0, 0);
        clients[1].1.set_input(0, yaw, 0, 0, 0);
        pump(&mut core, &stats, &mut clients, &[]);
    }
    assert!(
        clients[1].1.harvested.contains(key),
        "resync never restored the harvested cell"
    );
    assert!(clients[0].1.harvested.contains(key), "clean client kept it");
}
