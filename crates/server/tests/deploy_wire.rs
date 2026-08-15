//! The deployables-on-the-wire gate (M1): `ShardCore` ↔ `ClientCore`
//! through real encoded bytes — the deploy-def table drips to a joiner, a
//! deploy action consumes its item and broadcasts the record, the hearth
//! claim refuses a stranger's placement with its reason, feed moves
//! materials and acks the stock, a late joiner receives the placed set by
//! the sync walk, a decay removal broadcasts, a removal *storm* leaves the
//! piece walk of every client standing, and a decay *cliff* cannot run a
//! fresh walk's cursor off the end of the store. Deterministic, no
//! sockets; asserts are structural and exact (the build_wire shape).

use client_core::core::{
    ClientCore, APPLIED_DEPLOYS, APPLIED_DEPLOY_DEFS, APPLIED_DEPLOY_REFUSED, APPLIED_DEPLOY_RESET,
    APPLIED_PIECE_REMOVED, APPLIED_PIECE_RESET, APPLIED_STOCK,
};
use protocol::{ActionMsg, ItemCatalog, PIECE_SYNC_BATCH};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::build::{BuildContent, LOC_EDGE_XLO, LOC_PLANE};
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
    core.tick_bare(stats, |lane, slot, bytes| {
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
    let mut core = Box::new(ShardCore::new(SEED));
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
        flags[1] & client_core::core::APPLIED_BUILD_REFUSED,
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

    // Commit the spawn foundation: a twig piece is never upkept, so the
    // "hearth-paid piece survives" half of the decay check below needs a
    // graded one to be about anything (twig v0, `build::upgrade`).
    core.world.players[w0].inv[0] = ItemStack { item: 0, count: 10 };
    act(
        &mut core,
        0,
        ActionMsg::Upgrade {
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_PLANE,
            material: sim_core::build::MAT_STONE,
        },
    );
    pump(&mut core, &stats, &mut clients);
    assert_eq!(
        core.world.pieces.entries()[0].row,
        5,
        "the spawn foundation never climbed to its stone rung"
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
    // mirror agrees with the world. **It only survives because it was
    // upgraded above twig** (twig v0): the sweep never charges a scaffold
    // upkeep and therefore never protects one, so had it been left as
    // placed it would have rotted under a full hearth exactly like the far
    // one — which is the rule, not a defect, and this is the gate that
    // says so from the wire's side.
    assert_eq!(core.world.pieces.len(), 1);
    for (_, c) in &clients {
        assert_eq!(c.pieces.len(), core.world.pieces.len(), "mirror drifted");
    }

    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}

/// The door lane end to end (lock v1): a door places **bare** and closed
/// and anyone in reach may work it, a code lock bolted on and armed makes
/// it answer only to who it remembers, a refused press **knocks** to the
/// whole shard while the refusal itself stays with its sender, the right
/// code grants — to that sender alone — every client's mirror and its
/// predictor collision index follow, a late joiner learns all three bits
/// from the sync walk (not from having been there), and a use aimed at
/// something that isn't a door bounces with its reason.
#[test]
fn doors_toggle_across_the_wire() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
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

    // The kit: wood for the foundation and the doorway, one door, and a
    // code lock to bolt onto it.
    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].inv[0] = ItemStack { item: 0, count: 50 };
    core.world.players[w0].inv[1] = ItemStack { item: 4, count: 5 };
    core.world.players[w0].inv[2] = ItemStack { item: 7, count: 2 };

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
            loc: LOC_EDGE_XLO,
        },
    ] {
        act(&mut core, 0, a);
        pump(&mut core, &stats, &mut clients);
    }
    assert_eq!(core.world.pieces.len(), 2, "foundation + doorway");
    for (_, c) in &clients {
        assert_eq!(
            c.pieces.cols().get(CX, CZ).shut_xlo & 1,
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
            loc: LOC_EDGE_XLO,
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
            protocol::EventMsg::DeployPlaced { rec } if rec.loc == LOC_EDGE_XLO => {
                Some((slot, rec))
            }
            _ => None,
        })
        .collect();
    assert_eq!(placed.len(), 2, "the placement must reach both clients");
    for (slot, rec) in placed {
        assert!(
            !rec.locked && !rec.has_lock,
            "the placed-door broadcast to {slot} says it is secured, and a \
             door places BARE (lock v1) — the security is what costs"
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
            .find(|r| r.loc == LOC_EDGE_XLO)
            .expect("door in the mirror");
        assert!(!rec.open, "doors place closed");
        assert!(!rec.has_lock, "and bare (lock v1), and say so");
        assert_eq!(
            c.pieces.cols().get(CX, CZ).shut_xlo & 1,
            1,
            "a closed door must seal the doorway the predictor walks"
        );
    }

    // Bare, the bystander's hand works it — that is the whole reason a
    // lock costs anything (`reference/DOORS.md` §5).
    act(
        &mut core,
        1,
        ActionMsg::Use {
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_XLO,
        },
    );
    let flags = pump(&mut core, &stats, &mut clients);
    assert_eq!(flags[1] & APPLIED_DEPLOY_REFUSED, 0, "a bare door refused");
    assert!(
        core.world
            .deploys
            .find(CX, CZ, 0, LOC_EDGE_XLO)
            .expect("door in the world")
            .open,
        "a door nobody has secured is anyone's"
    );
    // Shut it again so the arc below starts where it did before.
    act(
        &mut core,
        1,
        ActionMsg::Use {
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_XLO,
        },
    );
    pump(&mut core, &stats, &mut clients);

    // The owner bolts a code lock on. No deploy record is minted — a lock
    // is a record *about* one — so what crosses is the door's own
    // announcement, carrying the new bit.
    act(
        &mut core,
        0,
        ActionMsg::Deploy {
            row: 5,
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_XLO,
        },
    );
    let deploys_before = core.world.deploys.len();
    seen.clear();
    pump_seen(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(
        core.world.deploys.len(),
        deploys_before,
        "a lock must not mint a deployable record"
    );
    assert_eq!(core.world.deploys.locks().len(), 1, "it minted a lock");
    assert_eq!(
        seen.iter()
            .filter(|(_, m)| matches!(
                m,
                protocol::EventMsg::Door {
                    has_lock: true,
                    locked: false,
                    ..
                }
            ))
            .count(),
        2,
        "bolting a lock on must announce has_lock to both clients, and \
         must not announce it as locked — an unarmed lock is not a locked \
         door"
    );

    // ...and arms it with a code. Now it is shut to everyone else.
    act(
        &mut core,
        0,
        ActionMsg::Access {
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_XLO,
            op: sim_core::deploy::ACCESS_OP_SET_CODE,
            code: 1234,
        },
    );
    pump(&mut core, &stats, &mut clients);
    for (_, c) in &clients {
        let rec = c
            .deploys
            .entries()
            .iter()
            .find(|r| r.loc == LOC_EDGE_XLO)
            .expect("door in the mirror");
        assert!(rec.locked && rec.has_lock, "the arming never crossed");
    }

    // The bystander's hand bounces off it — and KNOCKS. The refusal is
    // the sender's alone; the knock is everyone's, which is the point of
    // having one at all.
    act(
        &mut core,
        1,
        ActionMsg::Use {
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_XLO,
        },
    );
    seen.clear();
    let flags = pump_seen(&mut core, &stats, &mut clients, &mut seen);
    assert_ne!(flags[1] & APPLIED_DEPLOY_REFUSED, 0);
    assert_eq!(
        clients[1].1.pop_deploy_refusal(),
        Some(sim_core::deploy::REFUSE_D_OWNER as u8)
    );
    assert_eq!(clients[0].1.pop_deploy_refusal(), None, "refusal leaked");
    assert_eq!(
        seen.iter()
            .filter(|(_, m)| matches!(m, protocol::EventMsg::Knock { .. }))
            .count(),
        2,
        "a knock must reach the shard, not only the hand that knocked — \
         it is the one channel a locked-out player has to the person inside"
    );
    assert!(
        clients[0].1.pop_knock().is_some(),
        "the OWNER is who a knock is for"
    );
    assert!(
        !core
            .world
            .deploys
            .find(CX, CZ, 0, LOC_EDGE_XLO)
            .expect("door in the world")
            .open,
        "a refused use must not swing the door"
    );

    // The bystander enters the code. The grant is an own-fact: it reaches
    // them and nobody else, because a broadcast here would publish the
    // base's access list to the shard.
    act(
        &mut core,
        1,
        ActionMsg::Access {
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_XLO,
            op: sim_core::deploy::ACCESS_OP_ENTER,
            code: 1234,
        },
    );
    seen.clear();
    pump_seen(&mut core, &stats, &mut clients, &mut seen);
    let auths: Vec<_> = seen
        .iter()
        .filter(|(_, m)| matches!(m, protocol::EventMsg::Auth { .. }))
        .collect();
    assert_eq!(auths.len(), 1, "a grant is unicast, not broadcast");
    assert_eq!(auths[0].0, 1usize, "and it went to the wrong client");
    assert_eq!(
        clients[1].1.pop_auth().map(|a| a.4),
        Some(sim_core::lock::GRANT_FULL),
        "the main code grants full rights"
    );
    assert!(
        !core
            .world
            .deploys
            .find(CX, CZ, 0, LOC_EDGE_XLO)
            .expect("door in the world")
            .open,
        "entering a code is not opening a door — it is being remembered"
    );

    // The use action opens it — for everyone, including the bystander.
    act(
        &mut core,
        0,
        ActionMsg::Use {
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_XLO,
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
            .find(CX, CZ, 0, LOC_EDGE_XLO)
            .expect("door in the world")
            .open
    );
    for (_, c) in &clients {
        let rec = c
            .deploys
            .entries()
            .iter()
            .find(|r| r.loc == LOC_EDGE_XLO)
            .expect("door in the mirror");
        assert!(rec.open, "the open state never crossed");
        assert_eq!(
            c.pieces.cols().get(CX, CZ).shut_xlo & 1,
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
                        .any(|r| r.loc == LOC_EDGE_XLO && r.open && r.locked && r.has_lock)
        )),
        "the sync walk must carry all three of the door's bits"
    );
    let late = &clients[2].1;
    let rec = late
        .deploys
        .entries()
        .iter()
        .find(|r| r.loc == LOC_EDGE_XLO)
        .expect("late joiner missed the door");
    assert!(rec.open, "the walk carried the door shut");
    assert!(rec.locked, "the walk lost the door's lock bit");
    assert_eq!(
        late.pieces.cols().get(CX, CZ).shut_xlo & 1,
        0,
        "late joiner's predictor sealed a door that stands open"
    );

    // The owner unlocks it: everyone hears, and the leaf does not move.
    act(
        &mut core,
        0,
        ActionMsg::Access {
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_XLO,
            op: sim_core::deploy::ACCESS_OP_UNLOCK,
            code: 0,
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
            .find(|r| r.loc == LOC_EDGE_XLO)
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
            loc: LOC_EDGE_XLO,
        },
    );
    pump(&mut core, &stats, &mut clients);
    assert!(
        !core
            .world
            .deploys
            .find(CX, CZ, 0, LOC_EDGE_XLO)
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

/// The addresses a piece store holds, sorted — the only thing the wire
/// carries about a piece besides its row, so it is what "the mirror agrees
/// with the world" can honestly mean here (hp and the upkeep clock are
/// sim-only).
fn addrs(recs: &[sim_core::build::PieceRec]) -> Vec<(u16, u16, u8, u8, u8)> {
    let mut v: Vec<_> = recs
        .iter()
        .map(|r| (r.cx, r.cz, r.level, r.loc, r.row))
        .collect();
    v.sort_unstable();
    v
}

/// **A removal storm must not walk a joining client back to the start.**
///
/// The piece store swap-removes: taking entry `i` out moves the store's
/// *last* entry into the hole. A sync walk reading upward cannot trust its
/// cursor across that — the entry that moved can land below the cursor,
/// where the walk will never look again — and the wire layer's answer was
/// to zero the cursor and re-send the world from scratch. Correct, and
/// unbounded: a full walk is `len / PIECE_SYNC_BATCH` ticks, and a raid
/// removes pieces faster than that, so every client with a walk in flight
/// is restarted every tick and **no client ever converges**
/// (`reference/NETWORK.md` §9.2.1). The walk goes downward now, so the
/// entry that moves is always one already sent and the cursor survives.
///
/// The fixture is a 196-piece base whose upkeep clocks are staggered an
/// hour apart, so decay takes it down over eight consecutive ticks —
/// peaking at a tick that removes a whole sync batch, which is the
/// condition that makes the amplifier bite — instead of in one cliff. It
/// stops with pieces still standing, so the convergence asserted at the
/// end is convergence on a world with something in it.
///
/// Asserted on client state and on counters, never on the mechanism: no
/// client sees a reset batch after its first, no walk restarts, no client
/// ever loses mirror ground it had gained, both walks report completing,
/// and both mirrors end address-exact with the world.
#[test]
fn a_removal_storm_leaves_every_walk_standing() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    core.world.deploy = DeployContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;

    // The base, built before anyone is connected so that every client in
    // this test is a joiner meeting a world that is already there. It is
    // laid by `build::place` against a spare world record rather than by
    // the action lane, because 196 placements are 196 ticks that way and
    // the verb's rules are somebody else's gate (`build_wire.rs`).
    //
    // `place` stamps the piece's upkeep hour from the tick it is handed,
    // and that is the whole fixture: a piece with no hearth over it rots
    // at a fixed rate from the hour it was placed, so staggering the hour
    // staggers the death. The first 64 share hour 0 and go together (the
    // cliff a raid's first breach looks like); the rest follow an hour
    // apart (the long tail a raid actually is).
    let builder = sim_core::limits::MAX_PLAYERS - 1;
    let mut n = 0u16;
    let mut place = |core: &mut ShardCore, hour: u64| {
        let (cx, cz) = (CX + n / 24, CZ + n % 24);
        n += 1;
        let (ax, az) = sim_core::build::anchor(cx, cz, LOC_PLANE);
        let p = &mut core.world.players[builder];
        p.body = sim_core::movement::Body::at(SEED, ax, az);
        p.inv[0] = ItemStack { item: 0, count: 10 };
        sim_core::build::place(
            SEED,
            &core.world.build,
            &core.world.deploys,
            &mut core.world.pieces,
            &mut core.world.players[builder],
            hour * UPKEEP_PERIOD_TICKS,
            0,
            cx,
            cz,
            0,
            LOC_PLANE,
            &mut core.world.events,
        );
    };
    for _ in 0..64 {
        place(&mut core, 0);
    }
    for hour in 1..=11u64 {
        for _ in 0..12 {
            place(&mut core, hour);
        }
    }
    let built = core.world.pieces.len();
    assert!(
        built > 6 * PIECE_SYNC_BATCH,
        "the fixture must outlast several batches to have a mid-walk to interrupt: {built}"
    );

    assert!(core.connect(0, id_of(0)));
    let mut clients = vec![(0usize, ClientCore::new(SEED, id_of(0), 0))];
    // The second client joins one tick in, so its walk is still in flight
    // when the storm starts — the case the whole gate is about.
    const JOIN_AT: u64 = 1;
    const STORM_LAST: u64 = 10;
    const TICKS: u64 = 20;
    let joined_at = [0u64, JOIN_AT];

    let mut storm_ticks = 0usize;
    let mut peak_removed = 0usize;
    let mut mirror_was = [0usize; 2];
    for t in 0..TICKS {
        if t == JOIN_AT {
            assert!(core.connect(1, id_of(1)));
            clients.push((1usize, ClientCore::new(SEED, id_of(1), 0)));
        }
        // One upkeep hour per tick while the storm runs, then the clock
        // stands still and what is left stops rotting.
        if t <= STORM_LAST {
            core.world.tick += UPKEEP_PERIOD_TICKS;
        }
        // The seam a tail-down walk leans on, driven where it is hardest:
        // a piece placed after **both** walks have finished lands above
        // every cursor, so nothing will ever re-derive it and the
        // EV_PIECE_PLACED broadcast is the only way it can reach a client.
        // Placed on the storm's last hour so the clock stops before its own
        // upkeep comes due, and it is standing at the end to be compared.
        if t == STORM_LAST {
            let w0 = world_slot(&core, id_of(0));
            core.world.players[w0].inv[0] = ItemStack { item: 0, count: 10 };
            act(
                &mut core,
                0,
                ActionMsg::Place {
                    row: 0,
                    cx: CX - 1,
                    cz: CZ,
                    level: 0,
                    loc: LOC_PLANE,
                },
            );
        }
        let before = core.world.pieces.len();
        let flags = pump(&mut core, &stats, &mut clients);
        let removed = before.saturating_sub(core.world.pieces.len());
        if removed > 0 {
            storm_ticks += 1;
            peak_removed = peak_removed.max(removed);
        }
        for (i, (slot, c)) in clients.iter().enumerate() {
            let now = c.pieces.len();
            // The one thing a restart cannot do without being seen: a
            // client's mirror may only shrink by removals it was told
            // about, never by the server deciding to start again.
            assert!(
                now + removed >= mirror_was[i],
                "client {slot} lost mirror ground at t={t}: {} → {now} with {removed} removed",
                mirror_was[i]
            );
            mirror_was[i] = now;
            if t > joined_at[i] {
                assert_eq!(
                    flags[*slot] & APPLIED_PIECE_RESET,
                    0,
                    "client {slot} was sent a second reset batch at t={t}"
                );
            }
        }
    }

    // The storm was real: it ran long enough to outlast a full walk of the
    // base, and it peaked at a tick removing a whole batch's worth — the
    // rate at which the old restart rule could never be outrun.
    assert!(
        storm_ticks >= built.div_ceil(PIECE_SYNC_BATCH),
        "the storm ({storm_ticks} ticks) was shorter than a walk of {built} pieces"
    );
    assert!(
        peak_removed >= PIECE_SYNC_BATCH,
        "the storm never removed a whole batch in one tick: {peak_removed}"
    );

    // It stopped with a world still standing, so nothing below is the
    // agreement of two empty sets.
    let world = addrs(core.world.pieces.entries());
    assert!(!world.is_empty(), "the storm took the whole base");
    assert!(world.len() < built, "the storm removed nothing");
    assert!(
        world.contains(&(CX - 1, CZ, 0, LOC_PLANE, 0)),
        "the late placement never landed, so the mirrors below prove nothing about it"
    );
    for (slot, c) in &clients {
        assert_eq!(
            addrs(c.pieces.entries()),
            world,
            "client {slot}'s mirror is not the world"
        );
    }

    assert_eq!(
        ShardStats::get(&stats.piece_walk_restarts),
        0,
        "a removal restarted a piece walk"
    );
    assert_eq!(
        ShardStats::get(&stats.piece_walk_completes),
        clients.len() as u64,
        "not every client's walk reached the end"
    );
    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}

/// **A cliff must not run the piece cursor off the end of the store.**
///
/// The walk above reads its store from the tail down and no longer
/// restarts on a removal, so `piece_sync_cursor` is now a count of entries
/// still *owed* against a store that shrinks under it. That leaves exactly
/// one arithmetic that can go wrong, and it lands as a slice panic on the
/// sim thread: a client one batch into its walk owes
/// `len − PIECE_SYNC_BATCH`, and a tick removing more than a batch leaves
/// that count past the end of what is there. `drip_client`'s
/// `.min(pieces.len())` is the whole defence, and the decay sweep clears
/// the bar without trying — `UPKEEP_SWEEP_PER_TICK` visits 64 entries a
/// tick against a batch of 32.
///
/// `a_removal_storm_leaves_every_walk_standing` never reaches it, and that
/// is not a fault in it: its hour-staggered base removes at most one whole
/// batch in a tick, which is exactly the rate the cursor descends at, and
/// its widest tick falls while its clients are still being handed their
/// first batches. This fixture swaps those two things out. The doomed half
/// is due in a single hour, so the sweep takes it 64 at a time instead of
/// 32; and a joiner arrives on each of the three ticks the sweep is
/// removing on, so a walk exactly one batch old meets the widest of them.
///
/// The *condition* is asserted alongside the outcome. `outran` counts the
/// ticks on which a client owed more than the store held, and zero fails
/// the test: a gate that stops reaching its own bug passes for the wrong
/// reason, which is the one failure this test exists to answer.
#[test]
fn a_cliff_cannot_run_the_piece_cursor_off_the_store() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    core.world.deploy = DeployContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;

    /// Upkeep hours the clock is walked through, one per tick, before it
    /// stops. A piece stamped at this hour is never overdue afterwards.
    const HOURS: u64 = 3;
    /// The half that rots, all of it due in hour 0 so the sweep meets a
    /// whole tick's worth of overdue entries at once.
    const DOOMED: usize = 4 * PIECE_SYNC_BATCH;
    /// The half that does not, and the reason the agreement at the end is
    /// not the agreement of two empty sets.
    const STANDING: usize = 2 * PIECE_SYNC_BATCH;

    // Laid before anyone connects, by `build::place` against a spare world
    // record for the sibling test's reason: 192 placements through the
    // action lane are 192 ticks, and the verb's rules are `build_wire.rs`'s
    // gate. The upkeep hour `place` stamps from the tick it is handed is
    // the only thing telling the two halves apart.
    let builder = sim_core::limits::MAX_PLAYERS - 1;
    for k in 0..DOOMED + STANDING {
        let (cx, cz) = (CX + (k as u16) / 24, CZ + (k as u16) % 24);
        let (ax, az) = sim_core::build::anchor(cx, cz, LOC_PLANE);
        let p = &mut core.world.players[builder];
        p.body = sim_core::movement::Body::at(SEED, ax, az);
        p.inv[0] = ItemStack { item: 0, count: 10 };
        let hour = if k < DOOMED { 0 } else { HOURS };
        sim_core::build::place(
            SEED,
            &core.world.build,
            &core.world.deploys,
            &mut core.world.pieces,
            &mut core.world.players[builder],
            hour * UPKEEP_PERIOD_TICKS,
            0,
            cx,
            cz,
            0,
            LOC_PLANE,
            &mut core.world.events,
        );
    }
    assert_eq!(
        core.world.pieces.len(),
        DOOMED + STANDING,
        "the fixture must lay every piece it asks for"
    );

    // Slot 0 is here from the start. The other three arrive on consecutive
    // ticks while the sweep is working, so one of them is always exactly
    // one batch into its walk when the widest removal lands.
    const JOINS: [u64; 3] = [3, 4, 5];
    const TICKS: u64 = 12;
    assert!(core.connect(0, id_of(0)));
    let mut clients = vec![(0usize, ClientCore::new(SEED, id_of(0), 0))];
    let joined_at = [0u64, JOINS[0], JOINS[1], JOINS[2]];

    let mut outran = 0usize;
    let mut widest = 0usize;
    let mut mirror_was = [0usize; 4];
    for t in 0..TICKS {
        if let Some(i) = JOINS.iter().position(|&j| j == t) {
            let slot = i + 1;
            assert!(core.connect(slot, id_of(slot)));
            clients.push((slot, ClientCore::new(SEED, id_of(slot), 0)));
        }
        // One upkeep hour per tick, then the clock stops and what is left
        // of the base stops rotting.
        if t < HOURS {
            core.world.tick += UPKEEP_PERIOD_TICKS;
        }
        let before = core.world.pieces.len();
        // What each walk owes going in, read where `drip_client` reads it:
        // the comparison this test is about is exactly this number against
        // the store the drip finds after the sweep has run.
        let owed: Vec<usize> = clients
            .iter()
            .map(|(slot, _)| core.clients[*slot].piece_sync_cursor)
            .collect();
        let flags = pump(&mut core, &stats, &mut clients);
        let now = core.world.pieces.len();
        let removed = before - now;
        widest = widest.max(removed);
        outran += owed.iter().filter(|&&o| o > now).count();
        for (i, (slot, c)) in clients.iter().enumerate() {
            let mirror = c.pieces.len();
            // The sibling test's rule, and it is the one a clamp could
            // quietly break: a mirror may only shrink by removals it was
            // told about.
            assert!(
                mirror + removed >= mirror_was[i],
                "client {slot} lost mirror ground at t={t}: {} → {mirror} with {removed} removed",
                mirror_was[i]
            );
            mirror_was[i] = mirror;
            if t > joined_at[i] {
                assert_eq!(
                    flags[*slot] & APPLIED_PIECE_RESET,
                    0,
                    "client {slot} was sent a second reset batch at t={t}"
                );
            }
        }
    }

    // The condition, before the outcome: a tick really did remove more
    // than one batch, and a walk really did owe more than the store held.
    assert!(
        widest > PIECE_SYNC_BATCH,
        "no tick removed more than a batch ({widest}), so no cursor could outrun the store"
    );
    assert!(
        outran > 0,
        "no walk ever owed more than the store held — the clamp was never asked anything"
    );

    // And the outcome: the clamp costs nobody an entry. Every walk finished
    // and every mirror is the world, on a world with something in it.
    let world = addrs(core.world.pieces.entries());
    assert_eq!(world.len(), STANDING, "the sweep took the wrong half");
    for (slot, c) in &clients {
        assert_eq!(
            addrs(c.pieces.entries()),
            world,
            "client {slot}'s mirror is not the world"
        );
    }
    assert_eq!(
        ShardStats::get(&stats.piece_walk_completes),
        clients.len() as u64,
        "not every client's walk reached the end"
    );
    assert_eq!(
        ShardStats::get(&stats.piece_walk_restarts),
        0,
        "a removal restarted a piece walk"
    );
    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}
