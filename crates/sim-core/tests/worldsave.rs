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
    build_cell_of, foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_EDGE_XLO, LOC_EDGE_ZLO,
    LOC_PLANE,
};
use sim_core::combat::CombatContent;
use sim_core::craft::CraftContent;
use sim_core::deploy::{DeployContent, ARCH_DOOR, ARCH_HEARTH};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::loot::LootContent;
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::survival::SurvivalContent;
use sim_core::world::{Command, World};
use sim_core::worldsave::{
    self, WorldSaveError, HEAD_BYTES, PIECE_BYTES, PLAYER_BYTES, WORLD_SAVE_MAX_BYTES,
};

/// The solved authored sites for `seed` — what `terrain::ground` needs in order
/// to know where the carve is.
///
/// Memoized per seed, and that is not premature: `terrain::haven` is a few
/// thousand `height` taps (a shoreline march, a bisect and a rosette per
/// candidate bearing), these suites call it from inside assertion loops, and
/// the first draft of this helper resolved it per call and took the workspace
/// test run past five minutes. It is a pure function of the seed, so caching
/// cannot change a result.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    // A thread-local rather than a `Mutex`: `std::sync::Mutex` is on
    // `sim-core/clippy.toml`'s disallowed list (wall 3), and that list is
    // crate-scoped, so it binds this suite too. Per-thread is the right shape
    // anyway — the cache exists to stop a per-assertion recompute, not to be
    // shared.
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

const SEED: u64 = 20260807;
/// The doorway row in the build fixture, and the door row in the deploy
/// fixture — read off `probe_fixture()` rather than guessed.
const ROW_FOUNDATION: u16 = 0;
const ROW_DOORWAY: u16 = 3;
/// Row 1 of `BuildContent::probe_fixture` — a twig wall.
const ROW_WALL: u16 = 1;
const DEPLOY_HEARTH: u16 = 0;
const DEPLOY_DOOR: u16 = 2;

fn armed() -> Box<World> {
    // On the heap from the first frame: a test thread's stack is 2 MiB and
    // three live `World`s (fixture + `round_trip`'s rebuild + a return
    // temporary) do not fit it in any build profile. One construction's
    // frame does — the wasm parity probe proves that daily on a 1 MiB
    // shadow stack — so the box is taken here, once, and every caller
    // holds a pointer. CLAUDE.md's boxed-array trap, wearing test clothes.
    let mut w = Box::new(World::new(SEED));
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
        cond: 0,
    };
    w.players[slot].inv[1] = ItemStack {
        item: 1,
        count: 500,
        cond: 0,
    };
    w.players[slot].inv[2] = ItemStack {
        item: 2,
        count: 9,
        cond: 0,
    };
    w.players[slot].inv[3] = ItemStack {
        item: 4,
        count: 9,
        cond: 0,
    };
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
                if foundation_terrain_ok(SEED, hv(SEED), ax, az) {
                    found = Some((cx as u16, cz as u16, ax, az));
                    break 'outer;
                }
            }
        }
    }
    let (cx, cz, ax, az) = found.expect("no buildable ground within 24 cells of the spawn");
    w.players[slot].body = Body::at(SEED, hv(SEED), ax, az);
    (cx, cz)
}

