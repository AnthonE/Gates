//! Locks on boxes, at the world's own surface (`reference/DOORS.md`
//! §9.8; lock v1's follow-on): the box's **open path is the move verb**,
//! and this file is the gate on the one check that guards it.
//!
//! `deploy.rs`'s test mod owns the store-level half — bolting, arming,
//! the mirror, the pickup rule. What only a `World` can prove is the
//! seam: `Command::Move` resolving a `CONT_BOX` handle must ask
//! `Locks::passes` at the box's address before it reads a slot, and
//! refuse with the locked **door's** reason — `REFUSE_D_OWNER` on the
//! deploy-refused lane, because *this lock does not know you* is one
//! sentence whichever thing refused it, and the panel predicted nothing
//! (`ui/slots.rs`: a drag is not a prediction), so no move rollback is
//! owed.
//!
//! The first test is deliberately a **mutant-killer**: revert only the
//! `lock_passes` check in `World::move_item` and `a_locked_box_refuses_
//! a_stranger_and_answers_its_owner` goes red on its first assertion —
//! the stranger's withdrawal lands as `EV_MOVED` and the contents move.
//!
//! Also here: the wire-bits fact item 3 of the slice asked to have
//! verified. A locked box's `has_lock`/`locked` ride `DeployRec`, the
//! struct `protocol::write_deploy_rec` encodes arch-agnostically (the
//! record's trailing three bits, wire v19/v30), so the assertion that the
//! *record* carries them is the assertion that the wire does — zero
//! protocol edits, no goldens moved.

use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::deploy::{
    box_key, DeployContent, DeployDef, ARCH_BOX, PLACE_FOUNDATION, REFUSE_D_OWNER,
};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::inventory::{CONT_BOX, CONT_SELF};
use sim_core::lock::GRANT_GUEST;
use sim_core::world::{Command, World, EV_AUTH, EV_DEPLOY_REFUSED, EV_MOVED, EV_MOVE_REFUSED};
use sim_core::worldsave::WORLD_SAVE_MAX_BYTES;

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

const SEED: u64 = 0x6000_10CB;
const OWNER: u32 = 3;
const STRANGER: u32 = 9;

/// `probe_fixture` (rows 0..=5, the code lock on 5 / item 7) plus a box
/// on row 6 / item 8 — the shipped fixture is left alone so the replay
/// golden does not move under a row its script never places.
const BOX_ROW: u16 = 6;
const BOX_ITEM: u16 = 8;
const LOCK_ROW: u16 = 5;
const LOCK_ITEM: u16 = 7;
const FOUNDATION_ROW: u16 = 0;
/// The stackable the box holds in these tests (fixture "wood").
const GOODS: u16 = 0;

fn lockbox_fixture() -> DeployContent {
    let mut d = DeployContent::probe_fixture();
    d.defs[BOX_ROW as usize] = DeployDef {
        arch: ARCH_BOX,
        placement: PLACE_FOUNDATION,
        hp: 60,
        item: BOX_ITEM,
        ..DeployDef::INERT
    };
    d.def_count = 7;
    d
}

/// The gather fixture with a stack ladder for the box item, so a picked
/// up box has somewhere to land (its `stack_max` reads 0 off the shipped
/// eight-item table).
fn lockbox_gather() -> GatherContent {
    let mut g = GatherContent::probe_fixture();
    g.stack_max[BOX_ITEM as usize] = 100;
    g
}

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

