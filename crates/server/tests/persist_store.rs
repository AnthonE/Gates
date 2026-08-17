//! The player store end to end: **a shard restart that remembers you.**
//!
//! `store.rs`'s own tests own the record's bytes and the index's arithmetic;
//! `sim-core/tests/persist.rs` owns what a restore does to the world. This file
//! owns the join of the two — the loop a player actually lives through, driven
//! through `ShardCore` exactly as `net.rs` drives it, plus every refusal a
//! real save file can hand a booting shard.
//!
//! No sockets, like every other suite here. What is real is the file: these
//! tests write one, close it, open it again and read a character back out.

use server::core::{Admitted, ShardCore};
use server::stats::ShardStats;
use server::store::{self, PlayerKey, SaveStore, MAX_SAVED_PLAYERS, SAVE_BACKUP_COUNT};
use sim_core::combat::CombatContent;
use sim_core::craft::CraftContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::movement::POS_XZ_Q;
use sim_core::persist::PlayerSave;
use std::path::{Path, PathBuf};

const SEED: u64 = 20_260_731;
const CONTENT: u64 = 0x0123_4567_89ab_cdef;
/// The dev shard's own fixture, `dev_spawn`-style: two clients on one point so
/// a restored position is distinguishable from a spawn-ring one.
const SPAWN: (f32, f32) = (1024.0, 1024.0);

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

/// The id the slot table would mint for `slot`'s **second** tenant — a
/// reconnect after the first claim, `generation << 8 | slot` with the
/// generation bumped.
fn id2_of(slot: usize) -> u32 {
    (2 << 8) | slot as u32
}

fn key(s: &str) -> PlayerKey {
    PlayerKey::new(s.as_bytes()).expect("fits")
}

/// The ceilings every `store::open` in this file validates against
/// (`server::cond`). The probe fixture, because it is what `armed_core`
/// installs — so the ceilings the boot checks are the ceilings the sim in
/// these tests actually mints under.
fn gc() -> GatherContent {
    GatherContent::probe_fixture()
}

/// A scratch path under the test binary's temp dir. Named per test so the
/// suite can run in parallel, and cleared on the way in rather than the way
/// out — a failed test that leaves its file behind is evidence.
fn scratch(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("gates-save-{name}-{}.save", std::process::id()));
    sweep(&p);
    p
}

/// Remove a save file and every numbered backup a boot may have rotated beside
/// it. Each is ~1 MB, and `store::open` makes one on every call — so a suite
/// that only deleted the live file would leave megabytes per run on a box where
/// disk has been the binding constraint.
fn sweep(path: &Path) {
    let _ = std::fs::remove_file(path);
    for n in 1..=SAVE_BACKUP_COUNT + 2 {
        let mut b = path.as_os_str().to_os_string();
        b.push(format!(".{n}"));
        let _ = std::fs::remove_file(PathBuf::from(b));
    }
}

fn armed_core() -> Box<ShardCore> {
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.combat = CombatContent::probe_fixture();
    core.world.craft = CraftContent::probe_fixture();
    core.world.gather = GatherContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core
}

fn world_slot(core: &ShardCore, id: u32) -> usize {
    core.world
        .players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("player in world")
}

