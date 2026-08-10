//! The world file end to end: **a shard restart you can walk back into.**
//!
//! `sim-core/tests/worldsave.rs` owns the blob — what survives a round trip
//! and what the decoder refuses. `worldfile.rs`'s own tests own the header's
//! offsets. This file owns the join of the two and the thing neither can
//! reach on its own: the identity table.
//!
//! That is the part worth stating, because it is the one that makes world
//! persistence *worth having* rather than merely working. A body carries
//! the id it had, and an id is `generation << 8 | slot` — minted per
//! connection, meaningless after a restart. So a world file that saved
//! bodies and not identities would come back full of sleepers nobody could
//! claim: everyone's base still standing, everyone's body still standing in
//! it, and every player handed a fresh character while their own corpse
//! stood there as free loot. The blob cannot fix that (a key may not enter
//! `sim-core`) and the header cannot (it is one number). It takes both
//! files' halves meeting, which is here.
//!
//! No sockets, like every other suite in this directory. What is real is
//! the file: these tests write one, drop everything, open it again, and
//! walk a player back into the body they left.

use protocol::ActionMsg;
use server::core::{Admitted, ShardCore};
use server::stats::ShardStats;
use server::store::{PlayerKey, SAVE_BACKUP_COUNT};
use server::worldfile::{self, WorldFile};
use sim_core::build::{
    build_cell_of, foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE,
};
use sim_core::combat::CombatContent;
use sim_core::craft::CraftContent;
use sim_core::deploy::DeployContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::world::World;
use sim_core::worldsave::WORLD_SAVE_MAX_BYTES;
use std::path::{Path, PathBuf};

const SEED: u64 = 20_260_807;
const CONTENT: u64 = 0x0123_4567_89ab_cdef;
const INTERVAL: u64 = 1_800;
/// The island `SEED` actually generates, as `worldfile`'s header pins it.
/// A literal would be a second copy of a number the sim already computes, and
/// the whole point of the field is that it tracks worldgen rather than being
/// declared beside it.
fn world_digest() -> u64 {
    sim_core::probe::probe_terrain(SEED)
}

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn key(s: &str) -> PlayerKey {
    PlayerKey::new(s.as_bytes()).expect("fits")
}

/// A scratch path under the test binary's temp dir, cleared on the way in
/// rather than on the way out — a failed test that leaves its file behind
/// is evidence.
fn scratch(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("gates-world-{name}-{}.world", std::process::id()));
    sweep(&p);
    p
}

fn sweep(path: &Path) {
    let _ = std::fs::remove_file(path);
    let mut t = path.as_os_str().to_os_string();
    t.push(".tmp");
    let _ = std::fs::remove_file(PathBuf::from(t));
    for n in 1..=SAVE_BACKUP_COUNT + 2 {
        let mut b = path.as_os_str().to_os_string();
        b.push(format!(".{n}"));
        let _ = std::fs::remove_file(PathBuf::from(b));
    }
}

fn install_content(w: &mut World) {
    w.gather = GatherContent::probe_fixture();
    w.craft = CraftContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
}

fn armed_core() -> ShardCore {
    let mut core = ShardCore::new(SEED);
    install_content(&mut core.world);
    core
}

/// Ground that will hold a foundation, found with the sim's own predicate.
/// A spawn is a beach and a beach refuses a foundation, so this search is
/// required rather than defensive (`sim-core/tests/worldsave.rs` says the
/// same thing at more length, having learned it the same way).
fn stand_somewhere_buildable(core: &mut ShardCore, slot: usize) -> (u16, u16) {
    let b = core.world.players[slot].body;
    let c0x = build_cell_of(b.qx as f32 * POS_XZ_Q).max(0);
    let c0z = build_cell_of(b.qz as f32 * POS_XZ_Q).max(0);
    for r in 0..24i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                let (cx, cz) = (c0x + dx, c0z + dz);
                if cx < 0 || cz < 0 {
                    continue;
                }
                let ax = (cx as f32 + 0.5) * BUILD_CELL_M;
                let az = (cz as f32 + 0.5) * BUILD_CELL_M;
                if foundation_terrain_ok(SEED, ax, az) {
                    core.world.players[slot].body = Body::at(SEED, ax, az);
                    return (cx as u16, cz as u16);
                }
            }
        }
    }
    panic!("no buildable ground within 24 cells of the spawn");
}

