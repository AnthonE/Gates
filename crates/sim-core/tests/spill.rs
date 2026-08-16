//! The spill: what a full pack no longer destroys.
//!
//! `gather::inv_add` used to drop the remainder on the floor in the
//! figurative sense only — the items ceased to exist, and the only trace
//! was an `EV_GATHER` reporting a smaller number than the node paid. The
//! backpack module's own header named this as the last thing v0 did not
//! do. It does it now: a node's yield and a finished craft go into a tick
//! buffer the caller owns, and `world.rs` stands a bag up where the body
//! is standing.
//!
//! Everything here comes off the probe fixtures — `GatherContent` (all
//! items stack to 100), `BackpackContent` (90-tick floor, 360 for items
//! 0..3). Nothing invents a number.

use sim_core::backpack::{BackpackContent, Backpacks, LOOT_REACH_M};
use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::craft::{inv_count, CraftContent};
use sim_core::deploy::{
    DeployContent, DeployDef, ACCESS_OP_SET_CODE, ACCESS_OP_TAKE, ARCH_BOX, PLACE_FOUNDATION,
};
use sim_core::gather::{GatherContent, ItemStack, HIT_UNIT, SWING_INTERVAL_TICKS};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::limits::INV_SLOTS;
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::terrain::{self, Occupant, ScatterTable, CELL_SIZE};
use sim_core::world::{Command, EventQueue, World, EV_BAG_DROPPED, EV_GATHER};

/// The gather suite's seed, deliberately: the node this file swings at is
/// the node `tests/gather.rs` already proves pays its hand row, so a
/// failure here is about the spill and never about the scatter.
const SEED: u64 = 20_260_731;

/// A fixture item with the short 90-tick lifetime and no tool row, used to
/// wall off every inventory slot. Not the tree's output.
const JUNK: u16 = 7;
const STACK_MAX: u16 = 100;

/// A gatherable slot, the walkable point 1.2 m west of it, and the yaw
/// that best faces it. Ported from `tests/gather.rs::find_isolated`, which
/// is a test helper and not a public API; the isolation test is what makes
/// the swing target unambiguous. Panics rather than skipping — a seed that
/// offers no isolated node is a setup failure, and a skipped case that
/// reports success is the worst bug class (CLAUDE.md).
fn find_isolated(seed: u64, want: Occupant) -> ((f32, f32), u16) {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);
    for cz in 40..216i32 {
        for cx in 40..216i32 {
            let s = terrain::scatter(seed, &table, &haven, cx, cz);
            if s.occupant != want {
                continue;
            }
            let (px, pz) = (s.x - 1.2, s.z);
            let py = terrain::height(seed, px, pz);
            if (s.y - py).max(py - s.y) > 1.0 || py < 1.0 {
                continue; // node on a ledge or in the sea
            }
            let pcx = (px / CELL_SIZE) as i32;
            let pcz = (pz / CELL_SIZE) as i32;
            let mut rivals = 0;
            for dz in -1..=1i32 {
                for dx in -1..=1i32 {
                    let n = terrain::scatter(seed, &table, &haven, pcx + dx, pcz + dz);
                    if sim_core::gather::node_index(n.occupant).is_some() {
                        let d2 = (n.x - px) * (n.x - px) + (n.z - pz) * (n.z - pz);
                        if d2 <= 6.25 && (n.x != s.x || n.z != s.z) {
                            rivals += 1;
                        }
                    }
                }
            }
            if rivals > 0 {
                continue;
            }
            // Best of the 256 LUT headings toward the node. The heading
            // lives in the TOP byte of the u16 — `hi << 8`, not `hi`, or
            // every candidate is the same bearing to within a fraction of
            // a degree and the argmax returns whatever it started at.
            let (dx, dz) = (s.x - px, s.z - pz);
            let mut best_yaw = 0u16;
            let mut best_dot = f32::MIN;
            for hi in 0..=255u16 {
                let yaw = hi << 8;
                let (fx, fz) = sim_core::yaw_dir(yaw);
                let dot = fx * dx + fz * dz;
                if dot > best_dot {
                    best_dot = dot;
                    best_yaw = yaw;
                }
            }
            return ((px, pz), best_yaw);
        }
    }
    panic!("seed {seed} offers no isolated {want:?} — test setup failure");
}

fn hold_primary(yaw: u16, seq: u16) -> Command {
    Command::Input {
        id: 1,
        frame: InputFrame {
            seq,
            buttons: BTN_PRIMARY,
            yaw,
            pitch: 0,
            move_x: 0,
            move_z: 0,
            sel: 0,
        },
    }
}

