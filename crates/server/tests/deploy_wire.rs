//! The deployables-on-the-wire gate (M1): `ShardCore` ↔ `ClientCore`
//! through real encoded bytes — the deploy-def table drips to a joiner, a
//! deploy action consumes its item and broadcasts the record, the hearth
//! claim refuses a stranger's placement with its reason, feed moves
//! materials and acks the stock, a late joiner receives the placed set by
//! the sync walk, and a decay removal broadcasts and restarts an
//! in-progress walk. Deterministic, no sockets; asserts are structural
//! and exact (the build_wire shape).

use client_wasm::core::{
    ClientCore, APPLIED_DEPLOYS, APPLIED_DEPLOY_DEFS, APPLIED_DEPLOY_REFUSED, APPLIED_DEPLOY_RESET,
    APPLIED_PIECE_REMOVED, APPLIED_STOCK,
};
use protocol::{ActionMsg, ItemCatalog};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::build::{BuildContent, LOC_EDGE_W, LOC_PLANE};
use sim_core::deploy::{DeployContent, REFUSE_D_CLAIM, UPKEEP_PERIOD_TICKS};
use sim_core::gather::{GatherContent, ItemStack};

const SEED: u64 = 20_260_731;
/// The canonical dev spawn point, guarded walkable in sim-core
/// `world::tests` — walkable terrain also takes ground-class deploys.
const SPAWN: (f32, f32) = (1024.0, 1024.0);
const CX: u16 = 341;
const CZ: u16 = 341;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

/// One lockstep pump (the build_wire shape). Returns per-slot APPLIED flags.
fn pump(core: &mut ShardCore, stats: &ShardStats, clients: &mut [(usize, ClientCore)]) -> [u32; 4] {
    pump_seen(core, stats, clients, &mut Vec::new())
}

