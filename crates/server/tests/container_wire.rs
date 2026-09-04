//! The container view on the wire (wire v19): who is shown a container's
//! contents, and for exactly how long.
//!
//! Every other S→C message on the event lane is either a broadcast (a
//! placement, a death, a door) or an own-fact nobody else would want (your
//! gather, your health). This one is the first whose audience is a
//! *restriction*: a box's contents fanned out to AOI is a raider reading a
//! base's stock through its walls, and the message would be a working
//! feature with a security hole in it rather than a broken one. A gate
//! that only checked the opener receives contents would pass on exactly
//! that bug.
//!
//! So the claims here are made against `seen` — the decoded bytes the
//! server actually put on the lane, per connection slot — and not against
//! a client mirror. A client that agrees with the world proves nothing
//! about which lane the agreement arrived on, and the negative claim (the
//! *other* client was told nothing) cannot be made against a mirror at
//! all. The `backpack_wire` shape, for the same reason it uses it.
//!
//! Five things a container view can get wrong, one test each — and the
//! fifth was found the hard way, shipped and green, in 2026-08-14's sweep
//! of what world containers v0 left (`NOW.md` §0wc):
//!
//! 1. **It pays the wrong audience.** The whole security property.
//! 2. **It trusts the open.** An open is a subscription, not a permission
//!    — `inventory::CONT_BAG` states that reach is proved when a move
//!    resolves and never when a panel opened. If the view proved reach
//!    once and then streamed, a player could open a box, walk home, and
//!    keep reading it. So reach is re-proved every tick, and walking away
//!    must produce a *close* rather than silence: a panel the server has
//!    stopped feeding but never shut is a panel the player drags into.
//! 3. **It answers a forged handle.** The handle is 32 unvalidated bits by
//!    design (refusing it at decode would end the session — the disconnect
//!    the container verbs exist to never cause), so "that bag does not
//!    exist" has to be answered by the view, and answered with nothing.
//! 4. **It sends the wrong slots.** The diff after the open, which is
//!    where a shadow that was not zeroed shows up.
//! 5. **It reads the right slot of the wrong store.** A kind dispatch
//!    written as an `if/else` over the kinds that existed that day, which
//!    keeps compiling and stops being true when the next kind lands.

use client_core::core::{ClientCore, APPLIED2_CONT};
use protocol::{EventMsg, ItemCatalog};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::backpack::BackpackContent;
use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::combat::CombatContent;
use sim_core::deploy::{box_key, DeployContent, DeployDef, ARCH_BOX, PLACE_FOUNDATION};
use sim_core::gather::{cell_key, GatherContent, ItemStack, SWING_INTERVAL_TICKS};
use sim_core::input::BTN_PRIMARY;
use sim_core::inventory::{CONT_BAG, CONT_BOX, CONT_SELF, CONT_WEAR, CONT_WORLD};
use sim_core::limits::{BOX_SLOTS, INV_SLOTS, MAX_ITEM_DEFS, WEAR_SLOTS};
use sim_core::loot::{LootContent, LootEntryDef, LootTableDef, LOOT_CRATE};
use sim_core::movement::Body;
use sim_core::terrain::{self, Haven, Occupant, ScatterTable, CELL_SIZE, HAVEN_CRATES};
use sim_core::world::Command;

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

const SEED: u64 = 20_260_804;
/// The canonical dev spawn point, guarded walkable in sim-core
/// `world::tests` — the same one `backpack_wire` stands its bodies on.
const SPAWN: (f32, f32) = (1024.0, 1024.0);
/// Fixture items. Three different ones, and none of them a slot index or a
/// count that appears anywhere else in this file: a container message is a
/// positional payload with (slot, item, count) triples in it, which is the
/// shape `reference/FINDINGS.md` §1 counts ~27 shipped defects in.
const SPEAR: u16 = 0;
const FILLER: u16 = 7;
const OTHER: u16 = 11;
const THIRD: u16 = 19;
/// Slots inside a bag, spread rather than bunched so an off-by-one in the
/// slot field is visible. Both are also inside `BOX_SLOTS`, so the same
/// constants serve the box test.
const SLOT_A: usize = 2;
const SLOT_B: usize = 9;
/// Counts, all distinct from each other and from every slot and item above.
const COUNT_A: u16 = 40;
const COUNT_B: u16 = 63;
const COUNT_C: u16 = 27;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn world_slot(core: &ShardCore, id: u32) -> usize {
    core.world
        .players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("player in world")
}

/// One server tick with the event bytes decoded per slot. Returns the
/// `APPLIED2_*` word each client ended on, so a claim about the C ABI the
/// ui lane reads is made through the C ABI and not around it.
fn pump(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    seen: &mut Vec<(usize, EventMsg)>,
) -> [u32; 4] {
    let mut buf = [0u8; 1100];
    for (slot, c) in clients.iter_mut() {
        c.advance(1000.0 / 30.0);
        let n = c.poll_input(&mut buf);
        if n > 0 {
            let dg = protocol::decode_input(&buf[..n]).expect("client encodes valid input");
            core.push_input(*slot, &dg);
        }
    }
    let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
    core.tick_bare(stats, |lane, slot, bytes| {
        if matches!(lane, Lane::Event) {
            events.push((slot, bytes.to_vec()));
        }
        true
    });
    let mut applied2 = [0u32; 4];
    for (slot, bytes) in events {
        seen.push((
            slot,
            protocol::decode_event(&bytes).expect("server events decode"),
        ));
        if let Some(c) = clients.iter_mut().find(|(s, _)| *s == slot).map(|(_, c)| c) {
            c.on_stream(&bytes).expect("server events decode");
            applied2[slot] |= c.applied2();
        }
    }
    applied2
}

/// One container-sync message, flattened: (connection slot, container
/// kind, handle, reset, the slots it named). Named rather than written
/// inline because it is five positional fields with two integers and a
/// bool among them — the shape CLAUDE.md's trap list counts ~27 shipped
/// corrections on — so every reader of this file should be able to look up
/// what position 2 is.
type Sync = (usize, u8, u32, bool, Vec<(u8, ItemStack)>);

/// Every container-sync message on the lane.
fn syncs(seen: &[(usize, EventMsg)]) -> Vec<Sync> {
    cont_syncs(seen, false)
}

/// The same, for the **body's** stream.
///
/// The two are separated at the helper rather than at each call site
/// because they are now two independent subscriptions sharing one
/// message (`NOW.md` §0eq item 4): the ground container is opened, is
/// exclusive and can be shut by the server, and the body is none of
/// those — it is dripped unconditionally from the moment a player has a
/// world slot. Every assertion in this file below `two_clients` is about
/// one of them, and a helper that returned both would make each of those
/// assertions quietly depend on what the *other* stream did that tick.
fn wear_syncs(seen: &[(usize, EventMsg)]) -> Vec<Sync> {
    cont_syncs(seen, true)
}

