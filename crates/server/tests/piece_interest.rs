//! The class-S interest gate (`NETCODE.md` §7, `reference/NETWORK.md`
//! §9.2.1): the placed-piece walk is **aimed**, and what it is aimed at is
//! class D's own AOI band rather than a second set of numbers.
//!
//! Before this, `pump_events` dripped the entire piece store to every
//! connection with no distance test anywhere — 32 records a tick, so a
//! joiner spent `MAX_PIECES / PIECE_SYNC_BATCH` = 256 ticks (8.5 s)
//! learning about every structure on the island, near or far. This suite
//! holds the four things that replace it, in the order they matter:
//!
//! 1. the guarantee — a completed walk holds **every** piece within
//!    `AOI_ENTER_CM` of the player, which is the property the whole filter
//!    exists to keep while it drops things;
//! 2. the saving, in records rather than adjectives — a far base costs a
//!    joiner zero;
//! 3. the re-arm — walk out from under the anchor and the far base
//!    arrives, including a piece placed while the client was away, which is
//!    the seam the filtered `EV_PIECE_PLACED` broadcast opens;
//! 4. the arithmetic the scan window rests on — a walk must finish before a
//!    sprinter can re-arm it, or the filter re-introduces the livelock it
//!    was written to end.
//!
//! Deterministic, no sockets: the `deploy_wire.rs` lockstep shape.

use client_core::core::{ClientCore, APPLIED_PIECE_RESET};
use protocol::{ItemCatalog, PIECE_SYNC_BATCH};
use server::core::{Lane, ShardCore};
use server::interest::{
    body_cm, cell_cm, d2_cm, PIECE_INTEREST_CM, PIECE_REARM_CM, PIECE_SCAN_BATCH,
};
use server::stats::ShardStats;
use sim_core::build::{BuildContent, LOC_PLANE};
use sim_core::deploy::DeployContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::limits::{AOI_ENTER_CM, MAX_PIECES, TICK_HZ};
use sim_core::movement::SPRINT_SPEED;

/// The solved authored sites for `seed` (the `deploy_wire.rs` memo, same
/// reason: `terrain::haven` is thousands of height taps and these suites
/// call it from inside loops).
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