/// **The whole slice, as a player experiences it.** Join, gather something,
/// walk somewhere, drop the connection, and come back to find it all there —
/// through a real file, with the shard's process gone in between.
///
/// Every hop `net.rs` makes is made here in the same order: the sim hands back
/// a record at the disconnect, the index files it under a key, the file gets
/// it, a second shard loads the file, and the record rides `Command::JoinAs`
/// into a new world.
#[test]
fn a_shard_restart_remembers_a_player() {
    let path = scratch("restart");
    let me = key("dev-anthone");

    // --- session one -----------------------------------------------------
    let (want_inv, want_body) = {
        let (saves, found) = store::open(&path, SEED, CONTENT, &gc()).expect("a fresh file opens");
        assert!(found.created, "the first open must create the file");
        assert_eq!(found.live, 0, "a new file remembers nobody");
        let mut store = saves.store;
        let mut file = saves.file;

        let stats = ShardStats::default();
        let mut core = armed_core();
        assert!(core.connect(0, id_of(0)), "a fresh join");
        core.tick_bare(&stats, |_, _, _| true);

        // Do something worth keeping.
        let slot = world_slot(&core, id_of(0));
        core.world.players[slot].inv[0] = ItemStack {
            item: 3,
            count: 128,
            cond: 0,
        };
        core.world.players[slot].hp = 37;
        core.tick_bare(&stats, |_, _, _| true);
        let want_inv = core.world.players[slot].inv;
        let want_body = core.world.players[slot].body;

        // The connection drops. This is the exact save.
        let (id, save) = core.disconnect(0).expect("a leaving player has a record");
        assert_eq!(id, id_of(0));
        let put = store.put(&me, 1_700_000_000, save);
        assert!(!put.evicted);
        file.write(put.index, &me, 1_700_000_000, &save)
            .expect("the record writes");
        (want_inv, want_body)
    }; // both halves dropped: the file is closed, the index is gone

    // --- session two: a different process, the same file -----------------
    let (saves, found) = store::open(&path, SEED, CONTENT, &gc()).expect("the file reopens");
    assert!(!found.created, "the second open must find the file");
    assert_eq!(found.corrupt, 0, "a clean write must read back clean");
    assert_eq!(found.live, 1, "the shard forgot the only player it had");

    let restored = saves.store.find(&me).expect("the key must be remembered");
    let stats = ShardStats::default();
    let mut core = armed_core();
    // The key rides along now: it is how a join finds the body it left
    // behind. There is none here — this is a fresh process against an old
    // file, which is exactly the case the record exists for
    // (`reference/SAVES.md` §9.2) — so the door taken must be `Restored`.
    assert_eq!(
        core.connect_as(0, id_of(0), Some(me), Some(restored))
            .map(|(how, _)| how),
        Some(Admitted::Restored),
        "a restoring join"
    );
    core.tick_bare(&stats, |_, _, _| true);

    let slot = world_slot(&core, id_of(0));
    let p = core.world.players[slot];
    assert_eq!(p.inv, want_inv, "the inventory did not survive the restart");
    assert_eq!(p.hp, 37, "hp did not survive the restart");
    assert_eq!(p.body, want_body, "the body woke up somewhere else");
    // And the position is genuinely the saved one, not the fixture's spawn
    // that both sessions happen to share — otherwise this test would pass on
    // a shard that restored nothing.
    let (x, z) = (p.body.qx as f32 * POS_XZ_Q, p.body.qz as f32 * POS_XZ_Q);
    assert!(
        (x - SPAWN.0).abs() < 2.0 && (z - SPAWN.1).abs() < 2.0,
        "the fixture moved: ({x}, {z})"
    );
    sweep(&path);
}

/// A player the store has never heard of gets a fresh character, and that is
/// not a failure — a first visit, a guest with no key, or a shard with no file
/// all land here, which is every join this repo made before the store existed.
#[test]
fn an_unknown_key_is_a_fresh_character() {
    let store = SaveStore::new();
    assert_eq!(store.find(&key("nobody")), None);

    let stats = ShardStats::default();
    let mut core = armed_core();
    assert_eq!(
        core.connect_as(0, id_of(0), None, None).map(|(how, _)| how),
        Some(Admitted::Fresh),
        "a keyless join"
    );
    core.tick_bare(&stats, |_, _, _| true);
    let p = core.world.players[world_slot(&core, id_of(0))];
    assert_eq!(p.hp, core.world.combat.player_hp, "a fresh body is whole");
    assert!(p.inv.iter().all(|s| s.count == 0), "naked spawn");
}

/// The autosave sweep: one player per call, skipping anyone who has not moved.
///
/// Both halves are load-bearing. One per call is what keeps the work O(1) in a
/// tick whatever the population; skip-if-unchanged is what stops an idle full
/// shard writing thirty identical records a second forever.
#[test]
fn the_autosave_sweep_is_bounded_and_skips_the_unchanged() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    assert!(core.connect(0, id_of(0)));
    core.tick_bare(&stats, |_, _, _| true);

    // A full lap of the cursor: exactly one record for the one connected
    // player, whatever else the lap visits.
    let mut taken = Vec::new();
    for _ in 0..sim_core::limits::MAX_PLAYERS {
        if let Some((id, _)) = core.autosave() {
            taken.push(id);
        }
    }
    assert_eq!(
        taken,
        vec![id_of(0)],
        "a lap must take exactly one record per connected player"
    );

    // A second lap with nothing changed: nothing to write.
    let mut again = 0;
    for _ in 0..sim_core::limits::MAX_PLAYERS {
        if core.autosave().is_some() {
            again += 1;
        }
    }
    assert_eq!(again, 0, "an idle player was saved twice");

    // Move them, and the sweep notices — through `Body`'s `Eq`, which is only
    // exact because every field of it is quantized.
    let slot = world_slot(&core, id_of(0));
    core.world.players[slot].body.qx += 1;
    let mut noticed = 0;
    for _ in 0..sim_core::limits::MAX_PLAYERS {
        if core.autosave().is_some() {
            noticed += 1;
        }
    }
    assert_eq!(noticed, 1, "a player who moved was not saved");
}

