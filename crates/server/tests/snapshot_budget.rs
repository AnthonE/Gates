//! `test_snapshot_budget` (DESIGN.md §12): the worst-case scene — the
//! full shard cap clustered inside one AOI cell — and per-client snapshots
//! must hold the 1100 B budget, keep every interest entity inside the
//! staleness ceiling, always carry the client's own entity, and
//! reconstruct byte-exact through the client view once acks flow. All
//! counted/structural asserts: identical on this box and the reference
//! VPS.

use protocol::InputDatagram;
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use server::view::{Applied, ClientView};
use sim_core::limits::{
    DATAGRAM_BUDGET_BYTES, MAX_PLAYERS, SNAPSHOT_INTERVAL_TICKS, STALENESS_CEILING,
};

const SEED: u64 = 0xB1D_6E75;

/// Player id for connection slot `s` as `install` mints them (generation 1).
fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

/// A core with the full cap connected and every body herded into a 60 m
/// square (well inside the 176 m enter radius of everyone).
fn clustered_core(stats: &ShardStats) -> ShardCore {
    let mut core = ShardCore::new(SEED);
    for slot in 0..MAX_PLAYERS {
        assert!(core.connect(slot, id_of(slot)), "connect {slot}");
        if (slot + 1) % 32 == 0 || slot + 1 == MAX_PLAYERS {
            // Command budget reserves headroom; land joins in batches.
            core.tick(stats, |_, _, _| true);
        }
    }
    for (i, p) in core.world.players.iter_mut().enumerate() {
        if !p.active {
            continue;
        }
        // 10×10 grid, 6 m pitch, island interior.
        p.body.qx = ((900.0 + (i % 10) as f32 * 6.0) / 0.03) as i32;
        p.body.qz = ((900.0 + (i / 10) as f32 * 6.0) / 0.03) as i32;
    }
    core
}

/// Run until the next snapshot cadence tick, collecting (slot, bytes).
fn snapshot_round(core: &mut ShardCore, stats: &ShardStats) -> Vec<(usize, Vec<u8>)> {
    let mut out = Vec::new();
    loop {
        let mut sent = Vec::new();
        core.tick(stats, |lane, slot, bytes| {
            if lane == Lane::Snapshot {
                sent.push((slot, bytes.to_vec()));
            }
            true
        });
        if core.world.tick.is_multiple_of(SNAPSHOT_INTERVAL_TICKS) {
            out.extend(sent);
            return out;
        }
    }
}

