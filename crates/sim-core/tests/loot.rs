//! The barrel loop, end to end: swing at a barrel → it comes apart after
//! its content-declared hits → the table rolls into **loose stacks lying
//! where it stood** → the pickup verb takes them one at a time.
//!
//! ⚠ **The middle step changed on 2026-09-16** (ground items v0, the
//! operator's call) and this file is where the old shape was pinned: a
//! barrel used to stand up a `backpack.rs` container, and the assertions
//! below used to read `w.backpacks`. What they read now is
//! `w.ground_items`, and the interesting difference is not the store — it
//! is that a stack finds **its own ground height at its own landing spot**
//! rather than floating at the height the barrel stood at.
//!
//! Every number here comes out of a `probe_fixture`, never `content/`
//! (CLAUDE.md wall 7): `LootContent::probe_fixture` is two weighted rows
//! over items 0 and 1 at 2 hits, `GatherContent::probe_fixture` stacks
//! every fixture item to 100, and `BackpackContent::probe_fixture` is the
//! despawn ladder that lets the container stand up at all.
//!
//! What this file does not gate is the wire: a smashed barrel rides
//! `EV_SLOT_HARVESTED` and `EV_BAG_DROPPED`, both of which already existed
//! and neither of which changed shape, so `test_protocol_golden` covers
//! them unchanged and `PROTO_VER` did not move. `event_roles.rs` owns the
//! payload roles.

use sim_core::backpack::BackpackContent;
use sim_core::fmath::fabs;
use sim_core::gather::{cell_key, GatherContent, ItemStack};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::loot::{LootContent, LOOT_BARREL};
use sim_core::movement::{Body, POS_XZ_Q};
use sim_core::terrain::{self, Occupant};
use sim_core::world::{Command, World, EV_BAG_DROPPED};

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

const SEED: u64 = 20260804;
const PLAYER: u32 = 1;
/// `LootContent::probe_fixture` declares two swings to open.
const FIXTURE_HITS: u32 = 2;
/// Bounded: 38 ticks per swing (`SWING_INTERVAL_TICKS`), so this is room
/// for well over the fixture's hits and still a search rather than a wait.
const MAX_STEPS: u32 = 600;

/// The first cell holding `kind`, scanned off `terrain::scatter` rather
/// than typed in — a cell that held a barrel at one seed and one weight
/// table is a fixture that silently stops meaning what it says.
fn scanned_slot(kind: Occupant) -> (f32, f32, f32) {
    let scatter = terrain::ScatterTable::alpha_default();
    // The same haven the sim resolves at init: it vetoes scatter, so a fixture
    // that skipped it could return a cell the world does not actually place.
    let haven = terrain::haven(SEED);
    let span = (terrain::ISLAND_SIZE / terrain::CELL_SIZE) as i32;
    for cz in 0..span {
        for cx in 0..span {
            let s = terrain::scatter(SEED, &scatter, &haven, cx, cz);
            if s.occupant == kind {
                return (s.x, s.y, s.z);
            }
        }
    }
    panic!("no {kind:?} on this island — the scatter table changed under this gate");
}

/// One player stood exactly on the first barrel, looking straight down at
/// it (pitch 0). On purpose: from a 1.6 m eye over the drum's centre the
/// ray enters its 0.88 m top whatever the yaw, so the fixture never has to
/// reproduce a heading to make a swing land. It leant on a planar cone's
/// point-blank exemption until melee aim v1 made the swing a ray.
///
/// Returns the `World` bare and never inside a tuple or a wrapper: it is a
/// large fixed-capacity value and an unoptimized build puts a construction
/// temporary beside every live one, so a `(World, ..)` return overflows a
/// test thread's stack. `combat.rs` and `event_roles.rs` both say this;
/// this file learned it the same way they did. The barrel's position is a
/// pure function of the seed, so `barrel_pos()` re-derives it instead of
/// riding back alongside the world.
fn barrel_world() -> World {
    let mut w = World::new(SEED);
    w.gather = GatherContent::probe_fixture();
    w.loot = LootContent::probe_fixture();
    w.backpack = BackpackContent::probe_fixture();
    w.tick(&[Command::Join { id: PLAYER }]);
    let (x, _, z) = barrel_pos();
    w.players[0].body = Body::at(SEED, hv(SEED), x, z);
    w
}

/// The barrel the fixture stands its player on.
fn barrel_pos() -> (f32, f32, f32) {
    scanned_slot(Occupant::BarrelSlot)
}

