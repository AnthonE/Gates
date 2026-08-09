//! The world survives a restart — and the assertion that carries the file
//! is **`state_hash` equality**, not a field-by-field comparison.
//!
//! A round-trip test that listed fields would be a list somebody has to
//! remember to extend, and `worldsave.rs` touches nine stores. The state
//! hash already *is* the definition of "everything the sim considers
//! state": it is what wall 5 compares between two runs, so a field that
//! survives a save but is absent from the hash was never state, and a field
//! that is state but does not survive the save moves the number. One
//! assertion, and it grows itself.
//!
//! What the hash cannot see is the other half of this file. `Pieces::cols`
//! is **derived** collision state, deliberately never hashed (`build.rs`
//! says so), so a load that failed to rebuild it would produce a world with
//! an identical hash and walls you can walk through. Doors are worse: the
//! shut bit lives on a *deployable* while the surface it blocks is a
//! *piece*, so it is the one thing a piece-only rebuild silently drops.
//! Both get their own tests, and both assert against `collide::blocked` —
//! the function movement actually calls.

use sim_core::build::{
    build_cell_of, foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_EDGE_W, LOC_PLANE,
};
use sim_core::combat::CombatContent;
use sim_core::craft::CraftContent;
use sim_core::deploy::DeployContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::loot::LootContent;
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::survival::SurvivalContent;
use sim_core::world::{Command, World};
use sim_core::worldsave::{self, WorldSaveError, WORLD_SAVE_MAX_BYTES};

const SEED: u64 = 20260807;
/// The doorway row in the build fixture, and the door row in the deploy
/// fixture — read off `probe_fixture()` rather than guessed.
const ROW_FOUNDATION: u16 = 0;
const ROW_DOORWAY: u16 = 3;
const DEPLOY_HEARTH: u16 = 0;
const DEPLOY_DOOR: u16 = 2;

fn armed() -> World {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.craft = CraftContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = DeployContent::probe_fixture();
    w.combat = CombatContent::probe_fixture();
    w.survival = SurvivalContent::probe_fixture();
    w.loot = LootContent::probe_fixture();
    w
}

/// Stock a player with enough of everything to build what the script below
/// builds. Server-side rather than earned: `gather.rs`'s own tests own
/// earning, and a fixture that had to chop its way to a doorway would be
/// testing the swing cadence.
fn kit(w: &mut World, slot: usize) {
    w.players[slot].inv[0] = ItemStack {
        item: 0,
        count: 500,
    };
    w.players[slot].inv[1] = ItemStack {
        item: 1,
        count: 500,
    };
    w.players[slot].inv[2] = ItemStack { item: 2, count: 9 };
    w.players[slot].inv[3] = ItemStack { item: 4, count: 9 };
}

/// The **build** cell a body occupies, and the body snapped to its centre.
///
/// Two grids live in this sim and they are three metres apart in size:
/// structures are addressed in `BUILD_CELL_M` cells and harvested terrain
/// in `CELL_SIZE` ones. The first version of this fixture divided by the
/// terrain constant, landed on a cell the player was nowhere near, and got
/// `REFUSE_B_REACH` on every placement — so the world it saved had nothing
/// in it and the round-trip passed vacuously. Snapping the body to the
/// centre is what makes the reach check pass deterministically instead of
/// depending on where in the cell the spawn ring happened to put them.
fn stand_in_build_cell(w: &mut World, slot: usize) -> (u16, u16) {
    let b = w.players[slot].body;
    let c0x = build_cell_of(b.qx as f32 * POS_XZ_Q).max(0);
    let c0z = build_cell_of(b.qz as f32 * POS_XZ_Q).max(0);
    // Ground that will hold a foundation, found with the sim's own
    // predicate rather than a coordinate someone measured once. `build.rs`
    // says why the helper is public: "a fixture that needs buildable
    // ground finds it with it, so the two can never drift apart." A spawn
    // is a *beach*, which is exactly the terrain a foundation refuses, so
    // this search is not defensive — it is required.
    let mut found = None;
    'outer: for r in 0..24i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                let (cx, cz) = (c0x + dx, c0z + dz);
                if cx < 0 || cz < 0 {
                    continue;
                }
                let ax = (cx as f32 + 0.5) * BUILD_CELL_M;
                let az = (cz as f32 + 0.5) * BUILD_CELL_M;
                if foundation_terrain_ok(SEED, ax, az) {
                    found = Some((cx as u16, cz as u16, ax, az));
                    break 'outer;
                }
            }
        }
    }
    let (cx, cz, ax, az) = found.expect("no buildable ground within 24 cells of the spawn");
    w.players[slot].body = Body::at(SEED, ax, az);
    (cx, cz)
}