fn cont_syncs(seen: &[(usize, EventMsg)], want_wear: bool) -> Vec<Sync> {
    seen.iter()
        .filter(|(_, m)| {
            matches!(m, EventMsg::ContSync { kind, .. } if (*kind == CONT_WEAR) == want_wear)
        })
        .filter_map(|(slot, m)| match m {
            EventMsg::ContSync {
                kind,
                cont,
                reset,
                slots,
                count,
            } => Some((
                *slot,
                *kind,
                *cont,
                *reset,
                slots[..*count as usize]
                    .iter()
                    .map(|s| (s.slot, s.stack))
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

fn armed_core() -> Box<ShardCore> {
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.combat = CombatContent::probe_fixture();
    // Long-lived bags: these tests assert on a bag many ticks after it
    // stands up, and a despawn mid-test would read as a container view
    // bug. The despawn path has its own gate in `backpack_wire`.
    let mut bc = BackpackContent::probe_fixture();
    bc.despawn_ticks = [0; MAX_ITEM_DEFS];
    bc.base_ticks = 1 << 30;
    core.world.backpack = bc;
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    core
}

/// Two connected clients on the spawn point with their join drips settled.
fn two_clients(core: &mut ShardCore, stats: &ShardStats) -> Vec<(usize, ClientCore)> {
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    let mut warm = Vec::new();
    for _ in 0..8 {
        pump(core, stats, &mut clients, &mut warm);
    }
    assert!(
        syncs(&warm).is_empty(),
        "a join must not open a GROUND container by itself"
    );
    // **The body, on the other hand, opens itself and must.** That is the
    // whole of `NOW.md` §0eq item 4: the wear view stopped competing for
    // the ground subscription and is dripped from the moment a player has
    // a world slot, so a client never has to ask and a box can never
    // evict it. One reset apiece, carrying nothing — a fresh spawn wears
    // nothing, and `reset` with zero rows is how "the body is empty" is
    // said (a diff with no rows is never sent).
    let worn = wear_syncs(&warm);
    assert_eq!(
        worn.len(),
        2,
        "each joined client is owed exactly one opening body: {worn:?}"
    );
    for (_, kind, handle, reset, rows) in &worn {
        assert_eq!((*kind, *handle, *reset), (CONT_WEAR, 0, true));
        assert!(rows.is_empty(), "a fresh spawn wears nothing: {rows:?}");
    }
    clients
}

/// Kill slot 1 under slot 0's feet, leaving a bag holding two known
/// stacks.
///
/// The bag arrives by the path the game takes rather than by a store
/// insert: `Backpacks::stand_up` needs an `EventQueue` and nothing public
/// hands one out, and adding a constructor so a test could reach around
/// the sim would be API that exists for this file alone. The fight is
/// `backpack_wire`'s arrangement — both bodies coincident every tick, so
/// the aim cone has no bearing to fail on.
fn bag_from_a_kill(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
) -> u32 {
    let (w0, w1) = (world_slot(core, id_of(0)), world_slot(core, id_of(1)));
    core.world.players[w0].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
        cond: 0,
    };
    core.world.players[w1].inv = [ItemStack::default(); INV_SLOTS];
    core.world.players[w1].inv[SLOT_A] = ItemStack {
        item: FILLER,
        count: COUNT_A,
        cond: 0,
    };
    core.world.players[w1].inv[SLOT_B] = ItemStack {
        item: OTHER,
        count: COUNT_B,
        cond: 0,
    };
    let deaths_before = core.world.players[w1].deaths;
    let mut burn = Vec::new();
    clients[0].1.set_input(BTN_PRIMARY, 0, 128, 0, 0, 0);
    clients[1].1.set_input(0, 0, 128, 0, 0, 0);
    for _ in 0..(SWING_INTERVAL_TICKS * 8) {
        let (w0, w1) = (world_slot(core, id_of(0)), world_slot(core, id_of(1)));
        core.world.players[w1].body = core.world.players[w0].body;
        pump(core, stats, clients, &mut burn);
        let w1 = world_slot(core, id_of(1));
        if core.world.players[w1].deaths > deaths_before {
            clients[0].1.set_input(0, 0, 128, 0, 0, 0);
            assert_eq!(core.world.backpacks.len(), 1, "the death left one bag");
            let bag = core.world.backpacks.entries()[0];
            assert!(
                burn.iter()
                    .all(|(_, m)| !matches!(m, EventMsg::ContSync { .. })),
                "a death must not open a panel by itself"
            );
            return bag.id;
        }
    }
    panic!("three fixture spear hits must kill inside eight swing intervals");
}

/// Feed one client's open/close action in as if it had arrived on the
/// action lane — through `decode_action`, so the bytes are real.
fn ask(core: &mut ShardCore, slot: usize, kind: u8, cont: u32) {
    let mut buf = [0u8; 32];
    let n =
        protocol::encode_action_container(kind, cont, &mut buf).expect("a shape the wire takes");
    let msg = protocol::decode_action(&buf[..n]).expect("round trips");
    core.push_action(slot, msg);
}

#[test]
fn only_the_opener_is_shown_a_container() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    let mut clients = two_clients(&mut core, &stats);
    let bag = bag_from_a_kill(&mut core, &stats, &mut clients);
    // Stand slot 1 on the bag too. This is the whole point of the test:
    // the other client is not merely out of AOI or out of reach, it is
    // *standing on the container* and still must be told nothing. A test
    // that parked it across the map would pass against an AOI broadcast.
    let (w0, w1) = (world_slot(&core, id_of(0)), world_slot(&core, id_of(1)));
    core.world.players[w1].body = core.world.players[w0].body;
    // And give the neighbour a reason to be sent something on this very
    // lane in this very window. Slot 1 is a fresh corpse: it is quiet, so
    // without this the capture below holds nothing for slot 1 and the
    // negative claim is true for the wrong reason — it would stay green
    // the day the unicast becomes a broadcast, because the observation
    // would have broken first. One slot of its own inventory moves, which
    // the server owes it as a unicast `Inv` diff and nobody else.
    core.world.players[w1].inv[0] = ItemStack {
        item: THIRD,
        count: COUNT_C,
        cond: 0,
    };

    let mut seen = Vec::new();
    ask(&mut core, 0, CONT_BAG, bag);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }

    // Before the negative claim, prove the channel it is made on: this
    // harness records events for **both** slots, so "slot 1 got no
    // container message" is a statement about the server and not about a
    // capture that was only ever watching slot 0. A negative assertion
    // with no such check is the shape that passes after the observation
    // breaks.
    assert!(
        seen.iter()
            .any(|(slot, m)| *slot == 1 && matches!(m, EventMsg::Inv { .. })),
        "the capture recorded no event-lane traffic for slot 1 — the negative \
         claim below would pass against a broadcast, or against a harness that \
         had stopped watching: {seen:?}"
    );
    let got = syncs(&seen);
    assert!(
        got.iter().all(|(slot, ..)| *slot == 0),
        "a container's contents crossed to a client that did not open it: {got:?}"
    );
    let (_, kind, handle, reset, rows) = &got[0];
    assert_eq!((*kind, *handle, *reset), (CONT_BAG, bag, true));
    assert_eq!(
        rows,
        &vec![
            (
                SLOT_A as u8,
                ItemStack {
                    item: FILLER,
                    count: COUNT_A,
                    cond: 0,
                }
            ),
            (
                SLOT_B as u8,
                ItemStack {
                    item: OTHER,
                    count: COUNT_B,
                    cond: 0,
                }
            ),
        ],
        "the opener was shown the wrong slots"
    );
    assert_eq!(
        got.len(),
        1,
        "an open pays once, not once per tick: {got:?}"
    );

    // And the client mirror agrees with the bytes — through the same
    // fields the ui lane reads, so a readout that decoded but published
    // nothing would fail here.
    let c0 = &clients[0].1;
    assert_eq!((c0.cont_kind, c0.cont_handle), (CONT_BAG, bag));
    assert_eq!(
        c0.cont[SLOT_A],
        ItemStack {
            item: FILLER,
            count: COUNT_A,
            cond: 0,
        }
    );
    assert_eq!(c0.cont[SLOT_B].item, OTHER);
    let c1 = &clients[1].1;
    assert_eq!(
        (c1.cont_kind, c1.cont_handle),
        (CONT_SELF, 0),
        "the neighbour's panel must have nothing in it"
    );
}

#[test]
fn walking_away_closes_the_panel_rather_than_starving_it() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    let mut clients = two_clients(&mut core, &stats);
    let bag = bag_from_a_kill(&mut core, &stats, &mut clients);

    let mut seen = Vec::new();
    ask(&mut core, 0, CONT_BAG, bag);
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    assert_eq!(syncs(&seen).len(), 1, "the open paid");
    assert_eq!(clients[0].1.cont_kind, CONT_BAG);

    // Walk out of reach. Nothing else changes — the bag is still standing,
    // still in the store, still at the same address.
    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].body = Body::at(SEED, hv(SEED), SPAWN.0 + 200.0, SPAWN.1 + 200.0);
    assert_eq!(core.world.backpacks.len(), 1, "the bag did not go anywhere");

    let mut after = Vec::new();
    let applied2 = pump(&mut core, &stats, &mut clients, &mut after);
    let got = syncs(&after);
    assert_eq!(got.len(), 1, "walking away must say something: {got:?}");
    let (slot, kind, handle, reset, rows) = &got[0];
    assert_eq!(
        (*slot, *kind, *handle, *reset, rows.len()),
        (0, CONT_SELF, 0, true, 0),
        "the server must shut the panel, not merely stop feeding it"
    );
    assert_ne!(
        applied2[0] & APPLIED2_CONT,
        0,
        "the close reaches the C ABI"
    );
    assert_eq!(
        (clients[0].1.cont_kind, clients[0].1.cont_handle),
        (CONT_SELF, 0),
        "the client's panel closed with it"
    );

    // And it stays shut: no contents leak on later ticks, and the close is
    // not re-sent forever either.
    let mut later = Vec::new();
    for _ in 0..5 {
        pump(&mut core, &stats, &mut clients, &mut later);
    }
    assert!(
        syncs(&later).is_empty(),
        "a shut panel must be silent: {:?}",
        syncs(&later)
    );
}