/// One tick with the swing held, as a player holding the button sends it.
fn swing(w: &mut World, seq: &mut u16) {
    w.tick(&[Command::Input {
        id: PLAYER,
        frame: InputFrame {
            seq: *seq,
            buttons: BTN_PRIMARY,
            ..Default::default()
        },
        favour: 0,
    }]);
    *seq = seq.wrapping_add(1);
}

/// Swings until the barrel scatters, returning how many landed swings it
/// took. Panics rather than looping forever.
fn smash(w: &mut World) -> u32 {
    let mut seq = 0u16;
    let mut landed = 0u32;
    let before = w.ground_items.len();
    for _ in 0..MAX_STEPS {
        let hits_before = w.slot_lives.len();
        swing(w, &mut seq);
        if w.slot_lives.len() > hits_before || w.ground_items.len() > before {
            landed += 1;
        }
        if w.ground_items.len() > before {
            return landed;
        }
    }
    panic!("the barrel never came apart in {MAX_STEPS} ticks");
}

#[test]
fn a_barrel_comes_apart_into_loose_stacks_where_it_stood() {
    let (x, _, z) = barrel_pos();
    let mut w = barrel_world();
    smash(&mut w);

    assert!(
        !w.ground_items.is_empty(),
        "the barrel scattered nothing — the roll did not run"
    );
    let span = sim_core::grounditem::SCATTER_R_M * 1.4143; // the square's corner
    for g in w.ground_items.entries() {
        assert!(g.stack.count > 0, "an empty stack reached the ground");
        assert!(g.expires > w.tick, "a stack despawns in the past");
        let gx = g.qx as f32 * POS_XZ_Q;
        let gz = g.qz as f32 * POS_XZ_Q;
        assert!(
            fabs(gx - x) <= span && fabs(gz - z) <= span,
            "a stack landed {:.2} m / {:.2} m from the barrel, past the \
             scatter radius",
            fabs(gx - x),
            fabs(gz - z)
        );
        // **Its own ground height at its own spot**, which is the whole of
        // the operator's *"fall down"* in v0: a stack that lands over a
        // lip lies where the lip is, not at the height the barrel stood
        // at. Exact, because both sides call `terrain::ground`.
        assert_eq!(
            g.qy,
            sim_core::movement::quant_y(terrain::ground(SEED, hv(SEED), gx, gz)),
            "a stack is not resting on the ground under it"
        );
        // And still inside the arm that takes it, so one swing's loot
        // never needs a walk.
        let px = w.players[0].body.qx as f32 * POS_XZ_Q;
        let pz = w.players[0].body.qz as f32 * POS_XZ_Q;
        let d2 = (gx - px) * (gx - px) + (gz - pz) * (gz - pz);
        let reach = sim_core::backpack::LOOT_REACH_M;
        assert!(
            d2 <= reach * reach,
            "a stack landed out of the smasher's reach"
        );
    }
}

#[test]
fn the_loot_does_not_land_in_the_smashers_hands() {
    // The whole point of the item: a barrel pays a container, not an
    // inventory, because a loot panel is the verb the road is built for.
    let mut w = barrel_world();
    for s in w.players[0].inv.iter_mut() {
        *s = ItemStack::default();
    }
    smash(&mut w);
    assert!(
        w.players[0].inv.iter().all(|s| s.count == 0),
        "smashing a barrel filled the inventory directly"
    );
    assert!(w.ground_items.entries().iter().any(|g| g.stack.count > 0));
}

#[test]
fn the_barrel_takes_the_hits_its_content_declares() {
    let mut w = barrel_world();
    let landed = smash(&mut w);
    assert_eq!(
        landed, FIXTURE_HITS,
        "a barrel opened in {landed} landed swings, not the fixture's \
         {FIXTURE_HITS} — `hits` is content and this is what reads it"
    );
}

#[test]
fn a_smashed_barrel_is_harvested_and_holds_a_respawn_timer() {
    let mut w = barrel_world();
    smash(&mut w);
    let paid = w.ground_items.len();
    let life = w
        .slot_lives
        .entries()
        .iter()
        .find(|l| l.respawn_at > 0)
        .expect("the smashed slot kept no respawn record");
    assert!(
        life.respawn_at >= w.tick + sim_core::gather::RESPAWN_MIN_TICKS,
        "the barrel respawns inside the spoken 20-45 min window's floor"
    );
    assert!(
        w.slot_lives.is_harvested(life.cx, life.cz),
        "the smashed barrel is still standing"
    );
    // And it does not pay twice: swinging the same spot again finds
    // nothing, so the container count stays where it was.
    let mut seq = 100u16;
    for _ in 0..120 {
        swing(&mut w, &mut seq);
    }
    assert_eq!(
        w.ground_items.len(),
        paid,
        "the harvested barrel smashed a second time"
    );
}