#[test]
fn test_snapshot_budget() {
    let stats = ShardStats::default();
    let mut core = clustered_core(&stats);
    let mut views: Vec<ClientView> = (0..MAX_PLAYERS).map(|_| ClientView::new()).collect();

    // Phase 1 — no acks: every snapshot is zero-state absolute, the
    // hardest byte shape.
    let rounds: usize = 12;
    for round in 0..rounds {
        let sent = snapshot_round(&mut core, &stats);
        assert_eq!(
            sent.len(),
            MAX_PLAYERS,
            "round {round}: every connected client gets a snapshot"
        );
        for (slot, bytes) in &sent {
            assert!(
                bytes.len() <= DATAGRAM_BUDGET_BYTES,
                "round {round} slot {slot}: {} B blows the budget",
                bytes.len()
            );
            let view = &mut views[*slot];
            match view.apply(bytes).expect("server datagram decodes") {
                Applied::Ok { .. } => {}
                other => panic!("round {round} slot {slot}: {other:?}"),
            }
            // Own entity rides every snapshot (reconciliation needs it).
            assert!(
                view.get(id_of(*slot)).is_some(),
                "round {round} slot {slot}: own entity missing"
            );
            // Reconstructed states match the world exactly — quantize
            // both sides means equality, not tolerance.
            for (eid, state) in view.entities.iter() {
                let p = core
                    .world
                    .players
                    .iter()
                    .find(|p| p.active && p.id == *eid)
                    .unwrap_or_else(|| panic!("unknown entity {eid} in view"));
                assert_eq!(state.qx, p.body.qx, "qx of {eid}");
                assert_eq!(state.qy, p.body.qy, "qy of {eid}");
                assert_eq!(state.qz, p.body.qz, "qz of {eid}");
            }
        }
    }
    // The staleness ceiling (NETCODE.md §3): with 99 clustered peers and
    // ~38 absolute records per datagram, entities rotate — but no offered
    // entity may go more than STALENESS_CEILING snapshots unsent. During
    // this ack-free phase a snapshot's applied set IS its content (a
    // zero-state apply rebuilds the map), so the union over any CEILING
    // consecutive snapshots must cover every peer.
    let ceiling = STALENESS_CEILING as usize;
    let mut union: Vec<std::collections::BTreeSet<u32>> = vec![Default::default(); MAX_PLAYERS];
    let mut zero_len = 0usize;
    let mut zero_count = 0usize;
    for _ in 0..ceiling {
        let sent = snapshot_round(&mut core, &stats);
        for (slot, bytes) in &sent {
            zero_len += bytes.len();
            zero_count += 1;
            views[*slot].apply(bytes).expect("decodes");
            union[*slot].extend(views[*slot].entities.iter().map(|(id, _)| *id));
        }
    }
    for (slot, seen) in union.iter().enumerate() {
        assert_eq!(
            seen.len(),
            MAX_PLAYERS,
            "slot {slot}: staleness ceiling broken — {} of {} entities inside {} snapshots",
            seen.len(),
            MAX_PLAYERS,
            ceiling
        );
    }

    // Phase 2 — acks flow: deltas engage and shrink the wire.
    let mut delta_len = 0usize;
    let mut delta_rounds = 0usize;
    for _round in 0..8 {
        // Every client acks its newest applied snapshot.
        for (slot, view) in views.iter().enumerate() {
            let (ack, bits) = view.ack_fields();
            let dg = InputDatagram::new(ack, bits, 1);
            core.push_input(slot, &dg);
        }
        let sent = snapshot_round(&mut core, &stats);
        for (slot, bytes) in &sent {
            assert!(bytes.len() <= DATAGRAM_BUDGET_BYTES);
            let applied = views[*slot].apply(bytes).expect("decodes");
            if let Applied::Ok { delta: true } = applied {
                delta_len += bytes.len();
                delta_rounds += 1;
            }
            for (eid, state) in views[*slot].entities.iter() {
                let p = core
                    .world
                    .players
                    .iter()
                    .find(|p| p.active && p.id == *eid)
                    .expect("entity exists");
                assert_eq!(
                    (state.qx, state.qy, state.qz),
                    (p.body.qx, p.body.qy, p.body.qz),
                    "delta reconstruction of {eid}"
                );
            }
        }
    }
    assert!(
        delta_rounds >= MAX_PLAYERS * 4,
        "deltas engaged on {delta_rounds} snapshots only"
    );
    let avg_zero = zero_len / zero_count;
    let avg_delta = delta_len / delta_rounds;
    assert!(
        avg_delta < avg_zero,
        "delta snapshots ({avg_delta} B avg) not smaller than absolute ({avg_zero} B avg)"
    );

    // The encoder never met an out-of-range body or refused a begin.
    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
    assert_eq!(ShardStats::get(&stats.forced_resyncs), 0);
}