#[test]
fn a_forged_handle_is_answered_with_nothing() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    let mut clients = two_clients(&mut core, &stats);
    let real = bag_from_a_kill(&mut core, &stats, &mut clients);

    let mut seen = Vec::new();
    // A bag id that was never minted, and a box address where no box
    // stands. Both are shapes the wire happily carries — that is the
    // point, since refusing them at decode would end the session.
    ask(&mut core, 0, CONT_BAG, real.wrapping_add(1_000));
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    ask(&mut core, 0, CONT_BOX, box_key(77, 88, 1));
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }

    let got = syncs(&seen);
    assert!(
        got.iter()
            .all(|(_, kind, _, _, rows)| *kind == CONT_SELF && rows.is_empty()),
        "a forged open was paid in contents: {got:?}"
    );
    assert_eq!(
        clients[0].1.cont_kind, CONT_SELF,
        "nothing may be open after a forged handle"
    );
}

#[test]
fn a_change_inside_an_open_container_arrives_as_a_diff() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    let mut clients = two_clients(&mut core, &stats);
    let bag = bag_from_a_kill(&mut core, &stats, &mut clients);

    let mut seen = Vec::new();
    ask(&mut core, 0, CONT_BAG, bag);
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    assert_eq!(syncs(&seen).len(), 1);

    // Change one slot and empty another. Both are changes the diff owes
    // the client, and the second is the one a naive "send the non-empty
    // slots" implementation drops on the floor — the panel would keep
    // drawing a stack that is gone.
    core.world.backpacks.set_slot(
        0,
        SLOT_A,
        ItemStack {
            item: THIRD,
            count: COUNT_C,
            cond: 0,
        },
    );
    core.world
        .backpacks
        .set_slot(0, SLOT_B, ItemStack::default());

    let mut after = Vec::new();
    pump(&mut core, &stats, &mut clients, &mut after);
    let got = syncs(&after);
    assert_eq!(got.len(), 1, "one message for the tick's changes: {got:?}");
    let (slot, kind, handle, reset, rows) = &got[0];
    assert_eq!((*slot, *kind, *handle, *reset), (0, CONT_BAG, bag, false));
    assert_eq!(
        rows,
        &vec![
            (
                SLOT_A as u8,
                ItemStack {
                    item: THIRD,
                    count: COUNT_C,
                    cond: 0,
                }
            ),
            (SLOT_B as u8, ItemStack::default()),
        ],
        "a diff must name the emptied slot too, not only the changed one"
    );
    let c0 = &clients[0].1;
    assert_eq!(
        c0.cont[SLOT_A],
        ItemStack {
            item: THIRD,
            count: COUNT_C,
            cond: 0,
        }
    );
    assert_eq!(c0.cont[SLOT_B], ItemStack::default());

    // A tick with nothing changing says nothing.
    let mut quiet = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut quiet);
    }
    assert!(
        syncs(&quiet).is_empty(),
        "an unchanged container must not resend: {:?}",
        syncs(&quiet)
    );
}

#[test]
fn a_client_close_shuts_the_view_without_a_reply() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    let mut clients = two_clients(&mut core, &stats);
    let bag = bag_from_a_kill(&mut core, &stats, &mut clients);

    let mut seen = Vec::new();
    ask(&mut core, 0, CONT_BAG, bag);
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    ask(&mut core, 0, CONT_SELF, 0);
    let mut after = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut after);
    }
    assert!(
        syncs(&after).is_empty(),
        "the client already knows it closed; an echo is bytes for nothing: {:?}",
        syncs(&after)
    );

    // And the view really is shut server-side: a change inside the bag
    // must now cross to nobody.
    core.world.backpacks.set_slot(
        0,
        SLOT_A,
        ItemStack {
            item: THIRD,
            count: COUNT_C,
            cond: 0,
        },
    );
    let mut later = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut later);
    }
    assert!(
        syncs(&later).is_empty(),
        "a closed view kept streaming: {:?}",
        syncs(&later)
    );
}

// --- the box, by packed address -------------------------------------------

/// Row 4 of the fixture is the box, costing one unit of item 6 — the
/// `box_container` arrangement, so the two files agree about what a box is.
const BOX_ROW: u16 = 4;
const BOX_ITEM: u16 = 6;
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

/// The nearest cell the foundation's terrain rule accepts, in ring order
/// so it is a pure function of the seed (`box_container`'s helper).
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