/// **A scatter is silent on the event lane, and a bag was not.**
///
/// The old shape pushed `EV_BAG_DROPPED` with the smasher as the owner,
/// which bought two things: a feed line, and a marker on the smasher's own
/// map (`ui::map::resolve_marks` draws standing bags). Ground items give
/// up both deliberately — litter on the map is noise, and the client
/// learns about a stack from the sync that draws it rather than from an
/// announcement. The server watches `(next_id, len)` to know the store
/// moved, which is a complete fingerprint because every insert bumps
/// `next_id` (`server/core.rs`); a count alone would miss one-in-one-out.
///
/// Gated rather than left implicit: an event per expiring stack is the
/// obvious thing for a later reader to add, and it would spend the
/// reliable lane on scenery.
#[test]
fn a_scatter_says_nothing_on_the_event_lane() {
    let mut w = barrel_world();
    smash(&mut w);
    assert!(
        !w.ground_items.is_empty(),
        "nothing scattered, so this proves nothing"
    );
    assert!(
        !w.events.entries().iter().any(|e| e.code == EV_BAG_DROPPED),
        "a scattered stack announced itself as a bag"
    );
}

/// **The existing pickup verb takes a scattered stack** — no new opcode
/// ships with this slice. `Command::Pickup` was arrow recovery's
/// (`spent.rs`), payload-free and nearest-in-reach, which is exactly the
/// shape a loose stack wants; the sim tries the stack first and falls
/// through to the arrow, so one key serves both and the choice of which
/// never reaches the client.
///
/// One press takes **one** stack, which is the reference's rule
/// (`WorldItem.Pickup` is an RPC per entity) and is why this presses once
/// per stack rather than once.
#[test]
fn the_pickup_verb_takes_a_scattered_stack() {
    let mut w = barrel_world();
    for s in w.players[0].inv.iter_mut() {
        *s = ItemStack::default();
    }
    smash(&mut w);
    let on_ground: u32 = w
        .ground_items
        .entries()
        .iter()
        .map(|g| g.stack.count as u32)
        .sum();
    let stacks = w.ground_items.len();
    assert!(on_ground > 0 && stacks > 0);

    for _ in 0..stacks {
        w.tick(&[Command::Pickup { id: PLAYER }]);
    }
    let got: u32 = w.players[0].inv.iter().map(|s| s.count as u32).sum();
    assert_eq!(got, on_ground, "the pickup verb did not take every stack");
    assert!(
        w.ground_items.is_empty(),
        "a taken stack is supposed to leave the ground"
    );
}

#[test]
fn inert_loot_content_leaves_barrels_standing() {
    // A barrel that broke into nothing would be worse than one that does
    // not break, so an unarmed table must refuse the swing rather than
    // consume it — and the arm stays free for combat.
    let mut w = barrel_world();
    w.loot = LootContent::EMPTY;
    let mut seq = 0u16;
    for _ in 0..200 {
        swing(&mut w, &mut seq);
    }
    assert!(w.ground_items.is_empty(), "an inert table still paid loot");
    assert_eq!(
        w.slot_lives.len(),
        0,
        "an inert table still counted hits on the barrel"
    );
}

#[test]
fn two_barrels_roll_independently() {
    // The roll is addressed by cell, so two barrels are not one barrel
    // twice. Same tick, same seed, different cells.
    let gc = GatherContent::probe_fixture();
    let lc = LootContent::probe_fixture();
    let mut differed = false;
    for n in 0..64u16 {
        let (mut a, mut b) = (
            [ItemStack::default(); sim_core::limits::INV_SLOTS],
            [ItemStack::default(); sim_core::limits::INV_SLOTS],
        );
        lc.roll_into(LOOT_BARREL, &gc, SEED, cell_key(n, 5), 400, &mut a);
        lc.roll_into(LOOT_BARREL, &gc, SEED, cell_key(n + 1, 5), 400, &mut b);
        differed |= a != b;
    }
    assert!(
        differed,
        "every cell rolls the same barrel — cell is unused"
    );
}