/// Player 1 at `pos`, gather and backpack fixtures armed, and every one of
/// the 30 inventory slots walled off with junk at the ceiling — so the
/// very first swing has nowhere to pay. Boxed for the reason
/// `tests/gather.rs` states: a `World` through a test frame overflows the
/// default test-thread stack.
fn full_pack_world(pos: (f32, f32)) -> Box<World> {
    let mut w = Box::new(World::new(SEED));
    w.gather = GatherContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    // The weak spot is off for the same reason the base-pay gather test
    // turns it off: a stationary swinger lands in the roaming mark's
    // sector on some seeds and doubles a payout, which would make the
    // spilled count seed-dependent.
    for n in w.gather.nodes.iter_mut() {
        n.weak_pct = 0;
    }
    w.dev_spawn = Some(pos);
    w.tick(&[Command::Join { id: 1 }]);
    for s in w.players[0].inv.iter_mut() {
        *s = ItemStack {
            item: JUNK,
            count: STACK_MAX,
            cond: 0,
        };
    }
    w
}

/// The whole point. A swing at a node with no room for its yield used to
/// pay nothing and destroy the wood; it now stands a bag up underfoot
/// holding exactly what the node paid.
#[test]
fn a_full_pack_drops_the_yield_at_your_feet() {
    let (pos, yaw) = find_isolated(SEED, Occupant::Tree);
    let mut w = full_pack_world(pos);
    let tree = w.gather.nodes[0];
    let (px, pz) = (w.players[0].body.qx, w.players[0].body.qz);

    assert_eq!(w.backpacks.len(), 0, "no bag before the swing");
    w.tick(&[hold_primary(yaw, 0)]);

    assert_eq!(
        inv_count(&w.players[0].inv, tree.output),
        0,
        "the pack was full: nothing reached the hands"
    );
    assert_eq!(w.backpacks.len(), 1, "the yield stood a bag up instead");
    let bag = w.backpacks.entries()[0];
    assert_eq!(
        inv_count(&bag.items, tree.output),
        tree.hand_yield as u32,
        "the bag holds exactly what the node paid, not a rounded share"
    );
    assert_eq!((bag.qx, bag.qz), (px, pz), "it fell where the swinger is");
    assert_eq!(bag.owner, 1, "credited to the swinger");
    assert!(
        w.events
            .entries()
            .iter()
            .any(|e| e.code == EV_BAG_DROPPED && e.a == bag.id),
        "a new container on the ground announces itself"
    );
}

/// **The shipped shape the probe fixture does not have.** `content/
/// gatherables.toml` gives the tree a secondary (mushrooms beside the
/// wood), and the fixture's tree pays one thing — so every other case here
/// sees exactly one `EV_GATHER` and the two-payout path was covered
/// nowhere. It is the interesting one: two zeros in a single swing, which
/// is what the client has to draw two lines for. The merge-gate judge's
/// ranked fix 1 (pass 20260813-230343-10) named the gap from the client end.
#[test]
fn a_node_that_pays_twice_spills_twice_and_announces_both() {
    let (pos, yaw) = find_isolated(SEED, Occupant::Tree);
    let mut w = full_pack_world(pos);
    let tree = w.gather.nodes[0];
    // Any item that is neither the primary nor the junk walling the pack
    // off; the fixture stacks everything to 100, so none of it fits either.
    let sec: u16 = if tree.output == 5 { 6 } else { 5 };
    let sec_units: u16 = 3;
    w.gather.nodes[0].secondary = (sec, sec_units);

    w.tick(&[hold_primary(yaw, 0)]);

    let gathers: Vec<_> = w
        .events
        .entries()
        .iter()
        .filter(|e| e.code == EV_GATHER)
        .collect();
    assert_eq!(
        gathers.len(),
        2,
        "both payouts announce — the client draws one line per event, and a \
         swallowed one is a spill the player is never told about"
    );
    assert_eq!(gathers[0].b >> 16, tree.output as u32, "primary first");
    assert_eq!(gathers[0].b & 0xFFFF, 0, "and none of it reached the hands");
    assert_eq!(gathers[1].b >> 16, sec as u32, "then the secondary");
    assert_eq!(gathers[1].b & 0xFFFF, 0, "which did not fit either");

    // Both are on the floor, in the one bag, whole.
    assert_eq!(w.backpacks.len(), 1, "one swing stands up one bag");
    let bag = w.backpacks.entries()[0];
    assert_eq!(
        inv_count(&bag.items, tree.output),
        tree.hand_yield as u32,
        "the primary is in it"
    );
    assert_eq!(
        inv_count(&bag.items, sec),
        sec_units as u32,
        "and the secondary beside it, not destroyed"
    );
}

