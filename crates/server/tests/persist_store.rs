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

use server::core::ShardCore;
use server::stats::ShardStats;
use server::store::{self, PlayerKey, SaveStore, MAX_SAVED_PLAYERS};
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

fn key(s: &str) -> PlayerKey {
    PlayerKey::new(s.as_bytes()).expect("fits")
}

/// A scratch path under the test binary's temp dir. Named per test so the
/// suite can run in parallel, and removed on the way in rather than the way
/// out — a failed test that leaves its file behind is evidence.
fn scratch(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("gates-save-{name}-{}.save", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

fn armed_core() -> ShardCore {
    let mut core = ShardCore::new(SEED);
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
        let (saves, found) = store::open(&path, SEED, CONTENT).expect("a fresh file opens");
        assert!(found.created, "the first open must create the file");
        assert_eq!(found.live, 0, "a new file remembers nobody");
        let mut store = saves.store;
        let mut file = saves.file;

        let stats = ShardStats::default();
        let mut core = armed_core();
        assert!(core.connect(0, id_of(0)), "a fresh join");
        core.tick(&stats, |_, _, _| true);

        // Do something worth keeping.
        let slot = world_slot(&core, id_of(0));
        core.world.players[slot].inv[0] = ItemStack {
            item: 3,
            count: 128,
        };
        core.world.players[slot].hp = 37;
        core.tick(&stats, |_, _, _| true);
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
    let (saves, found) = store::open(&path, SEED, CONTENT).expect("the file reopens");
    assert!(!found.created, "the second open must find the file");
    assert_eq!(found.corrupt, 0, "a clean write must read back clean");
    assert_eq!(found.live, 1, "the shard forgot the only player it had");

    let restored = saves.store.find(&me).expect("the key must be remembered");
    let stats = ShardStats::default();
    let mut core = armed_core();
    assert!(
        core.connect_as(0, id_of(0), Some(restored)),
        "a restoring join"
    );
    core.tick(&stats, |_, _, _| true);

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
    let _ = std::fs::remove_file(&path);
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
    assert!(core.connect_as(0, id_of(0), None), "a keyless join");
    core.tick(&stats, |_, _, _| true);
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
    core.tick(&stats, |_, _, _| true);

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

/// A disconnected slot has nothing to hand over, and asking twice does not
/// invent a second record. `net.rs` calls this exactly once per leaving
/// connection, and a duplicate would file a stale record over a fresh one.
#[test]
fn a_second_disconnect_hands_back_nothing() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    assert!(core.connect(0, id_of(0)));
    core.tick(&stats, |_, _, _| true);
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
        let (_saves, found) = store::open(&path, SEED, CONTENT).expect("creates");
        assert!(found.created);
    }

    let other_seed = store::open(&path, SEED + 1, CONTENT).expect_err("another island");
    assert!(
        other_seed.contains("seed") && other_seed.contains("island"),
        "a seed mismatch must say so: {other_seed}"
    );

    let other_content = store::open(&path, SEED, CONTENT + 1).expect_err("moved content");
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
    let too_short = store::open(&junk, SEED, CONTENT).expect_err("too short");
    assert!(
        too_short.contains("too short to be a gates save file"),
        "a short file must say so: {too_short}"
    );
    let junk_long = scratch("junk-long");
    std::fs::write(&junk_long, vec![b'x'; store::SAVE_HEADER_BYTES * 4]).expect("write");
    let bad_magic = store::open(&junk_long, SEED, CONTENT).expect_err("not a save file");
    assert!(
        bad_magic.contains("not a gates save file"),
        "bad magic must say so: {bad_magic}"
    );

    // Truncated: the header describes a table longer than the file holds.
    let truncated = scratch("truncated");
    let whole = std::fs::read(&path).expect("read");
    std::fs::write(&truncated, &whole[..whole.len() / 2]).expect("write");
    let short = store::open(&truncated, SEED, CONTENT).expect_err("truncated");
    assert!(
        short.contains("bytes") && short.contains("header describes"),
        "a truncated file must say so rather than reading records out of it: {short}"
    );

    for p in [path, junk, junk_long, truncated] {
        let _ = std::fs::remove_file(p);
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
        let (saves, _) = store::open(&path, SEED, CONTENT).expect("creates");
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

    let (saves, found) = store::open(&path, SEED, CONTENT).expect("a torn record must still boot");
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
    let _ = std::fs::remove_file(&path);
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
        let (saves, _) = store::open(&path, SEED, CONTENT).expect("creates");
        let mut store = saves.store;
        let mut file = saves.file;
        for (i, k) in [a, b].iter().enumerate() {
            let mut save = PlayerSave::EMPTY;
            save.hp_max = 100;
            save.inv[0] = ItemStack {
                item: i as u16 + 1,
                count: (i as u16 + 1) * 10,
            };
            let put = store.put(k, 1_700_000_000, save);
            assert!(!put.evicted);
            file.write(put.index, k, 1_700_000_000, &save)
                .expect("writes");
        }
    }
    let (saves, found) = store::open(&path, SEED, CONTENT).expect("reopens");
    assert_eq!(found.live, 2);
    assert_eq!(
        saves.store.find(&a).expect("a").inv[0],
        ItemStack { item: 1, count: 10 }
    );
    assert_eq!(
        saves.store.find(&b).expect("b").inv[0],
        ItemStack { item: 2, count: 20 }
    );
    let _ = std::fs::remove_file(&path);
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
        let (saves, _) = store::open(&path, SEED, CONTENT).expect("creates");
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
    let (saves, found) = store::open(&path, SEED, CONTENT).expect("reopens");
    assert_eq!(found.live, 1, "500 saves of one player are one record");
    assert_eq!(
        saves.store.find(&key("busy")).expect("there").body.qx,
        499,
        "the last write did not win"
    );
    let _ = std::fs::remove_file(&path);
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
    let err = store::open(bad, SEED, CONTENT).expect_err("an unwritable path must refuse");
    assert!(
        err.contains("players.save"),
        "the refusal must name the path: {err}"
    );
}
