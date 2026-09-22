//! The trust ledger's sink (`server::trustlog`), end to end: `ShardCore`'s
//! tick → `Tap::drain` → the writer thread → the files → the reader.
//!
//! What each test holds, and the mutant it was watched going red under, is
//! in its doc comment. No test here waits on a clock. Where a test must wait
//! for the writer thread, it waits on the writer's own counter (or its
//! `close` line) and the deadline is only a guard against hanging.

use protocol::{ActionMsg, ItemCatalog};
use server::core::ShardCore;
use server::stats::ShardStats;
use server::store::PlayerKey;
use server::trustlog::{
    self, read_dir, summarize, Filter, Gap, Limits, Msg, Party, Role, Row, ShardIdent, Tap,
    TrustLog, Who, WriterStats, TRUST_LOG_FORMAT, TRUST_TAP_RING_CAP,
};
use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::deploy::{box_key, DeployContent, DeployDef, ARCH_BOX, PLACE_FOUNDATION};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::inventory::{CONT_BOX, CONT_SELF};
use sim_core::limits::MAX_PLAYERS;
use sim_core::movement::Body;
use sim_core::world::{
    Command, PRESENCE_ASLEEP, PRESENCE_AWAKE, PRESENCE_GONE, TRUST_AUTH, TRUST_CONT, TRUST_DOOR,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SEED: u64 = 20_260_922;
const OWNER_WALLET: &str = "0x00112233445566778899aabbccddeeff00112233";
const OUTSIDER_WALLET: &str = "0xffeeddccbbaa99887766554433221100ffeeddcc";

/// The fixture's box: `raid_storm.rs`'s row, appended at 8 / item 9.
const BOX_ROW: u16 = 8;
const BOX_ITEM: u16 = 9;
const GOODS: u16 = 7;
const WAIT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn key(s: &str) -> PlayerKey {
    PlayerKey::new(s.as_bytes()).expect("a legal key")
}

fn id_of(generation: u32, slot: usize) -> u32 {
    (generation << 8) | slot as u32
}

fn scratch(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("gates-trust-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

fn ident() -> ShardIdent {
    ShardIdent {
        domain: "trust.test".into(),
        seed: SEED,
        content: 0x0123_4567_89ab_cdef,
        world: 0xfeed,
        build: "0.0.0-gtest".into(),
        proto: protocol::PROTO_VER,
    }
}

/// Wait until the writer has written `n` rows. Asserts on the writer's own
/// counter; the deadline only stops a broken writer from hanging the test.
fn wait_written(log: &TrustLog, n: u64) {
    let end = Instant::now() + WAIT;
    while WriterStats::get(&log.stats.rows_written) < n {
        assert!(
            Instant::now() < end,
            "the writer wrote {} of {n} rows",
            WriterStats::get(&log.stats.rows_written)
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Stop a writer by dropping its tap, and wait for its `close` line.
fn close(tap: Tap, log: &mut TrustLog) {
    drop(tap);
    assert!(log.join_within(WAIT), "the writer never wrote its close line");
}

/// One row as the e2e test compares it: tick, verb, presence, actor id,
/// actor wallet, actor-is-guest, counterparty id, counterparty wallet.
type Seen<'a> = (u64, &'a str, &'a str, u32, Option<&'a str>, bool, u32, Option<&'a str>);

fn party(id: u32, who: Who) -> Party {
    Party { id, who }
}

fn row(tick: u64, actor: u32, counterparty: u32) -> Msg {
    Msg::Row(Row {
        tick,
        verb: TRUST_CONT,
        presence: PRESENCE_AWAKE,
        actor: party(actor, Who::Guest),
        counterparty: party(counterparty, Who::Wallet(key(OWNER_WALLET))),
    })
}

fn dir_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .expect("read the log dir")
        .map(|e| e.expect("entry").metadata().expect("metadata").len())
        .sum()
}

fn deploy_with_box() -> DeployContent {
    let mut d = DeployContent::probe_fixture();
    d.defs[BOX_ROW as usize] = DeployDef {
        arch: ARCH_BOX,
        placement: PLACE_FOUNDATION,
        hp: 60,
        item: BOX_ITEM,
        ..DeployDef::INERT
    };
    d.def_count = 9;
    d
}

/// `n` buildable cells in a ring scan out from the middle of the map, two
/// cells apart so no two boxes share an address.
fn cells(n: usize, haven: &sim_core::terrain::Haven) -> Vec<(u16, u16)> {
    let mut out = Vec::new();
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if (dx.abs() != r && dz.abs() != r) || dx % 2 != 0 || dz % 2 != 0 {
                    continue;
                }
                let (cx, cz) = ((512 + dx) as u16, (512 + dz) as u16);
                let (x, z) = center(cx, cz);
                if foundation_terrain_ok(SEED, haven, x, z) && !out.contains(&(cx, cz)) {
                    out.push((cx, cz));
                    if out.len() == n {
                        return out;
                    }
                }
            }
        }
    }
    panic!("seed {SEED} offered fewer than {n} buildable cells near the middle");
}

fn center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

/// A shard with the fixtures a box needs, everybody spawning on `at`.
fn shard(at: (u16, u16)) -> Box<ShardCore> {
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    core.world.deploy = deploy_with_box();
    core.world.dev_spawn = Some(center(at.0, at.1));
    core.catalog = ItemCatalog::EMPTY;
    core
}

fn tick(core: &mut ShardCore, stats: &ShardStats) {
    core.tick_bare(stats, |_, _, _| true);
}

/// One action from one connection, applied by one `ShardCore` tick.
fn act(core: &mut ShardCore, stats: &ShardStats, slot: usize, a: ActionMsg) {
    assert!(core.wants_action(slot), "slot {slot} has a hand free");
    core.push_action(slot, a);
    tick(core, stats);
}

fn wslot(core: &ShardCore, id: u32) -> usize {
    core.world
        .players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("a seated player")
}

fn stand(core: &mut ShardCore, haven: &sim_core::terrain::Haven, id: u32, at: (u16, u16)) {
    let (x, z) = center(at.0, at.1);
    let s = wslot(core, id);
    core.world.players[s].body = Body::at(SEED, haven, x, z);
}

/// `owner` (on connection `slot`) stands a foundation and a box at `at`,
/// through the wire's own actions, and the box is filled with goods.
fn stand_a_box(
    core: &mut ShardCore,
    stats: &ShardStats,
    haven: &sim_core::terrain::Haven,
    slot: usize,
    owner: u32,
    at: (u16, u16),
) -> u32 {
    stand(core, haven, owner, at);
    let s = wslot(core, owner);
    core.world.players[s].inv[0] = ItemStack {
        item: 0,
        count: 100,
        cond: 0,
    };
    core.world.players[s].inv[1] = ItemStack {
        item: BOX_ITEM,
        count: 5,
        cond: 0,
    };
    let (cx, cz) = at;
    act(
        core,
        stats,
        slot,
        ActionMsg::Place {
            row: 0,
            cx,
            cz,
            level: 0,
            loc: LOC_PLANE,
            freehand: false,
            plate: 0,
        },
    );
    act(
        core,
        stats,
        slot,
        ActionMsg::Deploy {
            row: BOX_ROW,
            cx,
            cz,
            level: 0,
            loc: LOC_PLANE,
        },
    );
    let k = box_key(cx, cz, 0);
    let bi = core.world.deploys.box_index(k).expect("the box stood up");
    assert_eq!(core.world.deploys.boxes()[bi].owner, owner);
    core.world.deploys.set_box_slot(
        bi,
        0,
        ItemStack {
            item: GOODS,
            count: 50,
            cond: 0,
        },
    );
    k
}

/// One unit out of box `cont`, by whoever is on connection `slot`.
fn withdraw(core: &mut ShardCore, stats: &ShardStats, slot: usize, cont: u32) {
    act(
        core,
        stats,
        slot,
        ActionMsg::Move {
            cont,
            from_kind: CONT_BOX,
            from_slot: 0,
            to_kind: CONT_SELF,
            to_slot: 20,
            count: 1,
        },
    );
}

// ---------------------------------------------------------------------------
// The shard's own path
// ---------------------------------------------------------------------------

/// **The shard drains its trust rows into the log, with wallets.** Through
/// `ShardCore::tick` and nothing else: a keyed outsider and a guest each
/// withdraw from a keyed owner's box while the owner watches, then again
/// after the owner logs off. Every row is on disk in order, with the right
/// ids, wallets, guest mark, verb, presence and tick.
///
/// The outsider joins **after** the tick-0 cadence refresh and acts on its
/// first tick in the world, so its wallet can only come from the refresh a
/// tick with rows forces. With every body joined on tick 0 that path was
/// never needed, and the mutant that deleted it survived.
///
/// Mutants watched red: the hook line removed from `core.rs` (no rows on
/// disk); `party` returning `Guest` before the identity tables (wallets
/// missing); identities refreshed on the cadence only (the late joiner is
/// logged as a guest).
#[test]
fn a_shard_tick_logs_trust_rows_with_their_wallets() {
    let dir = scratch("e2e");
    let haven = sim_core::terrain::haven(SEED);
    let at = cells(1, &haven)[0];
    let stats = ShardStats::default();
    let mut core = shard(at);
    let (tap, mut log) = trustlog::spawn(&dir, ident(), Limits::default()).expect("spawn");
    core.trust = tap;

    let (owner, outsider, guest) = (id_of(1, 0), id_of(1, 1), id_of(1, 2));
    assert!(core
        .connect_as(0, owner, Some(key(OWNER_WALLET)), None)
        .is_some());
    assert!(core.connect_as(2, guest, None, None).is_some());
    tick(&mut core, &stats);
    let cont = stand_a_box(&mut core, &stats, &haven, 0, owner, at);
    stand(&mut core, &haven, guest, at);
    // The late joiner: seated between cadence refreshes.
    assert!(core
        .connect_as(1, outsider, Some(key(OUTSIDER_WALLET)), None)
        .is_some());
    tick(&mut core, &stats);
    assert!(
        !core
            .world
            .tick
            .is_multiple_of(trustlog::TRUST_IDENT_REFRESH_TICKS),
        "the fixture must join the outsider off the refresh cadence"
    );
    stand(&mut core, &haven, outsider, at);

    withdraw(&mut core, &stats, 1, cont);
    let t1 = core.world.trust.tick();
    withdraw(&mut core, &stats, 2, cont);
    let t2 = core.world.trust.tick();
    // The owner logs off: the body sleeps, and the sleeper index keeps
    // whose it is.
    assert!(core.disconnect(0).is_some());
    tick(&mut core, &stats);
    withdraw(&mut core, &stats, 1, cont);
    let t3 = core.world.trust.tick();

    let tap = std::mem::take(&mut core.trust);
    close(tap, &mut log);
    let l = read_dir(&dir).expect("read the log");
    assert_eq!(l.segments.len(), 1);
    assert!(l.segments[0].closed, "a clean stop writes a close line");
    assert!(l.segments[0].shard.contains("\"domain\":\"trust.test\""));
    let got: Vec<Seen> = l
        .rows
        .iter()
        .map(|r| {
            (
                r.tick,
                r.verb.as_str(),
                r.presence.as_str(),
                r.actor.id,
                r.actor.wallet.as_deref(),
                r.actor.guest,
                r.counterparty.id,
                r.counterparty.wallet.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (t1, "cont", "awake", outsider, Some(OUTSIDER_WALLET), false, owner, Some(OWNER_WALLET)),
            (t2, "cont", "awake", guest, None, true, owner, Some(OWNER_WALLET)),
            (t3, "cont", "asleep", outsider, Some(OUTSIDER_WALLET), false, owner, Some(OWNER_WALLET)),
        ],
        "the log must hold every trust row the shard's ticks minted, in order, \
         with each party's wallet or guest mark"
    );
    assert_eq!(ShardStats::get(&stats.trust_rows), 3);
    assert_eq!(ShardStats::get(&stats.trust_ring_drops), 0);
    assert_eq!(ShardStats::get(&stats.trust_unlogged), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// **An evicted owner keeps their wallet in the row.** A full shard, the
/// owner logs off, and a newcomer's join evicts the owner's body through
/// the real two-phase path (`ShardCore::connect_as`). The owner's base is
/// then raided with nobody there. That row is `gone`, and it still names
/// the owner's wallet, from the tap's memory of evicted bodies. A guest
/// owner evicted the same way is `unresolved`: nobody ever knew who it was.
///
/// Mutant watched red: the eviction branch of `refresh` removed (the keyed
/// row goes `unresolved`).
#[test]
fn an_evicted_owner_keeps_their_wallet_in_the_row() {
    let dir = scratch("gone");
    let haven = sim_core::terrain::haven(SEED);
    let two = cells(2, &haven);
    let stats = ShardStats::default();
    let mut core = shard(two[0]);
    let (tap, mut log) = trustlog::spawn(&dir, ident(), Limits::default()).expect("spawn");
    core.trust = tap;

    // Slot 0 a keyed owner, slot 1 a guest owner, slot 2 the raider, and
    // every other seat filled, so the world is full.
    let (owner, guest_owner, raider) = (id_of(1, 0), id_of(1, 1), id_of(1, 2));
    assert!(core
        .connect_as(0, owner, Some(key(OWNER_WALLET)), None)
        .is_some());
    for slot in 1..MAX_PLAYERS {
        let k = (slot == 2).then(|| key(OUTSIDER_WALLET));
        assert!(core.connect_as(slot, id_of(1, slot), k, None).is_some());
    }
    tick(&mut core, &stats);
    assert_eq!(core.world.players.iter().filter(|p| p.active).count(), MAX_PLAYERS);
    let keyed_box = stand_a_box(&mut core, &stats, &haven, 0, owner, two[0]);
    let guest_box = stand_a_box(&mut core, &stats, &haven, 1, guest_owner, two[1]);

    // Both owners log off, the keyed one first, so it is the longest asleep
    // and the first join evicts it.
    assert!(core.disconnect(0).is_some());
    tick(&mut core, &stats);
    assert!(core.disconnect(1).is_some());
    tick(&mut core, &stats);
    for (slot, newcomer) in [(0usize, id_of(2, 0)), (1, id_of(2, 1))] {
        let (_, evicted) = core
            .connect_as(slot, newcomer, None, None)
            .expect("the join is admitted");
        let _ = evicted;
        tick(&mut core, &stats);
    }
    assert_eq!(core.world.evictions, 2, "both owners were evicted");
    assert_eq!(core.world.presence_of(owner), PRESENCE_GONE);
    assert_eq!(core.world.presence_of(guest_owner), PRESENCE_GONE);

    stand(&mut core, &haven, raider, two[0]);
    withdraw(&mut core, &stats, 2, keyed_box);
    stand(&mut core, &haven, raider, two[1]);
    withdraw(&mut core, &stats, 2, guest_box);

    let tap = std::mem::take(&mut core.trust);
    close(tap, &mut log);
    let l = read_dir(&dir).expect("read the log");
    assert_eq!(l.rows.len(), 2, "{:?}", l.rows);
    let (a, b) = (&l.rows[0], &l.rows[1]);
    assert_eq!((a.presence.as_str(), a.counterparty.id), ("gone", owner));
    assert_eq!(
        a.counterparty.wallet.as_deref(),
        Some(OWNER_WALLET),
        "the evicted owner's wallet must survive the eviction"
    );
    assert_eq!(a.actor.wallet.as_deref(), Some(OUTSIDER_WALLET));
    assert_eq!((b.presence.as_str(), b.counterparty.id), ("gone", guest_owner));
    assert_eq!(b.counterparty.wallet, None);
    assert!(
        !b.counterparty.guest,
        "an evicted guest is unresolved: the log does not guess what it cannot know"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A shard with no log still counts every row it could not keep.
#[test]
fn a_shard_without_a_log_counts_its_rows_unlogged() {
    let haven = sim_core::terrain::haven(SEED);
    let at = cells(1, &haven)[0];
    let stats = ShardStats::default();
    let mut core = shard(at);
    assert!(!core.trust.is_on());
    let (owner, outsider) = (id_of(1, 0), id_of(1, 1));
    assert!(core.connect_as(0, owner, None, None).is_some());
    assert!(core.connect_as(1, outsider, None, None).is_some());
    tick(&mut core, &stats);
    let cont = stand_a_box(&mut core, &stats, &haven, 0, owner, at);
    stand(&mut core, &haven, outsider, at);
    withdraw(&mut core, &stats, 1, cont);
    withdraw(&mut core, &stats, 1, cont);
    assert_eq!(ShardStats::get(&stats.trust_rows), 2);
    assert_eq!(
        ShardStats::get(&stats.trust_unlogged),
        2,
        "rows with no sink must be counted, never lost silently"
    );
}

// ---------------------------------------------------------------------------
// The writer
// ---------------------------------------------------------------------------

/// **Appends survive a restart, in order.** Two boots on one directory: the
/// second starts a new segment and leaves the first alone, both segments
/// carry a versioned header with their own boot id, and the reader returns
/// every row of both in the order they were written.
#[test]
fn rows_survive_a_restart_in_order() {
    let dir = scratch("restart");
    let stats = ShardStats::default();
    let mut boots = Vec::new();
    for boot in 0..2u64 {
        let (mut tap, mut log) = trustlog::spawn(&dir, ident(), Limits::default()).expect("spawn");
        for i in 0..100u64 {
            let t = boot * 1000 + i;
            tap.offer(row(t, 300 + i as u32, 7), &stats);
        }
        wait_written(&log, 100);
        boots.push(log.boot.clone());
        close(tap, &mut log);
    }
    let l = read_dir(&dir).expect("read the log");
    assert_eq!(l.segments.len(), 2, "each boot opens its own segment");
    assert!(l.segments.iter().all(|s| s.closed && !s.torn));
    assert_eq!(
        l.segments.iter().map(|s| s.boot.clone()).collect::<Vec<_>>(),
        boots
    );
    assert_ne!(boots[0], boots[1], "two boots are told apart");
    let ticks: Vec<u64> = l.rows.iter().map(|r| r.tick).collect();
    let want: Vec<u64> = (0..2u64)
        .flat_map(|b| (0..100u64).map(move |i| b * 1000 + i))
        .collect();
    assert_eq!(ticks, want, "every row of both boots, in the order written");
    let text =
        std::fs::read_to_string(dir.join(trustlog::segment_name(l.segments[0].seq))).unwrap();
    let head: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(head["version"], TRUST_LOG_FORMAT);
    let _ = std::fs::remove_dir_all(&dir);
}

/// **A crash costs at most the line it cut.** A segment whose last line has
/// no newline, and no `close` line, is what a killed shard leaves. The
/// reader keeps every whole line, marks the segment torn and not closed,
/// and the next boot appends a fresh segment beside it.
#[test]
fn a_crash_leaves_a_torn_line_and_everything_before_it() {
    let dir = scratch("torn");
    let stats = ShardStats::default();
    let (mut tap, mut log) = trustlog::spawn(&dir, ident(), Limits::default()).expect("spawn");
    for i in 0..10u64 {
        tap.offer(row(i, 400, 7), &stats);
    }
    wait_written(&log, 10);
    close(tap, &mut log);
    // Rewrite the segment as a crash would have left it: no close line, and
    // half of an eleventh row.
    let path = dir.join(trustlog::segment_name(log.segment));
    let text = std::fs::read_to_string(&path).unwrap();
    let mut kept: Vec<&str> = text.lines().filter(|l| !l.contains("\"close\"")).collect();
    let mut half = String::new();
    trustlog::format_msg(&row(10, 400, 7), &mut half);
    let cut = &half[..half.len() / 2];
    kept.push(cut);
    std::fs::write(&path, kept.join("\n")).unwrap();

    let (tap, mut log2) = trustlog::spawn(&dir, ident(), Limits::default()).expect("respawn");
    close(tap, &mut log2);
    let l = read_dir(&dir).expect("a torn segment still reads");
    assert_eq!(l.segments.len(), 2);
    assert!(l.segments[0].torn && !l.segments[0].closed);
    assert!(!l.segments[1].torn && l.segments[1].closed);
    assert_eq!(
        l.rows.iter().map(|r| r.tick).collect::<Vec<_>>(),
        (0..10).collect::<Vec<u64>>(),
        "every whole line before the cut survives, and the cut line is not a row"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **The directory stays inside its bound.** With kilobyte limits the
/// writer rotates many times. The directory never holds more than
/// `max_bytes` plus one write batch. The oldest segments are deleted, and
/// each deletion is named in a `pruned` line. What survives is a contiguous
/// run of the newest rows, ending at the last one written.
///
/// Mutant watched red: the prune loop's condition inverted (the directory
/// grows without bound).
#[test]
fn rotation_keeps_the_directory_inside_its_bound() {
    let dir = scratch("rotate");
    let stats = ShardStats::default();
    let limits = Limits {
        segment_bytes: 8_192,
        max_bytes: 32_768,
        sync_ms: 1_000,
        poll_ms: 1,
    };
    let (mut tap, mut log) = trustlog::spawn(&dir, ident(), limits).expect("spawn");
    let total = 4_000u64;
    for chunk in 0..(total / 500) {
        for i in 0..500 {
            let t = chunk * 500 + i;
            tap.offer(row(t, 500 + (t % 7) as u32, 7), &stats);
        }
        wait_written(&log, (chunk + 1) * 500);
        let on_disk = dir_bytes(&dir);
        assert!(
            on_disk <= limits.max_bytes + 65_536 + 1_024,
            "{on_disk} bytes on disk against a bound of {} plus one batch",
            limits.max_bytes
        );
    }
    close(tap, &mut log);
    assert_eq!(ShardStats::get(&stats.trust_ring_drops), 0, "no loss to the ring");
    let l = read_dir(&dir).expect("read the log");
    assert!(
        WriterStats::get(&log.stats.segments_pruned) > 0,
        "the fixture never rotated past the bound"
    );
    assert_eq!(
        l.pruned.len() as u64,
        WriterStats::get(&log.stats.segments_pruned) - pruned_before_the_oldest_survivor(&l),
        "every deletion the surviving segments witnessed is named in a pruned line"
    );
    let ticks: Vec<u64> = l.rows.iter().map(|r| r.tick).collect();
    assert_eq!(*ticks.last().unwrap(), total - 1, "the newest row survives");
    assert!(ticks[0] > 0, "the oldest rows were rotated away");
    assert!(
        ticks.windows(2).all(|w| w[1] == w[0] + 1),
        "what survives is contiguous: rotation deletes whole segments from the front"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `pruned` lines live in the segment that was current when its deletion
/// happened, and that segment may itself have been pruned since. Count the
/// deletions whose witness is gone.
fn pruned_before_the_oldest_survivor(l: &trustlog::Log) -> u64 {
    let oldest = l.segments[0].seq;
    // Segment n's opening pruned segments older than n; the survivors hold
    // the lines for every deletion from `oldest`'s opening onwards.
    let named: std::collections::BTreeSet<&str> = l.pruned.iter().map(|(n, _)| n.as_str()).collect();
    (1..oldest)
        .filter(|s| !named.contains(trustlog::segment_name(*s).as_str()))
        .count() as u64
}

/// **Back-pressure is counted, and the loss is written where it happened.**
/// A ring of eight with no writer draining it: twenty rows offered, eight
/// ride, twelve are dropped and counted. Once the ring drains, the next
/// offer writes a `gap` naming the twelve and their tick range *before* the
/// row that follows, so the log's order is still true.
///
/// Mutant watched red: `offer` pushing rows while a gap is pending (the row
/// arrives ahead of the gap that precedes it).
#[test]
fn back_pressure_is_counted_and_written_as_a_gap() {
    let stats = ShardStats::default();
    let (mut tap, mut rx) = Tap::channel(8);
    for t in 0..20 {
        tap.offer(row(t, 600, 7), &stats);
    }
    assert_eq!(ShardStats::get(&stats.trust_ring_drops), 12);
    let mut seen = Vec::new();
    while let Ok(m) = rx.pop() {
        seen.push(m);
    }
    tap.offer(row(20, 600, 7), &stats);
    while let Ok(m) = rx.pop() {
        seen.push(m);
    }
    let mut want: Vec<Msg> = (0..8).map(|t| row(t, 600, 7)).collect();
    want.push(Msg::Gap(Gap {
        from_tick: 8,
        to_tick: 19,
        ring_full: 12,
        sim_overflow: 0,
    }));
    want.push(row(20, 600, 7));
    assert_eq!(seen, want);
}

/// **Under a live consumer, back-pressure never reorders the log.** One tick,
/// one row, 200 000 ticks into a four-slot ring while another thread drains
/// it. Whatever the interleaving, what arrives must partition the ticks:
/// monotonic, each tick either a row or inside exactly one gap, and each
/// gap's count equal to its span. So no row ever lands ahead of the gap for
/// rows lost before it.
///
/// Correct code cannot fail this, whatever the scheduler does. The mutant it
/// exists for can only show under a race: a row pushed while a gap is
/// pending, into a slot the consumer freed between the two pushes. The
/// single-threaded test above is blind to it by construction, and survived
/// it. Here it goes red on most runs, never on a correct one.
#[test]
fn back_pressure_never_reorders_the_log_under_a_live_consumer() {
    const TICKS: u64 = 200_000;
    let stats = ShardStats::default();
    let (mut tap, mut rx) = Tap::channel(4);
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader = {
        let done = done.clone();
        std::thread::spawn(move || {
            let mut seen = Vec::new();
            loop {
                match rx.pop() {
                    Ok(m) => seen.push(m),
                    Err(_) if done.load(std::sync::atomic::Ordering::Acquire) => {
                        while let Ok(m) = rx.pop() {
                            seen.push(m);
                        }
                        return seen;
                    }
                    Err(_) => std::hint::spin_loop(),
                }
            }
        })
    };
    for t in 0..TICKS {
        tap.offer(row(t, 700, 7), &stats);
    }
    // The last gap, if one is pending, gets the room the reader makes. The
    // reader pops until told to stop, so this ends on state, not on a count.
    while tap.gap_pending() {
        tap.flush_gap();
        std::thread::yield_now();
    }
    done.store(true, std::sync::atomic::Ordering::Release);
    let seen = reader.join().expect("the reader thread");

    let mut next = 0u64;
    let mut lost = 0u64;
    for m in &seen {
        match m {
            Msg::Row(r) => {
                assert_eq!(r.tick, next, "a row out of order: the log was reordered");
                next += 1;
            }
            Msg::Gap(g) => {
                assert_eq!(
                    g.from_tick, next,
                    "a gap that does not start where the rows stopped"
                );
                assert_eq!(
                    g.ring_full,
                    g.to_tick - g.from_tick + 1,
                    "a gap whose count is not its span: rows were pushed while it waited"
                );
                lost += g.ring_full;
                next = g.to_tick + 1;
            }
        }
    }
    assert_eq!(next, TICKS, "every tick is a row or inside a gap");
    assert_eq!(lost, ShardStats::get(&stats.trust_ring_drops));
    assert!(lost > 0, "the fixture never filled the ring, so it proved nothing");
}

// ---------------------------------------------------------------------------
// The reader
// ---------------------------------------------------------------------------

/// **The reader round-trips, and filters without ranking anybody.** Rows
/// of three verbs, three presences, two wallets and a guest are written and
/// read back field for field, then selected by wallet (either side, or one
/// side), by player, verb, presence, tick range and boot. The summary counts
/// per verb and per verb × presence. The CLI bin prints the same summary.
#[test]
fn the_reader_round_trips_and_filters() {
    let dir = scratch("reader");
    let stats = ShardStats::default();
    let (mut tap, mut log) = trustlog::spawn(&dir, ident(), Limits::default()).expect("spawn");
    let (o, x) = (key(OWNER_WALLET), key(OUTSIDER_WALLET));
    let rows = [
        (10, TRUST_DOOR, PRESENCE_ASLEEP, party(2, Who::Wallet(x)), party(1, Who::Wallet(o))),
        (11, TRUST_AUTH, PRESENCE_GONE, party(2, Who::Wallet(x)), party(1, Who::Wallet(o))),
        (12, TRUST_CONT, PRESENCE_AWAKE, party(3, Who::Guest), party(1, Who::Wallet(o))),
        (13, TRUST_DOOR, PRESENCE_AWAKE, party(1, Who::Wallet(o)), party(2, Who::Wallet(x))),
        (14, TRUST_CONT, PRESENCE_GONE, party(2, Who::Wallet(x)), party(9, Who::Unresolved)),
    ];
    for (tick, verb, presence, actor, counterparty) in rows {
        tap.offer(
            Msg::Row(Row {
                tick,
                verb,
                presence,
                actor,
                counterparty,
            }),
            &stats,
        );
    }
    wait_written(&log, rows.len() as u64);
    close(tap, &mut log);

    let l = read_dir(&dir).expect("read the log");
    assert_eq!(l.rows.len(), 5);
    assert_eq!(l.rows[0].verb, "door");
    assert_eq!(l.rows[0].presence, "asleep");
    assert_eq!(l.rows[0].actor.wallet.as_deref(), Some(OUTSIDER_WALLET));
    assert!(l.rows[2].actor.guest);
    assert_eq!(l.rows[4].counterparty.wallet, None);
    assert!(!l.rows[4].counterparty.guest);

    let pick = |f: Filter| l.rows.iter().filter(|r| f.matches(r)).map(|r| r.tick).collect::<Vec<_>>();
    let upper = OWNER_WALLET.to_ascii_uppercase().replace("0X", "0x");
    assert_eq!(
        pick(Filter {
            wallet: Some(upper),
            ..Filter::default()
        }),
        vec![10, 11, 12, 13],
        "a wallet matches either side, case-insensitively"
    );
    assert_eq!(
        pick(Filter {
            wallet: Some(OWNER_WALLET.into()),
            role: Role::Actor,
            ..Filter::default()
        }),
        vec![13]
    );
    assert_eq!(
        pick(Filter {
            player: Some(2),
            role: Role::Counterparty,
            ..Filter::default()
        }),
        vec![13]
    );
    assert_eq!(
        pick(Filter {
            verb: Some("cont".into()),
            ..Filter::default()
        }),
        vec![12, 14]
    );
    assert_eq!(
        pick(Filter {
            presence: Some("gone".into()),
            from_tick: Some(12),
            ..Filter::default()
        }),
        vec![14]
    );
    assert_eq!(
        pick(Filter {
            boot: Some(log.boot[..4].to_string()),
            to_tick: Some(11),
            ..Filter::default()
        }),
        vec![10, 11]
    );
    let s = summarize(l.rows.iter());
    assert_eq!(s.rows, 5);
    assert_eq!(s.by_verb["door"], 2);
    assert_eq!(s.by_verb["cont"], 2);
    assert_eq!(s.by_verb_presence[&("door".into(), "asleep".into())], 1);

    // The bin, on the same directory.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_trust-log"))
        .arg(&dir)
        .args(["--wallet", OUTSIDER_WALLET, "--summary"])
        .output()
        .expect("run trust-log");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("-- 4 of 5 rows"), "{text}");
    assert!(text.contains("door") && text.contains("asleep 1"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A segment written by a newer format is refused by name, never misread.
#[test]
fn a_newer_format_is_refused_rather_than_misread() {
    let mut l = trustlog::Log::default();
    let head = format!(
        "{{\"kind\":\"header\",\"format\":\"{}\",\"version\":{}}}\n",
        trustlog::TRUST_LOG_NAME,
        TRUST_LOG_FORMAT + 1
    );
    let err = trustlog::read_segment(1, head.as_bytes(), &mut l).unwrap_err();
    assert!(err.contains("format version"), "{err}");
}

// ---------------------------------------------------------------------------
// Walls 2 and 3 on the sim thread's half
// ---------------------------------------------------------------------------

mod alloc {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! { pub static COUNT: Cell<Option<usize>> = const { Cell::new(None) }; }

    pub struct Counting;
    fn bump() {
        let _ = COUNT.try_with(|c| {
            if let Some(n) = c.get() {
                c.set(Some(n + 1));
            }
        });
    }
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            bump();
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            bump();
            unsafe { System.dealloc(ptr, layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            bump();
            unsafe { System.realloc(ptr, layout, size) }
        }
    }
}

#[global_allocator]
static ALLOCATOR: alloc::Counting = alloc::Counting;

/// **The drain allocates nothing** (wall 2), counted on the calling thread
/// only, so the writer thread's formatting does not blur the number. Every
/// measured drain carries a row, refreshes the identity tables and resolves
/// two wallets, which is the most work a drain does.
///
/// Mutant watched red: `refresh` collecting into a fresh `Vec`.
#[test]
fn the_drain_allocates_nothing() {
    let haven = sim_core::terrain::haven(SEED);
    let at = cells(1, &haven)[0];
    let stats = ShardStats::default();
    let mut core = shard(at);
    let (tap, mut rx) = Tap::channel(TRUST_TAP_RING_CAP);
    core.trust = tap;
    let (owner, outsider) = (id_of(1, 0), id_of(1, 1));
    assert!(core
        .connect_as(0, owner, Some(key(OWNER_WALLET)), None)
        .is_some());
    assert!(core
        .connect_as(1, outsider, Some(key(OUTSIDER_WALLET)), None)
        .is_some());
    tick(&mut core, &stats);
    let cont = stand_a_box(&mut core, &stats, &haven, 0, owner, at);
    stand(&mut core, &haven, outsider, at);
    let take = Command::Move {
        id: outsider,
        cont,
        from_kind: CONT_BOX,
        from_slot: 0,
        to_kind: CONT_SELF,
        to_slot: 20,
        count: 1,
    };
    let mut worst = 0;
    for _ in 0..20 {
        core.world.tick(&[take]);
        assert_eq!(core.world.trust.len(), 1, "the tick minted its row");
        alloc::COUNT.with(|c| c.set(Some(0)));
        Tap::drain(&mut core, &stats);
        let n = alloc::COUNT.with(|c| c.replace(None)).unwrap_or(0);
        worst = worst.max(n);
        while rx.pop().is_ok() {}
    }
    assert_eq!(worst, 0, "a trust drain allocated {worst} times on the sim thread");
    assert_eq!(ShardStats::get(&stats.trust_ring_drops), 0);
}