/// The bound. A player farming at a full pack pays a swing every
/// `SWING_INTERVAL_TICKS` — a bag apiece would churn `MAX_BACKPACKS` in
/// minutes and evict other people's death bags to do it. Standing still
/// costs one bag however long you swing, because the spill merges into the
/// bag already in reach.
#[test]
fn swinging_on_at_a_full_pack_grows_one_bag_not_many() {
    let (pos, yaw) = find_isolated(SEED, Occupant::Tree);
    let mut w = full_pack_world(pos);
    let tree = w.gather.nodes[0];

    // Every swing the node has in it, at the fixed cadence.
    for seq in 0..(SWING_INTERVAL_TICKS * (tree.hits as u64 + 1)) {
        w.tick(&[hold_primary(yaw, seq as u16)]);
        assert!(
            w.backpacks.len() <= 1,
            "a second bag was minted at tick {} — the merge is what bounds this",
            w.tick
        );
    }
    assert_eq!(w.backpacks.len(), 1, "one bag caught the whole node");
    assert_eq!(
        inv_count(&w.backpacks.entries()[0].items, tree.output),
        (tree.hits as u32) * tree.hand_yield as u32,
        "and it holds the node's whole yield — the total gather.rs pins \
         for a pack that had room"
    );
}

/// A bag that grows keeps the longer clock. This is the case that makes
/// the `if want > expires` guard load-bearing rather than decorative: a
/// **partly looted** bag still carries the expiry its rare contents bought,
/// and recomputing the clock on a merge would pull it in by the difference
/// between the ladder's rungs — the bag would despawn early because
/// somebody dropped wood in it.
///
/// Driven against `spill_at` directly. Going through a swing cannot reach
/// it: whatever the node pays is still inside, so the recomputed clock is
/// always the later one and the guard never has to say no.
#[test]
fn a_merge_never_pulls_a_partly_looted_bag_s_clock_in() {
    let bc = BackpackContent::probe_fixture();
    let gc = GatherContent::probe_fixture();
    let mut bp = Backpacks::new();
    let mut ev = EventQueue::default();

    // A bag holding one rare item (360 ticks) and one junk (90).
    let mut held = [ItemStack::default(); INV_SLOTS];
    held[0] = ItemStack {
        item: 0,
        count: 1,
        cond: 0,
    };
    held[1] = ItemStack {
        item: JUNK,
        count: 1,
        cond: 0,
    };
    let id = bp
        .stand_up(&bc, 0, 0, 0, 1, &held, 100, &mut ev)
        .expect("the fixture ladder is armed");
    assert_eq!(bp.entries()[0].expires, 100 + 360, "the rare item set it");

    // The move verb takes the rare item out — the ordinary partly-looted
    // bag. Its clock does not shorten, by design (nothing recomputes it).
    bp.set_slot(0, 0, ItemStack::default());
    assert_eq!(bp.entries()[0].expires, 460);

    // Now spill junk into it. Recomputed from scratch this would be
    // 200 + 90 = 290, a quarter of an hour early at 30 Hz.
    let mut spill = [ItemStack::default(); INV_SLOTS];
    spill[0] = ItemStack {
        item: JUNK,
        count: 5,
        cond: 0,
    };
    assert_eq!(
        bp.spill_at(&bc, &gc, 0, 0, 0, 1, &mut spill, 200, &mut ev),
        Some(id),
        "same address: merged into the standing bag, not a new one"
    );
    assert_eq!(bp.len(), 1, "merged rather than minted");
    assert_eq!(
        bp.entries()[0].expires,
        460,
        "the clock must stand, never be recomputed downward"
    );
    assert_eq!(inv_count(&bp.entries()[0].items, JUNK), 6, "1 + 5 merged");
    assert!(
        spill.iter().all(|s| s.count == 0),
        "the caller's buffer is drained by what the bag took"
    );
}

