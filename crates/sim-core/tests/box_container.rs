//! The deployed box, as a container: storage, an address, and the move
//! verb reaching it.
//!
//! `ARCH_BOX` has baked and placed since the deployable slice landed and
//! held nothing — the judges' "a raid takes nothing" half. What closes it
//! is not a new verb. `Command::Move` already spans `CONT_SELF` and
//! `CONT_BAG` with validation ordered before the mutation; a box is a
//! third `CONT_*` kind resolved against a third store, and **not one line
//! of `plan_move` moves**. So this file is mostly about the two things a
//! third kind can get wrong that the first two could not:
//!
//! 1. **A box is smaller than an inventory.** `BOX_SLOTS` is 12 and
//!    `INV_SLOTS` is 30, and the wire bounds every slot at the widest
//!    container because a frame decoder may not need a content table. So
//!    slots 12..30 of a box arrive as a legal *shape* naming an illegal
//!    *address*, and the sim owns that refusal. Get it wrong and items
//!    land in the tail of the record where every reader stops short —
//!    invisible, unmovable, and in the state hash.
//! 2. **A box is addressed, not identified.** A bag has an id; a box is
//!    furniture and has a packed `box_key(cx, cz, level)`. The handle
//!    field is one `u32` shared by both sides of the move, which is why a
//!    bag→box move is refused rather than encoded — the message has no
//!    room to name the second container.
//!
//! And one thing about what a box *is*: emptying one leaves it standing,
//! where emptying a bag removes it. Breaking one spills what was inside
//! onto the floor it stood on, by the same `stand_up` a corpse and a
//! barrel use — otherwise a raid that breaks the box destroys the reward
//! for breaking it.

use sim_core::backpack::BackpackContent;
use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::deploy::{box_key, DeployContent, DeployDef, ARCH_BOX, PLACE_FOUNDATION};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::inventory::{
    CONT_BAG, CONT_BOX, CONT_SELF, REFUSE_M_NO_CONTAINER, REFUSE_M_REACH, REFUSE_M_SLOT,
};
use sim_core::limits::{BOX_SLOTS, INV_SLOTS, MAX_BUILD_COORD, MAX_BUILD_LEVELS, MAX_ITEM_DEFS};
use sim_core::world::{Command, World, EV_MOVED, EV_MOVE_REFUSED};

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

const SEED: u64 = 0x6000_00B0;
const PLAYER: u32 = 3;

/// Row 4 of the fixture below is the box; it costs one unit of item 6.
/// Rows 0..3 are `DeployContent::probe_fixture`'s and are left alone.
const BOX_ROW: u16 = 4;
const BOX_ITEM: u16 = 6;
/// `BuildContent::probe_fixture` row 0 is the foundation, at five item 0.
const FOUNDATION_ROW: u16 = 0;

fn box_fixture() -> DeployContent {
    let mut d = DeployContent::probe_fixture();
    d.defs[4] = DeployDef {
        arch: ARCH_BOX,
        placement: PLACE_FOUNDATION,
        hp: 100,
        item: BOX_ITEM,
        ..DeployDef::INERT
    };
    d.def_count = 5;
    d
}

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

/// The nearest cell the foundation's terrain rule accepts. Ring order, so
/// this is deterministic for a seed and does not depend on scan direction.
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

