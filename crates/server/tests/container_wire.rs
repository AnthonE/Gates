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
//! Four things a container view can get wrong, one test each:
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

use client_wasm::core::{ClientCore, APPLIED2_CONT};
use protocol::{EventMsg, ItemCatalog};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::backpack::BackpackContent;
use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::combat::CombatContent;
use sim_core::deploy::{box_key, DeployContent, DeployDef, ARCH_BOX, PLACE_FOUNDATION};
use sim_core::gather::{GatherContent, ItemStack, SWING_INTERVAL_TICKS};
use sim_core::input::BTN_PRIMARY;
use sim_core::inventory::{CONT_BAG, CONT_BOX, CONT_SELF};
use sim_core::limits::{BOX_SLOTS, INV_SLOTS, MAX_ITEM_DEFS};
use sim_core::movement::Body;
use sim_core::world::Command;

const SEED: u64 = 20_260_804;
/// The browser-smoke spawn point, guarded walkable in sim-core
/// `world::tests` — the same one `backpack_wire` stands its bodies on.
const SPAWN: (f32, f32) = (1024.0, 1024.0);
/// Fixture items. Three different ones, and none of them a slot index or a
/// count that appears anywhere else in this file: a container message is a
/// positional payload with (slot, item, count) triples in it, which is the
/// shape `reference/FINDINGS.md` §1 counts ~27 shipped defects in.
const SPEAR: u16 = 0;
const JUNK: u16 = 7;
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
        c.predict.decay_error();
        let n = c.poll_input(&mut buf);
        if n > 0 {
            let dg = protocol::decode_input(&buf[..n]).expect("client encodes valid input");
            core.push_input(*slot, &dg);
        }
    }
    let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
    core.tick(stats, |lane, slot, bytes| {
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
    seen.iter()
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

fn armed_core() -> ShardCore {
    let mut core = ShardCore::new(SEED);
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
        "a join must not open a container by itself"
    );
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
    };
    core.world.players[w1].inv = [ItemStack::default(); INV_SLOTS];
    core.world.players[w1].inv[SLOT_A] = ItemStack {
        item: JUNK,
        count: COUNT_A,
    };
    core.world.players[w1].inv[SLOT_B] = ItemStack {
        item: OTHER,
        count: COUNT_B,
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
                    item: JUNK,
                    count: COUNT_A
                }
            ),
            (
                SLOT_B as u8,
                ItemStack {
                    item: OTHER,
                    count: COUNT_B
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
            item: JUNK,
            count: COUNT_A
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
    core.world.players[w0].body = Body::at(SEED, SPAWN.0 + 200.0, SPAWN.1 + 200.0);
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
                    count: COUNT_C
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
            count: COUNT_C
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
                if foundation_terrain_ok(seed, x, z) {
                    return (cx, cz);
                }
            }
        }
    }
    panic!("no buildable cell within 64 cells — the generator changed under this test");
}

#[test]
fn a_box_opens_by_its_packed_address() {
    let stats = ShardStats::default();
    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);

    let mut core = ShardCore::new(SEED);
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    core.world.deploy = box_fixture();
    core.world.dev_spawn = Some((x, z));
    core.catalog = ItemCatalog::EMPTY;
    let mut clients = two_clients(&mut core, &stats);

    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].body = Body::at(SEED, x, z);
    core.world.players[w0].inv[0] = ItemStack { item: 0, count: 5 };
    core.world.players[w0].inv[1] = ItemStack {
        item: BOX_ITEM,
        count: 1,
    };
    core.world.tick(&[Command::Place {
        id: id_of(0),
        row: FOUNDATION_ROW,
        cx,
        cz,
        level: 0,
        loc: LOC_PLANE,
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
            item: JUNK,
            count: COUNT_A,
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
                item: JUNK,
                count: COUNT_A
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