/// **A box open no longer costs you your body, which is the whole slice.**
///
/// `NOW.md` §0eq item 4, and the merge-gate judge's second ranked fix on
/// pass `-06`. Armor v1 gave `CONT_WEAR` an arm in the server's *one*
/// container subscription, so the two views took turns: opening a box
/// evicted the wear panel, and the route from a looted helmet to a head
/// was take it, close the box, open the inventory, drag again — with the
/// box's own gate (`box_container.rs`) celebrating a move the client had
/// no path to make.
///
/// The claim here is that both streams run at once. A box is opened, its
/// contents arrive on the ground stream, and then the player is dressed
/// **while it is still open** — and the body arrives on its own stream,
/// addressed to `CONT_WEAR` with handle 0, without a second open and
/// without disturbing the box.
///
/// The mutant that matters is the old code: put `CONT_WEAR` back on the
/// ground subscription and the wear stream has no reset to send, the
/// helmet never crosses, and `clients[0].1.worn` stays empty while
/// `cont_kind` reads `CONT_BOX`. Both halves of the final assertion pair
/// fail, and they fail for the two different reasons the split exists
/// for: the box is not evicted, and the body is not absent.
#[test]
fn the_body_is_still_fed_while_a_box_is_open() {
    let stats = ShardStats::default();
    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);

    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    core.world.deploy = box_fixture();
    core.world.dev_spawn = Some((x, z));
    core.catalog = ItemCatalog::EMPTY;
    let mut clients = two_clients(&mut core, &stats);

    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].body = Body::at(SEED, hv(SEED), x, z);
    core.world.players[w0].inv[0] = ItemStack {
        item: 0,
        count: 5,
        cond: 0,
    };
    core.world.players[w0].inv[1] = ItemStack {
        item: BOX_ITEM,
        count: 1,
        cond: 0,
    };
    core.world.tick(&[Command::Place {
        id: id_of(0),
        row: FOUNDATION_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        freehand: false,
    }]);
    core.world.tick(&[Command::PlaceDeploy {
        id: id_of(0),
        row: BOX_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    core.world.deploys.set_box_slot(
        0,
        SLOT_A,
        ItemStack {
            item: FILLER,
            count: COUNT_A,
            cond: 0,
        },
    );
    let key = box_key(cx, cz, 0);

    // 1 · open the box and let it land.
    let mut seen = Vec::new();
    ask(&mut core, 0, CONT_BOX, key);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    assert_eq!(syncs(&seen).len(), 1, "the box must have opened");
    assert!(
        wear_syncs(&seen).is_empty(),
        "nothing was worn, so the body owed nothing: {:?}",
        wear_syncs(&seen)
    );

    // 2 · dress the player with the box still open. In the game this is
    //     the move `box_container.rs` gates; here the store is written
    //     directly, because the claim is about the two VIEWS and not
    //     about the verb, which has its own suite.
    let helmet = ItemStack {
        item: OTHER,
        count: 1,
        cond: 9_100,
    };
    core.world.players[w0].worn[0] = helmet;

    let mut after = Vec::new();
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut after);
    }

    let worn = wear_syncs(&after);
    assert_eq!(
        worn.len(),
        1,
        "the body changed under an open box and owed exactly one diff: {worn:?}"
    );
    let (slot, kind, handle, reset, rows) = &worn[0];
    assert_eq!((*slot, *kind, *handle, *reset), (0, CONT_WEAR, 0, false));
    assert_eq!(
        rows,
        &vec![(0u8, helmet)],
        "the body's diff must carry the helmet and nothing else"
    );
    // The box was not disturbed: it changed nothing, so it owed nothing.
    assert!(
        syncs(&after).is_empty(),
        "the ground container must not have been re-sent or shut: {:?}",
        syncs(&after)
    );

    // 3 · and the client holds both at once, which is what the panel
    //     draws. This pair is the feature: a kind that says a box is
    //     open, and a body that is legible beside it.
    assert_eq!(
        (clients[0].1.cont_kind, clients[0].1.cont_handle),
        (CONT_BOX, key),
        "the box must still be the open container"
    );
    assert_eq!(
        clients[0].1.worn[0], helmet,
        "the body must be readable while a box is open — the whole of §0eq item 4"
    );
}

/// **An old client's `open_worn` press is answered with a resync, not an
/// eviction.**
///
/// `ACT_CONTAINER(CONT_WEAR, 0)` was the armor v1 open and is still a
/// shape the wire takes, so a client built before 2026-08-28 sends it
/// every time it raises the inventory — and the one thing it must not do
/// is what it used to: take the ground subscription and shut whatever
/// box was open. The server answers it by re-sending the body, which is
/// the honest reading of "send me my body" and is what it was already
/// doing anyway.
#[test]
fn asking_for_the_body_resyncs_it_and_keeps_the_box() {
    let stats = ShardStats::default();
    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);

    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    core.world.deploy = box_fixture();
    core.world.dev_spawn = Some((x, z));
    core.catalog = ItemCatalog::EMPTY;
    let mut clients = two_clients(&mut core, &stats);

    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].body = Body::at(SEED, hv(SEED), x, z);
    core.world.players[w0].inv[0] = ItemStack {
        item: 0,
        count: 5,
        cond: 0,
    };
    core.world.players[w0].inv[1] = ItemStack {
        item: BOX_ITEM,
        count: 1,
        cond: 0,
    };
    core.world.tick(&[Command::Place {
        id: id_of(0),
        row: FOUNDATION_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        freehand: false,
    }]);
    core.world.tick(&[Command::PlaceDeploy {
        id: id_of(0),
        row: BOX_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    let key = box_key(cx, cz, 0);
    let helmet = ItemStack {
        item: OTHER,
        count: 1,
        cond: 9_100,
    };
    core.world.players[w0].worn[0] = helmet;

    let mut seen = Vec::new();
    ask(&mut core, 0, CONT_BOX, key);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    assert_eq!(syncs(&seen).len(), 1, "the box must have opened");

    // The old press, arriving under an open box.
    let mut after = Vec::new();
    ask(&mut core, 0, CONT_WEAR, 0);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut after);
    }

    let worn = wear_syncs(&after);
    assert_eq!(worn.len(), 1, "the press is owed one body: {worn:?}");
    let (_, kind, handle, reset, rows) = &worn[0];
    assert_eq!(
        (*kind, *handle, *reset),
        (CONT_WEAR, 0, true),
        "a press is a resync, so the batch carries the reset bit"
    );
    assert_eq!(rows, &vec![(0u8, helmet)], "the whole body, not a diff");
    assert!(
        syncs(&after).is_empty(),
        "asking for the body must not shut the box: {:?}",
        syncs(&after)
    );
    assert_eq!(
        (clients[0].1.cont_kind, clients[0].1.cont_handle),
        (CONT_BOX, key),
        "the box must still be open after the press"
    );

    // **The box is still being FED, which is the assertion with teeth.**
    //
    // The three above are all satisfied by an eviction, and finding that
    // out is what the mutant run is for: restore `open_container`'s
    // `CONT_WEAR` arm and the press takes the ground subscription — but
    // the server sends no close (it re-opened, it did not shut), the
    // client routes the reply to `worn` by kind, and `cont_kind` is left
    // reading `CONT_BOX` from before. Every one of them stays green over
    // a subscription that is silently pointed at a body.
    //
    // A live subscription is one that still notices a change, so the
    // proof is to make one.
    core.world.deploys.set_box_slot(
        0,
        SLOT_A,
        ItemStack {
            item: FILLER,
            count: COUNT_A,
            cond: 0,
        },
    );
    let mut later = Vec::new();
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut later);
    }
    let ground = syncs(&later);
    assert_eq!(
        ground.len(),
        1,
        "the box changed and the subscription owed a diff — if this is \
         empty the press evicted it: {ground:?}"
    );
    let (_, kind, handle, reset, rows) = &ground[0];
    assert_eq!((*kind, *handle, *reset), (CONT_BOX, key, false));
    assert_eq!(
        rows,
        &vec![(
            SLOT_A as u8,
            ItemStack {
                item: FILLER,
                count: COUNT_A,
                cond: 0,
            }
        )],
        "the diff must carry the box's slot, not the body's"
    );
}