/// The nearest cell the foundation's terrain rule accepts, in ring order
/// (deterministic for a seed) — `tests/box_container.rs`'s scan.
fn buildable_cell(seed: u64) -> (u16, u16) {
    for r in 0..64i32 {
        for dz in -r..=r {
            for dx in -r..=r {
                if dx.abs() != r && dz.abs() != r {
                    continue;
                }
                let cx = (512 + dx).clamp(0, 1023) as u16;
                let cz = (512 + dz).clamp(0, 1023) as u16;
                let (x, z) = cell_center(cx, cz);
                if foundation_terrain_ok(seed, hv(seed), x, z) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell within 64 cells — the generator changed under this test");
}

/// A world with the owner standing on a foundation, a box on it holding
/// some goods, and a **code lock bolted on and armed** with `code`.
/// Returns the world and the box's cell.
fn locked_box_world(code: u16) -> (World, u16, u16) {
    let mut w = World::new(SEED);
    w.gather = lockbox_gather();
    w.build = BuildContent::probe_fixture();
    w.deploy = lockbox_fixture();

    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: OWNER }]);
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z);

    w.players[0].inv[0] = ItemStack {
        item: GOODS,
        count: 50,
        cond: 0,
    };
    w.players[0].inv[1] = ItemStack {
        item: BOX_ITEM,
        count: 1,
        cond: 0,
    };
    w.players[0].inv[2] = ItemStack {
        item: LOCK_ITEM,
        count: 1,
        cond: 0,
    };
    // Foundation costs 5 of item 0 off slot 0 (build fixture row 0).
    w.tick(&[Command::Place {
        id: OWNER,
        row: FOUNDATION_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.pieces.len(), 1, "the fixture needs its foundation");
    w.tick(&[Command::PlaceDeploy {
        id: OWNER,
        row: BOX_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.deploys.len(), 1, "the fixture needs its box placed");

    // Ten of the goods go inside while the box is still bare — which is
    // itself the rule under test working from the other side.
    w.tick(&[Command::Move {
        id: OWNER,
        cont: box_key(cx, cz, 0),
        from_kind: CONT_SELF,
        from_slot: 0,
        to_kind: CONT_BOX,
        to_slot: 0,
        count: 10,
    }]);
    assert_eq!(
        w.deploys.boxes()[0].items[0],
        ItemStack {
            item: GOODS,
            count: 10,
            cond: 0,
        },
        "a bare box banks the owner's deposit"
    );

    // Bolt the lock on and arm it.
    w.tick(&[Command::PlaceDeploy {
        id: OWNER,
        row: LOCK_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.deploys.locks().len(), 1, "the lock bolts onto the box");
    w.tick(&[Command::Access {
        id: OWNER,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        op: sim_core::deploy::ACCESS_OP_SET_CODE,
        code,
    }]);
    let d = w
        .deploys
        .find(cx, cz, 0, LOC_PLANE)
        .expect("the box record");
    assert!(
        d.has_lock && d.locked,
        "the armed lock mirrors onto the record the wire encodes"
    );
    (w, cx, cz)
}

/// Seat a second player beside the box.
fn join_at(w: &mut World, id: u32, cx: u16, cz: u16) -> usize {
    let (x, z) = cell_center(cx, cz);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id }]);
    let slot = (0..w.players.len())
        .find(|&s| w.players[s].active && w.players[s].id == id)
        .expect("the joiner seats");
    w.players[slot].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z);
    slot
}

/// One move against the box; what came back on the two lanes a box move
/// can answer on: `(moved, move_refused_reason, deploy_refused_reason)`.
fn try_withdraw(w: &mut World, id: u32, cx: u16, cz: u16, to_slot: u8) -> (bool, u32, u32) {
    w.tick(&[Command::Move {
        id,
        cont: box_key(cx, cz, 0),
        from_kind: CONT_BOX,
        from_slot: 0,
        to_kind: CONT_SELF,
        to_slot,
        count: 1,
    }]);
    let mut moved = false;
    let mut move_refused = 0;
    let mut deploy_refused = u32::MAX;
    for e in w.events.entries() {
        match e.code {
            EV_MOVED if e.a == id => moved = true,
            EV_MOVE_REFUSED if e.a == id => move_refused = e.b,
            EV_DEPLOY_REFUSED if e.a == id => deploy_refused = e.b,
            _ => {}
        }
    }
    (moved, move_refused, deploy_refused)
}

/// The mutant-killer. Revert only the `lock_passes` check in
/// `World::move_item`'s `CONT_BOX` arm and the first assertion goes red:
/// the stranger's withdrawal lands, the contents move, and the refusal
/// never fires.
#[test]
fn a_locked_box_refuses_a_stranger_and_answers_its_owner() {
    let (mut w, cx, cz) = locked_box_world(1234);
    let stranger_slot = join_at(&mut w, STRANGER, cx, cz);

    let before = w.deploys.boxes()[0].items[0];
    let (moved, _, deploy_refused) = try_withdraw(&mut w, STRANGER, cx, cz, 5);
    assert!(
        !moved,
        "a locked box must not answer a hand it does not know"
    );
    assert_eq!(
        deploy_refused, REFUSE_D_OWNER,
        "and the refusal is the locked door's own reason"
    );
    assert_eq!(
        w.deploys.boxes()[0].items[0],
        before,
        "the contents did not move"
    );

    // Deposit is the same gate: the to-side resolves the same box.
    w.players[stranger_slot].inv[0] = ItemStack {
        item: GOODS,
        count: 5,
        cond: 0,
    };
    w.tick(&[Command::Move {
        id: STRANGER,
        cont: box_key(cx, cz, 0),
        from_kind: CONT_SELF,
        from_slot: 0,
        to_kind: CONT_BOX,
        to_slot: 3,
        count: 5,
    }]);
    assert!(
        w.events
            .entries()
            .iter()
            .any(|e| e.code == EV_DEPLOY_REFUSED && e.a == STRANGER && e.b == REFUSE_D_OWNER),
        "a deposit is refused by the same predicate"
    );
    assert_eq!(
        w.deploys.boxes()[0].items[3],
        ItemStack::default(),
        "nothing landed in the box"
    );

    // The owner is remembered (bolting it on is the first membership) and
    // the box simply works.
    let (moved, _, _) = try_withdraw(&mut w, OWNER, cx, cz, 5);
    assert!(moved, "the lock remembers the hand that bolted it on");
}

/// The guest code authorizes at a box exactly as at a door: entering it
/// is not opening anything, it is being remembered — and thereafter the
/// box simply works.
#[test]
fn the_guest_code_opens_the_box_thereafter() {
    let (mut w, cx, cz) = locked_box_world(1234);
    join_at(&mut w, STRANGER, cx, cz);
    w.tick(&[Command::Access {
        id: OWNER,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        op: sim_core::deploy::ACCESS_OP_SET_GUEST,
        code: 4321,
    }]);

    let (moved, _, deploy_refused) = try_withdraw(&mut w, STRANGER, cx, cz, 5);
    assert!(!moved && deploy_refused == REFUSE_D_OWNER, "not yet known");

    w.tick(&[Command::Access {
        id: STRANGER,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        op: sim_core::deploy::ACCESS_OP_ENTER,
        code: 4321,
    }]);
    assert!(
        w.events
            .entries()
            .iter()
            .any(|e| e.code == EV_AUTH && e.c == STRANGER && (e.b & 0xff) as u8 == GRANT_GUEST),
        "the correct guest code announces the grant to its sender"
    );
    let (moved, _, _) = try_withdraw(&mut w, STRANGER, cx, cz, 5);
    assert!(moved, "and thereafter the box simply works");
}

/// Unlocking reopens the box to anyone — the shop-front state — and the
/// mirror bit on the wire's own record follows the store both ways.
#[test]
fn unlocking_reopens_the_box_to_anyone() {
    let (mut w, cx, cz) = locked_box_world(1234);
    join_at(&mut w, STRANGER, cx, cz);
    w.tick(&[Command::Access {
        id: OWNER,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        op: sim_core::deploy::ACCESS_OP_UNLOCK,
        code: 0,
    }]);

    let d = w
        .deploys
        .find(cx, cz, 0, LOC_PLANE)
        .expect("the box record");
    assert!(
        d.has_lock && !d.locked,
        "the record the deploy sync encodes says: keypad on, open to all"
    );
    assert!(!w.deploys.locks()[0].locked, "and it mirrors the store");

    let (moved, _, _) = try_withdraw(&mut w, STRANGER, cx, cz, 5);
    assert!(moved, "an unlocked box is anyone's, like an unlocked door");
}

/// A locked box survives the save. The decoder zeroes `has_lock` and
/// distrusts `locked` by design (`worldsave.rs`), so a load that skipped
/// re-deriving the mirror for boxes — `rebuild_doors` before this slice —
/// would come back drawing a locked box the sim agrees is locked while
/// the record claims it has no keypad at all.
#[test]
fn a_locked_box_round_trips_through_the_save() {
    let (mut w, cx, cz) = locked_box_world(1234);
    w.tick(&[Command::Leave { id: OWNER }]);

    let mut buf = vec![0u8; WORLD_SAVE_MAX_BYTES];
    let n = w.save_world(&mut buf).expect("a live world encodes");
    let mut back = World::new(SEED);
    back.gather = lockbox_gather();
    back.build = BuildContent::probe_fixture();
    back.deploy = lockbox_fixture();
    back.load(&buf[..n]).expect("its own bytes must load");

    assert_eq!(back.deploys.locks().len(), 1, "the lock rode the file");
    let d = back
        .deploys
        .find(cx, cz, 0, LOC_PLANE)
        .expect("the box record");
    assert!(
        d.has_lock && d.locked,
        "the mirror is re-derived for a box exactly as for a door"
    );
    // And the predicate answers off the loaded store: a stranger is still
    // out.
    assert!(!back.deploys.lock_passes(cx, cz, 0, LOC_PLANE, STRANGER));
    assert!(back.deploys.lock_passes(cx, cz, 0, LOC_PLANE, OWNER));
}