/// **The raid-then-evict window, closed — the scenario `NOW.md` §0y item 3
/// names.** A sleeper's store record is frozen at the moment they left
/// (`disconnect` reads the body before queueing the `Leave`, and the
/// autosave sweep walks *connection* slots, which a sleeper no longer
/// holds). The world file made that harmless for a restart — the body
/// itself persists — and left exactly one case: eviction. Before two-phase
/// eviction, a sleeper raided after their leave and then evicted for slot
/// pressure came back from the stale record, raid quietly undone.
///
/// This walks the whole window: leave (record frozen), raid (body
/// changes), slot pressure (eviction — and the record handed back is the
/// **current** body), rejoin (the current state comes back, not the
/// frozen one).
#[test]
fn an_evicted_sleeper_comes_back_from_the_current_body_not_the_stale_record() {
    const MAX: usize = sim_core::limits::MAX_PLAYERS;
    let stats = ShardStats::default();
    let mut core = armed_core();
    let mut store = SaveStore::new();
    let me = key("gets-raided");
    let carried = ItemStack {
        item: 3,
        count: 128,
        cond: 0,
    };

    // Session one: join keyed, carry something, log off. The disconnect's
    // exact save is the frozen record — the last one any sweep will take,
    // because the sweep walks connection slots and this player no longer
    // has one.
    assert_eq!(
        core.connect_as(0, id_of(0), Some(me), None)
            .map(|(how, _)| how),
        Some(Admitted::Fresh)
    );
    core.tick_bare(&stats, |_, _, _| true);
    let slot = world_slot(&core, id_of(0));
    core.world.players[slot].inv[0] = carried;
    let (_, frozen) = core.disconnect(0).expect("a leaving player has a record");
    assert_eq!(frozen.inv[0], carried);
    store.put(&me, 1, frozen);
    core.tick_bare(&stats, |_, _, _| true);
    assert_eq!(core.world.sleepers(), 1);

    // The raid, after the record froze: the sleeper is looted where it
    // stands. From here the record and the body disagree.
    core.world.players[slot].inv[0] = ItemStack::default();

    // Slot pressure: fill every remaining world slot with awake players.
    for s in 1..MAX {
        assert!(core.connect(s, id_of(s)), "filler {s}");
    }
    core.tick_bare(&stats, |_, _, _| true);
    assert_eq!(core.world.sleepers(), 1, "the fill must not evict");

    // The join that needs the sleeper's slot. Phase one hands back the
    // victim's record **off the live body** — the raided state, not the
    // frozen one — keyed, because the victim has no connection slot for
    // the accept loop to resolve.
    let stranger = id2_of(0);
    let (how, evicted) = core
        .connect_as(0, stranger, Some(key("newcomer")), None)
        .expect("admitted");
    assert_eq!(how, Admitted::Fresh);
    let (victim_key, current) = evicted.expect("a full shard must evict, keyed");
    assert_eq!(victim_key, me, "the record is filed under the victim's key");
    assert_eq!(
        current.inv[0],
        ItemStack::default(),
        "the eviction save is the frozen record, not the raided body"
    );
    // File it exactly as `drain_saves` would: by key, over the frozen one.
    store.put(&me, 2, current);
    core.tick_bare(&stats, |_, _, _| true);
    assert_eq!(
        core.world.evictions, 1,
        "the evict must land under the join"
    );
    assert_eq!(core.world.sleepers(), 0);
    assert!(
        core.world
            .players
            .iter()
            .any(|p| p.active && p.id == stranger),
        "the join must land on the freed slot"
    );

    // The victim returns. No body (evicted), so the store's record — and
    // it is the current one. The connection slot comes from a filler
    // logging off; its keyless sleeper is the next eviction's victim,
    // which also pins the guest arm: no key, no record, still evicted.
    core.disconnect(5);
    core.tick_bare(&stats, |_, _, _| true);
    let restored = store.find(&me).expect("the eviction filed a record");
    assert_eq!(restored.inv[0], ItemStack::default());
    let (how, evicted) = core
        .connect_as(5, id2_of(5), Some(me), Some(restored))
        .expect("admitted");
    assert_eq!(
        how,
        Admitted::Restored,
        "an evicted sleeper comes back through the store"
    );
    assert!(
        evicted.is_none(),
        "a keyless victim has no record to file — and never had one"
    );
    core.tick_bare(&stats, |_, _, _| true);
    let p = core.world.players[world_slot(&core, id2_of(5))];
    assert_eq!(
        p.inv[0],
        ItemStack::default(),
        "the rejoin restored the stale pre-raid record — the raid was undone"
    );
    assert_eq!(core.world.evictions, 2);
}