/// The same pump, keeping every event message the server sent this tick,
/// decoded from the bytes it actually put on the lane. A client mirror
/// can agree with the world for reasons the *encoder* never earned (the
/// sync walk re-sends what a broadcast got wrong), so anything the wire
/// alone is responsible for gets asserted here, on `seen`.
fn pump_seen(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    seen: &mut Vec<(usize, protocol::EventMsg)>,
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

fn act(core: &mut ShardCore, slot: usize, a: ActionMsg) {
    assert!(core.wants_action(slot), "hand should be open");
    core.push_action(slot, a);
}

#[test]
fn deployables_ride_the_wire() {
    let fixture = DeployContent::probe_fixture();
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    core.world.deploy = fixture;
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];

    // Join drip: the deploy-def table reaches both clients row-exact.
    let mut defs_flags = 0u32;
    for _ in 0..4 {
        defs_flags |= pump(&mut core, &stats, &mut clients)[0];
    }
    assert_ne!(
        defs_flags & APPLIED_DEPLOY_DEFS,
        0,
        "deploy defs never dripped"
    );
    for (_, c) in &clients {
        assert_eq!(c.deploy_defs.def_count, fixture.def_count);
        assert_eq!(c.deploy_defs_have, fixture.def_count);
        for i in 0..fixture.def_count as usize {
            assert_eq!(c.deploy_defs.defs[i], fixture.defs[i], "row {i} drifted");
        }
    }

    // Grant the owner a kit server-side (gather_wire covers earning).
    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].inv[0] = ItemStack { item: 0, count: 50 };
    core.world.players[w0].inv[1] = ItemStack { item: 1, count: 50 };
    core.world.players[w0].inv[2] = ItemStack { item: 2, count: 5 };

    // Foundation + hearth at the spawn cell: both broadcast; the hearth
    // consumes its item.
    act(
        &mut core,
        0,
        ActionMsg::Place {
            row: 0,
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_PLANE,
        },
    );
    pump(&mut core, &stats, &mut clients);
    act(
        &mut core,
        0,
        ActionMsg::Deploy {
            row: 0,
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_PLANE,
        },
    );
    let flags = pump(&mut core, &stats, &mut clients);
    assert_ne!(flags[0] & APPLIED_DEPLOYS, 0, "owner never saw the hearth");
    assert_ne!(flags[1] & APPLIED_DEPLOYS, 0, "broadcast missed bystander");
    for (_, c) in &clients {
        assert_eq!(c.deploys.len(), 1);
        let rec = c.deploys.entries()[0];
        assert_eq!(
            (rec.cx, rec.cz, rec.level, rec.loc, rec.row),
            (CX, CZ, 0, LOC_PLANE, 0)
        );
    }
    assert_eq!(
        sim_core::craft::inv_count(&core.world.players[w0].inv, 2),
        4,
        "hearth item unconsumed"
    );

    // The bystander (same spawn cell, inside the radius, not the owner)
    // is refused by the claim, with the reason delivered to them only.
    let w1 = world_slot(&core, id_of(1));
    core.world.players[w1].inv[0] = ItemStack { item: 0, count: 50 };
    act(
        &mut core,
        1,
        ActionMsg::Place {
            row: 0,
            cx: CX + 1,
            cz: CZ,
            level: 0,
            loc: LOC_PLANE,
        },
    );
    let flags = pump(&mut core, &stats, &mut clients);
    assert_ne!(
        flags[1] & client_wasm::core::APPLIED_BUILD_REFUSED,
        0,
        "claim refusal never arrived"
    );
    assert_eq!(
        clients[1].1.pop_build_refusal(),
        Some(sim_core::build::REFUSE_B_CLAIM as u8)
    );
    // And the deploy lane refuses the stranger too, with its own reason.
    act(
        &mut core,
        1,
        ActionMsg::Deploy {
            row: 3,
            cx: CX + 1,
            cz: CZ,
            level: 0,
            loc: LOC_PLANE,
        },
    );
    let flags = pump(&mut core, &stats, &mut clients);
    assert_ne!(flags[1] & APPLIED_DEPLOY_REFUSED, 0);
    assert_eq!(
        clients[1].1.pop_deploy_refusal(),
        Some(REFUSE_D_CLAIM as u8)
    );
    assert_eq!(clients[0].1.pop_deploy_refusal(), None, "refusal leaked");

    // Feed: materials leave the feeder's inventory, the stock ack carries
    // the hearth's rows (mats are the build fixture's items 0 and 1).
    act(
        &mut core,
        0,
        ActionMsg::Feed {
            cx: CX,
            cz: CZ,
            level: 0,
        },
    );
    let flags = pump(&mut core, &stats, &mut clients);
    assert_ne!(flags[0] & APPLIED_STOCK, 0, "stock ack never arrived");
    let c0 = &clients[0].1;
    assert_eq!(c0.stock_addr, (CX, CZ, 0));
    assert_eq!(c0.stock_count, 2);
    // 50 − 5 foundation cost = 45 of item 0 fed; all 50 of item 1.
    assert_eq!(c0.stock[0], (0, 45));
    assert_eq!(c0.stock[1], (1, 50));
    assert_eq!(
        sim_core::craft::inv_count(&core.world.players[w0].inv, 0),
        0,
        "feed left materials behind"
    );

    // A late joiner receives the placed set by the sync walk.
    assert!(core.connect(2, id_of(2)));
    clients.push((2usize, ClientCore::new(SEED, id_of(2), 0)));
    let mut late_flags = 0u32;
    for _ in 0..4 {
        late_flags |= pump(&mut core, &stats, &mut clients)[2];
    }
    assert_ne!(late_flags & APPLIED_DEPLOY_RESET, 0, "no reset batch");
    assert_eq!(
        clients[2].1.deploys.len(),
        1,
        "late joiner missed the deploy set"
    );

    // Decay: unpaid pieces vanish and the removal broadcast reaches every
    // client. Leap the sim clock far enough that the far foundation
    // (placed outside any hearth radius) decays to zero. First place one
    // far away, walk the owner there via a direct teleport (the pump's
    // inputs would take minutes of ticks).
    let far_cx = CX + 20;
    let far_x = (far_cx as f32 + 0.5) * sim_core::build::BUILD_CELL_M;
    core.world.players[w0].body = sim_core::movement::Body::at(SEED, far_x, SPAWN.1);
    core.world.players[w0].inv[0] = ItemStack { item: 0, count: 10 };
    act(
        &mut core,
        0,
        ActionMsg::Place {
            row: 0,
            cx: far_cx,
            cz: CZ,
            level: 0,
            loc: LOC_PLANE,
        },
    );
    pump(&mut core, &stats, &mut clients);
    assert_eq!(core.world.pieces.len(), 2);
    // One upkeep period per pump: the far foundation (100 hp, 5%/period)
    // is gone by period 20; the loop bound leaves headroom.
    let mut decayed = false;
    for _ in 1..=25u64 {
        core.world.tick += UPKEEP_PERIOD_TICKS;
        let flags = pump(&mut core, &stats, &mut clients);
        for f in flags {
            if f & APPLIED_PIECE_REMOVED != 0 {
                decayed = true;
            }
        }
        if decayed {
            break;
        }
    }
    assert!(decayed, "the far foundation never decayed on the wire");
    // The spawn foundation is hearth-paid and survives; every client's
    // mirror agrees with the world.
    assert_eq!(core.world.pieces.len(), 1);
    for (_, c) in &clients {
        assert_eq!(c.pieces.len(), core.world.pieces.len(), "mirror drifted");
    }

    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}