#[test]
fn a_box_opens_by_its_packed_address() {
    let stats = ShardStats::default();
    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);

    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    core.world.deploy = box_fixture();
    core.world.dev_spawn = Some((x, z));
    core.catalog = ItemCatalog::EMPTY;
    let mut clients = two_clients(&mut core, &stats);

    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].body = Body::at(SEED, hv(SEED), x, z);
    core.world.players[w0].inv[0] = ItemStack {
        item: 0,
        count: 5,
        cond: 0,
    };
    core.world.players[w0].inv[1] = ItemStack {
        item: BOX_ITEM,
        count: 1,
        cond: 0,
    };
    core.world.tick(&[Command::Place {
        id: id_of(0),
        row: FOUNDATION_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        freehand: false,
    }]);
    core.world.tick(&[Command::PlaceDeploy {
        id: id_of(0),
        row: BOX_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(
        core.world.deploys.boxes().len(),
        1,
        "the box must be placed"
    );

    // Put something in it the way a move would, then open it.
    core.world.deploys.set_box_slot(
        0,
        SLOT_A,
        ItemStack {
            item: FILLER,
            count: COUNT_A,
            cond: 0,
        },
    );
    let key = box_key(cx, cz, 0);

    let mut seen = Vec::new();
    ask(&mut core, 0, CONT_BOX, key);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }

    let got = syncs(&seen);
    assert_eq!(got.len(), 1, "one open, one payment: {got:?}");
    let (slot, kind, handle, reset, rows) = &got[0];
    assert_eq!((*slot, *kind, *handle, *reset), (0, CONT_BOX, key, true));
    assert_eq!(
        rows,
        &vec![(
            SLOT_A as u8,
            ItemStack {
                item: FILLER,
                count: COUNT_A,
                cond: 0,
            }
        )],
        "the box's contents did not cross correctly"
    );
    assert_eq!(
        (clients[0].1.cont_kind, clients[0].1.cont_handle),
        (CONT_BOX, key),
        "the handle must round-trip exactly — it is what a move will carry"
    );
    // The tail past `BOX_SLOTS` is empty on both sides and stays that way:
    // a box is twelve slots, and a view that padded it to thirty would let
    // a panel draw slots the sim refuses to move into.
    assert!(clients[0].1.cont[BOX_SLOTS..].iter().all(|s| s.count == 0));
}

// --- the corpse: a subscription does not outlive the subscriber -----------

/// A death shuts the panel, and the death screen cannot open a new one.
///
/// One more thing a container view can get wrong, and the only one whose
/// victim is the player who is *still alive*: the view's whole security
/// argument (`core.rs`, above the resolution) is that the set of containers
/// a client can see is exactly the set it can move items in. A corpse can
/// move nothing — `World::die` empties the body and every mutation verb
/// refuses a `dead` player — but it kept its slot, its position and its
/// `own_wslot`, so a reach-and-lock-only resolution kept answering it. The
/// result was a death screen streaming a box's slots at 30 Hz while the
/// killer emptied it: raid intelligence bought by dying next to your own
/// loot, which is the one thing the sentence above the resolution promises
/// cannot happen.
///
/// Two halves, because the bug has two mouths:
///
/// (a) a subscription opened **alive** and never closed by anything on the
///     death path — nothing calls `close_container` at a death, and the
///     client's death arm does not clear its mirror either, so this half
///     needs no forged client at all; and
/// (b) an open issued **from the death screen**, which the action layer
///     takes like any other (`core.rs`'s `ActionMsg::Container` arm is not
///     a command and asks the sim nothing).
///
/// Half (a) **mutates the box**, and that is not decoration: an unchanged
/// container emits nothing at all (the diff is empty and `open_cont_reset`
/// is false), so a test that only asserted "no further syncs arrive" would
/// pass with the whole defect present. The mutation is what makes silence
/// mean something.
///
/// Clock-free, like every gate here: liveness is `players[w].dead`, a bit
/// the fight writes, and the loop below spins on `dead` rather than on any
/// elapsed span.
#[test]
fn a_corpse_is_shown_no_container() {
    let stats = ShardStats::default();
    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);

    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.combat = CombatContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    core.world.deploy = box_fixture();
    core.world.dev_spawn = Some((x, z));
    core.catalog = ItemCatalog::EMPTY;
    // The backpack module stays inert (`base_ticks == 0`), so the death
    // drops no bag: the only container in this world is the box, and a
    // sync that arrives can only be about it.
    let mut clients = two_clients(&mut core, &stats);

    // Client 0 stands the box up on its own foundation and is the one who
    // will die on it. Client 1 is the killer.
    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].body = Body::at(SEED, hv(SEED), x, z);
    core.world.players[w0].inv[0] = ItemStack {
        item: 0,
        count: 5,
        cond: 0,
    };
    core.world.players[w0].inv[1] = ItemStack {
        item: BOX_ITEM,
        count: 1,
        cond: 0,
    };
    core.world.tick(&[Command::Place {
        id: id_of(0),
        row: FOUNDATION_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        freehand: false,
    }]);
    core.world.tick(&[Command::PlaceDeploy {
        id: id_of(0),
        row: BOX_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(core.world.deploys.boxes().len(), 1, "the box must place");
    core.world.deploys.set_box_slot(
        0,
        SLOT_A,
        ItemStack {
            item: FILLER,
            count: COUNT_A,
            cond: 0,
        },
    );
    let key = box_key(cx, cz, 0);

    // The living open, asserted in full — otherwise the claims below could
    // all be true because the view never worked at this address at all.
    let mut alive = Vec::new();
    ask(&mut core, 0, CONT_BOX, key);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut alive);
    }
    let got = syncs(&alive);
    assert_eq!(got.len(), 1, "the living open paid once: {got:?}");
    let (slot, kind, handle, reset, rows) = &got[0];
    assert_eq!((*slot, *kind, *handle, *reset), (0, CONT_BOX, key, true));
    assert_eq!(
        rows,
        &vec![(
            SLOT_A as u8,
            ItemStack {
                item: FILLER,
                count: COUNT_A,
                cond: 0,
            }
        )],
        "the living view is the baseline the corpse must lose"
    );

    // Kill client 0 where it stands — on the box, in reach, panel open.
    // Both bodies are pinned coincident every tick, `bag_from_a_kill`'s
    // arrangement, so the aim cone has no bearing to fail on and the
    // corpse falls at the address its subscription resolves against.
    let w1 = world_slot(&core, id_of(1));
    core.world.players[w1].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
        cond: 0,
    };
    clients[1].1.set_input(BTN_PRIMARY, 0, 128, 0, 0, 0);
    clients[0].1.set_input(0, 0, 128, 0, 0, 0);
    let mut dying = Vec::new();
    let mut fell = false;
    for _ in 0..(SWING_INTERVAL_TICKS * 8) {
        let (w0, w1) = (world_slot(&core, id_of(0)), world_slot(&core, id_of(1)));
        core.world.players[w0].body = Body::at(SEED, hv(SEED), x, z);
        core.world.players[w1].body = Body::at(SEED, hv(SEED), x, z);
        pump(&mut core, &stats, &mut clients, &mut dying);
        if core.world.players[world_slot(&core, id_of(0))].dead {
            clients[1].1.set_input(0, 0, 128, 0, 0, 0);
            fell = true;
            break;
        }
    }
    assert!(
        fell,
        "three fixture spear hits must kill inside eight swing intervals"
    );

    // The mutation the corpse must not witness: the killer empties one
    // slot and fills another, which is what looting a box looks like from
    // the view's side.
    core.world
        .deploys
        .set_box_slot(0, SLOT_A, ItemStack::default());
    core.world.deploys.set_box_slot(
        0,
        SLOT_B,
        ItemStack {
            item: THIRD,
            count: COUNT_C,
            cond: 0,
        },
    );
    let mut after = Vec::new();
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut after);
    }

    // Everything the server said to slot 0 about a container from the
    // death onward — the death tick's own traffic included, since the
    // close is owed on the tick the body falls and not one later.
    let post: Vec<Sync> = syncs(&dying)
        .into_iter()
        .chain(syncs(&after))
        .filter(|(slot, ..)| *slot == 0)
        .collect();
    assert!(
        !post.is_empty(),
        "a death must shut the panel, not merely stop feeding it"
    );
    let (_, kind, handle, reset, rows) = &post[0];
    assert_eq!(
        (*kind, *handle, *reset, rows.len()),
        (CONT_SELF, 0, true, 0),
        "the first thing a corpse is told about its panel must be the close: {post:?}"
    );
    assert_eq!(
        post.len(),
        1,
        "a corpse watched the box change after its panel shut: {post:?}"
    );
    assert_eq!(
        (clients[0].1.cont_kind, clients[0].1.cont_handle),
        (CONT_SELF, 0),
        "the client's panel closed with the body"
    );

    // (b) The death screen asks for itself. The action layer takes the
    // open — it is not a command and the sim never hears it — so the
    // refusal has to be the view's, and it degrades exactly as a box that
    // stopped existing does: the close, no rows, no new message.
    let mut screen = Vec::new();
    ask(&mut core, 0, CONT_BOX, key);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut screen);
    }
    let got: Vec<Sync> = syncs(&screen)
        .into_iter()
        .filter(|(slot, ..)| *slot == 0)
        .collect();
    assert!(
        !got.is_empty(),
        "a corpse's open must be answered with the close, not with silence"
    );
    assert!(
        got.iter()
            .all(|(_, kind, _, _, rows)| *kind == CONT_SELF && rows.is_empty()),
        "a corpse opened a box from the death screen and was paid its contents: {got:?}"
    );
    assert_eq!(
        (clients[0].1.cont_kind, clients[0].1.cont_handle),
        (CONT_SELF, 0),
        "the death screen's panel must have nothing in it"
    );
}