/// Two joins in one window must nominate two different victims. Both picks
/// happen against the same un-ticked world, so without the queue-aware
/// arithmetic (`ShardCore::slots_short` / `spoken_for`) both would name the
/// same longest-asleep body: the first `Evict` frees one slot, the first
/// join takes it, the second `Evict` misses, and the second join lands on a
/// full world and seats nobody.
#[test]
fn two_joins_in_one_window_evict_two_different_sleepers() {
    const MAX: usize = sim_core::limits::MAX_PLAYERS;
    let stats = ShardStats::default();
    let mut core = armed_core();
    let (a, b) = (key("older-sleeper"), key("newer-sleeper"));

    // Two keyed sleepers, slept in a known order — a's body first, so it
    // is the longest asleep and must be the first pick.
    assert!(core.connect_as(0, id_of(0), Some(a), None).is_some());
    assert!(core.connect_as(1, id_of(1), Some(b), None).is_some());
    core.tick_bare(&stats, |_, _, _| true);
    core.disconnect(0);
    core.tick_bare(&stats, |_, _, _| true);
    core.disconnect(1);
    core.tick_bare(&stats, |_, _, _| true);
    assert_eq!(core.world.sleepers(), 2);

    for s in 2..MAX {
        assert!(core.connect(s, id_of(s)), "filler {s}");
    }
    core.tick_bare(&stats, |_, _, _| true);

    // One window, no tick between: the second pick must see the first's
    // queued `Evict` and pass over its victim.
    let (_, first) = core
        .connect_as(0, id2_of(0), Some(key("stranger-one")), None)
        .expect("admitted");
    let (_, second) = core
        .connect_as(1, id2_of(1), Some(key("stranger-two")), None)
        .expect("admitted");
    assert_eq!(
        first.expect("keyed victim").0,
        a,
        "the first join must take the longest-asleep"
    );
    assert_eq!(
        second.expect("keyed victim").0,
        b,
        "the second join re-picked the first victim — both evicts name one \
         body, and the second join seats nobody"
    );

    core.tick_bare(&stats, |_, _, _| true);
    assert_eq!(core.world.evictions, 2);
    assert_eq!(core.world.sleepers(), 0);
    for id in [id2_of(0), id2_of(1)] {
        assert!(
            core.world.players.iter().any(|p| p.active && p.id == id),
            "join {id:#x} was refused on a shard with a victim to give"
        );
    }
    assert_eq!(
        core.world.players.iter().filter(|p| p.active).count(),
        MAX,
        "the world over- or under-seated"
    );
}

/// A takeover reuses its own sleeper's slot, so it must evict nobody — the
/// `needs_slot` half of the eviction decision. A returning owner on a full
/// shard is the commonest full-shard join there is, and charging a stranger
/// a body for it would be an eviction with no join needing one.
#[test]
fn a_takeover_under_slot_pressure_evicts_nobody() {
    const MAX: usize = sim_core::limits::MAX_PLAYERS;
    let stats = ShardStats::default();
    let mut core = armed_core();
    let me = key("comes-back");

    assert!(core.connect_as(0, id_of(0), Some(me), None).is_some());
    core.tick_bare(&stats, |_, _, _| true);
    core.disconnect(0);
    core.tick_bare(&stats, |_, _, _| true);
    for s in 1..MAX {
        assert!(core.connect(s, id_of(s)), "filler {s}");
    }
    core.tick_bare(&stats, |_, _, _| true);
    assert_eq!(core.world.sleepers(), 1);

    let (how, evicted) = core
        .connect_as(0, id2_of(0), Some(me), None)
        .expect("admitted");
    assert_eq!(how, Admitted::TookOver);
    assert!(evicted.is_none(), "a takeover charged somebody a body");
    core.tick_bare(&stats, |_, _, _| true);
    assert_eq!(core.world.evictions, 0);
    assert_eq!(core.world.sleepers(), 0, "the body is awake, not gone");
}