/// Out of reach is out of reach: a spill more than `LOOT_REACH_M` from a
/// standing bag mints its own rather than teleporting into one across the
/// clearing.
#[test]
fn a_spill_out_of_reach_stands_its_own_bag_up() {
    let bc = BackpackContent::probe_fixture();
    let gc = GatherContent::probe_fixture();
    let mut bp = Backpacks::new();
    let mut ev = EventQueue::default();

    let mut held = [ItemStack::default(); INV_SLOTS];
    held[0] = ItemStack {
        item: JUNK,
        count: 1,
        cond: 0,
    };
    let near = bp.stand_up(&bc, 0, 0, 0, 1, &held, 100, &mut ev).unwrap();

    // Two reaches away, in the movement quantum the store speaks.
    let far_q = ((LOOT_REACH_M * 2.0) / POS_XZ_Q) as i32;
    let mut spill = [ItemStack::default(); INV_SLOTS];
    spill[0] = ItemStack {
        item: JUNK,
        count: 3,
        cond: 0,
    };
    let made = bp
        .spill_at(&bc, &gc, far_q, 0, 0, 1, &mut spill, 110, &mut ev)
        .unwrap();
    assert_ne!(made, near, "a second bag, not the first one moved");
    assert_eq!(bp.len(), 2);
    assert_eq!(
        inv_count(&bp.entries()[0].items, JUNK),
        1,
        "the far bag is untouched"
    );
}

/// The disarm survives. `base_ticks == 0` is content that never armed the
/// backpack module, and its stated meaning is "the world before this
/// slice" — so overflow is destroyed exactly as it used to be, and no bag
/// appears on a shard whose content asked for none.
#[test]
fn an_inert_ladder_still_destroys_the_overflow() {
    let (pos, yaw) = find_isolated(SEED, Occupant::Tree);
    let mut w = full_pack_world(pos);
    w.backpack = BackpackContent::EMPTY;
    let tree = w.gather.nodes[0];

    for seq in 0..(SWING_INTERVAL_TICKS * 3) {
        w.tick(&[hold_primary(yaw, seq as u16)]);
    }
    assert_eq!(w.backpacks.len(), 0, "an inert ladder stands nothing up");
    assert_eq!(
        inv_count(&w.players[0].inv, tree.output),
        0,
        "and the yield is gone, as it was before the spill lane"
    );
}

/// Wall 5 applied to the new state: the bag the spill mints, its id, its
/// address and its clock are all sim state, so two runs of the same
/// commands must hash identically.
#[test]
fn the_spill_replays_bit_identically() {
    let (pos, yaw) = find_isolated(SEED, Occupant::Tree);
    let run = || {
        let mut w = full_pack_world(pos);
        for seq in 0..(SWING_INTERVAL_TICKS * 2) {
            w.tick(&[hold_primary(yaw, seq as u16)]);
        }
        (w.state_hash(), w.backpacks.len())
    };
    let (a, bags_a) = run();
    let (b, bags_b) = run();
    assert_eq!(bags_a, 1, "the run actually spilled something");
    assert_eq!(bags_a, bags_b);
    assert_eq!(a, b, "the spill must replay bit-identically");
}