/// A world with one player standing on a foundation with a box on it.
/// Returns the box's packed address alongside, because every move below
/// names it and recomputing it in each test is how a test starts agreeing
/// with a bug.
fn box_world() -> (World, u32, u16, u16) {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = box_fixture();
    // The spill stands a real bag up, so the bag ladder has to be armed or
    // `stand_up` refuses and the spill vanishes silently. Long lifetime:
    // these tests assert on a bag several ticks after it drops.
    let mut bc = BackpackContent::probe_fixture();
    bc.despawn_ticks = [0; MAX_ITEM_DEFS];
    bc.base_ticks = 1 << 30;
    w.backpack = bc;

    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: PLAYER }]);
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), x, z);

    w.players[0].inv[0] = ItemStack {
        item: 0,
        count: 5,
        cond: 0,
    };
    w.players[0].inv[1] = ItemStack {
        item: BOX_ITEM,
        count: 1,
        cond: 0,
    };
    w.tick(&[Command::Place {
        id: PLAYER,
        row: FOUNDATION_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        freehand: false,
    }]);
    assert_eq!(w.pieces.len(), 1, "the fixture needs its foundation");
    w.tick(&[Command::PlaceDeploy {
        id: PLAYER,
        row: BOX_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.deploys.len(), 1, "the fixture needs its box placed");
    (w, box_key(cx, cz, 0), cx, cz)
}

/// Everything a move is allowed to touch: the player's inventory, every
/// box slot, and every bag slot. `state_hash` cannot be used for this —
/// it mixes in `tick`, and every `do_move` advances one — so the
/// conservation checks compare this instead.
fn contents(w: &World) -> (Vec<ItemStack>, Vec<ItemStack>, Vec<ItemStack>) {
    (
        w.players[0].inv.to_vec(),
        w.deploys
            .boxes()
            .iter()
            .flat_map(|b| b.items.iter().copied())
            .collect(),
        w.backpacks
            .entries()
            .iter()
            .flat_map(|b| b.items.iter().copied())
            .collect(),
    )
}

/// Run one move and return the single event it produced: `(code, b, c)`.
/// Asserts *exactly one* move event, which makes this a double-emit gate
/// too — an emit site that announced twice would be two rollback
/// instructions for one drag on the client.
#[allow(clippy::too_many_arguments)]
fn do_move(
    w: &mut World,
    cont: u32,
    from_kind: u8,
    from_slot: u8,
    to_kind: u8,
    to_slot: u8,
    count: u16,
) -> (u8, u32, u32) {
    w.tick(&[Command::Move {
        id: PLAYER,
        cont,
        from_kind,
        from_slot,
        to_kind,
        to_slot,
        count,
    }]);
    let found: Vec<_> = w
        .events
        .entries()
        .iter()
        .filter(|e| e.code == EV_MOVED || e.code == EV_MOVE_REFUSED)
        .map(|e| (e.code, e.b, e.c))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "a move must answer exactly once — got {} for \
         ({from_kind}:{from_slot} -> {to_kind}:{to_slot} x{count})",
        found.len()
    );
    found[0]
}

/// Placing a box allocates a record and that record answers to the packed
/// address the client can compute from the deploy sync it already draws.
#[test]
fn a_placed_box_takes_a_record_and_an_address() {
    let (w, key, cx, cz) = box_world();
    assert_eq!(w.deploys.boxes().len(), 1, "the box store did not allocate");
    let b = w.deploys.boxes()[0];
    assert_eq!((b.cx, b.cz, b.level), (cx, cz, 0));
    assert_eq!(b.owner, PLAYER, "the placer owns the record");
    assert!(b.is_empty(), "a fresh box holds nothing");
    assert_eq!(
        w.deploys.box_index(key),
        Some(0),
        "the packed address must resolve to the record"
    );
    // Every one of the twelve slots reads empty, and the thirteenth is not
    // a slot at all — a total reader, like `Backpacks::slot`.
    for s in 0..BOX_SLOTS {
        assert_eq!(w.deploys.box_slot(0, s), ItemStack::default());
    }
    assert_eq!(w.deploys.box_slot(0, BOX_SLOTS), ItemStack::default());
    assert_eq!(w.deploys.box_slot(9, 0), ItemStack::default());
}

/// The verb itself, both directions, with the counts checked on both
/// sides — a move that credited the destination without debiting the
/// source is the dupe this whole module is shaped to prevent.
#[test]
fn items_move_into_and_out_of_a_box() {
    let (mut w, key, _, _) = box_world();
    w.players[0].inv[4] = ItemStack {
        item: 1,
        count: 40,
        cond: 0,
    };

    let (code, _, _) = do_move(&mut w, key, CONT_SELF, 4, CONT_BOX, 0, 25);
    assert_eq!(code, EV_MOVED, "a legal deposit must be applied");
    assert_eq!(
        w.players[0].inv[4],
        ItemStack {
            item: 1,
            count: 15,
            cond: 0,
        }
    );
    assert_eq!(
        w.deploys.box_slot(0, 0),
        ItemStack {
            item: 1,
            count: 25,
            cond: 0,
        }
    );

    // Back out, into a different slot, so the return path is not just the
    // deposit read backwards.
    let (code, _, _) = do_move(&mut w, key, CONT_BOX, 0, CONT_SELF, 7, 10);
    assert_eq!(code, EV_MOVED, "a legal withdrawal must be applied");
    assert_eq!(
        w.deploys.box_slot(0, 0),
        ItemStack {
            item: 1,
            count: 15,
            cond: 0,
        }
    );
    assert_eq!(
        w.players[0].inv[7],
        ItemStack {
            item: 1,
            count: 10,
            cond: 0,
        }
    );

    // And within the box, which is the case that reads and writes the
    // same array twice through the same index.
    let (code, _, _) = do_move(&mut w, key, CONT_BOX, 0, CONT_BOX, 11, 15);
    assert_eq!(code, EV_MOVED);
    assert_eq!(w.deploys.box_slot(0, 0), ItemStack::default());
    assert_eq!(
        w.deploys.box_slot(0, 11),
        ItemStack {
            item: 1,
            count: 15,
            cond: 0,
        }
    );
    // The foundation and the box each spent their own cost at placement,
    // so the forty deposited units are all that is left to conserve.
    let total: u16 = (0..BOX_SLOTS)
        .map(|s| w.deploys.box_slot(0, s).count)
        .sum::<u16>()
        + w.players[0].inv.iter().map(|s| s.count).sum::<u16>();
    assert_eq!(total, 40, "no move may create or destroy a unit");
}