/// Shedding is the designed degradation and it must be **visible**
/// (`reference/NETWORK.md` §9.2.3 · §3). A snapshot that cannot carry every
/// ranked entity drops the tail rather than fragmenting — correct, and until
/// `snap_entities_shed` existed it was the only path in the pipeline by
/// which quality falls under load with nothing recording it.
///
/// Two-sided on purpose, which is the whole point of the test: a counter
/// that only ever goes up proves nothing, because a `bump` on the wrong line
/// would pass a one-sided assert. So the same core shape is driven twice —
/// clustered, where 99 peers cannot fit in 1,100 B and the counter must
/// move; and sparse, where nothing is offered that does not fit and the
/// counter must not move at all.
#[test]
fn the_shed_counter_sees_the_budget_refuse() {
    // Clustered: the full cap inside one AOI cell. `test_snapshot_budget`
    // measures ~38 absolute records per datagram against 99 peers, so the
    // fill refuses on every snapshot of every client.
    let stats = ShardStats::default();
    let mut core = clustered_core(&stats);
    for _ in 0..4 {
        snapshot_round(&mut core, &stats);
    }
    let shed = ShardStats::get(&stats.snap_entities_shed);
    assert!(
        shed > 0,
        "the worst-case scene shed nothing — either the fill stopped \
         refusing or the counter is not wired to it"
    );
    // Sanity on the magnitude, not a pinned number: each of the 100 clients
    // is offered 99 peers per snapshot and can carry a few dozen, so four
    // snapshot rounds shed hundreds at minimum. A counter reading 1 would
    // satisfy `> 0` and mean the bump landed inside a branch it should not.
    assert!(
        shed > MAX_PLAYERS as u64,
        "shed {shed} across 4 rounds of {MAX_PLAYERS} clients is too low to \
         be the tail of a refused fill"
    );

    // Sparse: two players 500 m apart — well past the 208 m exit — and no
    // animals, so every client's candidate list is empty and the only
    // record in a snapshot is its own entity. Nothing is offered that does
    // not fit, so nothing may be counted as shed.
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    core.tick(&stats, |_, _, _| true);
    for m in core.world.mobs.m.iter_mut() {
        // The roster is worldgen's, not this test's: an animal that happens
        // to stand near a player is a candidate, and this assertion is about
        // the counter rather than about where the pigs spawned.
        m.alive = false;
    }
    let q = |m: f32| (m / 0.03) as i32;
    for (i, p) in core.world.players.iter_mut().filter(|p| p.active).enumerate() {
        p.body.qx = q(700.0 + i as f32 * 500.0);
        p.body.qz = q(700.0);
    }
    for _ in 0..4 {
        snapshot_round(&mut core, &stats);
    }
    assert_eq!(
        ShardStats::get(&stats.snap_entities_shed),
        0,
        "nothing was offered that did not fit, so nothing may read as shed"
    );
}

/// AOI hysteresis (DESIGN.md §5.5): enter under 176 m, leave over 208 m,
/// and the band between flaps neither way. Structural, two players.
#[test]
fn test_aoi_hysteresis() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    core.tick(&stats, |_, _, _| true);

    let q = |m: f32| (m / 0.03) as i32;
    let place = |core: &mut ShardCore, x0: f32, x1: f32| {
        for p in core.world.players.iter_mut() {
            if !p.active {
                continue;
            }
            p.body.qz = q(1000.0);
            p.body.qx = if p.id == id_of(0) { q(x0) } else { q(x1) };
        }
    };
    let saw_peer = |sent: &[(usize, Vec<u8>)]| {
        let bytes = &sent.iter().find(|(s, _)| *s == 0).expect("slot 0 sent").1;
        let snap = protocol::decode_snapshot(bytes, &[]).expect("zero-state decodes");
        snap.entities().iter().any(|e| e.id == id_of(1))
    };

    // 190 m apart: outside enter radius, never subscribed.
    place(&mut core, 1000.0, 1190.0);
    let sent = snapshot_round(&mut core, &stats);
    assert!(!saw_peer(&sent), "190 m: must not enter");

    // 170 m: inside enter radius — subscribed.
    place(&mut core, 1000.0, 1170.0);
    let sent = snapshot_round(&mut core, &stats);
    assert!(saw_peer(&sent), "170 m: must enter");

    // Back to 190 m: inside the hysteresis band — stays subscribed.
    place(&mut core, 1000.0, 1190.0);
    let sent = snapshot_round(&mut core, &stats);
    assert!(saw_peer(&sent), "190 m after enter: hysteresis holds");

    // 215 m: past exit — unsubscribed.
    place(&mut core, 1000.0, 1215.0);
    let sent = snapshot_round(&mut core, &stats);
    assert!(!saw_peer(&sent), "215 m: must leave");
}