/// Write whatever `core` holds to `path`, the way the store thread does.
fn write_world(core: &ShardCore, file: &mut WorldFile) -> (usize, usize) {
    let mut buf = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let len = core.encode_world(&mut buf).expect("a live world encodes");
    let mut ids = vec![(PlayerKey::PLACEHOLDER, 0u32); sim_core::limits::MAX_PLAYERS];
    let n = core.identities(&mut ids);
    ids.truncate(n);
    assert!(
        file.write(core.world.tick, &buf[..len], &ids)
            .expect("the world writes"),
        "an open file must report the write"
    );
    (len, n)
}

/// **The whole slice, as a player experiences it.** Build a base, log off,
/// kill the shard, start a new one on the same file — and walk back into
/// your own body, with the base still standing.
#[test]
fn a_shard_restart_is_a_world_you_walk_back_into() {
    let path = scratch("restart");
    let me = key("dev-anthone");

    // --- session one ------------------------------------------------------
    let (want_hash, want_body, cell) = {
        let mut trial = World::new(SEED);
        install_content(&mut trial);
        let (boot, found) =
            worldfile::open(&path, &mut trial, SEED, CONTENT, world_digest(), INTERVAL)
                .expect("a fresh file opens");
        assert!(found.created, "the first open must create nothing yet");
        let mut file = boot.file;

        let stats = ShardStats::default();
        let mut core = armed_core();
        assert_eq!(
            core.connect_as(0, id_of(0), Some(me), None)
                .map(|(how, _)| how),
            Some(Admitted::Fresh),
            "a first visit"
        );
        core.tick(&stats, |_, _, _| true);

        // Build something worth coming back to.
        let slot = core
            .world
            .players
            .iter()
            .position(|p| p.active && p.id == id_of(0))
            .expect("seated");
        let cell = stand_somewhere_buildable(&mut core, slot);
        core.world.players[slot].inv[0] = ItemStack {
            item: 0,
            count: 500,
        };
        core.world.players[slot].inv[1] = ItemStack {
            item: 1,
            count: 500,
        };
        // Through the action lane, which is the path a real client takes.
        core.push_action(
            0,
            ActionMsg::Place {
                row: 0,
                cx: cell.0,
                cz: cell.1,
                level: 0,
                loc: LOC_PLANE,
            },
        );
        core.tick(&stats, |_, _, _| true);
        assert_eq!(core.world.pieces.len(), 1, "the fixture built nothing");

        // Log off. The body stays as a sleeper — that is the previous
        // slice — and this one is about it surviving the process too.
        core.disconnect(0);
        core.tick(&stats, |_, _, _| true);
        assert_eq!(core.world.sleepers(), 1);
        let body = core.world.players[slot].body;

        let (_, idents) = write_world(&core, &mut file);
        assert_eq!(idents, 1, "the sleeper was saved with nobody's name on it");
        (core.world.state_hash(), body, cell)
    }; // everything dropped: the file is all that is left

    // --- session two: a different process, the same file ------------------
    let mut boot_world = World::new(SEED);
    install_content(&mut boot_world);
    let (boot, found) = worldfile::open(
        &path,
        &mut boot_world,
        SEED,
        CONTENT,
        world_digest(),
        INTERVAL,
    )
    .expect("the file reopens");
    assert!(!found.created, "the second open must find the file");
    assert_eq!(found.bodies, 1, "the shard forgot the only body it had");
    assert_eq!(found.claimable, 1, "the body came back with no name on it");

    let stats = ShardStats::default();
    let mut core = armed_core();
    core.world.load(&boot.blob).expect("the sim loads it too");
    core.adopt_identities(&boot.idents);

    assert_eq!(
        core.world.state_hash(),
        want_hash,
        "the world that came back is not the world that was saved"
    );
    assert_eq!(core.world.pieces.len(), 1, "the base did not survive");
    assert_eq!(core.world.pieces.entries()[0].cx, cell.0);

    // And the player walks back into their own body — a *different*
    // connection id, resolved through the key the file carried.
    assert_eq!(
        core.connect_as(0, id_of(0), Some(me), None)
            .map(|(how, _)| how),
        Some(Admitted::TookOver),
        "the returning player did not get their body back"
    );
    core.tick(&stats, |_, _, _| true);
    let p = core
        .world
        .players
        .iter()
        .find(|p| p.active && p.id == id_of(0))
        .expect("seated");
    assert!(!p.sleeping, "the body is awake now");
    assert_eq!(p.body.qx, want_body.qx, "woke up somewhere else");
    assert_eq!(p.body.qz, want_body.qz, "woke up somewhere else");
    sweep(&path);
}