/// **The slot-width gate.** A box has twelve slots and the wire bounds a
/// slot at thirty, so 12..30 is a legal frame naming an address that does
/// not exist. Every one of them must refuse, and — the half that actually
/// bites — must leave the record untouched, because a write that landed
/// past `BOX_SLOTS` would be invisible to every reader and still hashed.
#[test]
fn a_box_slot_past_its_size_is_refused_and_writes_nothing() {
    let (mut w, key, _, _) = box_world();
    w.players[0].inv[4] = ItemStack {
        item: 1,
        count: 40,
        cond: 0,
    };

    for s in BOX_SLOTS..INV_SLOTS {
        let before = contents(&w);
        let (code, why, _) = do_move(&mut w, key, CONT_SELF, 4, CONT_BOX, s as u8, 1);
        assert_eq!(code, EV_MOVE_REFUSED, "box slot {s} must not be nameable");
        assert_eq!(why, REFUSE_M_SLOT, "slot {s} is an address error");
        assert_eq!(
            contents(&w),
            before,
            "a refused move at slot {s} moved something"
        );
        // The source is intact, so nothing was taken and dropped either.
        assert_eq!(
            w.players[0].inv[4],
            ItemStack {
                item: 1,
                count: 40,
                cond: 0,
            }
        );
        // And the same slot as a *source* is equally not an address.
        let (code, why, _) = do_move(&mut w, key, CONT_BOX, s as u8, CONT_SELF, 5, 1);
        assert_eq!(code, EV_MOVE_REFUSED);
        assert_eq!(why, REFUSE_M_SLOT);
    }
    // The last real slot is still real — a bound that refused everything
    // would pass every assert above and be useless.
    let (code, _, _) = do_move(
        &mut w,
        key,
        CONT_SELF,
        4,
        CONT_BOX,
        (BOX_SLOTS - 1) as u8,
        1,
    );
    assert_eq!(code, EV_MOVED, "slot {} is inside the box", BOX_SLOTS - 1);
}

/// An address with no box standing at it, and a box that is real but out
/// of arm's reach. Separate reasons because they are separate
/// player-facing facts: one means "it is gone", the other "walk back".
#[test]
fn an_absent_or_distant_box_refuses_rather_than_disconnects() {
    let (mut w, key, cx, cz) = box_world();
    w.players[0].inv[4] = ItemStack {
        item: 1,
        count: 40,
        cond: 0,
    };

    // Right cell, wrong storey: the level is part of the address, which is
    // the whole reason `box_key` is not `gather::cell_key`.
    let (code, why, _) = do_move(&mut w, box_key(cx, cz, 1), CONT_SELF, 4, CONT_BOX, 0, 1);
    assert_eq!(code, EV_MOVE_REFUSED);
    assert_eq!(why, REFUSE_M_NO_CONTAINER, "no box stands on level 1");

    let (code, why, _) = do_move(&mut w, box_key(cx + 3, cz, 0), CONT_SELF, 4, CONT_BOX, 0, 1);
    assert_eq!(code, EV_MOVE_REFUSED);
    assert_eq!(why, REFUSE_M_NO_CONTAINER, "no box stands three cells over");

    // Real box, player walked away. Reach is judged when the move
    // resolves, never when a panel opened.
    let (fx, fz) = cell_center(cx + 7, cz);
    w.players[0].body = sim_core::movement::Body::at(SEED, hv(SEED), fx, fz);
    let (code, why, _) = do_move(&mut w, key, CONT_SELF, 4, CONT_BOX, 0, 1);
    assert_eq!(code, EV_MOVE_REFUSED);
    assert_eq!(why, REFUSE_M_REACH, "a box 21 m away is out of reach");
    assert!(w.deploys.boxes()[0].is_empty(), "nothing was deposited");
}