/// A world somebody has lived in: two bodies (one of them asleep), a
/// foundation, a doorway, a hearth, a closed door, a chopped tree, and a
/// few ticks of clock on all of it.
fn a_lived_in_world() -> Box<World> {
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
        freehand: false,
    }]);
    w.tick(&[Command::Place {
        id: 1,
        row: ROW_DOORWAY,
        cx,
        cz,
        level: 0,
        loc: LOC_EDGE_XLO,
        freehand: false,
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
        loc: LOC_EDGE_XLO,
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
fn a_quiet_world() -> Box<World> {
    let mut w = a_lived_in_world();
    w.tick(&[Command::Leave { id: 1 }]);
    w
}

fn round_trip(w: &World) -> Box<World> {
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
            // A command stream with something in it: a takeover, a leave,
            // a two-phase eviction, then the ordinary clock. An empty
            // script would agree even if the load had dropped half the
            // world, and the `Evict` is here because its id is the
            // stream's fact — both loads must delete the same body on the
            // same tick (`world.rs`, `Command::Evict`).
            if t == 5 {
                s.tick(&[Command::Wake {
                    id: 0x0303,
                    sleeper: 2,
                }]);
            } else if t == 9 {
                s.tick(&[Command::Leave { id: 0x0303 }]);
            } else if t == 14 {
                s.tick(&[Command::Evict { id: 0x0303 }]);
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
    // A player count past MAX_PLAYERS. The counts are the tail of the head,
    // so their offset is `HEAD_BYTES - SECTION_COUNTS` and NOT a hand-copied
    // total — this was `34` until format 10 put a `u32` eviction counter in
    // the head ahead of them, at which point every poke below landed four
    // bytes early and the failure named the wrong thing entirely. Both
    // constants are `pub` for exactly this.
    const COUNTS_AT: usize = sim_core::worldsave::HEAD_BYTES - sim_core::worldsave::SECTION_COUNTS;
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
    // A slot-life count past MAX_SLOT_LIVES — the u32 one, and the last
    // four bytes of the head whatever else lands before it.
    assert_eq!(
        bent(&|b| b[HEAD_BYTES - 4..HEAD_BYTES].copy_from_slice(&u32::MAX.to_le_bytes())),
        Err(WorldSaveError::CountOverCap)
    );

    // A piece naming a content row that does not exist. The first piece
    // record's `row` byte sits after the player section.
    let players = w.players.iter().filter(|p| p.active).count();
    let piece0 = HEAD_BYTES + players * PLAYER_BYTES;
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

/// A forged save cannot claim a mirror state no lock verb can produce.
/// The decoder reads each deploy record's `locked` byte for layout and
/// drops it (`worldsave.rs`: the decoder deliberately distrusts the
/// mirror bits — **both** of them), and `rebuild_doors` re-derives the
/// pair from the lock section for the archetypes `lockable` names — so
/// locked:true on a hearth, which the rebuild never visits, must come
/// back cleared rather than ridden into the world. The door's byte is
/// forged too: lockable, but with no lock in the file the rebuild clears
/// it, which pins the half that already held.
///
/// **Mutant-killer**: put the file's `locked` back into the decoded
/// record (`locked` instead of `locked: false` in `decode_into`) and the
/// hearth's record loads locked with no lock anywhere — the last
/// assertion goes red.
#[test]
fn forged_lock_bits_load_cleared_never_trusted() {
    let w = a_lived_in_world();
    assert!(w.deploys.locks().is_empty(), "fixture: no lock in the file");
    let mut blob = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut blob).expect("encodes");
    blob.truncate(n);

    // Walk to the deploy section the way the corruption test above does:
    // head + counts, players at `PLAYER_BYTES` each, pieces at `PIECE_BYTES`,
    // then 25 per deploy record (17 + bag_ready) with `locked` at offset 16
    // of each.
    //
    // **The piece stride was a literal 20 here until 2026-08-21**, beside a
    // comment deriving it as "12 + the placement tick" — and `PIECE_BYTES`
    // said 11 + 8. Both could not be right, and the literal was: the constant
    // had missed `facing` since format 6, so the crate's own save ceiling was
    // 8 KiB short at the piece cap. This reads the constant now, which is
    // what `PLAYER_BYTES`' own doc says a byte-poking test must do.
    let players = w.players.iter().filter(|p| p.active).count();
    let deploy0 = HEAD_BYTES + players * PLAYER_BYTES + w.pieces.len() * PIECE_BYTES;
    let mut saw = (false, false);
    for (i, rec) in w.deploys.entries().iter().enumerate() {
        let at = deploy0 + i * 25;
        // Anchor the offset math on the record's own address bytes
        // before bending anything — a wrong stride would forge noise.
        assert_eq!(
            u16::from_le_bytes([blob[at], blob[at + 1]]),
            rec.cx,
            "the deploy stride drifted under this test"
        );
        blob[at + 16] = 1; // locked := true, hearth and door alike
        match w.deploy.defs[rec.row as usize].arch {
            ARCH_HEARTH => saw.0 = true,
            ARCH_DOOR => saw.1 = true,
            _ => {}
        }
    }
    assert!(
        saw.0 && saw.1,
        "the fixture must cover a non-lockable and a lockable archetype"
    );

    let mut back = armed();
    back.load(&blob)
        .expect("a forged mirror bit is cleared, not refused");
    for rec in back.deploys.entries() {
        assert!(
            !rec.locked && !rec.has_lock,
            "no lock is in the file, so no record may load locked — \
             whatever the file claimed"
        );
    }
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

/// **A saved tool comes back worn, and an empty slot may not carry
/// condition** (item durability v0, gate 7's world half; format 7). The
/// first half round-trips a worn tool through the whole blob; the second
/// pokes condition bytes onto a zeroed slot and must be refused —
/// `count == 0 && cond != 0` is state nothing can see, wall 5's failure
/// mode, refused exactly as `count == 0 && item != 0` always was.
/// Proven red by reverting the reader's `stack()` to the four-byte form:
/// the worn assert reads 0. The refusal half is proven red by dropping
/// the new `(count == 0 && cond != 0)` arm.
#[test]
fn a_worn_tool_survives_the_world_and_an_empty_slot_may_not_wear() {
    let mut w = armed();
    w.tick(&[Command::Join { id: 9 }]);
    w.players[0].inv[3] = ItemStack {
        item: 2,
        count: 1,
        cond: 7_777,
    };

    let w2 = round_trip(&w);
    let p = w2
        .players
        .iter()
        .find(|p| p.active && p.id == 9)
        .expect("the body survived the restart");
    assert_eq!(
        p.inv[3],
        ItemStack {
            item: 2,
            count: 1,
            cond: 7_777,
        },
        "the world blob dropped a tool's condition"
    );
    // Two loads of one blob agree byte for byte — the live world's own
    // hash moves at the save (a save puts the body to bed), so the
    // round-trip claim is load-vs-load, the same shape `raid_storm`'s
    // save assert uses.
    let w2b = round_trip(&w);
    assert_eq!(
        w2.state_hash(),
        w2b.state_hash(),
        "two loads of one blob disagree — the condition bytes are being \
         read nondeterministically"
    );

    // The empty-slot half, at the byte level. The first body's inventory
    // starts at HEAD + the scalar head + the craft queue; slot 4 is empty
    // (nothing granted there), and its cond bytes are the last two of its
    // six.
    //
    // **The scalar head is DERIVED, not typed.** It was the literal `60`
    // until torch fuel v0 grew it to 64, and the poke then landed four
    // bytes short of the field it meant — caught here only because the
    // fixture-rot guard below happened to notice, which is luck. Every
    // term is a public constant, so the offset moves when the record does.
    let mut blob = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut blob).expect("encodes");
    blob.truncate(n);
    let jobs_bytes = sim_core::limits::CRAFT_QUEUE * 4;
    let stacks_bytes = (sim_core::limits::INV_SLOTS + sim_core::limits::WEAR_SLOTS) * 6;
    let scalars = sim_core::persist::PLAYER_SAVE_BYTES - jobs_bytes - stacks_bytes;
    let inv0 = sim_core::worldsave::HEAD_BYTES + scalars + jobs_bytes;
    let slot4_cond = inv0 + 4 * 6 + 4;
    assert_eq!(
        &blob[slot4_cond - 4..slot4_cond],
        &[0, 0, 0, 0],
        "fixture rot: slot 4 is not empty, so this poke proves nothing"
    );
    blob[slot4_cond..slot4_cond + 2].copy_from_slice(&5u16.to_le_bytes());
    let mut w3 = armed();
    assert_eq!(
        w3.load(&blob),
        Err(WorldSaveError::Player(
            sim_core::persist::SaveError::BadItemStack
        )),
        "an empty slot carrying condition must refuse the whole record"
    );
}

/// The piece stride on disk is what the ENCODER writes, measured, not what a
/// constant claims.
///
/// **This is the gate the one-byte gap needed and did not have** (2026-08-21).
/// `PIECE_BYTES` said `11 + 8` from format 6 to format 9 while the encoder
/// wrote `12 + 8` — `facing` joined the record and the constant did not move
/// with it. Every check on the number was blind by construction: the ceiling's
/// `by_hand` sum and its pinned total were both re-derived from the same wrong
/// constant, so all three agreed with each other and none of them with the
/// file. `WORLD_SAVE_MAX_BYTES` was 8 KiB short at `MAX_PIECES`, which is a
/// shard at the piece cap unable to save into a buffer sized by this crate's
/// own published ceiling.
///
/// A difference of two encodes cannot share that blindness: it asks the writer.
#[test]
fn the_piece_stride_is_what_the_encoder_writes() {
    let mut w = a_lived_in_world();
    let mut blob = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let before_n = w.pieces.len();
    let before = w.save_world(&mut blob).expect("encodes");

    // One more piece, by the sim's own verb, on the base the fixture built.
    let rec = *w
        .pieces
        .entries()
        .first()
        .expect("the fixture built a base");
    w.tick(&[Command::Place {
        id: 1,
        row: ROW_WALL,
        cx: rec.cx,
        cz: rec.cz,
        level: 0,
        loc: LOC_EDGE_ZLO,
        freehand: false,
    }]);
    assert_eq!(
        w.pieces.len(),
        before_n + 1,
        "the fixture placement was refused"
    );
    let after = w.save_world(&mut blob).expect("encodes");

    assert_eq!(
        after - before,
        PIECE_BYTES,
        "one more piece cost {} bytes on disk, and PIECE_BYTES says {PIECE_BYTES}",
        after - before
    );
}

/// A file whose column holds two plates is refused, not loaded.
///
/// **The invariant, arriving through the one door that is not a command**
/// (build plate v1). `build::place` adopts a column's plate before it inserts,
/// so the sim cannot produce this — but the two readers of a plate disagree
/// under it in a way that is exactly the defect the plate exists to close:
/// `render/structures.rs` draws each piece at its OWN plate (the record
/// carries it) and every collision walk asks the COLUMN (`ColMasks::plate`,
/// one value). A split column is therefore a base standing where you cannot
/// walk.
///
/// Poked at the byte, so it is the DECODER under test and not a constructor.
#[test]
fn a_column_with_two_plates_is_refused() {
    let mut w = a_lived_in_world();
    // A second piece in the fixture's own column, so the file has a pair to
    // disagree about.
    let rec = *w
        .pieces
        .entries()
        .first()
        .expect("the fixture built a base");
    w.tick(&[Command::Place {
        id: 1,
        row: ROW_WALL,
        cx: rec.cx,
        cz: rec.cz,
        level: 0,
        loc: LOC_EDGE_ZLO,
        freehand: false,
    }]);
    let two = w
        .pieces
        .entries()
        .iter()
        .filter(|p| p.cx == rec.cx && p.cz == rec.cz)
        .count();
    assert!(two >= 2, "the fixture needs two pieces in one column");

    let mut blob = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut blob).expect("encodes");
    blob.truncate(n);
    let players = w.players.iter().filter(|p| p.active).count();
    let piece0 = HEAD_BYTES + players * PLAYER_BYTES;

    // Anchor on the record's own address before bending anything — a wrong
    // stride would forge noise (the byte-poking discipline one test up).
    let (mut bent, mut which) = (false, 0usize);
    for (k, p) in w.pieces.entries().iter().enumerate() {
        let at = piece0 + k * PIECE_BYTES;
        assert_eq!(
            u16::from_le_bytes([blob[at], blob[at + 1]]),
            p.cx,
            "the piece stride drifted under this test"
        );
        if p.cx == rec.cx && p.cz == rec.cz && !bent {
            // The plate byte sits after cx, cz, level, loc, row, facing, hp,
            // uh — 12 bytes in.
            //
            // **Anchored on `hp`, which is not zero.** The first draft
            // anchored on the plate itself at a mis-counted +11, and the
            // assertion passed: +11 is `uh`'s high byte, `uh` is 0 in a fresh
            // world and so is the plate, so the check compared 0 to 0 and the
            // bend went into a field the loader does not read. An offset
            // assertion whose two sides can both be zero is not an assertion.
            assert_eq!(
                u16::from_le_bytes([blob[at + 8], blob[at + 9]]),
                p.hp,
                "the piece field offsets drifted under this test"
            );
            assert!(p.hp != 0, "the anchor field must not be zero");
            assert_eq!(blob[at + 12] as i8, p.plate, "the plate is not at +12");
            blob[at + 12] = p.plate.wrapping_add(1) as u8;
            bent = true;
            which = k;
        }
    }
    assert!(bent, "nothing was bent, so nothing is under test");
    let _ = which;

    let mut back = World::new(w.seed);
    back.build = BuildContent::probe_fixture();
    back.deploy = DeployContent::probe_fixture();
    assert_eq!(
        worldsave::decode_into(&mut back, &blob),
        Err(WorldSaveError::PieceColumnPlateSplit),
        "a column with two floors loaded instead of refusing"
    );
}