/// **The spill's announcement, and why the zero had to be made unique.**
///
/// A spill was silent (`NOW.md` §0sp2) and the question the item posed was
/// whether the client could read it off facts already on the wire. It can,
/// on one field, but only once `added == 0` has exactly one cause.
///
/// It had two. The cumulative pay schedule legitimately pays nothing on
/// some swings — `pool` need not divide the hit budget — and those swings
/// pushed an `EV_GATHER` saying "0 units entered your inventory", which is
/// the same sentence a full pack produces. The client could not separate a
/// swing that earned nothing from a swing whose whole yield hit the floor,
/// so it showed neither.
///
/// Both halves are asserted here, on the same node, because the guard is
/// only worth anything if the paying case still announces itself.
#[test]
fn only_a_full_pack_makes_the_gather_event_say_zero() {
    let (pos, yaw) = find_isolated(SEED, Occupant::Tree);

    // Half one: a swing owed nothing, and room to take it if it were.
    // The node is reshaped to be worth 4 over 4 hits while withholding
    // half for the finisher, so `pool` is 2 and the first swing is owed
    // `floor(2 × 1 / 4) − 0` = nothing. Silence, not a zero.
    let mut w = Box::new(World::new(SEED));
    w.gather = GatherContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    for n in w.gather.nodes.iter_mut() {
        n.weak_pct = 0;
    }
    w.gather.nodes[0].hand_yield = 1;
    w.gather.nodes[0].hits = 4;
    w.gather.nodes[0].finish_pct = 50;
    w.dev_spawn = Some(pos);
    w.tick(&[Command::Join { id: 1 }]);
    let thin = w.gather.nodes[0];
    w.tick(&[hold_primary(yaw, 0)]);

    // The positive control first, because everything below it is a
    // NEGATIVE. Half one builds its own world rather than inheriting
    // `full_pack_world`'s proof that a swing lands at this pos/yaw, so if
    // the scatter, the spawn or the reach ever drifts and the swing MISSES,
    // three assertions about what a swing did not do all go green while
    // guarding nothing. A miss touches no slot: one entry, one unmarked
    // `HIT_UNIT` of budget spent (`weak_pct` is zeroed above, so a marked
    // bite cannot be what landed).
    assert_eq!(
        w.slot_lives.len(),
        1,
        "the swing missed the node entirely — every negative below is vacuous"
    );
    assert_eq!(
        w.slot_lives.entries()[0].hits as u32,
        HIT_UNIT,
        "the swing connected but spent the wrong budget"
    );

    assert_eq!(
        inv_count(&w.players[0].inv, thin.output),
        0,
        "the reshaped node no longer pays zero on its first swing"
    );
    assert_eq!(w.backpacks.len(), 0, "nothing was owed, so nothing spilled");
    assert!(
        !w.events.entries().iter().any(|e| e.code == EV_GATHER),
        "a swing that earned nothing announced a gather, which is the \
         zero a full pack has to own"
    );

    // Half two: the stock node, whose first swing IS owed its hand row,
    // against a full pack. The event lands and its zero is the spill —
    // so the guard above suppressed the unowed swing and nothing else.
    let mut w = full_pack_world(pos);
    let tree = w.gather.nodes[0];
    w.tick(&[hold_primary(yaw, 0)]);

    let gathers: Vec<_> = w
        .events
        .entries()
        .iter()
        .filter(|e| e.code == EV_GATHER)
        .collect();
    assert_eq!(gathers.len(), 1, "the paying swing must still announce");
    assert_eq!(
        gathers[0].b & 0xFFFF,
        0,
        "the units that reached the hands are the client's spill signal"
    );
    assert_eq!(
        gathers[0].b >> 16,
        tree.output as u32,
        "and it names the item that hit the floor"
    );
    assert_eq!(
        w.backpacks.len(),
        1,
        "which is on the ground, not destroyed"
    );
}

// ── The four give-backs (`NOW.md` §0sp2) ─────────────────────────────────
//
// A give-back is the other direction from a payout: the world is handing
// something back that was already yours — a demolished wall's materials, a
// deployable you lifted, a lock you unbolted, a craft you cancelled. All
// four discarded their leftover until 2026-08-14, so a full pack destroyed
// it, and all four now take the same lane a node's yield takes.
//
// **The fall-point is the body in every case, and that is arithmetic
// rather than taste.** Each of these verbs refuses beyond `BUILD_REACH_M`
// and `LOOT_REACH_M` *is* `BUILD_REACH_M` (one `pub use`), so the object's
// own address is inside the merge reach of the feet by construction — the
// two candidate answers §0sp2 posed cannot produce a bag a player would
// look for in a different place. The assertions below read `(bag.qx,
// bag.qz)` against the body directly rather than asking `in_reach`, which
// at max range cannot tell the two apart.

/// Ballast for the give-back cases, and **not `JUNK`**: item 7 is the
/// fixture's code lock, so walling the pack off with it would make the
/// lock-removal assertion read against 3 000 of the item it is trying to
/// count. Item 1 is paid by none of the four give-backs (it is a lock
/// *cost*, spent before the fill) and stacks to 100 like the rest.
const BALLAST: u16 = 1;

const OWNER: u32 = 1;
const FOUNDATION_ROW: u16 = 0;
/// Row 4 of the deploy fixture: the oven. `PLACE_GROUND`, so a pick-up
/// case needs no foundation under it, and its item (6) is not a give-back
/// of any other case here.
const FIRE_ROW: u16 = 4;
const FIRE_ITEM: u16 = 6;
/// Row 5: the code lock, item 7, priced at 12 × item 1.
const LOCK_ROW: u16 = 5;
const LOCK_ITEM: u16 = 7;
/// A lockable box over the recycler row — `deploy::lockable` admits only a
/// door or a box, and a box needs no doorway piece and no door deployable.
/// `tests/lock_box.rs` makes the same override for the same reason.
const BOX_ROW: u16 = 6;
const BOX_ITEM: u16 = 9;

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        (cx as f32 + 0.5) * BUILD_CELL_M,
        (cz as f32 + 0.5) * BUILD_CELL_M,
    )
}

/// The nearest cell the foundation's terrain rule accepts, in ring order
/// (deterministic for a seed) — `tests/lock_box.rs`'s scan.
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
                if foundation_terrain_ok(seed, x, z) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell within 64 cells — the generator changed under this test");
}