/// The door lane end to end: a door places closed, the use action toggles
/// it, every client's mirror **and** its predictor collision index follow,
/// a late joiner learns the state from the sync walk (not from having
/// been there), and a use aimed at something that isn't a door bounces
/// with its reason.
#[test]
fn doors_toggle_across_the_wire() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    core.world.deploy = DeployContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients);
    }

    // The kit: wood for the foundation and the doorway, one door.
    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].inv[0] = ItemStack { item: 0, count: 50 };
    core.world.players[w0].inv[1] = ItemStack { item: 4, count: 5 };

    for a in [
        ActionMsg::Place {
            row: 0,
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_PLANE,
        },
        ActionMsg::Place {
            row: 3,
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_W,
        },
    ] {
        act(&mut core, 0, a);
        pump(&mut core, &stats, &mut clients);
    }
    assert_eq!(core.world.pieces.len(), 2, "foundation + doorway");
    for (_, c) in &clients {
        assert_eq!(
            c.pieces.cols().get(CX, CZ).shut_w & 1,
            0,
            "an empty doorway must not be sealed"
        );
    }

    // The door lands closed, on both mirrors and in both predictors.
    act(
        &mut core,
        0,
        ActionMsg::Deploy {
            row: 2,
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_W,
        },
    );
    let mut seen = Vec::new();
    let flags = pump_seen(&mut core, &stats, &mut clients, &mut seen);
    // The *broadcast* itself must carry the lock — not merely the sync
    // walk riding behind it, which would repair a wrong record and hide
    // an encoder that dropped the bit.
    let placed: Vec<_> = seen
        .iter()
        .filter_map(|(slot, m)| match m {
            protocol::EventMsg::DeployPlaced { rec } if rec.loc == LOC_EDGE_W => Some((slot, rec)),
            _ => None,
        })
        .collect();
    assert_eq!(placed.len(), 2, "the placement must reach both clients");
    for (slot, rec) in placed {
        assert!(
            rec.locked,
            "the placed-door broadcast to {slot} lost its lock"
        );
        assert!(!rec.open, "and it must announce the leaf shut");
    }
    assert_ne!(flags[0] & APPLIED_DEPLOYS, 0, "owner never saw the door");
    assert_ne!(flags[1] & APPLIED_DEPLOYS, 0, "broadcast missed bystander");
    for (_, c) in &clients {
        let rec = c
            .deploys
            .entries()
            .iter()
            .find(|r| r.loc == LOC_EDGE_W)
            .expect("door in the mirror");
        assert!(!rec.open, "doors place closed");
        assert!(rec.locked, "doors place locked (lock v0), and say so");
        assert_eq!(
            c.pieces.cols().get(CX, CZ).shut_w & 1,
            1,
            "a closed door must seal the doorway the predictor walks"
        );
    }

    // The bystander's hand bounces off it: locked, and not theirs. The
    // refusal is the sender's alone, and the door never moves.
    act(
        &mut core,
        1,
        ActionMsg::Use {
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_W,
        },
    );
    let flags = pump(&mut core, &stats, &mut clients);
    assert_ne!(flags[1] & APPLIED_DEPLOY_REFUSED, 0);
    assert_eq!(
        clients[1].1.pop_deploy_refusal(),
        Some(sim_core::deploy::REFUSE_D_OWNER as u8)
    );
    assert_eq!(clients[0].1.pop_deploy_refusal(), None, "refusal leaked");
    assert!(
        !core
            .world
            .deploys
            .find(CX, CZ, 0, LOC_EDGE_W)
            .expect("door in the world")
            .open,
        "a refused use must not swing the door"
    );

    // The use action opens it — for everyone, including the bystander.
    act(
        &mut core,
        0,
        ActionMsg::Use {
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_W,
        },
    );
    seen.clear();
    let flags = pump_seen(&mut core, &stats, &mut clients, &mut seen);
    assert_ne!(flags[0] & APPLIED_DEPLOYS, 0, "toggler never heard back");
    assert_ne!(flags[1] & APPLIED_DEPLOYS, 0, "door state is a broadcast");
    // Absolute, and the whole door: the announcement carries the leaf it
    // just swung AND the lock it did not touch.
    assert_eq!(
        seen.iter()
            .filter(|(_, m)| matches!(
                m,
                protocol::EventMsg::Door {
                    open: true,
                    locked: true,
                    ..
                }
            ))
            .count(),
        2,
        "the door announcement must reach both clients open and still locked"
    );
    assert!(
        core.world
            .deploys
            .find(CX, CZ, 0, LOC_EDGE_W)
            .expect("door in the world")
            .open
    );
    for (_, c) in &clients {
        let rec = c
            .deploys
            .entries()
            .iter()
            .find(|r| r.loc == LOC_EDGE_W)
            .expect("door in the mirror");
        assert!(rec.open, "the open state never crossed");
        assert_eq!(
            c.pieces.cols().get(CX, CZ).shut_w & 1,
            0,
            "an open door must stop sealing the predictor's doorway"
        );
    }

    // A late joiner reads the open door off the sync walk.
    assert!(core.connect(2, id_of(2)));
    clients.push((2usize, ClientCore::new(SEED, id_of(2), 0)));
    seen.clear();
    for _ in 0..5 {
        pump_seen(&mut core, &stats, &mut clients, &mut seen);
    }
    // The walk's own bytes, not just what the mirror settled on.
    assert!(
        seen.iter().any(|(slot, m)| matches!(
            m,
            protocol::EventMsg::DeploySync { recs, count, .. }
                if *slot == 2
                    && recs[..*count as usize]
                        .iter()
                        .any(|r| r.loc == LOC_EDGE_W && r.open && r.locked)
        )),
        "the sync walk must carry the door's open AND locked bits"
    );
    let late = &clients[2].1;
    let rec = late
        .deploys
        .entries()
        .iter()
        .find(|r| r.loc == LOC_EDGE_W)
        .expect("late joiner missed the door");
    assert!(rec.open, "the walk carried the door shut");
    assert!(rec.locked, "the walk lost the door's lock bit");
    assert_eq!(
        late.pieces.cols().get(CX, CZ).shut_w & 1,
        0,
        "late joiner's predictor sealed a door that stands open"
    );

    // The owner unlocks it: everyone hears, and the leaf does not move.
    act(
        &mut core,
        0,
        ActionMsg::Lock {
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_W,
            locked: false,
        },
    );
    seen.clear();
    let flags = pump_seen(&mut core, &stats, &mut clients, &mut seen);
    for (slot, f) in flags.iter().enumerate().take(clients.len()) {
        assert_ne!(
            f & APPLIED_DEPLOYS,
            0,
            "client {slot} missed the lock change"
        );
    }
    assert_eq!(
        seen.iter()
            .filter(|(_, m)| matches!(
                m,
                protocol::EventMsg::Door {
                    open: true,
                    locked: false,
                    ..
                }
            ))
            .count(),
        3,
        "the unlock must cross to all three clients with the leaf untouched"
    );
    for (_, c) in &clients {
        let rec = c
            .deploys
            .entries()
            .iter()
            .find(|r| r.loc == LOC_EDGE_W)
            .expect("door in the mirror");
        assert!(!rec.locked, "the unlock never crossed");
        assert!(rec.open, "unlocking must not move the leaf");
    }

    // And now any hand in reach shuts it — door v0's behavior, kept for
    // the doors their owners choose to leave public.
    act(
        &mut core,
        1,
        ActionMsg::Use {
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_W,
        },
    );
    pump(&mut core, &stats, &mut clients);
    assert!(
        !core
            .world
            .deploys
            .find(CX, CZ, 0, LOC_EDGE_W)
            .expect("door in the world")
            .open,
        "an unlocked door takes any hand in reach"
    );
    assert_eq!(clients[1].1.pop_deploy_refusal(), None, "a use was refused");

    // Using something that is not a door bounces, and only at the sender.
    act(
        &mut core,
        0,
        ActionMsg::Use {
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_PLANE,
        },
    );
    let flags = pump(&mut core, &stats, &mut clients);
    assert_ne!(flags[0] & APPLIED_DEPLOY_REFUSED, 0);
    assert_eq!(
        clients[0].1.pop_deploy_refusal(),
        Some(sim_core::deploy::REFUSE_D_DOOR as u8)
    );
    assert_eq!(clients[1].1.pop_deploy_refusal(), None, "refusal leaked");

    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}