// --- the locked box: the view asks the lock, not only reach ---------------

/// `probe_fixture` (rows 0..=5, the code lock on 5 / item 7) plus a box on
/// row 6 / item 8 — `sim-core/tests/lock_box.rs`'s arrangement, so the two
/// files agree about what a locked box is.
const LOCKED_BOX_ROW: u16 = 6;
const LOCKED_BOX_ITEM: u16 = 8;
const LOCK_ROW: u16 = 5;
const LOCK_ITEM: u16 = 7;

fn locked_box_fixture() -> DeployContent {
    let mut d = DeployContent::probe_fixture();
    d.defs[LOCKED_BOX_ROW as usize] = DeployDef {
        arch: ARCH_BOX,
        placement: PLACE_FOUNDATION,
        hp: 60,
        item: LOCKED_BOX_ITEM,
        ..DeployDef::INERT
    };
    d.def_count = 7;
    d
}

/// The view-side half of locks on boxes (`NOW.md` §0z item 1), and a
/// **mutant-killer**: every mutation was already refused by the sim
/// (`lock_box.rs`), so nothing in this test moves an item — delete only the
/// `lock_passes` filter in the container-view resolution (`core.rs`) and
/// the first assertion goes red, because the stranger's subscription
/// streams a locked box's slots read-only, every tick, which is the raid
/// intelligence the lock exists to hide.
///
/// The refused view degrades exactly as a box that stopped existing: the
/// close (`CONT_SELF`, reset, no rows), not a new refusal — one fact to a
/// panel, no wire change.
#[test]
fn a_locked_box_shows_a_stranger_nothing_until_it_unlocks() {
    let stats = ShardStats::default();
    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);

    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    core.world.deploy = locked_box_fixture();
    core.world.dev_spawn = Some((x, z));
    core.catalog = ItemCatalog::EMPTY;
    // Client 0 is the owner (the hand that bolts the lock on), client 1
    // the stranger — standing on the box, in reach, exactly where the old
    // reach-only view would have paid them.
    let mut clients = two_clients(&mut core, &stats);
    let (w0, w1) = (world_slot(&core, id_of(0)), world_slot(&core, id_of(1)));
    core.world.players[w0].body = Body::at(SEED, hv(SEED), x, z);
    core.world.players[w1].body = Body::at(SEED, hv(SEED), x, z);

    // Foundation, box, goods inside, lock bolted on and armed — the
    // `lock_box.rs` fixture, driven with the owner's connected id.
    core.world.players[w0].inv[0] = ItemStack {
        item: 0,
        count: 5,
        cond: 0,
    };
    core.world.players[w0].inv[1] = ItemStack {
        item: LOCKED_BOX_ITEM,
        count: 1,
        cond: 0,
    };
    core.world.players[w0].inv[2] = ItemStack {
        item: LOCK_ITEM,
        count: 1,
        cond: 0,
    };
    core.world.tick(&[Command::Place {
        id: id_of(0),
        row: FOUNDATION_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        freehand: false,
    }]);
    core.world.tick(&[Command::PlaceDeploy {
        id: id_of(0),
        row: LOCKED_BOX_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    assert_eq!(core.world.deploys.boxes().len(), 1, "the box must place");
    core.world.deploys.set_box_slot(
        0,
        SLOT_A,
        ItemStack {
            item: FILLER,
            count: COUNT_A,
            cond: 0,
        },
    );
    core.world.tick(&[Command::PlaceDeploy {
        id: id_of(0),
        row: LOCK_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
    }]);
    core.world.tick(&[Command::Access {
        id: id_of(0),
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        op: sim_core::deploy::ACCESS_OP_SET_CODE,
        code: 1234,
    }]);
    let d = core
        .world
        .deploys
        .find(cx, cz, 0, LOC_PLANE)
        .expect("the box record");
    assert!(d.has_lock && d.locked, "the fixture needs its lock armed");
    let key = box_key(cx, cz, 0);

    // The stranger subscribes. What comes back is the same close a box
    // that stopped existing yields — and never a slot.
    let mut seen = Vec::new();
    ask(&mut core, 1, CONT_BOX, key);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    let got = syncs(&seen);
    assert!(
        got.iter()
            .filter(|(slot, ..)| *slot == 1)
            .all(|(_, kind, _, _, rows)| *kind == CONT_SELF && rows.is_empty()),
        "a locked box's contents crossed to a hand its lock does not know: {got:?}"
    );
    assert!(
        got.iter().any(|(slot, ..)| *slot == 1),
        "the refused view must degrade to the close, not to silence: {got:?}"
    );
    assert_eq!(
        (clients[1].1.cont_kind, clients[1].1.cont_handle),
        (CONT_SELF, 0),
        "the stranger's panel must have nothing in it"
    );

    // The owner's subscription simply works — the lock remembers the hand
    // that bolted it on, at the view exactly as at the move.
    let mut owner_seen = Vec::new();
    ask(&mut core, 0, CONT_BOX, key);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut owner_seen);
    }
    let got = syncs(&owner_seen);
    assert_eq!(got.len(), 1, "one open, one payment: {got:?}");
    let (slot, kind, handle, reset, rows) = &got[0];
    assert_eq!((*slot, *kind, *handle, *reset), (0, CONT_BOX, key, true));
    assert_eq!(
        rows,
        &vec![(
            SLOT_A as u8,
            ItemStack {
                item: FILLER,
                count: COUNT_A,
                cond: 0,
            }
        )],
        "the owner was shown the wrong slots"
    );
    ask(&mut core, 0, CONT_SELF, 0);

    // Unlocking reopens the view to anyone, like the shop-front state
    // reopens the move — the stranger asks again (their panel was shut,
    // not starved) and is paid.
    core.world.tick(&[Command::Access {
        id: id_of(0),
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
        op: sim_core::deploy::ACCESS_OP_UNLOCK,
        code: 0,
    }]);
    let mut after = Vec::new();
    ask(&mut core, 1, CONT_BOX, key);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut after);
    }
    let got: Vec<Sync> = syncs(&after)
        .into_iter()
        .filter(|(slot, ..)| *slot == 1)
        .collect();
    assert_eq!(got.len(), 1, "an unlocked box answers anyone: {got:?}");
    let (_, kind, handle, reset, rows) = &got[0];
    assert_eq!((*kind, *handle, *reset), (CONT_BOX, key, true));
    assert_eq!(
        rows,
        &vec![(
            SLOT_A as u8,
            ItemStack {
                item: FILLER,
                count: COUNT_A,
                cond: 0,
            }
        )],
        "the unlocked view pays the same contents the owner saw"
    );
}

