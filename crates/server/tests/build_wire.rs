//! The build-on-the-wire gate (M1): `ShardCore` ↔ `ClientCore` through
//! real encoded bytes — the piece-def table drips to a joiner, a place
//! action pays its cost and broadcasts the piece to everyone, a refusal
//! reaches the placer with its reason, a late joiner receives the placed
//! set by the sync walk, and the broadcast/walk overlap dedups on the
//! client. Deterministic, no sockets; asserts are structural and exact.

use client_core::core::{
    ClientCore, APPLIED_BUILD_REFUSED, APPLIED_PIECES, APPLIED_PIECE_DEFS, APPLIED_PIECE_RESET,
};
use protocol::{ActionMsg, ItemCatalog};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::build::{BuildContent, LOC_EDGE_XLO, LOC_PLANE, REFUSE_B_SPOT, REFUSE_B_TIER};
use sim_core::gather::GatherContent;

const SEED: u64 = 20_260_731;
/// The canonical dev spawn point, guarded walkable in sim-core
/// `world::tests` — walkable terrain is also foundation-buildable.
const SPAWN: (f32, f32) = (1024.0, 1024.0);
/// The spawn point's build cell: (1024 m, 1024 m) / 3 m.
const CX: u16 = 341;
const CZ: u16 = 341;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

/// One lockstep pump (the craft_wire shape): inputs in, tick, events and
/// snapshots back out through real bytes. Returns per-slot APPLIED flags.
fn pump(core: &mut ShardCore, stats: &ShardStats, clients: &mut [(usize, ClientCore)]) -> [u32; 4] {
    pump_filter(core, stats, clients, |_, _| true)
}

fn pump_filter(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    mut accept: impl FnMut(usize, &[u8]) -> bool,
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
            Lane::Event => {
                if !accept(slot, bytes) {
                    return false;
                }
                events.push((slot, bytes.to_vec()));
            }
        }
        true
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

fn world_slot(core: &ShardCore, id: u32) -> usize {
    core.world
        .players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("player in world")
}

/// Round-trip a decoded action the way the net loop does.
fn act(core: &mut ShardCore, slot: usize, a: ActionMsg) {
    assert!(core.wants_action(slot), "hand should be open");
    core.push_action(slot, a);
}

#[test]
fn build_rides_the_wire() {
    let fixture = BuildContent::probe_fixture();
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = fixture;
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];

    // Join drip: the piece-def table reaches both clients row-exact.
    let mut defs_flags = 0u32;
    for _ in 0..4 {
        defs_flags |= pump(&mut core, &stats, &mut clients)[0];
    }
    assert_ne!(
        defs_flags & APPLIED_PIECE_DEFS,
        0,
        "piece defs never dripped"
    );
    for (_, c) in &clients {
        assert_eq!(c.piece_defs.piece_count, fixture.piece_count);
        assert_eq!(c.piece_defs_have, fixture.piece_count);
        for i in 0..fixture.piece_count as usize {
            assert_eq!(c.piece_defs.pieces[i], fixture.pieces[i], "row {i} drifted");
        }
    }

    // Grant the builder its materials server-side (gather_wire covers how
    // resources are earned; this gate is about the build lane).
    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].inv[0] = sim_core::gather::ItemStack {
        item: 0,
        count: 20,
        cond: 0,
        skin: 0,
    };

    // Foundation at the spawn cell: the piece lands for the placer AND
    // the bystander (placed pieces broadcast), and the cost is paid.
    act(
        &mut core,
        0,
        ActionMsg::Place {
            row: 0,
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_PLANE,
            freehand: false,
            plate: 0,
        },
    );
    let flags = pump(&mut core, &stats, &mut clients);
    assert_ne!(flags[0] & APPLIED_PIECES, 0, "placer never saw the piece");
    assert_ne!(
        flags[1] & APPLIED_PIECES,
        0,
        "broadcast missed the bystander"
    );
    for (_, c) in &clients {
        assert_eq!(c.pieces.len(), 1);
        let rec = c.pieces.entries()[0];
        assert_eq!(
            (rec.cx, rec.cz, rec.level, rec.loc, rec.row),
            (CX, CZ, 0, LOC_PLANE, 0)
        );
    }
    assert!(core.wants_action(0), "the tick should consume the action");
    assert_eq!(
        sim_core::craft::inv_count(&core.world.players[w0].inv, 0),
        15,
        "foundation cost unpaid"
    );

    // A wall on the foundation's low-x edge rides the same lane.
    act(
        &mut core,
        0,
        ActionMsg::Place {
            row: 1,
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_XLO,
            freehand: false,
            plate: 0,
        },
    );
    let flags = pump(&mut core, &stats, &mut clients);
    assert_ne!(flags[1] & APPLIED_PIECES, 0, "wall missed the bystander");
    assert_eq!(clients[1].1.pieces.len(), 2);

    // Placing into the occupied plane refuses, to the placer only.
    act(
        &mut core,
        0,
        ActionMsg::Place {
            row: 0,
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_PLANE,
            freehand: false,
            plate: 0,
        },
    );
    let flags = pump(&mut core, &stats, &mut clients);
    assert_ne!(flags[0] & APPLIED_BUILD_REFUSED, 0, "refusal never arrived");
    assert_eq!(
        clients[0].1.pop_build_refusal(),
        Some(REFUSE_B_SPOT as u8),
        "wrong refusal reason"
    );
    assert_eq!(
        clients[1].1.pop_build_refusal(),
        None,
        "refusal leaked to a bystander"
    );
    assert_eq!(core.world.pieces.len(), 2, "refusal placed something");

    // A late joiner receives the placed set by the sync walk (reset batch
    // first), and the walk's overlap with past broadcasts dedups: the
    // placer's mirror holds exactly the world's two pieces after its own
    // walk catches up.
    assert!(core.connect(2, id_of(2)));
    clients.push((2usize, ClientCore::new(SEED, id_of(2), 0)));
    let mut late_flags = 0u32;
    for _ in 0..4 {
        late_flags |= pump(&mut core, &stats, &mut clients)[2];
    }
    assert_ne!(late_flags & APPLIED_PIECE_RESET, 0, "no reset batch");
    let c2 = &clients[2].1;
    assert_eq!(c2.pieces.len(), 2, "late joiner missed the placed set");
    for (_, c) in &clients {
        assert_eq!(c.pieces.len(), 2, "broadcast/walk overlap duplicated");
    }

    // Nothing in this run tripped an encoder range check.
    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}