/// A disconnected slot has nothing to hand over, and asking twice does not
/// invent a second record. `net.rs` calls this exactly once per leaving
/// connection, and a duplicate would file a stale record over a fresh one.
#[test]
fn a_second_disconnect_hands_back_nothing() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    assert!(core.connect(0, id_of(0)));
    core.tick_bare(&stats, |_, _, _| true);
    assert!(core.disconnect(0).is_some(), "the first leave has a record");
    assert!(
        core.disconnect(0).is_none(),
        "a second leave invented a record"
    );
}

/// Every boot refusal, each one an operator mistake with a one-command fix.
///
/// **A refusal rather than a silent wipe is the whole posture**: the
/// alternative is a hundred players logging in to an empty inventory because
/// somebody edited a content file, and nobody finding out until they say so in
/// chat. Each message has to name what is wrong — asserted here, because an
/// error nobody can act on is the same as no error.
#[test]
fn a_mismatched_file_refuses_to_boot_and_says_why() {
    let path = scratch("mismatch");
    {
        let (_saves, found) = store::open(&path, SEED, CONTENT, &gc()).expect("creates");
        assert!(found.created);
    }

    let other_seed = store::open(&path, SEED + 1, CONTENT, &gc()).expect_err("another island");
    assert!(
        other_seed.contains("seed") && other_seed.contains("island"),
        "a seed mismatch must say so: {other_seed}"
    );

    let other_content = store::open(&path, SEED, CONTENT + 1, &gc()).expect_err("moved content");
    assert!(
        other_content.contains("content") && other_content.contains("wipe"),
        "a content mismatch must name the content and the way out: {other_content}"
    );

    // Not a save file at all — the first thing checked, so pointing the knob
    // at the wrong path says that instead of misreading bytes as a seed. Both
    // shapes, because they take different branches and the short one used to
    // surface as `failed to fill whole buffer`: an io error about a read, for
    // what is really a knob pointing at the wrong file.
    let junk = scratch("junk");
    std::fs::write(&junk, b"not a save file, a certificate").expect("write");
    let too_short = store::open(&junk, SEED, CONTENT, &gc()).expect_err("too short");
    assert!(
        too_short.contains("too short to be a gates save file"),
        "a short file must say so: {too_short}"
    );
    let junk_long = scratch("junk-long");
    std::fs::write(&junk_long, vec![b'x'; store::SAVE_HEADER_BYTES * 4]).expect("write");
    let bad_magic = store::open(&junk_long, SEED, CONTENT, &gc()).expect_err("not a save file");
    assert!(
        bad_magic.contains("not a gates save file"),
        "bad magic must say so: {bad_magic}"
    );

    // Truncated: the header describes a table longer than the file holds.
    let truncated = scratch("truncated");
    let whole = std::fs::read(&path).expect("read");
    std::fs::write(&truncated, &whole[..whole.len() / 2]).expect("write");
    let short = store::open(&truncated, SEED, CONTENT, &gc()).expect_err("truncated");
    assert!(
        short.contains("bytes") && short.contains("header describes"),
        "a truncated file must say so rather than reading records out of it: {short}"
    );

    for p in [path, junk, junk_long, truncated] {
        sweep(&p);
    }
}

/// A torn record costs one player their save and nothing else: it is counted,
/// the slot is reusable, and the shard boots. The alternative — refusing the
/// whole file over one bad record — would take everybody's base away because
/// one write was interrupted.
#[test]
fn one_corrupt_record_costs_one_player_and_boots() {
    let path = scratch("corrupt");
    let (good, bad) = (key("keeps-theirs"), key("loses-theirs"));
    {
        let (saves, _) = store::open(&path, SEED, CONTENT, &gc()).expect("creates");
        let mut store = saves.store;
        let mut file = saves.file;
        for (i, k) in [good, bad].iter().enumerate() {
            let mut save = PlayerSave::EMPTY;
            save.hp = i as u16 + 1;
            save.hp_max = 100;
            let put = store.put(k, 1_700_000_000 + i as u64, save);
            file.write(put.index, k, 1_700_000_000 + i as u64, &save)
                .expect("writes");
        }
    }

    // Flip a byte inside the second record's body, the way an interrupted
    // write or a bad sector would.
    let mut raw = std::fs::read(&path).expect("read");
    let second = store::SAVE_HEADER_BYTES + store::SAVE_RECORD_BYTES + 80;
    raw[second] ^= 0xff;
    std::fs::write(&path, &raw).expect("write");

    let (saves, found) = store::open(&path, SEED, CONTENT, &gc()).expect("a torn record must still boot");
    assert_eq!(found.corrupt, 1, "the torn record was not counted");
    assert_eq!(found.live, 1, "the intact record was lost with it");
    assert!(
        saves.store.find(&good).is_some(),
        "the intact save vanished"
    );
    assert!(
        saves.store.find(&bad).is_none(),
        "a torn record was handed to the sim"
    );
    sweep(&path);
}