fn giveback_deploy() -> DeployContent {
    let mut d = DeployContent::probe_fixture();
    d.defs[BOX_ROW as usize] = DeployDef {
        arch: ARCH_BOX,
        placement: PLACE_FOUNDATION,
        hp: 60,
        item: BOX_ITEM,
        ..DeployDef::INERT
    };
    d
}

/// Player 1 standing in a buildable cell with every content table the four
/// give-backs need, and a stocked pack. The pack is **not** full yet: each
/// case has to spend its own setup costs first, and you cannot hold a
/// lock's price and a full load at the same time.
fn giveback_world() -> (Box<World>, u16, u16) {
    let mut w = Box::new(World::new(SEED));
    w.gather = GatherContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    w.build = BuildContent::probe_fixture();
    w.deploy = giveback_deploy();
    w.craft = CraftContent::probe_fixture();
    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);
    w.dev_spawn = Some((x, z));
    w.tick(&[Command::Join { id: OWNER }]);
    w.players[0].body = Body::at(SEED, x, z);
    // A build piece is paid in its cost rows; a **deployable is paid by
    // carrying it** — `costs` on a `DeployDef` is the repair price, which
    // is what `REFUSE_D_COST` means at a `PlaceDeploy`. So the stock is
    // both: bulk item 0 for the foundation, and one each of the three
    // deployables these cases stand up.
    w.players[0].inv[0] = ItemStack {
        item: 0,
        count: 100,
        cond: 0,
    };
    w.players[0].inv[1] = ItemStack {
        item: FIRE_ITEM,
        count: 1,
        cond: 0,
    };
    w.players[0].inv[2] = ItemStack {
        item: LOCK_ITEM,
        count: 1,
        cond: 0,
    };
    w.players[0].inv[3] = ItemStack {
        item: BOX_ITEM,
        count: 1,
        cond: 0,
    };
    (w, cx, cz)
}

/// Wall off all 30 slots so a give-back has nowhere to land. Called after
/// the setup has paid its costs, never before.
fn wall_off(w: &mut World) {
    for s in w.players[0].inv.iter_mut() {
        *s = ItemStack {
            item: BALLAST,
            count: STACK_MAX,
            cond: 0,
        };
    }
}