/// The upgrade lane end to end: a wood wall re-rows into stone on the
/// wire, for the upgrader and for a bystander, without becoming a second
/// piece — and a rung that isn't above the wall's own bounces back to the
/// asker alone. The client mirror's collision index must follow the row,
/// since a stone wall blocks exactly what the wood one did.
#[test]
fn upgrade_rides_the_wire() {
    let fixture = BuildContent::probe_fixture();
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = fixture;
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

    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].inv[0] = sim_core::gather::ItemStack {
        item: 0,
        count: 20,
        cond: 0,
        skin: 0,
    };
    core.world.players[w0].inv[1] = sim_core::gather::ItemStack {
        item: 1,
        count: 10,
        cond: 0,
        skin: 0,
    };
    for (row, loc) in [(0u16, LOC_PLANE), (1u16, LOC_EDGE_XLO)] {
        act(
            &mut core,
            0,
            ActionMsg::Place {
                row,
                cx: CX,
                cz: CZ,
                level: 0,
                loc,
                freehand: false,
                plate: 0,
            },
        );
        pump(&mut core, &stats, &mut clients);
    }
    assert_eq!(core.world.pieces.len(), 2);
    let sealed = clients[0].1.pieces.cols().get(CX, CZ);

    // Row 4 is the fixture's stone rung for the wall shape.
    act(
        &mut core,
        0,
        ActionMsg::Upgrade {
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_XLO,
            material: sim_core::build::MAT_STONE,
        },
    );
    let flags = pump(&mut core, &stats, &mut clients);
    assert_ne!(
        flags[0] & APPLIED_PIECES,
        0,
        "upgrader never saw the re-row"
    );
    assert_ne!(
        flags[1] & APPLIED_PIECES,
        0,
        "the re-row missed the bystander"
    );
    assert_eq!(core.world.pieces.len(), 2, "an upgrade added a piece");
    assert_eq!(
        core.world
            .pieces
            .find(CX, CZ, 0, LOC_EDGE_XLO)
            .expect("wall stands")
            .row,
        4
    );
    assert_eq!(
        sim_core::craft::inv_count(&core.world.players[w0].inv, 1),
        6,
        "the stone rung's cost went unpaid"
    );
    for (slot, c) in &clients {
        assert_eq!(c.pieces.len(), 2, "a mirror grew a second piece");
        let rec = c
            .pieces
            .entries()
            .iter()
            .find(|r| r.loc == LOC_EDGE_XLO)
            .unwrap_or_else(|| panic!("slot {slot} lost the wall"));
        assert_eq!(rec.row, 4, "slot {slot} still mirrors the wood row");
        assert_eq!(
            c.pieces.cols().get(CX, CZ),
            sealed,
            "slot {slot}: the shape held, so the collision masks must too"
        );
    }

    // Asking for wood back bounces, to the asker alone, and changes
    // nothing.
    act(
        &mut core,
        0,
        ActionMsg::Upgrade {
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_EDGE_XLO,
            material: sim_core::build::MAT_WOOD,
        },
    );
    let flags = pump(&mut core, &stats, &mut clients);
    assert_ne!(flags[0] & APPLIED_BUILD_REFUSED, 0, "no refusal arrived");
    assert_eq!(
        clients[0].1.pop_build_refusal(),
        Some(REFUSE_B_TIER as u8),
        "wrong refusal reason"
    );
    assert_eq!(
        clients[1].1.pop_build_refusal(),
        None,
        "refusal leaked to a bystander"
    );
    assert_eq!(
        core.world.pieces.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().row,
        4
    );

    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}