/// Two players are two saves. Trivial to state and the single worst thing to
/// get wrong: one shared record would hand somebody else's base away, and the
/// symptom (an inventory that is not yours) reads as a duplication bug rather
/// than an identity one.
#[test]
fn two_keys_never_share_a_save() {
    let path = scratch("two");
    let (a, b) = (key("player-a"), key("player-b"));
    {
        let (saves, _) = store::open(&path, SEED, CONTENT, &gc()).expect("creates");
        let mut store = saves.store;
        let mut file = saves.file;
        for (i, k) in [a, b].iter().enumerate() {
            let mut save = PlayerSave::EMPTY;
            save.hp_max = 100;
            save.inv[0] = ItemStack {
                item: i as u16 + 1,
                count: (i as u16 + 1) * 10,
                cond: 0,
            };
            let put = store.put(k, 1_700_000_000, save);
            assert!(!put.evicted);
            file.write(put.index, k, 1_700_000_000, &save)
                .expect("writes");
        }
    }
    let (saves, found) = store::open(&path, SEED, CONTENT, &gc()).expect("reopens");
    assert_eq!(found.live, 2);
    assert_eq!(
        saves.store.find(&a).expect("a").inv[0],
        ItemStack {
            item: 1,
            count: 10,
            cond: 0
        }
    );
    assert_eq!(
        saves.store.find(&b).expect("b").inv[0],
        ItemStack {
            item: 2,
            count: 20,
            cond: 0
        }
    );
    sweep(&path);
}

/// The file is the size the table says, and it does not grow. That is the
/// reason for fixed slots over an append log: the autosave sweep writes
/// steadily forever, and an append log would need a compactor nobody has
/// written.
#[test]
fn the_file_is_a_fixed_size_and_writes_do_not_grow_it() {
    let path = scratch("size");
    let expect = (store::SAVE_HEADER_BYTES + MAX_SAVED_PLAYERS * store::SAVE_RECORD_BYTES) as u64;
    {
        let (saves, _) = store::open(&path, SEED, CONTENT, &gc()).expect("creates");
        let mut store = saves.store;
        let mut file = saves.file;
        assert_eq!(
            std::fs::metadata(&path).expect("stat").len(),
            expect,
            "a fresh file is not the table's size"
        );
        let me = key("busy");
        for i in 0..500u64 {
            let mut save = PlayerSave::EMPTY;
            save.hp_max = 100;
            save.body.qx = i as i32;
            let put = store.put(&me, 1_700_000_000 + i, save);
            file.write(put.index, &me, 1_700_000_000 + i, &save)
                .expect("writes");
        }
    }
    assert_eq!(
        std::fs::metadata(&path).expect("stat").len(),
        expect,
        "500 saves grew the file — the record is not being written in place"
    );
    let (saves, found) = store::open(&path, SEED, CONTENT, &gc()).expect("reopens");
    assert_eq!(found.live, 1, "500 saves of one player are one record");
    assert_eq!(
        saves.store.find(&key("busy")).expect("there").body.qx,
        499,
        "the last write did not win"
    );
    sweep(&path);
}