const SEED: u64 = 20_260_731;
const SPAWN: (f32, f32) = (1024.0, 1024.0);
const CX: u16 = 341;
const CZ: u16 = 341;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn pump(core: &mut ShardCore, stats: &ShardStats, clients: &mut [(usize, ClientCore)]) -> [u32; 4] {
    let mut buf = [0u8; 1100];
    for (slot, c) in clients.iter_mut() {
        c.advance(1000.0 / 30.0);
        c.predict.decay_error(1000.0 / 30.0);
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

/// A world with a fixture base of `n` pieces laid around `(cx0, cz0)`,
/// before anyone is connected — so every client in these tests is a joiner
/// meeting a world that is already there.
///
/// Laid by `build::place` against a spare world record rather than through
/// the action lane, for `deploy_wire.rs`'s reason: `n` placements are `n`
/// ticks that way, and the verb's rules are somebody else's gate. The
/// upkeep hour is stamped far in the future so nothing in these tests
/// decays — a removal is a different suite's subject.
fn lay_base(core: &mut ShardCore, cx0: u16, cz0: u16, n: usize) -> Vec<(u16, u16)> {
    let builder = sim_core::limits::MAX_PLAYERS - 1;
    let mut laid = Vec::new();
    for k in 0..n {
        let (cx, cz) = (cx0 + (k as u16) / 24, cz0 + (k as u16) % 24);
        let (ax, az) = sim_core::build::anchor(cx, cz, LOC_PLANE);
        let p = &mut core.world.players[builder];
        p.body = sim_core::movement::Body::at(SEED, hv(SEED), ax, az);
        p.inv[0] = ItemStack {
            item: 0,
            count: 10,
            cond: 0,
        };
        let before = core.world.pieces.len();
        sim_core::build::place(
            SEED,
            hv(SEED),
            &core.world.build,
            &core.world.deploys,
            &mut core.world.pieces,
            &mut core.world.players[builder],
            core.world.tick,
            0,
            cx,
            cz,
            0,
            LOC_PLANE,
            false,
            &mut core.world.events,
        );
        if core.world.pieces.len() > before {
            laid.push((cx, cz));
        }
    }
    core.world.players[builder] = sim_core::world::Player::default();
    laid
}

fn fixture() -> Box<ShardCore> {
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    core.world.deploy = DeployContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    core
}

/// Move a player's body outright. Legal here for the reason
/// `deploy_wire.rs` moves its builder: the fixture is about *where a
/// client stands*, and walking one 500 m through the movement verb would
/// be a test of the movement verb.
fn stand_at(core: &mut ShardCore, id: u32, x: f32, z: f32) {
    let w = world_slot(core, id);
    core.world.players[w].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z);
}

fn player_cm(core: &ShardCore, id: u32) -> (i64, i64) {
    let b = core.world.players[world_slot(core, id)].body;
    body_cm(b.qx, b.qz)
}

/// **The guarantee, and the saving.**
///
/// One base at the spawn cell and one 500 m away — far outside
/// `PIECE_INTEREST_CM` (208 m) and outside the promise
/// (`AOI_ENTER_CM`, 176 m). A joiner standing at spawn must end holding
/// *every* piece of the near base and *no* piece of the far one, and its
/// walk must report completing: dropping the far base is only correct if
/// the walk still ends, because a walk that never ends is §9.2.1's
/// livelock wearing a filter.
///
/// The near half is the load-bearing assertion. A filter that drops too
/// much passes every byte-count test ever written and breaks the game, so
/// the count that is checked exactly is the one that must NOT move.
#[test]
fn a_far_base_costs_a_joiner_nothing_and_the_near_one_arrives_whole() {
    let stats = ShardStats::default();
    let mut core = fixture();

    let near = lay_base(&mut core, CX, CZ, 4 * PIECE_SYNC_BATCH);
    // 500 m east: 167 cells at 3 m. Far outside both radii, and the walk
    // has to scan past all of it to reach the near base, which is the
    // scan-window path this fixture exists to drive.
    const FAR_CELLS: u16 = 167;
    let far = lay_base(&mut core, CX + FAR_CELLS, CZ, 4 * PIECE_SYNC_BATCH);
    assert!(
        near.len() >= 3 * PIECE_SYNC_BATCH && far.len() >= 3 * PIECE_SYNC_BATCH,
        "the fixture must lay both bases: near {} far {}",
        near.len(),
        far.len()
    );
    let laid = core.world.pieces.len();

    assert!(core.connect(0, id_of(0)));
    let mut clients = vec![(0usize, ClientCore::new(SEED, id_of(0), 0))];
    for _ in 0..40 {
        pump(&mut core, &stats, &mut clients);
    }

    // The fixture is what it claims: the far base really is out of range
    // and the near base really is inside the promise, measured off the
    // player the server is aiming from.
    let own = player_cm(&core, id_of(0));
    for &(cx, cz) in &far {
        assert!(
            d2_cm(cell_cm(cx, cz), own) > PIECE_INTEREST_CM * PIECE_INTEREST_CM,
            "far base cell ({cx},{cz}) is inside the streaming radius"
        );
    }
    for &(cx, cz) in &near {
        assert!(
            d2_cm(cell_cm(cx, cz), own) <= AOI_ENTER_CM * AOI_ENTER_CM,
            "near base cell ({cx},{cz}) is outside the promise"
        );
    }

    let mirror = clients[0].1.pieces.entries().to_vec();
    let held: Vec<(u16, u16)> = mirror.iter().map(|r| (r.cx, r.cz)).collect();
    for &cell in &near {
        assert!(
            held.contains(&cell),
            "the promise broke: near cell {cell:?} never reached the client"
        );
    }
    for &cell in &far {
        assert!(
            !held.contains(&cell),
            "the filter leaked: far cell {cell:?} was sent"
        );
    }
    assert_eq!(
        mirror.len(),
        near.len(),
        "the mirror is the near base and nothing else"
    );
    assert!(
        laid > mirror.len(),
        "the fixture never gave the filter anything to drop"
    );

    assert_eq!(
        ShardStats::get(&stats.piece_walk_completes),
        1,
        "the walk never finished — a filtered walk that cannot end is the livelock with a filter on it"
    );
    assert!(
        ShardStats::get(&stats.piece_sync_skipped) >= far.len() as u64,
        "the skipped counter did not see the far base: {}",
        ShardStats::get(&stats.piece_sync_skipped)
    );
    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}

/// **The re-arm, and the seam it covers.**
///
/// A client that starts far from a base and walks to it must end holding
/// that base — including a piece placed while it was too far to be told
/// about, which is the hole the filtered `EV_PIECE_PLACED` broadcast would
/// otherwise leave. The re-arm is what closes it, and it is asserted on the
/// counter as well as on the outcome, because a mirror that agrees for some
/// other reason is a gate that has stopped reaching its own subject.
///
/// The late piece goes down through the **action lane**, by a second client
/// standing on the base: an event pushed straight into `world.events` by a
/// fixture never survives the next `world.tick`, so a broadcast is the only
/// way to drive the arm this test is about. The near client is therefore
/// also the control — it must receive what the far one does not.
#[test]
fn walking_to_a_base_delivers_it_including_what_was_placed_while_away() {
    let stats = ShardStats::default();
    let mut core = fixture();
    let base = lay_base(&mut core, CX, CZ, 2 * PIECE_SYNC_BATCH);
    assert!(
        base.len() >= PIECE_SYNC_BATCH,
        "fixture laid {}",
        base.len()
    );

    // Slot 0 spawns 500 m from the base, so its first walk is aimed at
    // nothing. Slot 1 spawns on it and does the building.
    let far = (SPAWN.0 + 500.0, SPAWN.1);
    core.world.dev_spawn = Some(far);
    assert!(core.connect(0, id_of(0)));
    let mut clients = vec![(0usize, ClientCore::new(SEED, id_of(0), 0))];
    // One tick with the far spawn in force: a `Join` reads `dev_spawn`
    // when the sim executes it, not when `connect` queues it, so both
    // seats would land on whichever value stood at the next tick.
    pump(&mut core, &stats, &mut clients);
    core.world.dev_spawn = Some(SPAWN);
    assert!(core.connect(1, id_of(1)));
    clients.push((1usize, ClientCore::new(SEED, id_of(1), 0)));
    for _ in 0..20 {
        pump(&mut core, &stats, &mut clients);
    }
    let own0 = player_cm(&core, id_of(0));
    for &(cx, cz) in &base {
        assert!(
            d2_cm(cell_cm(cx, cz), own0) > PIECE_INTEREST_CM * PIECE_INTEREST_CM,
            "the far seat is not actually far from base cell ({cx},{cz})"
        );
    }
    assert_eq!(
        clients[0].1.pieces.len(),
        0,
        "a base 500 m away reached a standing client"
    );
    assert_eq!(
        clients[1].1.pieces.len(),
        base.len(),
        "the client standing on the base did not get it"
    );
    assert_eq!(
        ShardStats::get(&stats.piece_walk_completes),
        2,
        "a walk never finished"
    );

    // One more piece, placed by the near client while the far one is still
    // too far to hear about it. Nothing re-derives this: it lands above
    // every cursor, so the broadcast is its only path — and the broadcast
    // is filtered.
    let w1 = world_slot(&core, id_of(1));
    core.world.players[w1].inv[0] = ItemStack {
        item: 0,
        count: 10,
        cond: 0,
    };
    let late = (CX - 1, CZ);
    assert!(core.wants_action(1), "hand should be open");
    core.push_action(
        1,
        protocol::ActionMsg::Place {
            row: 0,
            cx: late.0,
            cz: late.1,
            level: 0,
            loc: LOC_PLANE,
            freehand: false,
        },
    );
    let skipped_before = ShardStats::get(&stats.piece_events_skipped);
    pump(&mut core, &stats, &mut clients);
    assert!(
        core.world
            .pieces
            .find(late.0, late.1, 0, LOC_PLANE)
            .is_some(),
        "the late placement was refused, so nothing below proves anything"
    );
    assert_eq!(
        clients[1].1.pieces.len(),
        base.len() + 1,
        "the broadcast never reached the client standing beside it"
    );
    assert_eq!(
        clients[0].1.pieces.len(),
        0,
        "a filtered broadcast reached a client 500 m away"
    );
    assert_eq!(
        ShardStats::get(&stats.piece_events_skipped),
        skipped_before + 1,
        "the placement was sent to the far client rather than filtered"
    );

    // Now walk over. One teleport is more than `PIECE_REARM_CM`, so the
    // next tick re-arms; the walk then runs to completion from the new
    // anchor.
    let rearms_before = ShardStats::get(&stats.piece_walk_rearms);
    stand_at(&mut core, id_of(0), SPAWN.0, SPAWN.1);
    let mut resets = 0u32;
    for _ in 0..20 {
        let flags = pump(&mut core, &stats, &mut clients);
        resets += u32::from(flags[0] & APPLIED_PIECE_RESET != 0);
    }
    assert_eq!(
        ShardStats::get(&stats.piece_walk_rearms),
        rearms_before + 1,
        "moving out from under the anchor did not re-arm the walk exactly once"
    );
    assert_eq!(
        resets, 0,
        "a re-arm sent a reset batch — the client must keep what it holds"
    );

    let held: Vec<(u16, u16)> = clients[0]
        .1
        .pieces
        .entries()
        .iter()
        .map(|r| (r.cx, r.cz))
        .collect();
    for cell in base.iter().chain(core::iter::once(&late)) {
        assert!(
            held.contains(cell),
            "walking to the base did not deliver {cell:?}"
        );
    }
    assert_eq!(
        held.len(),
        base.len() + 1,
        "the re-armed walk delivered something the base does not have"
    );
    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}

/// **A standing client's walk costs nothing once it is done.**
///
/// The filter's other half: a completed walk sends no records at all, so
/// the per-tick cost of a client that has arrived is zero regardless of how
/// big the world is. Asserted on the bytes the server actually put on the
/// lane, not on a counter.
#[test]
fn a_finished_walk_says_nothing_more() {
    let stats = ShardStats::default();
    let mut core = fixture();
    lay_base(&mut core, CX, CZ, 2 * PIECE_SYNC_BATCH);
    assert!(core.connect(0, id_of(0)));
    let mut clients = vec![(0usize, ClientCore::new(SEED, id_of(0), 0))];
    for _ in 0..40 {
        pump(&mut core, &stats, &mut clients);
    }
    let held = clients[0].1.pieces.len();
    assert!(held > 0, "the fixture delivered nothing to measure against");

    let mut syncs = 0usize;
    for _ in 0..10 {
        let mut buf = [0u8; 1100];
        let (slot, c) = &mut clients[0];
        c.advance(1000.0 / 30.0);
        c.predict.decay_error(1000.0 / 30.0);
        let n = c.poll_input(&mut buf);
        if n > 0 {
            let dg = protocol::decode_input(&buf[..n]).expect("client encodes valid input");
            core.push_input(*slot, &dg);
        }
        core.tick_bare(&stats, |lane, _, bytes| {
            if lane == Lane::Event {
                if let Ok(protocol::EventMsg::PieceSync { .. }) = protocol::decode_event(bytes) {
                    syncs += 1;
                }
            }
            true
        });
    }
    assert_eq!(
        syncs, 0,
        "a settled client is still being sent piece batches"
    );
    assert_eq!(clients[0].1.pieces.len(), held);
}

/// **The scan window must outrun a sprinter, or the filter re-introduces
/// the livelock.**
///
/// A walk that cannot finish before the player travels `PIECE_REARM_CM` is
/// re-armed forever and never converges — §9.2.1's failure with a new
/// cause. The inequality is held here against the constants it is derived
/// from rather than restated in a comment, so moving `MAX_PIECES`,
/// `SPRINT_SPEED`, `TICK_HZ` or either AOI radius reddens rather than
/// drifts.
#[test]
fn a_full_walk_finishes_before_a_sprinter_can_re_arm_it() {
    let ticks = MAX_PIECES.div_ceil(PIECE_SCAN_BATCH) as f32;
    let travelled_cm = ticks / TICK_HZ as f32 * SPRINT_SPEED * 100.0;
    assert!(
        travelled_cm < PIECE_REARM_CM as f32,
        "a full walk is {ticks} ticks, in which a sprinter covers {travelled_cm:.0} cm \
         against a {PIECE_REARM_CM} cm re-arm threshold — the walk cannot converge"
    );
    // And with margin, not by a centimetre: the bound above is the floor,
    // and a walk that only just clears it converges only for a player who
    // never turns around.
    assert!(
        travelled_cm * 4.0 < PIECE_REARM_CM as f32,
        "the scan window clears its own floor by less than 4x: {travelled_cm:.0} cm"
    );
    // The batch is the wire's and the window is the work's; a window under
    // the batch would mean a tick that cannot fill a message it is allowed
    // to send.
    const { assert!(PIECE_SCAN_BATCH >= PIECE_SYNC_BATCH) };
}