/// Exercise the encoded intent, not just the server's decoded-action seam.
fn rotate(core: &mut ShardCore, slot: usize, loc: u8) {
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    let n = protocol::encode_action_rotate(CX, CZ, 0, loc, &mut buf).unwrap();
    act(core, slot, protocol::decode_action(&buf[..n]).unwrap());
}

fn rotation_fixture() -> (Box<ShardCore>, ShardStats, Vec<(usize, ClientCore)>) {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    let stair_row = core.world.build.piece_count;
    core.world.build.pieces[stair_row as usize] = sim_core::build::PieceDef {
        shape: sim_core::build::SHAPE_STAIRS,
        ..core.world.build.pieces[1]
    };
    core.world.build.piece_count += 1;
    core.world.dev_spawn = Some(SPAWN);
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0, ClientCore::new(SEED, id_of(0), 0)),
        (1, ClientCore::new(SEED, id_of(1), 0)),
    ];
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients);
    }
    let builder = world_slot(&core, id_of(0));
    core.world.players[builder].inv[0] = sim_core::gather::ItemStack {
        item: 0,
        count: 20,
        cond: 0,
        skin: 0,
    };
    for (row, loc) in [
        (0, LOC_PLANE),
        (1, LOC_EDGE_XLO),
        (stair_row, sim_core::build::LOC_RISER),
    ] {
        act(
            &mut core,
            0,
            ActionMsg::Place {
                row,
                cx: CX,
                cz: CZ,
                level: 0,
                loc,
                freehand: false,
                plate: 1,
            },
        );
        pump(&mut core, &stats, &mut clients);
        assert!(core.world.pieces.find(CX, CZ, 0, loc).is_some());
    }
    // Damage is fixture setup. Rotation's broadcast must derive the band
    // from authoritative hp, just as a late joiner's full sync does.
    let mut budget = sim_core::limits::MAX_REMOVALS_PER_TICK;
    for i in 1..3 {
        assert!(!sim_core::deploy::damage_piece(
            &core.world.deploy,
            &core.world.build,
            &mut core.world.pieces,
            &mut core.world.deploys,
            i,
            40,
            &mut budget,
            &mut core.world.events,
        ));
    }
    (core, stats, clients)
}

fn assert_rotation_mirrors(core: &ShardCore, clients: &[(usize, ClientCore)]) {
    for (slot, client) in clients {
        assert_eq!(client.pieces.len(), core.world.pieces.len());
        for rec in core.world.pieces.entries() {
            let mirrored = client
                .pieces
                .entries()
                .iter()
                .find(|r| (r.cx, r.cz, r.level, r.loc) == (rec.cx, rec.cz, rec.level, rec.loc))
                .unwrap_or_else(|| panic!("slot {slot} lost loc {}", rec.loc));
            assert_eq!(mirrored.row, rec.row);
            assert_eq!(mirrored.facing, rec.facing);
            assert_eq!(mirrored.plate, rec.plate);
            assert_eq!(
                mirrored.dmg,
                sim_core::build::damage_band(rec.hp, core.world.build.pieces[rec.row as usize].hp),
                "slot {slot} lost the damage band at loc {}",
                rec.loc
            );
        }
        assert_eq!(
            client.pieces.cols().get(CX, CZ),
            core.world.pieces.cols().get(CX, CZ)
        );
    }
}