// --- the world crate: the panel is drawn from the crate's own store ------

/// A world container's view carries the *world container's* contents.
///
/// The fifth thing a container view can get wrong, and the only one the
/// four above could not see: **it reads the right slot of the wrong
/// store.** Every test in this file until now named `CONT_BAG` or
/// `CONT_BOX`, which were the two ground kinds alive when the drip was
/// written, and the drip dispatched them with a two-way
/// `if kind == CONT_BAG { backpacks } else { deploys }`. World containers
/// v0 (wire v37) added a third kind and that `else` swallowed it: opening
/// the pad's crate indexed `deploys.box_slot` with a `world_conts` index.
///
/// It could not crash — `MAX_WORLD_CONTS` is 64 and the deploy store is
/// 1 024, so the index is always in range and `box_slot` answers a deploy
/// that is usually not a box, which reads as empty. So the crate opened,
/// the panel drew, the handle round-tripped, the move verb worked (it
/// resolves through `World::cont_slot`, which had all three arms), and the
/// player saw an **empty crate** holding four stacks of loot. Seventeen
/// sim checks and eighty-six protocol fixtures were green over it, because
/// the defect lives in neither: it is one store read on the server, in the
/// one code path no test named.
///
/// That is `CLAUDE.md`'s byte-golden trap one level out — three green
/// gates over a wrong *store* rather than a wrong *field* — and it is why
/// `NOW.md` §0wc item 1 says nobody has opened one in the running game.
/// The claim this test makes is the one nothing else could: the bytes on
/// the lane match `world_conts`, not `deploys`.
#[test]
fn a_world_crate_is_drawn_from_the_crate_store() {
    let stats = ShardStats::default();
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(SEED);
    let (cx, cz, x, z) = a_pad_crate(&table, &haven);

    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.loot = crate_fixture();
    core.world.dev_spawn = Some((x, z));
    core.catalog = ItemCatalog::EMPTY;
    let mut clients = two_clients(&mut core, &stats);

    // Stand on the crate. `LOOT_REACH_M` is 5 m against an 8 m cell, so
    // the anchor is the only place the open resolves from.
    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].body = Body::at(SEED, hv(SEED), x, z);

    let key = cell_key(cx, cz);
    let mut seen = Vec::new();
    ask(&mut core, 0, CONT_WORLD, key);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }

    // The store minted and rolled on the open, and the fixture rolls a
    // constant: four stacks of one `CRATE_LOOT`. Asserted before the wire
    // claim so a failure says which half broke.
    assert_eq!(
        core.world.world_conts.len(),
        1,
        "the open must mint exactly one record"
    );
    let held: Vec<(u8, ItemStack)> = (0..INV_SLOTS)
        .map(|s| (s as u8, core.world.world_conts.slot(0, s)))
        .filter(|(_, st)| st.count > 0)
        .collect();
    // Units, not slots: four rolls of one stack into a single slot of
    // four, and how many slots that lands in is the stack limit's business
    // rather than this test's. What has to be true is that the crate is
    // **not empty** — an empty crate is exactly what the defect below
    // produced, so a vacuous fixture would let the wire claim pass on the
    // broken code.
    let units: u32 = held.iter().map(|(_, st)| st.count as u32).sum();
    assert_eq!(
        units, CRATE_ROLLS,
        "the crate fixture must roll {CRATE_ROLLS} units: {held:?}"
    );
    assert!(
        held.iter().all(|(_, st)| st.item == CRATE_LOOT),
        "the crate fixture rolls one item only: {held:?}"
    );

    let got = syncs(&seen);
    assert_eq!(got.len(), 1, "one open, one payment: {got:?}");
    let (slot, kind, handle, reset, rows) = &got[0];
    assert_eq!((*slot, *kind, *handle, *reset), (0, CONT_WORLD, key, true));

    // **The claim.** Under the two-way dispatch this was `[]` — the drip
    // read `deploys.box_slot(0, s)` on a shard with no deploys placed, so
    // every slot came back empty, no slot differed from `last_cont`, and
    // the open sent a reset with zero rows. A crate full of loot drew as
    // an empty panel and nothing anywhere went red.
    assert_eq!(
        rows, &held,
        "the crate's panel must carry the crate's own store, not a deploy's"
    );

    // And it crossed the ABI, so this is a claim about what the client
    // draws rather than about bytes alone.
    assert_eq!(
        (clients[0].1.cont_kind, clients[0].1.cont_handle),
        (CONT_WORLD, key),
        "the handle must round-trip exactly — it is what a move will carry"
    );
    let mirrored: Vec<(u8, ItemStack)> = clients[0].1.cont[..INV_SLOTS]
        .iter()
        .enumerate()
        .filter(|(_, st)| st.count > 0)
        .map(|(s, st)| (s as u8, *st))
        .collect();
    assert_eq!(
        mirrored, held,
        "the client's mirror of the crate must agree with the store"
    );
}

/// The pad's crate, named rather than scanned.
///
/// `worldcont.rs` sweeps all 65 536 cells for the first `CrateSlot`
/// because it only needs *a* container. This file wants **the pad's**, the
/// one `NOW.md` §0wc is about, so it asks `terrain::haven_crate` for the
/// anchor and re-derives the cell — then confirms through `terrain::
/// scatter` that the cell really reports a crate, because the shelter is
/// tested first and can shadow one. Five scatter calls instead of 65 536.
fn a_pad_crate(table: &ScatterTable, haven: &Haven) -> (u16, u16, f32, f32) {
    for k in 0..HAVEN_CRATES {
        let (ax, az, _) = terrain::haven_crate(haven, k);
        let (cx, cz) = (
            (ax * (1.0 / CELL_SIZE)) as i32,
            (az * (1.0 / CELL_SIZE)) as i32,
        );
        let s = terrain::scatter(SEED, table, haven, cx, cz);
        if s.occupant == Occupant::CrateSlot {
            return (cx as u16, cz as u16, s.x, s.z);
        }
    }
    panic!("seed {SEED} puts no crate cell on its haven pad — the generator moved");
}

