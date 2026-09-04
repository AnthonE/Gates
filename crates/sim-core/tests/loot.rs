//! The barrel loop, end to end: swing at a barrel → it comes apart after
//! its content-declared hits → the table rolls into a container standing
//! where the barrel stood → the existing loot verb empties it.
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
use sim_core::movement::{Body, POS_XZ_Q, POS_Y_Q};
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

/// One player stood exactly on the first barrel. Point-blank on purpose:
/// `POINT_BLANK_M2` bypasses the aim cone, so the fixture never has to
/// reproduce a yaw to make a swing land.
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

/// Swings until a container stands up, returning how many landed swings it
/// took. Panics rather than looping forever.
fn smash(w: &mut World) -> u32 {
    let mut seq = 0u16;
    let mut landed = 0u32;
    let before = w.backpacks.len();
    for _ in 0..MAX_STEPS {
        let hits_before = w.slot_lives.len();
        swing(w, &mut seq);
        if w.slot_lives.len() > hits_before || w.backpacks.len() > before {
            landed += 1;
        }
        if w.backpacks.len() > before {
            return landed;
        }
    }
    panic!("the barrel never came apart in {MAX_STEPS} ticks");
}

#[test]
fn a_barrel_comes_apart_into_a_container_where_it_stood() {
    let (x, y, z) = barrel_pos();
    let mut w = barrel_world();
    smash(&mut w);

    assert_eq!(w.backpacks.len(), 1, "exactly one container per barrel");
    let c = w.backpacks.entries()[0];
    // The sim sims on the values it transmits: the container's address is
    // the barrel's own quantized position, so the client draws the loot
    // where it drew the barrel rather than where the smasher stood.
    assert_eq!(c.qx, sim_core::movement::quant_xz(x), "container x");
    assert_eq!(c.qy, sim_core::movement::quant_y(y), "container y");
    assert_eq!(c.qz, sim_core::movement::quant_xz(z), "container z");
    let px = w.players[0].body.qx as f32 * POS_XZ_Q;
    assert!(
        fabs(c.qx as f32 * POS_XZ_Q - px) < 2.0,
        "the container is out of the smasher's reach"
    );
    assert!(
        c.items.iter().any(|s| s.count > 0),
        "the container stood up empty — the roll did not run"
    );
    assert!(c.expires > w.tick, "the container despawns in the past");
    assert!(
        fabs(c.qy as f32 * POS_Y_Q - y) < 1.0,
        "the container is not at the barrel's height"
    );
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
    assert!(w.backpacks.entries()[0].items.iter().any(|s| s.count > 0));
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
        w.backpacks.len(),
        1,
        "the harvested barrel smashed a second time"
    );
}

#[test]
fn the_container_announces_itself_and_the_smasher_owns_it() {
    let mut w = barrel_world();
    smash(&mut w);
    let dropped: Vec<_> = w
        .events
        .entries()
        .iter()
        .filter(|e| e.code == EV_BAG_DROPPED)
        .collect();
    assert_eq!(dropped.len(), 1, "the container did not announce itself");
    assert_eq!(dropped[0].a, w.backpacks.entries()[0].id, "a is the id");
    assert_eq!(dropped[0].b, PLAYER, "b is the owner — here, the smasher");
}

#[test]
fn the_existing_loot_verb_empties_a_smashed_barrel() {
    // No new verb ships with this: the container is the store the move
    // and loot paths already resolve, so the client half is a panel and
    // not a protocol.
    let mut w = barrel_world();
    for s in w.players[0].inv.iter_mut() {
        *s = ItemStack::default();
    }
    smash(&mut w);
    let held: u32 = w.backpacks.entries()[0]
        .items
        .iter()
        .map(|s| s.count as u32)
        .sum();
    assert!(held > 0);

    w.tick(&[Command::Loot { id: PLAYER }]);
    let got: u32 = w.players[0].inv.iter().map(|s| s.count as u32).sum();
    assert_eq!(got, held, "the loot verb did not move the whole container");
    assert!(
        w.backpacks.is_empty(),
        "an emptied container is supposed to leave"
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
    assert!(w.backpacks.is_empty(), "an inert table still paid loot");
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