/// One handle, one ground container. A bag→box move names a destination
/// the message has no field for, so it refuses instead of guessing which
/// container the handle meant — guessing is how an item leaves one
/// container and arrives in none.
#[test]
fn a_move_between_two_ground_containers_is_refused() {
    let (mut w, key, _, _) = box_world();
    w.players[0].inv[4] = ItemStack {
        item: 1,
        count: 40,
        cond: 0,
    };
    // A bag at the player's feet, dropped the way one reaches the world in
    // play — `drop_for` is the only route, so a fixture that reached into
    // the store could pass while the real one was broken.
    let mut body = w.players[0];
    body.inv = [ItemStack::default(); INV_SLOTS];
    body.inv[0] = ItemStack {
        item: 1,
        count: 9,
        cond: 0,
    };
    let tick = w.tick;
    let bag = w
        .backpacks
        .drop_for(&w.backpack, &body, tick, &mut w.events)
        .expect("the probe ladder is armed and the body is carrying something");
    do_move(&mut w, key, CONT_SELF, 4, CONT_BOX, 0, 5);
    assert_ne!(bag, key, "the test is meaningless if the handles collide");

    for (from, to, handle) in [(CONT_BAG, CONT_BOX, bag), (CONT_BOX, CONT_BAG, key)] {
        let before = contents(&w);
        let (code, why, _) = do_move(&mut w, handle, from, 0, to, 0, 1);
        assert_eq!(code, EV_MOVE_REFUSED, "{from} -> {to} must refuse");
        assert_eq!(why, REFUSE_M_NO_CONTAINER, "{from} -> {to} reason");
        assert_eq!(contents(&w), before, "{from} -> {to} moved something");
    }
}

/// **Mutation note, recorded rather than chased.** Widening the emptied-
/// container check from `ground == CONT_BAG` to `ground != CONT_SELF` —
/// which aims `Backpacks::drop_if_empty` at a *box* index — escapes this
/// test, and it escapes because it is an **equivalent mutant**, not
/// because the test is weak. `drop_if_empty` is total (`i < len`) and
/// removes only an empty bag, and no bag in the store is ever empty:
/// `stand_up` refuses an all-empty set and every path that empties one
/// removes it. So the wrong index can never remove anything. The narrow
/// guard is kept because it is correct on its own terms rather than on
/// another store's invariant, but no assertion here can distinguish them.
///
/// A bag is litter with a timer and vanishes when you empty it; a box is
/// furniture you paid for and placed, and stays. The two containers share
/// every line of the move verb and differ here, so this is the assert that
/// keeps a later edit from unifying them.
#[test]
fn emptying_a_box_leaves_it_standing() {
    let (mut w, key, _, _) = box_world();
    w.players[0].inv[4] = ItemStack {
        item: 1,
        count: 6,
        cond: 0,
    };
    // A bag in the store at the same index the box holds in *its* store.
    // The two are addressed by different handles into different arrays, so
    // an emptied-container check aimed at the wrong one would remove this
    // bag and nothing else would notice — the cross-store index confusion
    // is the actual failure mode, not the box disappearing.
    let mut body = w.players[0];
    body.inv = [ItemStack::default(); INV_SLOTS];
    body.inv[0] = ItemStack {
        item: 2,
        count: 3,
        cond: 0,
    };
    let tick = w.tick;
    let bag = w
        .backpacks
        .drop_for(&w.backpack, &body, tick, &mut w.events)
        .expect("the probe ladder is armed");
    assert_eq!(w.backpacks.len(), 1);

    do_move(&mut w, key, CONT_SELF, 4, CONT_BOX, 0, 6);
    assert_eq!(w.deploys.boxes().len(), 1);

    let (code, _, _) = do_move(&mut w, key, CONT_BOX, 0, CONT_SELF, 4, 6);
    assert_eq!(code, EV_MOVED);
    assert!(w.deploys.boxes()[0].is_empty(), "the box is empty");
    assert_eq!(
        w.deploys.boxes().len(),
        1,
        "an emptied box must stay standing — it is not a bag"
    );
    assert_eq!(w.deploys.len(), 1, "and its deploy record with it");
    assert_eq!(
        w.backpacks.len(),
        1,
        "emptying a box removed a bag — the two stores share an index and \
         nothing else"
    );
    assert!(w.backpacks.find(bag).is_some(), "and it is the same bag");
    // Which means it still takes a deposit afterwards.
    let (code, _, _) = do_move(&mut w, key, CONT_SELF, 4, CONT_BOX, 3, 2);
    assert_eq!(code, EV_MOVED, "the emptied box still accepts items");
}