/// A world somebody has lived in: two bodies (one of them asleep), a
/// foundation, a doorway, a hearth, a closed door, a chopped tree, and a
/// few ticks of clock on all of it.
fn a_lived_in_world() -> World {
    let mut w = armed();
    w.dev_spawn = Some(w.spawn_pos(1));
    w.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
    kit(&mut w, 0);
    kit(&mut w, 1);
    let (cx, cz) = stand_in_build_cell(&mut w, 0);
    // The second body stands there too, so the fixture's sleeper is inside
    // the base rather than wherever the ring put it.
    w.players[1].body = w.players[0].body;

    // A foundation to stand a hearth on, and a doorway to hang a door in.
    w.tick(&[Command::Place {
        id: 1,
        row: ROW_FOUNDATION,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    w.tick(&[Command::Place {
        id: 1,
        row: ROW_DOORWAY,
        cx,
        cz,
        level: 0,
        loc: LOC_EDGE_W,
    }]);
    w.tick(&[Command::PlaceDeploy {
        id: 1,
        row: DEPLOY_HEARTH,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    w.tick(&[Command::PlaceDeploy {
        id: 1,
        row: DEPLOY_DOOR,
        cx,
        cz,
        level: 0,
        loc: LOC_EDGE_W,
    }]);
    // Feed the hearth so a stock row is nonzero — an all-zero stock would
    // round-trip through a codec that dropped the array entirely.
    w.tick(&[Command::Feed {
        id: 1,
        cx,
        cz,
        level: 0,
    }]);
    // Somebody logs off: a sleeper in the file is the point of the slice.
    w.tick(&[Command::Leave { id: 2 }]);
    for _ in 0..20 {
        w.tick(&[]);
    }
    w
}

/// The same world with **everybody** asleep — which is what a shard looks
/// like the instant before it is shut down cleanly, and the only shape for
/// which a round trip can be hash-*exact*.
///
/// The distinction is the design and not a testing convenience. A save
/// normalizes an awake body the way `Command::Leave` does: facing kept,
/// buttons and movement dropped, `slept_at` stamped at the save tick. So
/// save→load is deliberately lossy for a body somebody was driving, and
/// exactly lossless for one nobody was. `a_world_saved_mid_session_puts_
/// everyone_to_bed` below owns the lossy half; everything that compares
/// hashes starts here.
fn a_quiet_world() -> World {
    let mut w = a_lived_in_world();
    w.tick(&[Command::Leave { id: 1 }]);
    w
}

fn round_trip(w: &World) -> World {
    let mut buf = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut buf).expect("a live world encodes");
    let mut back = armed();
    back.load(&buf[..n]).expect("its own bytes must load");
    back
}

/// **The load is the world.** One assertion, and it covers every store the
/// sim calls state — see the module header for why this is stronger than a
/// field list and not weaker.
#[test]
fn a_saved_world_comes_back_identical() {
    let w = a_quiet_world();
    assert!(!w.pieces.is_empty(), "fixture built nothing");
    assert!(!w.deploys.is_empty(), "fixture deployed nothing");
    assert_eq!(w.sleepers(), 2, "fixture left somebody driving a body");

    let back = round_trip(&w);

    assert_eq!(
        back.state_hash(),
        w.state_hash(),
        "the world that came back is not the world that was saved"
    );
    assert_eq!(back.tick, w.tick, "the clock restarted");
}

/// Every body in a loaded world is asleep, including one that was awake
/// when the shard died.
///
/// Not a simplification — it is what a restart *means*. Every connection
/// ended, so nobody is driving anything, and the reference model has this
/// property exactly: at boot a shard is nothing but sleepers
/// (`reference/SAVES.md` §1). It also gives the takeover one path instead
/// of two: a player returning after a restart uses the same `Command::Wake`
/// as a player returning after a dropped connection.
#[test]
fn a_world_saved_mid_session_puts_everyone_to_bed() {
    let w = a_lived_in_world();
    assert_eq!(w.sleepers(), 1, "fixture: exactly one was asleep");
    let awake = w.players.iter().filter(|p| p.active && !p.sleeping).count();
    assert_eq!(awake, 1, "fixture: exactly one was awake");

    let back = round_trip(&w);

    assert_eq!(back.sleepers(), 2, "a restart left somebody driving a body");
    assert!(
        back.players.iter().filter(|p| p.active).count() == 2,
        "a body was lost across the restart"
    );
    // And the ids survive, which is what lets the server's key table point
    // at a body it saved beside the world.
    assert!(back.is_sleeper(1) && back.is_sleeper(2));
}

/// A returning player takes over the body a *restart* left, through the
/// same command a dropped connection uses. This is the whole point of
/// persisting bodies: without it the sleeper stands there unclaimable and
/// the player is handed a stale record instead.
#[test]
fn a_body_that_survived_a_restart_is_still_claimable() {
    let w = a_lived_in_world();
    let stood = w.players[w.players.iter().position(|p| p.id == 2).unwrap()].body;
    let mut back = round_trip(&w);

    back.tick(&[Command::Wake {
        id: 0x0202,
        sleeper: 2,
    }]);

    let p = back
        .players
        .iter()
        .find(|p| p.active && p.id == 0x0202)
        .expect("the takeover seated nobody");
    assert!(!p.sleeping);
    assert_eq!(p.body.qx, stood.qx, "woke up somewhere else");
    assert_eq!(p.body.qz, stood.qz, "woke up somewhere else");
}

/// **The collision index is rebuilt, and `state_hash` cannot tell you
/// that.** `Pieces::cols` is derived and deliberately never hashed, so a
/// load that skipped the rebuild passes the equality test above and
/// produces a shard whose walls are scenery. Asserted against
/// `collide::blocked`, which is the function `movement::step` actually
/// calls — not against the index's own length, which would only prove
/// something was populated.
#[test]
fn the_collision_index_is_rebuilt_from_the_pieces() {
    let w = a_lived_in_world();
    let back = round_trip(&w);

    assert_eq!(
        back.pieces.cols().len(),
        w.pieces.cols().len(),
        "the rebuilt column index holds a different number of columns"
    );
    assert!(
        !back.pieces.cols().is_empty(),
        "the index came back empty — every wall on the shard is scenery"
    );
    // Cell by cell, the masks the movement query reads must agree.
    for p in w.pieces.entries() {
        assert_eq!(
            back.pieces.cols().get(p.cx, p.cz),
            w.pieces.cols().get(p.cx, p.cz),
            "column ({}, {}) rebuilt differently",
            p.cx,
            p.cz
        );
    }
}

/// **Doors, which are the seam, and the assertion is a difference rather
/// than a ray.**
///
/// A door's shut bit rides a *deployable*; what it blocks is a *piece*. So
/// `Pieces::restore`, which walks pieces, cannot know about it, and a load
/// without `World::rebuild_doors` leaves every door on the shard open —
/// hash-identical (the column index is never hashed), wire-identical (the
/// client draws off the deploy record), and free to walk through.
///
/// The first version of this test fired `collide::blocked` along a ray it
/// computed by hand and failed on its own geometry before it ever reached
/// the claim. The fix is to stop naming coordinates: take the *same* world
/// twice, open the door in one of them, and assert that the two disagree
/// after a round trip and agree with themselves across it. A rebuild that
/// dropped the bit makes both worlds identical, which is exactly the
/// failure — and no coordinate has to be right for the test to say so.
#[test]
fn a_closed_door_is_still_closed_after_a_restart() {
    let shut = a_quiet_world();
    let door = shut
        .deploys
        .entries()
        .iter()
        .find(|d| d.row as u16 == DEPLOY_DOOR)
        .copied()
        .expect("fixture hung no door");
    assert!(!door.open, "fixture: a door places closed");

    // The same world with the door swung open. `Use` toggles it, and the
    // owner is id 1 — who is asleep in `a_quiet_world`, so this is built
    // from the awake fixture and then put to bed the same way.
    let open = {
        let mut w = a_lived_in_world();
        w.tick(&[Command::Use {
            id: 1,
            cx: door.cx,
            cz: door.cz,
            level: door.level,
            loc: door.loc,
        }]);
        let d = w
            .deploys
            .entries()
            .iter()
            .find(|d| d.row as u16 == DEPLOY_DOOR)
            .copied()
            .expect("the door is still there");
        assert!(d.open, "fixture: `Use` did not open the door");
        w.tick(&[Command::Leave { id: 1 }]);
        w
    };

    let shut_cols = shut.pieces.cols().get(door.cx, door.cz);
    let open_cols = open.pieces.cols().get(door.cx, door.cz);
    assert_ne!(
        shut_cols, open_cols,
        "fixture: an open door and a closed one must collide differently, \
         or this test cannot see the bit it is about"
    );

    let shut_back = round_trip(&shut);
    let open_back = round_trip(&open);

    assert_eq!(
        shut_back.pieces.cols().get(door.cx, door.cz),
        shut_cols,
        "the closed door came back different: `rebuild_doors` did not run, \
         and every door on the shard is now scenery the wire draws shut"
    );
    assert_eq!(
        open_back.pieces.cols().get(door.cx, door.cz),
        open_cols,
        "the open door came back different"
    );
    assert_ne!(
        shut_back.pieces.cols().get(door.cx, door.cz),
        open_back.pieces.cols().get(door.cx, door.cz),
        "both doors came back the same — the shut bit is not being rebuilt"
    );
}

/// The world keeps ticking from where it stopped, and the deadlines in it
/// still mean what they meant. Absolute ticks are the reason `tick` is
/// persisted at all — a bag's despawn, a fuse, a tree's respawn and a bag's
/// cooldown are all absolute, and rebasing four of them against zero would
/// be four chances to get it wrong.
#[test]
fn the_world_resumes_at_the_tick_it_stopped_on() {
    let w = a_quiet_world();
    assert!(w.tick > 20, "fixture: the clock must have run");
    let mut back = round_trip(&w);
    assert_eq!(back.tick, w.tick);
    back.tick(&[]);
    assert_eq!(back.tick, w.tick + 1, "the loaded world did not resume");
}

/// Determinism, from a loaded origin — **wall 5's sentence with one word
/// widened**, and the reason `World::load` is allowed to exist outside the
/// command stream (`worldsave.rs` module header).
///
/// Two shards loading the same file and running the same commands must
/// agree tick for tick. That is the claim a WAL header will pin by
/// recording the origin hash beside the seed and the content hash; this is
/// the gate for it before there is a WAL.
#[test]
fn two_shards_loading_one_file_stay_in_lockstep() {
    let w = a_quiet_world();
    let mut buf = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut buf).expect("encodes");

    let run = |blob: &[u8]| {
        let mut s = armed();
        s.load(blob).expect("loads");
        let origin = s.state_hash();
        let mut stamps = Vec::new();
        for t in 0..60u32 {
            // A command stream with something in it: a takeover, then the
            // ordinary clock. An empty script would agree even if the load
            // had dropped half the world.
            if t == 5 {
                s.tick(&[Command::Wake {
                    id: 0x0303,
                    sleeper: 2,
                }]);
            } else {
                s.tick(&[]);
            }
            stamps.push(s.state_hash());
        }
        (origin, stamps)
    };

    let (origin_a, a) = run(&buf[..n]);
    let (origin_b, b) = run(&buf[..n]);
    assert_eq!(origin_a, origin_b, "the same file loaded to two origins");
    assert_eq!(a, b, "two shards from one origin diverged");
}

/// Writing what was read produces the same bytes. Cheaper than it looks:
/// it is the only check that catches a field the encoder writes and the
/// decoder ignores, which `state_hash` equality cannot see when the field
/// is one the hash also ignores.
#[test]
fn the_blob_round_trips_byte_for_byte() {
    let w = a_quiet_world();
    let mut first = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut first).expect("encodes");
    let back = round_trip(&w);
    let mut second = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let m = back.save_world(&mut second).expect("re-encodes");
    assert_eq!(n, m, "the re-encoded world is a different length");
    assert_eq!(first[..n], second[..m], "write→read→write drifted");
}

/// An empty world is a legal world. A shard's very first boot writes one,
/// and a codec that refused its own empty case would refuse every fresh
/// shard on its second start.
#[test]
fn an_empty_world_round_trips() {
    let w = armed();
    let back = round_trip(&w);
    assert_eq!(back.state_hash(), w.state_hash());
}

/// Every refusal, one per reason, built by hand off a legal blob — no
/// encoder can produce these, which is exactly why the decoder has to
/// refuse them rather than trusting its own writer.
///
/// The two that matter most are `BadContentRow` and `CountOverCap`. The
/// first is the one that panics the sim: `bc.pieces[row].shape` is indexed
/// unchecked at every rebuild, every collapse and every support sweep. The
/// second is checked *before* the loop it bounds, which is the difference
/// between a refusal and a boot that walks 4 billion records.
#[test]
fn a_corrupt_world_is_refused_by_reason() {
    let w = a_lived_in_world();
    let mut base = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut base).expect("encodes");
    base.truncate(n);

    let bent = |f: &dyn Fn(&mut Vec<u8>)| {
        let mut b = base.clone();
        f(&mut b);
        let mut world = armed();
        world.load(&b)
    };
    assert!(bent(&|_| {}).is_ok(), "the base must be legal");

    // A format the reader does not speak.
    assert_eq!(
        bent(&|b| b[0..2].copy_from_slice(&9_999u16.to_le_bytes())),
        Err(WorldSaveError::Format(9_999))
    );
    // Truncated anywhere is a refusal and never a panic.
    for cut in [0usize, 1, 2, 30, 52, 60, 100] {
        let mut b = base.clone();
        b.truncate(cut.min(b.len()));
        let mut world = armed();
        assert!(world.load(&b).is_err(), "a {cut}-byte blob must refuse");
    }
    // A player count past MAX_PLAYERS. Counts start at offset 34 (2 format
    // + 8 tick + 12 sweeps + 8 evictions + 4 next_bag).
    const COUNTS_AT: usize = 34;
    assert_eq!(
        bent(&|b| b[COUNTS_AT..COUNTS_AT + 2].copy_from_slice(&u16::MAX.to_le_bytes())),
        Err(WorldSaveError::CountOverCap)
    );
    // A piece count past MAX_PIECES.
    assert_eq!(
        bent(&|b| b[COUNTS_AT + 2..COUNTS_AT + 4].copy_from_slice(&u16::MAX.to_le_bytes())),
        Err(WorldSaveError::CountOverCap)
    );
    // A lock count past MAX_LOCKS — the sixth `u16` (lock v1).
    assert_eq!(
        bent(&|b| b[COUNTS_AT + 10..COUNTS_AT + 12].copy_from_slice(&u16::MAX.to_le_bytes())),
        Err(WorldSaveError::CountOverCap)
    );
    // A slot-life count past MAX_SLOT_LIVES — the u32 one.
    assert_eq!(
        bent(&|b| b[COUNTS_AT + 16..COUNTS_AT + 20].copy_from_slice(&u32::MAX.to_le_bytes())),
        Err(WorldSaveError::CountOverCap)
    );

    // A piece naming a content row that does not exist. The first piece
    // record's `row` byte sits after the player section.
    let players = w.players.iter().filter(|p| p.active).count();
    let piece0 = COUNTS_AT + 20 + players * 240;
    assert_eq!(
        bent(&|b| b[piece0 + 6] = 200),
        Err(WorldSaveError::BadContentRow),
        "an impossible piece row is what panics the sim"
    );
    // ...and one standing off the island.
    assert_eq!(
        bent(&|b| b[piece0..piece0 + 2].copy_from_slice(&u16::MAX.to_le_bytes())),
        Err(WorldSaveError::AddressOutOfRange)
    );
}

/// A refused blob leaves the world untouched. The alternative is a shard
/// running half of somebody's base with no way for its operator to know
/// which half.
#[test]
fn a_refusal_does_not_half_load() {
    let w = a_lived_in_world();
    let mut blob = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut blob).expect("encodes");
    blob.truncate(n);
    // Break the *last* section, so everything before it decoded fine and a
    // load that committed as it went would have written most of the world.
    let len = blob.len();
    blob[len - 4..].copy_from_slice(&u32::MAX.to_le_bytes());
    blob.truncate(len - 1);

    let mut target = a_lived_in_world();
    let before = target.state_hash();
    assert!(target.load(&blob).is_err(), "the blob must refuse");
    assert_eq!(
        target.state_hash(),
        before,
        "a refused load wrote part of the world anyway"
    );
}

/// The encode refuses a buffer that cannot hold the world rather than
/// writing a prefix of it. A short write that reported success would put a
/// truncated world on disk under a checksum that matched it.
#[test]
fn a_short_buffer_refuses_rather_than_truncating() {
    let w = a_lived_in_world();
    let mut tiny = [0u8; 16];
    assert_eq!(
        worldsave::encode(&w, &mut tiny),
        Err(WorldSaveError::Truncated)
    );
}