#[test]
fn rotation_updates_facing_stair_collision_and_late_join_damage() {
    use sim_core::build::{LOC_RISER, LOC_RISER_XHI, LOC_RISER_XLO, LOC_RISER_ZLO};
    let (mut core, stats, mut clients) = rotation_fixture();
    let initial = *core.world.pieces.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap();
    let clocks = core.world.pieces.placed().to_vec();
    rotate(&mut core, 0, LOC_EDGE_XLO);
    let flags = pump(&mut core, &stats, &mut clients);
    assert_ne!(flags[0] & APPLIED_PIECES, 0);
    assert_ne!(flags[1] & APPLIED_PIECES, 0);
    let flipped = core.world.pieces.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap();
    assert_eq!(flipped.facing, initial.facing ^ 1);
    assert_eq!(flipped.hp, initial.hp);
    for (_, client) in &clients {
        let wall = client
            .pieces
            .entries()
            .iter()
            .find(|r| r.loc == LOC_EDGE_XLO)
            .unwrap();
        assert_eq!(wall.facing, flipped.facing);
        assert_ne!(wall.dmg, 0, "a facing update visually healed the wall");
    }

    rotate(&mut core, 0, LOC_RISER);
    let flags = pump(&mut core, &stats, &mut clients);
    for flag in flags.iter().take(2) {
        assert_ne!(flag & client_core::core::APPLIED_PIECE_REMOVED, 0);
        assert_ne!(flag & APPLIED_PIECES, 0);
    }
    assert!(core.world.pieces.find(CX, CZ, 0, LOC_RISER).is_none());
    assert!(core.world.pieces.find(CX, CZ, 0, LOC_RISER_XHI).is_some());
    assert_rotation_mirrors(&core, &clients);

    // Two players can address successive orientations in the same tick.
    // The intermediate placement no longer exists when fanout reads the
    // final store, but its following removal must still clear each mirror.
    rotate(&mut core, 0, LOC_RISER_XHI);
    rotate(&mut core, 1, LOC_RISER_ZLO);
    pump(&mut core, &stats, &mut clients);
    assert!(core.world.pieces.find(CX, CZ, 0, LOC_RISER_XLO).is_some());
    assert_eq!(core.world.pieces.placed(), clocks);
    assert_rotation_mirrors(&core, &clients);

    assert!(core.connect(2, id_of(2)));
    clients.push((2, ClientCore::new(SEED, id_of(2), core.world.tick as u32)));
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients);
    }
    assert_rotation_mirrors(&core, &clients);

    // A delayed press against the old address is refused only to its sender.
    rotate(&mut core, 0, LOC_RISER);
    pump(&mut core, &stats, &mut clients);
    assert_eq!(clients[0].1.pop_build_refusal(), Some(REFUSE_B_SPOT as u8));
    assert_eq!(clients[1].1.pop_build_refusal(), None);
    assert_rotation_mirrors(&core, &clients);
    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}

#[test]
fn interrupted_rotation_pair_resyncs_the_whole_piece_set() {
    use sim_core::build::{LOC_RISER, LOC_RISER_XHI, LOC_RISER_ZLO};
    let (mut core, stats, mut clients) = rotation_fixture();
    rotate(&mut core, 0, LOC_EDGE_XLO);
    pump(&mut core, &stats, &mut clients);
    for (drop_removal, from, to) in [
        (true, LOC_RISER, LOC_RISER_XHI),
        (false, LOC_RISER_XHI, LOC_RISER_ZLO),
    ] {
        rotate(&mut core, 0, from);
        let mut blocked = false;
        pump_filter(&mut core, &stats, &mut clients, |slot, bytes| {
            if slot != 1 {
                return true;
            }
            if blocked {
                return false;
            }
            let event = protocol::decode_event(bytes).unwrap();
            blocked = if drop_removal {
                matches!(event, protocol::EventMsg::PieceRemoved { loc, .. } if loc == from)
            } else {
                matches!(event, protocol::EventMsg::PiecePlaced { rec } if rec.loc == to)
            };
            !blocked
        });
        assert!(blocked, "the intended half of the rotation never arrived");
        let mut flags = 0;
        for _ in 0..4 {
            flags |= pump(&mut core, &stats, &mut clients)[1];
        }
        assert_ne!(flags & APPLIED_PIECE_RESET, 0);
        assert_rotation_mirrors(&core, &clients);
    }
    assert!(ShardStats::get(&stats.ev_resyncs) >= 2);
    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}

#[test]
fn rotating_then_demolishing_stairs_preserves_the_raised_base_collision() {
    use sim_core::build::{LOC_RISER, LOC_RISER_XHI};
    let (mut core, stats, mut clients) = rotation_fixture();
    rotate(&mut core, 0, LOC_EDGE_XLO);
    pump(&mut core, &stats, &mut clients);
    let plate = core.world.pieces.cols().get(CX, CZ).plate;
    assert_ne!(plate, 0, "the regression needs a raised supporting plane");

    rotate(&mut core, 0, LOC_RISER);
    act(
        &mut core,
        1,
        ActionMsg::Demolish {
            deploy: false,
            cx: CX,
            cz: CZ,
            level: 0,
            loc: LOC_RISER_XHI,
        },
    );
    pump(&mut core, &stats, &mut clients);
    assert_eq!(core.world.pieces.len(), 2);
    assert!(core.world.pieces.find(CX, CZ, 0, LOC_RISER).is_none());
    assert!(core.world.pieces.find(CX, CZ, 0, LOC_RISER_XHI).is_none());
    for (slot, client) in &clients {
        assert_eq!(
            client.pieces.cols().get(CX, CZ).plate,
            plate,
            "slot {slot}: a transient stair overwrote the surviving foundation's collision height"
        );
    }
    assert_rotation_mirrors(&core, &clients);
    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}