/// **The raid payoff.** Breaking a box must not destroy what is inside it,
/// or breaking the box is worse than leaving it alone. The contents leave
/// by the same `stand_up` a corpse and a barrel use, so the wire sees one
/// bag contract however the items reached the ground.
#[test]
fn a_broken_box_spills_its_contents_onto_the_floor() {
    let (mut w, key, _, _) = box_world();
    w.players[0].inv[4] = ItemStack {
        item: 1,
        count: 30,
        cond: 0,
    };
    w.players[0].inv[5] = ItemStack {
        item: 2,
        count: 7,
        cond: 0,
    };
    do_move(&mut w, key, CONT_SELF, 4, CONT_BOX, 0, 30);
    do_move(&mut w, key, CONT_SELF, 5, CONT_BOX, 6, 7);
    let bags_before = w.backpacks.len();

    // Straight to zero hp, through the same `drop_deploy` decay and a raid
    // both reach.
    sim_core::deploy::damage_deploy(
        &w.deploy,
        &mut w.pieces,
        &mut w.deploys,
        0,
        9999,
        &mut w.events,
    );
    assert_eq!(w.deploys.len(), 0, "the box came apart");
    assert_eq!(w.deploys.boxes().len(), 0, "and freed its record");
    assert_eq!(w.deploys.box_index(key), None, "the address is dead");

    // The spill drains at the end of the next step, because the removal
    // path holds neither the bag store nor the clock.
    w.tick(&[]);
    assert_eq!(
        w.backpacks.len(),
        bags_before + 1,
        "a broken box must leave a bag"
    );
    let bag = w.backpacks.entries()[bags_before];
    let got: Vec<_> = bag.items.iter().filter(|s| s.count > 0).collect();
    assert_eq!(got.len(), 2, "both stacks survived the break");
    assert!(got.contains(&&ItemStack {
        item: 1,
        count: 30,
        cond: 0,
    }));
    assert!(got.contains(&&ItemStack {
        item: 2,
        count: 7,
        cond: 0,
    }));
    assert_eq!(bag.owner, PLAYER, "the spill belongs to whoever placed it");
    // Drained exactly once: a second tick must not stand a second bag up.
    w.tick(&[]);
    assert_eq!(
        w.backpacks.len(),
        bags_before + 1,
        "the spill buffer was drained twice"
    );
}

/// An empty box that breaks leaves no litter — `stand_up` would refuse it
/// anyway, and parking it would spend the bounded spill buffer on boxes
/// with nothing in them.
#[test]
fn a_broken_empty_box_leaves_no_bag() {
    let (mut w, _, _, _) = box_world();
    let bags_before = w.backpacks.len();
    sim_core::deploy::damage_deploy(
        &w.deploy,
        &mut w.pieces,
        &mut w.deploys,
        0,
        9999,
        &mut w.events,
    );
    w.tick(&[]);
    assert_eq!(w.backpacks.len(), bags_before, "an empty box is not litter");
}

/// Box contents are sim state, so they are in the hash — wall 5. Without
/// this, two shards could disagree about the contents of every box on the
/// island and every replay would still be green.
#[test]
fn box_contents_are_in_the_state_hash() {
    let (mut w, key, _, _) = box_world();
    w.players[0].inv[4] = ItemStack {
        item: 1,
        count: 40,
        cond: 0,
    };
    let before = w.state_hash();
    do_move(&mut w, key, CONT_SELF, 4, CONT_BOX, 0, 20);
    assert_ne!(before, w.state_hash(), "a deposit must move the state hash");
}

/// The test above is necessary and **not sufficient**: a deposit also
/// debits the player's inventory, which is hashed, so it would move the
/// number even with the box store left out of the digest entirely. It
/// escaped exactly that mutant.
///
/// So the check that the *box* is in the digest has to differ **only**
/// inside the box: same seed, same tick count, same commands, same
/// resulting inventory, one slot index apart. Separate test rather than a
/// second half of that one, because `World` is ~416 kB on the stack and a
/// frame holding two overflows a test thread (`NOW.md`'s boxing item).
#[test]
fn two_boxes_differing_only_in_slot_order_hash_differently() {
    fn deposit_into(slot: u8) -> u64 {
        let (mut w, key, _, _) = box_world();
        w.players[0].inv[4] = ItemStack {
            item: 1,
            count: 40,
            cond: 0,
        };
        do_move(&mut w, key, CONT_SELF, 4, CONT_BOX, slot, 20);
        assert_eq!(
            w.players[0].inv[4],
            ItemStack {
                item: 1,
                count: 20,
                cond: 0,
            }
        );
        w.state_hash()
    }
    assert_ne!(
        deposit_into(0),
        deposit_into(5),
        "two boxes holding the same items in different slots hashed the \
         same — box contents are not in the digest"
    );
}