/// Without the identity table the body is still there and **nobody can have
/// it** — which is the failure mode this whole seam exists to prevent, and
/// it is worth a test because it is invisible from either half alone: the
/// blob round-trips, the header verifies, the base is standing, and every
/// player is quietly handed a fresh character while their own body stands
/// in their own base as free loot.
#[test]
fn a_body_with_no_identity_beside_it_is_unclaimable() {
    let path = scratch("nameless");
    let me = key("dev-anthone");
    let stats = ShardStats::default();

    let blob = {
        let mut core = armed_core();
        assert!(core.connect(0, id_of(0)), "a keyless join");
        core.tick(&stats, |_, _, _| true);
        core.disconnect(0);
        core.tick(&stats, |_, _, _| true);
        assert_eq!(core.world.sleepers(), 1);

        let mut trial = World::new(SEED);
        install_content(&mut trial);
        let (boot, _) = worldfile::open(&path, &mut trial, SEED, CONTENT, world_digest(), INTERVAL)
            .expect("opens");
        let mut file = boot.file;
        // A keyless session leaves a body and no name for it — which is
        // exactly what a guest does on a shard with `require_auth = false`.
        let (_, idents) = write_world(&core, &mut file);
        assert_eq!(idents, 0, "a guest cannot be named");
        let mut buf = vec![0u8; WORLD_SAVE_MAX_BYTES];
        let n = core.encode_world(&mut buf).expect("encodes");
        buf.truncate(n);
        buf
    };

    let mut core = armed_core();
    core.world.load(&blob).expect("loads");
    assert_eq!(core.world.sleepers(), 1, "the body survived");
    // ...and the returning player is a stranger to it.
    assert_eq!(
        core.connect_as(0, id_of(0), Some(me), None)
            .map(|(how, _)| how),
        Some(Admitted::Fresh),
        "an unnamed body was handed to somebody the file never named"
    );
    sweep(&path);
}

/// **The shutdown flush's ordering, which is load-bearing and looks
/// arbitrary.** The world must be taken while players are still connected;
/// taking it after the disconnects loses every name.
///
/// `ShardCore::identities` reads two places: the key table, which holds
/// people who are connected *now*, and the sleeper index, which holds
/// people whose `Leave` has already been applied by a tick. A disconnect
/// moves someone from the first to the second and **queues** the `Leave` —
/// so between the disconnect and the tick that applies it they are in
/// neither, and at shutdown that tick never comes. Saving in the wrong
/// order therefore writes a world full of bodies with nobody's name on
/// them: every base standing, every body standing in it, and every player
/// coming back to a fresh character while their own body is free loot.
///
/// Nothing about the shape of the code says this. This does.
#[test]
fn the_shutdown_flush_takes_the_world_before_it_drops_the_players() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    let me = key("dev-anthone");
    assert_eq!(
        core.connect_as(0, id_of(0), Some(me), None)
            .map(|(how, _)| how),
        Some(Admitted::Fresh)
    );
    core.tick(&stats, |_, _, _| true);

    let mut ids = vec![(PlayerKey::PLACEHOLDER, 0u32); sim_core::limits::MAX_PLAYERS];
    assert_eq!(
        core.identities(&mut ids),
        1,
        "a connected player must be nameable — this is the shutdown order"
    );

    // Now do it the wrong way round.
    core.disconnect(0);
    assert_eq!(
        core.identities(&mut ids),
        0,
        "a disconnected-but-not-yet-ticked player is nameable by neither \
         table — which is why the shutdown flush saves the world first"
    );

    // And once the `Leave` has actually been applied, they are back — as a
    // sleeper, through the other table.
    core.tick(&stats, |_, _, _| true);
    assert_eq!(core.world.sleepers(), 1);
    assert_eq!(
        core.identities(&mut ids),
        1,
        "a settled sleeper must be nameable through the sleeper index"
    );
}