/// Boots rotate the numbered backups, higher number = older, and the depth is
/// a bound — the reference game's `....sav.1` / `.sav.2` convention, matched so
/// an operator who knows theirs knows ours (`reference/SAVES.md` §6).
///
/// Asserted on **raw bytes**, not by opening the backup: a copy has to be exact,
/// and `store::open` rotates as a side effect of booting, so reading a backup
/// through it would perturb the thing under test.
///
/// Walked over four boots rather than two, because the bug this guards is an
/// off-by-one in the rename *order* that only appears once the oldest slot is
/// occupied. Renaming upward from the newest would clobber `.2` with `.1` before
/// `.2` had moved, and two boots cannot tell that apart from correct behaviour.
#[test]
fn boots_rotate_the_backups_oldest_first() {
    let path = scratch("rotate");
    let at = |n: usize| {
        let mut s = path.as_os_str().to_os_string();
        s.push(format!(".{n}"));
        PathBuf::from(s)
    };
    let me = key("rotates");

    // Each generation leaves a distinguishable live file; `history[g]` is what
    // the file held at the end of generation g.
    let mut history: Vec<Vec<u8>> = Vec::new();
    for gen in 1..=4u16 {
        let (saves, _) = store::open(&path, SEED, CONTENT, &gc()).expect("opens");
        let mut store = saves.store;
        let mut file = saves.file;
        let mut save = PlayerSave::EMPTY;
        save.hp_max = 100;
        save.hp = gen;
        let stamp = 1_700_000_000 + gen as u64;
        let put = store.put(&me, stamp, save);
        file.write(put.index, &me, stamp, &save).expect("writes");
        drop(file); // flushed already; this closes the handle
        history.push(std::fs::read(&path).expect("read live"));

        // `.1` is the live file as the PREVIOUS generation left it. Generation 1
        // created the file, so there is nothing behind it to have copied.
        if gen == 1 {
            assert!(!at(1).exists(), "a first boot invented a backup");
            continue;
        }
        let prev = &history[gen as usize - 2];
        assert_eq!(
            &std::fs::read(at(1)).expect("read .1"),
            prev,
            "at boot {gen}, .1 must be the previous run byte for byte"
        );
        // And `.2`, once there is a run old enough to be in it.
        if gen >= 3 {
            let older = &history[gen as usize - 3];
            assert_eq!(
                &std::fs::read(at(2)).expect("read .2"),
                older,
                "at boot {gen}, .2 must be the run before .1 — the rename order \
                 clobbered it"
            );
        }
    }

    // The depth is a bound: nothing is written past it, however many boots run.
    assert!(
        !at(SAVE_BACKUP_COUNT + 1).exists(),
        "rotation wrote past SAVE_BACKUP_COUNT"
    );
    for n in 0..=SAVE_BACKUP_COUNT + 2 {
        let _ = std::fs::remove_file(at(n));
    }
    sweep(&path);
}

/// Rotation is best-effort and must never stop a boot. A read-only directory or
/// a full disk has to cost the backup, not the players — so this asserts the
/// shard still comes up when the copy cannot be made.
#[test]
fn a_boot_survives_a_backup_it_cannot_write() {
    let dir = std::env::temp_dir().join(format!("gates-ro-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("players.save");
    // One boot to create the file, then make the directory unwritable so the
    // rotation's rename and copy both fail.
    store::open(&path, SEED, CONTENT, &gc()).expect("first boot creates");
    let mut perms = std::fs::metadata(&dir).expect("stat").permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o500); // r-x: can read and traverse, cannot create
    }
    std::fs::set_permissions(&dir, perms).expect("chmod");

    let booted = store::open(&path, SEED, CONTENT, &gc());

    // Restore write permission before asserting, so a failure still cleans up.
    let mut perms = std::fs::metadata(&dir).expect("stat").permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o700);
    }
    let _ = std::fs::set_permissions(&dir, perms);
    let ok = booted.is_ok();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        ok,
        "a backup that cannot be written must not refuse the boot"
    );
}

/// A shard with no save file writes nothing and remembers nobody — today's
/// behaviour exactly, which is what every other suite in this crate depends on.
#[test]
fn saves_off_touches_no_disk() {
    let mut saves = store::Saves::off();
    assert!(!saves.file.is_open(), "off must hold no file");
    // The write is a no-op rather than an error — a shard told not to remember
    // is not a shard failing to — and it says so in its return value, which is
    // what keeps `saves_written` from counting a write that never happened.
    let wrote = saves
        .file
        .write(0, &key("nobody"), 1, &PlayerSave::EMPTY)
        .expect("a closed store swallows a write");
    assert!(!wrote, "a closed store must report that it wrote nothing");
    assert_eq!(saves.store.live(), 0);
    assert_eq!(saves.store.find(&key("nobody")), None);
}