/// Give-back 1 of 4. A demolish is an undo and refunds the piece's cost
/// rows whole; at a full pack that refund used to cease to exist, which
/// made "undo" a way to delete your own materials.
#[test]
fn a_demolish_refund_a_full_pack_cannot_hold_falls_at_your_feet() {
    let (mut w, cx, cz) = giveback_world();
    w.tick(&[Command::Place {
        id: OWNER,
        row: FOUNDATION_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.pieces.len(), 1, "the case needs its foundation");
    let cost = w.build.pieces[FOUNDATION_ROW as usize].costs[0];
    assert!(cost.1 > 0, "the fixture's foundation must cost something");

    wall_off(&mut w);
    let (px, pz) = (w.players[0].body.qx, w.players[0].body.qz);
    w.tick(&[Command::Demolish {
        id: OWNER,
        deploy: false,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);

    assert_eq!(w.pieces.len(), 0, "the foundation still came down");
    assert_eq!(
        inv_count(&w.players[0].inv, cost.0),
        0,
        "the pack was full: the refund reached no slot"
    );
    assert_eq!(w.backpacks.len(), 1, "so it stood a bag up instead");
    let bag = w.backpacks.entries()[0];
    assert_eq!(
        inv_count(&bag.items, cost.0),
        cost.1 as u32,
        "the bag holds the whole refund — a demolish is an undo, not a tax"
    );
    assert_eq!(
        (bag.qx, bag.qz),
        (px, pz),
        "at the demolisher's feet, not at the wall"
    );
    assert_eq!(bag.owner, OWNER, "credited to the demolisher");
}

/// Give-back 2 of 4. Lifting a deployable hands the item back; a full pack
/// used to eat the oven and leave the ground empty.
#[test]
fn a_picked_up_deployable_a_full_pack_cannot_hold_falls_at_your_feet() {
    let (mut w, cx, cz) = giveback_world();
    w.tick(&[Command::PlaceDeploy {
        id: OWNER,
        row: FIRE_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.deploys.len(), 1, "the case needs its oven placed");

    wall_off(&mut w);
    let (px, pz) = (w.players[0].body.qx, w.players[0].body.qz);
    w.tick(&[Command::Demolish {
        id: OWNER,
        deploy: true,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);

    assert_eq!(w.deploys.len(), 0, "the oven still came up");
    assert_eq!(
        inv_count(&w.players[0].inv, FIRE_ITEM),
        0,
        "the pack was full: the oven reached no slot"
    );
    assert_eq!(w.backpacks.len(), 1, "so it stood a bag up instead");
    let bag = w.backpacks.entries()[0];
    assert_eq!(
        inv_count(&bag.items, FIRE_ITEM),
        1,
        "the bag holds the oven — picking a thing up may not destroy it"
    );
    assert_eq!((bag.qx, bag.qz), (px, pz), "at the picker's feet");
}

/// Give-back 3 of 4. A lock is a separate thing bolted on (`DOORS.md` §1),
/// so unbolting it returns the item. `lock_op`'s own comment used to argue
/// that losing it at a full pack was correct because every other give here
/// did the same — which stopped being true the moment the others took the
/// spill.
#[test]
fn an_unbolted_lock_a_full_pack_cannot_hold_falls_at_your_feet() {
    let (mut w, cx, cz) = giveback_world();
    w.tick(&[Command::Place {
        id: OWNER,
        row: FOUNDATION_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    w.tick(&[Command::PlaceDeploy {
        id: OWNER,
        row: BOX_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.deploys.len(), 1, "the case needs its box");
    w.tick(&[Command::PlaceDeploy {
        id: OWNER,
        row: LOCK_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(w.deploys.locks().len(), 1, "the lock must bolt on");
    w.tick(&[Command::Access {
        id: OWNER,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        op: ACCESS_OP_SET_CODE,
        code: 1234,
    }]);

    wall_off(&mut w);
    let (px, pz) = (w.players[0].body.qx, w.players[0].body.qz);
    w.tick(&[Command::Access {
        id: OWNER,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        op: ACCESS_OP_TAKE,
        code: 0,
    }]);

    assert_eq!(w.deploys.locks().len(), 0, "the lock still came off");
    assert_eq!(
        inv_count(&w.players[0].inv, LOCK_ITEM),
        0,
        "the pack was full: the lock reached no slot"
    );
    assert_eq!(w.backpacks.len(), 1, "so it stood a bag up instead");
    let bag = w.backpacks.entries()[0];
    assert_eq!(
        inv_count(&bag.items, LOCK_ITEM),
        1,
        "the bag holds the lock — a full load may still unbolt one and keep it"
    );
    assert_eq!((bag.qx, bag.qz), (px, pz), "at the unbolter's feet");
}

/// Give-back 4 of 4, and the one that lost the most. A cancel refunds the
/// remaining units' inputs in `u16` chunks, and the loop used to **break**
/// at the first chunk that did not fit whole — so it dropped that chunk's
/// tail *and every chunk after it*. Nothing now leaves the loop early.
#[test]
fn a_cancelled_craft_s_refund_a_full_pack_cannot_hold_falls_at_your_feet() {
    let (mut w, _cx, _cz) = giveback_world();
    let recipe = w.craft.recipes[0];
    let (input, per) = recipe.inputs[0];
    let units = 3u16;
    w.tick(&[Command::Craft {
        id: OWNER,
        recipe: 0,
        count: units,
    }]);
    assert_eq!(
        w.players[0].jobs[0].remaining, units,
        "the case needs its queued batch"
    );

    // The whole batch's inputs were consumed at enqueue, so the refund is
    // every unit's price. Wall the pack off before the cancel; the job has
    // not had `recipe.ticks` to complete anything, so no output competes
    // for the same buffer.
    wall_off(&mut w);
    let (px, pz) = (w.players[0].body.qx, w.players[0].body.qz);
    w.tick(&[Command::CraftCancel {
        id: OWNER,
        index: 0,
    }]);

    assert_eq!(w.players[0].jobs[0].remaining, 0, "the job still cancelled");
    assert_eq!(
        inv_count(&w.players[0].inv, input),
        0,
        "the pack was full: the refund reached no slot"
    );
    assert_eq!(w.backpacks.len(), 1, "so it stood a bag up instead");
    let bag = w.backpacks.entries()[0];
    assert_eq!(
        inv_count(&bag.items, input),
        per as u32 * units as u32,
        "the WHOLE refund, every chunk — the break used to keep the tail"
    );
    assert_eq!((bag.qx, bag.qz), (px, pz), "at the crafter's feet");
}

/// Wall 5 over the give-backs: the bag a give-back mints is sim state, so
/// the same commands must hash identically twice. The four verbs run in
/// `World::apply`, a different drain call than the player loop's, and a
/// bag whose clock depended on which caller stood it up would show here.
#[test]
fn the_give_back_spill_replays_bit_identically() {
    let run = || {
        let (mut w, cx, cz) = giveback_world();
        w.tick(&[Command::Place {
            id: OWNER,
            row: FOUNDATION_ROW,
            cx,
            cz,
            level: 0,
            loc: LOC_PLANE,
        }]);
        wall_off(&mut w);
        w.tick(&[Command::Demolish {
            id: OWNER,
            deploy: false,
            cx,
            cz,
            level: 0,
            loc: LOC_PLANE,
        }]);
        (w.state_hash(), w.backpacks.len())
    };
    let (a, bags_a) = run();
    let (b, bags_b) = run();
    assert_eq!(bags_a, 1, "the run actually spilled something");
    assert_eq!(bags_a, bags_b);
    assert_eq!(a, b, "a give-back's bag must replay bit-identically");
}

/// The contract `spill_at` states, on the path where it used to be false
/// (merge-gate judge, pass -08, ranked fix 2). `stand_up` copies the
/// buffer wholesale, so a mint left it holding exactly what the new bag
/// had just taken — and the sweep above is the second caller, which is the
/// arrangement that turns a stale buffer into items duplicated into the
/// world. Drained twice here on purpose: that is the shape of the bug.
#[test]
fn a_minted_bag_leaves_the_caller_s_buffer_empty() {
    let bc = BackpackContent::probe_fixture();
    let gc = GatherContent::probe_fixture();
    let mut bp = Backpacks::new();
    let mut ev = EventQueue::default();

    let mut buf = [ItemStack::default(); INV_SLOTS];
    buf[0] = ItemStack {
        item: JUNK,
        count: 5,
        cond: 0,
    };
    let made = bp.spill_at(&bc, &gc, 0, 0, 0, 1, &mut buf, 10, &mut ev);
    assert!(made.is_some(), "nothing stood up — the case did not run");
    assert!(
        buf.iter().all(|s| s.count == 0),
        "the mint took everything, so the buffer is a duplicate and not a \
         remainder"
    );

    // The same buffer, drained again in the same tick. A caller doing this
    // must not be able to conjure a second copy.
    bp.spill_at(&bc, &gc, 0, 0, 0, 1, &mut buf, 10, &mut ev);
    assert_eq!(
        bp.len(),
        1,
        "no second bag: there was nothing left to spill"
    );
    assert_eq!(
        inv_count(&bp.entries()[0].items, JUNK),
        5,
        "and the world still holds five, not ten"
    );
}

/// The disarm guard on `spill_at` itself, in the one state that can tell
/// it apart from `stand_up`'s (merge-gate judge, pass -08, ranked fix 3:
/// `an_inert_ladder_still_destroys_the_overflow` stays green under a
/// revert of this guard, because under inert content no bag exists for the
/// merge scan to find). A bag standing under armed content is that state,
/// and it is reachable as a contract on the store even though a shard
/// cannot swap its content mid-run: without the guard the merge scan runs
/// first and an inert ladder would quietly grow a bag it promised not to
/// touch.
#[test]
fn an_inert_ladder_does_not_even_merge_into_a_bag_already_standing() {
    let armed = BackpackContent::probe_fixture();
    let gc = GatherContent::probe_fixture();
    let mut bp = Backpacks::new();
    let mut ev = EventQueue::default();

    let mut held = [ItemStack::default(); INV_SLOTS];
    held[0] = ItemStack {
        item: JUNK,
        count: 1,
        cond: 0,
    };
    bp.stand_up(&armed, 0, 0, 0, 1, &held, 10, &mut ev)
        .expect("the case needs a bag standing under armed content");

    let mut buf = [ItemStack::default(); INV_SLOTS];
    buf[0] = ItemStack {
        item: JUNK,
        count: 5,
        cond: 0,
    };
    let made = bp.spill_at(
        &BackpackContent::EMPTY,
        &gc,
        0,
        0,
        0,
        1,
        &mut buf,
        20,
        &mut ev,
    );
    assert!(made.is_none(), "an inert ladder catches nothing");
    assert_eq!(
        inv_count(&bp.entries()[0].items, JUNK),
        1,
        "the standing bag took none of it — the guard runs before the merge"
    );
    assert_eq!(
        inv_count(&buf, JUNK),
        5,
        "and the buffer still holds it, for the caller to destroy as the \
         pre-spill world did"
    );
}