/// One item, a constant number of stacks: "what did this pay" has to be a
/// constant, or the wire claim cannot tell "read the wrong store" from
/// "rolled differently". `worldcont.rs`'s fixture, minus the cache table
/// this file never opens.
const CRATE_LOOT: u16 = 2;
const CRATE_ROLLS: u32 = 4;

fn crate_fixture() -> LootContent {
    let mut c = LootContent::probe_fixture();
    let mut t = LootTableDef::INERT;
    t.entries[0] = LootEntryDef {
        item: CRATE_LOOT,
        weight: 1,
        count_min: 1,
        count_max: 1,
    };
    t.len = 1;
    t.total_weight = 1;
    t.rolls_min = CRATE_ROLLS as u16;
    t.rolls_max = CRATE_ROLLS as u16;
    c.tables[LOOT_CRATE] = t;
    c
}

/// **When a fifth container kind lands, this file stops compiling.**
///
/// The defect above was not a typo — it was a file that covered every kind
/// alive when it was written and had no way to notice a new one. The kinds
/// are wire `u8` constants, so no `match` can be exhaustive over them and
/// the compiler cannot ask for the missing arm. This can: `CONT_MAX` is
/// declared as an alias of the newest kind, so raising it breaks this
/// assertion, and whoever raises it has to come here and add the test that
/// proves their kind is drawn from its own store.
///
/// Do not "fix" a failure here by bumping the literal. The failure is the
/// notice.
const _: () = assert!(
    sim_core::inventory::CONT_MAX == CONT_WEAR,
    "a container kind was added without a container_wire test that opens it"
);

/// A body's wear panel carries the *body's* contents, not its backpack's.
///
/// The sixth thing a container view can get wrong, and it is the fifth one
/// again — `cont_slot`'s dispatch — with the failure mode inverted, which
/// is why it earns its own test rather than a line in the one above.
///
/// `CONT_WORLD` fell through an `else` into the wrong store and drew a
/// crate as a deploy box. `CONT_WEAR` would have fallen through
/// `cont_slot`'s `_` arm into `players[slot].inv` — the player's own
/// backpack — and that is worse in the one way that matters here: the
/// wrong store is a *plausible* store. A crate showing a box's contents is
/// visibly nonsense; a wear panel showing your own inventory reads as a
/// UI that opened the wrong tab, and both of its slots are real, occupied
/// and in range. It could have shipped.
///
/// So the fixture makes the two stores **disagree by construction**:
/// `worn[0..2]` holds `OTHER`/`THIRD` and `inv[0..2]` holds `SPEAR`/
/// `FILLER` at the same two indices. Reading the wrong array cannot
/// produce these bytes, and reading the right one cannot produce the
/// other's.
///
/// It makes a second claim nothing else can, and it is the one that would
/// have been a remote panic rather than a wrong picture: the panel is
/// `WEAR_SLOTS` wide. `Player::worn` is a two-element array and
/// `slots_in`'s `_` arm answers `INV_SLOTS`, so a `CONT_WEAR` without its
/// own arm sends the drip walking `cont_slot(.., s, ..)` for `s` in
/// `0..30` across a `[ItemStack; 2]`. That is an out-of-bounds index on
/// the server tick, reachable from one wire field, in the module whose
/// header says every reachable input lands on an announced refusal. The
/// assertion below is `rows.len() <= WEAR_SLOTS`, but the test's real
/// proof of it is that the tick returns at all.
#[test]
fn a_wear_panel_is_drawn_from_the_body_not_the_backpack() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    let mut clients = two_clients(&mut core, &stats);

    let w0 = world_slot(&core, id_of(0));
    // The two stores, deliberately disagreeing at the same indices.
    let worn = [
        ItemStack {
            item: OTHER,
            count: 1,
            cond: 9_100,
        },
        ItemStack {
            item: THIRD,
            count: 1,
            cond: 10_000,
        },
    ];
    core.world.players[w0].worn = worn;
    core.world.players[w0].inv[0] = ItemStack {
        item: SPEAR,
        count: COUNT_A,
        cond: 0,
    };
    core.world.players[w0].inv[1] = ItemStack {
        item: FILLER,
        count: COUNT_B,
        cond: 0,
    };

    // The handle is zero and stays zero: a body has no address, which is
    // the whole of what `inventory::is_own` means on the wire.
    //
    // **Nothing is asked for.** This test opened the body with an
    // `ACT_CONTAINER` until 2026-08-28; there is no open any more, so
    // dressing the player is the whole of the stimulus and the drip is
    // expected to notice. `reset` is false for the same reason — the
    // reset was spent at join, which `two_clients` now asserts — so this
    // is a *diff*, and a diff is only sent when the two stores differ.
    let mut seen = Vec::new();
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }

    let got = wear_syncs(&seen);
    assert_eq!(got.len(), 1, "one change, one payment: {got:?}");
    let (slot, kind, handle, reset, rows) = &got[0];
    assert_eq!((*slot, *kind, *handle, *reset), (0, CONT_WEAR, 0, false));

    // **The claim.** Under `cont_slot`'s `_` arm this was the backpack:
    // `[(0, SPEAR x COUNT_A), (1, FILLER x COUNT_B)]`, a full and
    // believable panel drawn from the wrong array.
    let expect: Vec<(u8, ItemStack)> = worn
        .iter()
        .enumerate()
        .map(|(s, st)| (s as u8, *st))
        .collect();
    assert_eq!(
        rows, &expect,
        "the wear panel must carry `Player::worn`, not `Player::inv`"
    );
    assert!(
        rows.len() <= WEAR_SLOTS,
        "a wear panel is {WEAR_SLOTS} slots wide, got {}: {rows:?}",
        rows.len()
    );

    // The audience restriction holds for the fifth kind too, and for a
    // body it is not a nicety: what someone is wearing is how much damage
    // they will take, so a fanned-out wear panel is the same raid
    // intelligence a box's contents are, read off a person.
    assert!(
        seen.iter()
            .all(|(s, m)| *s == 0
                || !matches!(m, EventMsg::ContSync { kind, .. } if *kind == CONT_WEAR)),
        "nobody else may read what a body is wearing: {seen:?}"
    );

    // And it crossed the ABI, so the claim is about what the client draws.
    //
    // **Into `worn`, and `cont_kind` must not have moved.** The client
    // holds two views now, and this is the assertion that says the wear
    // stream did not arrive through the ground one: nothing was opened
    // here, so a `cont_kind` of `CONT_WEAR` would mean the body had
    // taken the ground subscription again — which is the defect §0eq
    // item 4 names, seen from the other end.
    assert_eq!(
        (clients[0].1.cont_kind, clients[0].1.cont_handle),
        (CONT_SELF, 0),
        "the body must not have taken the ground container's slot"
    );
    let mirrored: Vec<(u8, ItemStack)> = clients[0].1.worn[..WEAR_SLOTS]
        .iter()
        .enumerate()
        .filter(|(_, st)| st.count > 0)
        .map(|(s, st)| (s as u8, *st))
        .collect();
    assert_eq!(
        mirrored, expect,
        "the client's mirror of the body must agree with the store"
    );
}