/// The knob: `save_file` is off by default, refuses an empty value, and reads a
/// path. Off by default is what keeps every test in this repo from writing a
/// file it never asked for.
#[test]
fn the_save_file_knob_defaults_off_and_refuses_empty() {
    use server::config::parse_shard_toml;
    let base = "bind = \"127.0.0.1:1\"\nseed = 7\n";
    assert_eq!(
        parse_shard_toml(base).expect("parses").save_file,
        None,
        "persistence must be opt-in"
    );
    assert_eq!(
        server::config::ShardConfig::ephemeral(1).save_file,
        None,
        "the test config must never write a file"
    );
    let on = parse_shard_toml(&format!("{base}save_file = \"/srv/gates/players.save\"\n"))
        .expect("parses");
    assert_eq!(on.save_file.as_deref(), Some("/srv/gates/players.save"));
    let empty = parse_shard_toml(&format!("{base}save_file = \"\"\n"))
        .expect_err("an empty path must be refused, not read as off");
    assert!(
        empty.contains("omit the key"),
        "the refusal must say how to turn it off: {empty}"
    );
}

/// The store's directory has to exist; a bad path is a boot error naming the
/// path, not a panic four frames deep in `std::fs`.
#[test]
fn an_unopenable_path_is_a_boot_error() {
    let bad = Path::new("/this/directory/does/not/exist/players.save");
    let err = store::open(bad, SEED, CONTENT, &gc()).expect_err("an unwritable path must refuse");
    assert!(
        err.contains("players.save"),
        "the refusal must name the path: {err}"
    );
}

/// **The condition wall** (NOW.md §0dur remainder 4, review finding
/// 2026-08-16): `PlayerSave::read_le` runs without the content tables, so a
/// save file could smuggle a `cond` above the item's `condition_max`
/// ceiling, or a nonzero `cond` onto an item whose ceiling is 0 — states no
/// command can mint, arriving through the one non-command path into
/// `World`. `store::open` now checks the loaded record against the baked
/// ceilings (`server::cond` — refused as corrupt, never clamped; the
/// module header carries the why) and this test drives it through the REAL
/// boot path under the REAL shipped content, indices read off the baked
/// table rather than hardcoded.
///
/// Proven red by making `server::cond::violation` return `None`: both
/// forged records then load live and reach the index.
#[test]
fn a_record_with_unmintable_condition_is_refused_as_corrupt() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");
    let content = content::Content::load_dir(&dir).expect("shipped content loads");
    let gather = content.bake_gather().expect("shipped content bakes");
    let hash = content.hash();

    // Indices from the shipped table itself: one item that wears, one that
    // carries no condition at all.
    let worn = (0..gather.item_count)
        .find(|&i| gather.cond_max[i as usize] > 0)
        .expect("shipped content has a wearing item (the kit's rock)");
    let inert = (0..gather.item_count)
        .find(|&i| gather.cond_max[i as usize] == 0)
        .expect("shipped content has a conditionless item (wood)");
    let ceiling = gather.cond_max[worn as usize];

    let mk = |item: u16, cond: u16| {
        let mut s = PlayerSave::EMPTY;
        s.hp = 1;
        s.hp_max = 100;
        s.inv[0] = ItemStack {
            item,
            count: 1,
            cond,
        };
        s
    };

    let path = scratch("cond-wall");
    {
        // Written through the store's own writer — the checksum is valid,
        // so what refuses these can only be the wall, not the torn-write
        // detector.
        let (mut saves, found) = store::open(&path, SEED, hash, &gather).expect("creates");
        assert!(found.created);
        for (i, (who, save)) in [
            ("over", mk(worn, ceiling + 1)), // (i) above the ceiling
            ("ghost", mk(inert, 1)),         // (ii) cond on a conditionless item
            ("legal", mk(worn, ceiling)),    // control: exactly at the ceiling
        ]
        .into_iter()
        .enumerate()
        {
            let put = saves.store.put(&key(who), i as u64 + 1, save);
            assert!(saves
                .file
                .write(put.index, &key(who), i as u64 + 1, &save)
                .expect("writes"));
        }
    }

    let (saves, found) = store::open(&path, SEED, hash, &gather)
        .expect("a file with forged records still boots — the blast radius is per record");
    assert_eq!(
        found.corrupt, 2,
        "both un-mintable records must be refused as corrupt"
    );
    assert_eq!(found.live, 1, "the legal record must survive the wall");
    assert!(
        saves.store.find(&key("over")).is_none(),
        "a condition past its ceiling reached the index"
    );
    assert!(
        saves.store.find(&key("ghost")).is_none(),
        "condition on a conditionless item reached the index"
    );
    assert_eq!(
        saves.store.find(&key("legal")).map(|s| s.inv[0].cond),
        Some(ceiling),
        "a tool at exactly its ceiling is mintable and must load"
    );
    sweep(&path);
}