/// Every boot refusal, one per reason. Each is an operator mistake with a
/// one-line fix, and each would otherwise present as a shard quietly
/// starting a fresh island on top of everybody's bases — which is the worst
/// available outcome and the reason these are refusals and not warnings.
#[test]
fn a_wrong_world_file_is_refused_by_reason() {
    let path = scratch("refusals");
    // Write a legal world first.
    {
        let mut trial = World::new(SEED);
        install_content(&mut trial);
        let (boot, _) = worldfile::open(&path, &mut trial, SEED, CONTENT, world_digest(), INTERVAL)
            .expect("opens");
        let mut file = boot.file;
        let stats = ShardStats::default();
        let mut core = armed_core();
        assert!(core.connect(0, id_of(0)));
        core.tick(&stats, |_, _, _| true);
        write_world(&core, &mut file);
    }
    let reopen_with = |seed: u64, content: u64, digest: u64| {
        let mut w = World::new(seed);
        install_content(&mut w);
        worldfile::open(&path, &mut w, seed, content, digest, INTERVAL).map(|(_, f)| f)
    };
    let reopen = |seed: u64, content: u64| reopen_with(seed, content, world_digest());
    assert!(reopen(SEED, CONTENT).is_ok(), "the base file must be legal");

    // **The island moved under the same seed.** The seed test above cannot
    // see this one: worldgen changing is not an operator pointing at the
    // wrong file, it is the ground under every base changing shape while
    // every coordinate in the file stays where it was. A worldgen change is
    // a wipe (operator, 2026-08-10) and this is what makes that a refusal
    // rather than something everyone has to remember.
    let e = reopen_with(SEED, CONTENT, world_digest() ^ 1).expect_err("a moved island must refuse");
    assert!(e.contains("island digest"), "{e}");
    assert!(e.contains("wipe"), "the message must name the remedy: {e}");

    // A different island: every base in the file stands where there is now
    // sea. Refused, and the message says which seed the file was made on.
    let e = reopen(SEED + 1, CONTENT).expect_err("a foreign seed must refuse");
    assert!(e.contains("seed"), "{e}");

    // Moved content: item indices shifted, so a box would hand somebody a
    // different item than the one they put in it.
    let e = reopen(SEED, CONTENT + 1).expect_err("moved content must refuse");
    assert!(e.contains("content hash"), "{e}");

    // Bent bytes: the checksum catches the torn write a killed shard leaves.
    {
        let mut bytes = std::fs::read(&path).expect("read back");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        std::fs::write(&path, &bytes).expect("bend it");
    }
    let e = reopen(SEED, CONTENT).expect_err("a torn file must refuse");
    assert!(e.contains("checksum"), "{e}");

    // Not our file at all.
    std::fs::write(&path, vec![0u8; 256]).expect("clobber");
    let e = reopen(SEED, CONTENT).expect_err("a foreign file must refuse");
    assert!(e.contains("magic"), "{e}");

    // Too short to be anything.
    std::fs::write(&path, b"hi").expect("clobber");
    let e = reopen(SEED, CONTENT).expect_err("a stub must refuse");
    assert!(e.contains("too short"), "{e}");
    sweep(&path);
}

/// A refused boot leaves the world it was handed untouched, so a shard that
/// refuses can say so and exit rather than running half an island.
#[test]
fn a_refused_file_does_not_half_load_the_world() {
    let path = scratch("untouched");
    {
        let mut trial = World::new(SEED);
        install_content(&mut trial);
        let (boot, _) = worldfile::open(&path, &mut trial, SEED, CONTENT, world_digest(), INTERVAL)
            .expect("opens");
        let mut file = boot.file;
        let stats = ShardStats::default();
        let mut core = armed_core();
        assert!(core.connect(0, id_of(0)));
        core.tick(&stats, |_, _, _| true);
        write_world(&core, &mut file);
    }
    let mut w = World::new(SEED);
    install_content(&mut w);
    let before = w.state_hash();
    assert!(worldfile::open(&path, &mut w, SEED + 9, CONTENT, world_digest(), INTERVAL).is_err());
    assert_eq!(w.state_hash(), before, "a refused boot wrote to the world");
    sweep(&path);
}

/// The write is atomic: a temp file, fsynced, then renamed. What this
/// checks is the observable half — that no `.tmp` survives a successful
/// write, so a boot can never find one and mistake it for the world.
#[test]
fn a_successful_write_leaves_no_temp_file_behind() {
    let path = scratch("atomic");
    let mut trial = World::new(SEED);
    install_content(&mut trial);
    let (boot, _) =
        worldfile::open(&path, &mut trial, SEED, CONTENT, world_digest(), INTERVAL).expect("opens");
    let mut file = boot.file;
    let stats = ShardStats::default();
    let mut core = armed_core();
    assert!(core.connect(0, id_of(0)));
    core.tick(&stats, |_, _, _| true);
    write_world(&core, &mut file);

    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    assert!(
        !Path::new(&tmp).exists(),
        "a .tmp survived a successful write — a boot could find it"
    );
    assert!(path.exists(), "the world was not published");
    sweep(&path);
}

/// World persistence off is a no-op and never an error: a shard told not to
/// remember is not a shard failing to. The bool is what says so — a caller
/// that counted every `Ok` would report a shard persisting a world it has
/// no path for.
#[test]
fn no_world_file_is_a_no_op_and_not_a_failure() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    assert!(core.connect(0, id_of(0)));
    core.tick(&stats, |_, _, _| true);

    let mut off = WorldFile::closed();
    assert!(!off.is_open());
    let mut buf = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = core.encode_world(&mut buf).expect("encodes");
    assert!(
        !off.write(core.world.tick, &buf[..n], &[])
            .expect("no error"),
        "a closed file must report that it wrote nothing"
    );
}