/// The same commands from the same seed land on the same hash — the box
/// store must not have introduced an iteration-order dependency (wall 1
/// forbids the usual source of one, and this is the check that says so).
///
/// One world at a time on purpose: `World` is ~416 kB of fixed capacity
/// built on the stack, and two of them live at once overflows a test
/// thread. That is `NOW.md`'s standing item about boxing the big members,
/// not anything this slice introduced — the box store is already on the
/// heap.
#[test]
fn the_same_commands_land_on_the_same_box_state() {
    fn run() -> u64 {
        let (mut w, key, _, _) = box_world();
        w.players[0].inv[4] = ItemStack {
            item: 1,
            count: 40,
            cond: 0,
        };
        do_move(&mut w, key, CONT_SELF, 4, CONT_BOX, 2, 11);
        do_move(&mut w, key, CONT_BOX, 2, CONT_BOX, 9, 4);
        w.state_hash()
    }
    assert_eq!(
        run(),
        run(),
        "two runs of the same commands disagreed about the boxes"
    );
}

/// **The address must be injective over its whole legal domain**, or two
/// boxes share a record and items teleport across the island. Proved by
/// round-trip rather than by a set: if unpacking recovers every legal
/// triple exactly, the packing collides on none of them.
///
/// The domain is the one `place_deploy` enforces before a box record can
/// exist (`cx`, `cz` < `MAX_BUILD_COORD`, `level` < `MAX_BUILD_LEVELS`),
/// so this covers every key the sim can actually mint.
#[test]
fn the_box_address_is_injective_over_every_legal_cell() {
    fn unpack(k: u32) -> (u16, u16, u8) {
        ((k >> 16) as u16, ((k >> 4) & 0x3FF) as u16, (k & 0xF) as u8)
    }
    for cx in 0..MAX_BUILD_COORD as u16 {
        for cz in 0..MAX_BUILD_COORD as u16 {
            for level in 0..MAX_BUILD_LEVELS as u8 {
                assert_eq!(
                    unpack(box_key(cx, cz, level)),
                    (cx, cz, level),
                    "box_key aliased at ({cx}, {cz}, {level})"
                );
            }
        }
    }
}

/// **Handle 0 is not a box.** The decode half of the pair; `build.rs`'s
/// `a_box_is_refused_at_the_one_address_that_packs_to_zero` owns the
/// minting half, because standing a foundation at grid origin needs a
/// fixture that can bypass terrain and this file's boxes go up through
/// the real build verb.
///
/// `box_key(0, 0, 0)` packs to 0, and 0 is what every other layer already
/// says when it means *no container is open*: `CONT_SELF` zeroes the
/// handle field, the client bridge answers 0 for "nothing open", and a
/// server-side close writes 0. One build cell therefore minted an address
/// indistinguishable from the absence of one, and the client had to
/// refuse a ground handle of 0 outright to avoid sending "no container
/// known" as a real destination (`invmove.js`, and the ui lane's request
/// this answers).
///
/// `Backpacks` never had the problem because it closes it twice: ids
/// start at 1 so 0 is never minted, and `index_of_id` guards 0 anyway.
/// Boxes now close it the same way, and the two halves are gated apart
/// because either alone leaves a side unable to trust the other.
#[test]
fn the_zero_handle_never_resolves_to_a_box() {
    let (w, key, _cx, _cz) = box_world();

    assert_eq!(box_key(0, 0, 0), 0, "the corner this guards has moved");
    assert!(!w.deploys.boxes().is_empty(), "the fixture needs its box");
    assert!(
        w.deploys.box_index(key).is_some(),
        "the fixture's own box stopped resolving"
    );

    // A non-empty store is the case a bare `position()` over records gets
    // wrong: with the guard removed this answers `Some` for any box that
    // happened to stand at the origin, and `None` only by luck otherwise.
    assert_eq!(
        w.deploys.box_index(0),
        None,
        "handle 0 resolved to a box; the client cannot tell it from 'nothing open'"
    );
}
