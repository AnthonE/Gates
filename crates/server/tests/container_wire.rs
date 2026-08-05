//! `container_wire` — the gate for the one rule the container lane has:
//! **a container's contents go to the client that opened it and to
//! nobody else.**
//!
//! Everything else on this lane is a diff and a handle. This is the part
//! that can be wrong in a way no other gate can see, and it can be wrong
//! in two opposite directions:
//!
//! 1. **Too wide.** A sync fanned out over AOI is an ESP feed — every
//!    stash within 176 m readable by anyone standing near it, with no
//!    wall to notice. `test_snapshot_budget` measures bytes and would
//!    call that a rounding error; `test_protocol_golden` pins the
//!    encoder's bytes and an audience is not the encoder; `test_replay`
//!    hashes the world and who was *told* is not in the world. So the
//!    audience is asserted here, negatively: the non-opener's slot must
//!    receive **zero** `ContSync`, and the assertion counts messages
//!    rather than trusting a client mirror to stay empty.
//!
//! 2. **Too long.** A subscription that outlives its authorisation is the
//!    same leak one tick later — walk away, or break the box, and a panel
//!    still being fed is a panel reading a container the sim would refuse
//!    every move into. `inventory.rs` states the law it must obey ("reach
//!    is checked at the moment the move resolves, not at the moment a
//!    panel opened"), and the read side keeps it by being re-earned every
//!    tick it is served. Two tests below walk a client out of reach and
//!    take a container out of the world, and both must close.
//!
//! And one thing that must NOT happen, which is the whole reason this
//! lane is written the way it is: **nothing here may end the session.**
//! `CLAUDE.md`'s trap list has the reference shipping three container
//! fixes in twenty-eight minutes, all landing as the server dropping the
//! client, because a request computed against a container the server does
//! not agree about reads as a forged one. So a handle naming no container
//! at all is a `cont_closed`, not a disconnect, and that is asserted.
//!
//! Structural asserts against the decoded bytes the server actually put
//! on the lane — the `backpack_wire` shape — never against a client
//! mirror alone, because a client can agree with the world for reasons
//! the encoder never earned.

use client_wasm::core::{ClientCore, APPLIED2_CONT};
use protocol::{encode_action_open, EventMsg, ItemCatalog};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::backpack::BackpackContent;
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::inventory::{CLOSE_ASKED, CLOSE_GONE, CLOSE_REACH, CONT_BAG, CONT_SELF};
use sim_core::limits::INV_SLOTS;
use sim_core::movement::POS_XZ_Q;

const SEED: u64 = 20_260_805;
/// The browser-smoke spawn point, guarded walkable in sim-core
/// `world::tests` — the same one every other wire test stands on.
const SPAWN: (f32, f32) = (1024.0, 1024.0);
/// Two fixture items with different ids and different counts, so a
/// (item, count) transposition inside a slot record moves the assertion
/// as well as the bytes.
const JUNK_A: (u16, u16) = (7, 42);
const JUNK_B: (u16, u16) = (3, 5);

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn armed_core() -> ShardCore {
    let mut core = ShardCore::new(SEED);
    core.world.gather = GatherContent::probe_fixture();
    core.world.backpack = BackpackContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    core
}

/// One tick, collecting every event byte the server emitted and to which
/// connection slot. Events are decoded from the wire and also fed to the
/// matching client, so a claim can be made against either.
fn pump(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    seen: &mut Vec<(usize, EventMsg)>,
) {
    let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
    core.tick(stats, |lane, slot, bytes| {
        if let Lane::Event = lane {
            events.push((slot, bytes.to_vec()));
        }
        true
    });
    for (slot, bytes) in events {
        seen.push((
            slot,
            protocol::decode_event(&bytes).expect("server events decode"),
        ));
        if let Some(c) = clients.iter_mut().find(|(s, _)| *s == slot).map(|(_, c)| c) {
            c.on_stream(&bytes).expect("server events decode");
        }
    }
}

fn world_slot(core: &ShardCore, id: u32) -> usize {
    core.world
        .players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("player in world")
}

/// Send an open (or, with `CONT_SELF`, a close) the way a browser would:
/// through the real encoder and the real decoder, never by poking server
/// state. A test that set `open_kind` directly would be asserting on a
/// field rather than on a verb.
fn open(core: &mut ShardCore, slot: usize, kind: u8, cont: u32) {
    let mut buf = [0u8; 64];
    let n = encode_action_open(kind, cont, &mut buf).expect("open encodes");
    let act = protocol::decode_action(&buf[..n]).expect("open decodes");
    core.push_action(slot, act);
}

/// Stand a bag on the ground at the player's feet, holding two items.
/// Returns its id.
fn bag_at_feet(core: &mut ShardCore, id: u32) -> u32 {
    let w = world_slot(core, id);
    let b = core.world.players[w].body;
    let mut items = [ItemStack::default(); INV_SLOTS];
    items[0] = ItemStack {
        item: JUNK_A.0,
        count: JUNK_A.1,
    };
    items[4] = ItemStack {
        item: JUNK_B.0,
        count: JUNK_B.1,
    };
    let tick = core.world.tick;
    let w = &mut core.world;
    w.backpacks
        .stand_up(
            &w.backpack,
            b.qx,
            b.qy,
            b.qz,
            id,
            &items,
            tick,
            &mut w.events,
        )
        .expect("the fixture ladder is armed and the items are not empty")
}

/// One `ContSync` as this file wants to assert on it: the handle it names,
/// whether it is a whole container or a diff, and the slot records that
/// actually crossed.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Sync {
    kind: u8,
    cont: u32,
    reset: bool,
    slots: Vec<(u8, ItemStack)>,
}

/// Every `ContSync` the server sent to `slot`.
fn syncs(seen: &[(usize, EventMsg)], slot: usize) -> Vec<Sync> {
    seen.iter()
        .filter(|(s, _)| *s == slot)
        .filter_map(|(_, m)| match m {
            EventMsg::ContSync {
                kind,
                cont,
                reset,
                slots,
                count,
            } => Some(Sync {
                kind: *kind,
                cont: *cont,
                reset: *reset,
                slots: slots
                    .iter()
                    .take(*count as usize)
                    .map(|s| (s.slot, s.stack))
                    .collect(),
            }),
            _ => None,
        })
        .collect()
}

/// Every `ContClosed` reason the server sent to `slot`.
fn closes(seen: &[(usize, EventMsg)], slot: usize) -> Vec<u8> {
    seen.iter()
        .filter(|(s, _)| *s == slot)
        .filter_map(|(_, m)| match m {
            EventMsg::ContClosed { why } => Some(*why),
            _ => None,
        })
        .collect()
}

/// Two connected clients standing on the same spot, joins drained.
fn two_clients(core: &mut ShardCore, stats: &ShardStats) -> Vec<(usize, ClientCore)> {
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    // Let the join drips settle: the catalog, recipe and sync walks each
    // take a tick, and a container assertion racing one of them would be
    // asserting on the drip order rather than on the container.
    let mut warm = Vec::new();
    for _ in 0..24 {
        pump(core, stats, &mut clients, &mut warm);
    }
    assert!(
        syncs(&warm, 0).is_empty() && syncs(&warm, 1).is_empty(),
        "nobody has opened anything, so no contents may have crossed"
    );
    clients
}

#[test]
fn the_contents_reach_the_opener_and_only_the_opener() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    let mut clients = two_clients(&mut core, &stats);
    let bag = bag_at_feet(&mut core, id_of(0));

    // Both players are standing on the bag — slot 1 is every bit as much
    // "in reach" as slot 0, which is what makes this a test of audience
    // rather than a test of distance. Only slot 0 opens it.
    let w1 = world_slot(&core, id_of(1));
    let w0 = world_slot(&core, id_of(0));
    core.world.players[w1].body = core.world.players[w0].body;
    assert!(
        core.world.cont_view(id_of(1), CONT_BAG, bag).is_ok(),
        "the non-opener must be standing close enough to be allowed it — \
         otherwise this test would pass on distance and prove nothing"
    );

    open(&mut core, 0, CONT_BAG, bag);
    let mut seen = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }

    // The opener got the container, whole, with the reset bit and both
    // items in the slots they actually occupy.
    let got = syncs(&seen, 0);
    assert!(!got.is_empty(), "the opener must receive its container");
    let got = got[0].clone();
    assert_eq!(
        (got.kind, got.cont),
        (CONT_BAG, bag),
        "the handle must be echoed"
    );
    assert!(
        got.reset,
        "the first sync of a container is the whole of it"
    );
    let slots = got.slots;
    assert_eq!(
        slots.len(),
        INV_SLOTS,
        "a reset carries every slot, including the empty ones — a client \
         cannot tell 'empty' from 'unmentioned' otherwise"
    );
    assert_eq!(
        slots[0].1,
        ItemStack {
            item: JUNK_A.0,
            count: JUNK_A.1
        },
        "slot 0's stack, by value, in slot 0's position"
    );
    assert_eq!(
        slots[4].1,
        ItemStack {
            item: JUNK_B.0,
            count: JUNK_B.1
        },
        "slot 4's stack — a different item AND a different count from \
         slot 0's, so a swapped pair cannot pass"
    );

    // The wall. Not "slot 1's mirror is empty" — slot 1 was sent nothing.
    assert_eq!(
        syncs(&seen, 1).len(),
        0,
        "a container's contents went to a client that did not open it: \
         this is the ESP leak the lane exists to not have"
    );
    assert_eq!(
        clients[1].1.cont_kind, CONT_SELF,
        "and the non-opener therefore has no container to draw"
    );
    assert_eq!(
        clients[0].1.cont_handle, bag,
        "the opener's mirror names the bag it asked for"
    );
}

#[test]
fn an_unchanged_container_sends_nothing_and_a_change_sends_one_slot() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    let mut clients = two_clients(&mut core, &stats);
    let bag = bag_at_feet(&mut core, id_of(0));
    open(&mut core, 0, CONT_BAG, bag);

    let mut opening = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut opening);
    }
    assert_eq!(
        syncs(&opening, 0).len(),
        1,
        "the container is sent once, not once per tick — a panel open for \
         a minute must not cost 1800 messages"
    );

    // Idle: the container has not moved, so neither has the wire.
    let mut idle = Vec::new();
    for _ in 0..5 {
        pump(&mut core, &stats, &mut clients, &mut idle);
    }
    assert_eq!(
        syncs(&idle, 0).len(),
        0,
        "an unchanged container sends nothing"
    );

    // Change one slot in the sim. Only that slot may cross.
    let i = core.world.backpacks.index_of_id(bag).expect("bag is live");
    core.world.backpacks.set_slot(
        i,
        4,
        ItemStack {
            item: JUNK_B.0,
            count: 1,
        },
    );
    let mut changed = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut changed);
    }
    let got = syncs(&changed, 0);
    assert_eq!(got.len(), 1, "one change, one message");
    let got = got[0].clone();
    assert!(!got.reset, "a diff is not a reset");
    assert_eq!(
        got.slots,
        vec![(
            4u8,
            ItemStack {
                item: JUNK_B.0,
                count: 1
            }
        )],
        "exactly the slot that moved, and nothing else"
    );
    assert_eq!(
        clients[0].1.cont[4],
        ItemStack {
            item: JUNK_B.0,
            count: 1
        },
        "and the client applied it to the slot the record named"
    );
    assert_eq!(
        clients[0].1.cont[0],
        ItemStack {
            item: JUNK_A.0,
            count: JUNK_A.1
        },
        "a diff must not disturb the slots it did not mention"
    );
}

#[test]
fn walking_away_closes_the_panel() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    let mut clients = two_clients(&mut core, &stats);
    let bag = bag_at_feet(&mut core, id_of(0));
    open(&mut core, 0, CONT_BAG, bag);

    let mut seen = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    assert_eq!(syncs(&seen, 0).len(), 1, "open first, so the close is real");

    // Teleport the opener 100 m east — far past any reach a container
    // has. Reach is the sim's to define and this test does not restate
    // it; it asserts the panel closes on whatever the sim refuses, which
    // is checked directly first so a widened reach cannot silently turn
    // this test into a no-op.
    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].body.qx += (100.0 / POS_XZ_Q) as i32;
    assert_eq!(
        core.world.cont_view(id_of(0), CONT_BAG, bag),
        Err(CLOSE_REACH),
        "the arrangement must actually be out of reach"
    );

    let mut after = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut after);
    }
    assert_eq!(
        closes(&after, 0),
        vec![CLOSE_REACH],
        "walking away closes the panel, once, and says why"
    );
    assert_eq!(
        syncs(&after, 0).len(),
        0,
        "and no contents crossed on the way out"
    );
    assert_eq!(
        clients[0].1.cont_kind, CONT_SELF,
        "the client dropped the container"
    );
    assert_eq!(
        clients[0].1.cont[0],
        ItemStack::default(),
        "and its contents with it — a mirror that outlives the \
         subscription is the same leak, one tick late"
    );
    assert_eq!(clients[0].1.cont_close_why, CLOSE_REACH);

    // It stays closed: the server must not keep re-testing a
    // subscription it has already given up, or an out-of-reach panel
    // would emit one close per tick forever.
    let mut later = Vec::new();
    for _ in 0..5 {
        pump(&mut core, &stats, &mut clients, &mut later);
    }
    assert_eq!(
        closes(&later, 0),
        Vec::<u8>::new(),
        "closed once, not once a tick"
    );
}

#[test]
fn a_container_that_leaves_the_world_closes_the_panel() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    let mut clients = two_clients(&mut core, &stats);
    let bag = bag_at_feet(&mut core, id_of(0));
    open(&mut core, 0, CONT_BAG, bag);

    let mut seen = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    assert_eq!(syncs(&seen, 0).len(), 1);

    // Empty it the way a withdrawal does, which is the route that
    // actually retires a bag (`world::move_item` → `drop_if_empty`).
    let i = core.world.backpacks.index_of_id(bag).expect("bag is live");
    for s in 0..INV_SLOTS {
        core.world.backpacks.set_slot(i, s, ItemStack::default());
    }
    let w = &mut core.world;
    w.backpacks.drop_if_empty(i, &mut w.events);
    assert!(
        core.world.backpacks.index_of_id(bag).is_none(),
        "the bag is off the store"
    );

    let mut after = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut after);
    }
    assert_eq!(
        closes(&after, 0),
        vec![CLOSE_GONE],
        "a container that left the world closes the panel as GONE, not \
         REACH — 'it is gone' and 'walk back' are different sentences"
    );
    assert_eq!(clients[0].1.cont_kind, CONT_SELF);
}

#[test]
fn closing_is_a_verb_and_a_handle_naming_nothing_is_answered_not_punished() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    let mut clients = two_clients(&mut core, &stats);
    let bag = bag_at_feet(&mut core, id_of(0));

    open(&mut core, 0, CONT_BAG, bag);
    let mut seen = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    assert_eq!(syncs(&seen, 0).len(), 1);

    // CONT_SELF is the close. Your own inventory is never an open target,
    // so kind zero on this action is the one value that could only have
    // been a refusal — spent as the close instead.
    open(&mut core, 0, CONT_SELF, 0);
    let mut closed = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut closed);
    }
    assert_eq!(
        closes(&closed, 0),
        vec![CLOSE_ASKED],
        "the player asked, and is told — one close path, not two"
    );
    assert_eq!(clients[0].1.cont_kind, CONT_SELF);

    // A handle naming no container at all. This is the shape that the
    // reference shipped three times as a disconnect (`CLAUDE.md`): the
    // server disagrees about a container, so the request reads as forged.
    // Here it is a message.
    let ghost = bag.wrapping_add(9999);
    assert!(core.world.backpacks.index_of_id(ghost).is_none());
    open(&mut core, 0, CONT_BAG, ghost);
    let mut ghosted = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut ghosted);
    }
    assert_eq!(
        closes(&ghosted, 0),
        vec![CLOSE_GONE],
        "a container that never existed is answered, not punished"
    );
    assert!(
        core.clients[0].connected,
        "and above all the session survives — the failure mode this whole \
         lane is written to never have"
    );
    assert_eq!(
        syncs(&ghosted, 0).len(),
        0,
        "and nothing was served for a container that is not there"
    );
}

#[test]
fn reopening_the_same_container_resends_it_whole() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    let mut clients = two_clients(&mut core, &stats);
    let bag = bag_at_feet(&mut core, id_of(0));

    open(&mut core, 0, CONT_BAG, bag);
    let mut first = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut first);
    }
    assert_eq!(syncs(&first, 0).len(), 1);

    open(&mut core, 0, CONT_SELF, 0);
    let mut shut = Vec::new();
    for _ in 0..2 {
        pump(&mut core, &stats, &mut clients, &mut shut);
    }
    assert_eq!(closes(&shut, 0), vec![CLOSE_ASKED]);

    // Something happens to the bag while nobody is looking.
    let i = core.world.backpacks.index_of_id(bag).expect("bag is live");
    core.world.backpacks.set_slot(i, 0, ItemStack::default());

    open(&mut core, 0, CONT_BAG, bag);
    let mut second = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut second);
    }
    let got = syncs(&second, 0);
    assert_eq!(got.len(), 1);
    assert!(
        got[0].reset,
        "a reopen is a reset: the shadow went with the close, so a diff \
         against it would be a diff against nothing"
    );
    assert_eq!(
        clients[0].1.cont[0],
        ItemStack::default(),
        "and the stack that vanished while the panel was shut is not \
         still drawn"
    );
}

#[test]
fn the_applied_flag_announces_both_halves() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    let mut clients = two_clients(&mut core, &stats);
    let bag = bag_at_feet(&mut core, id_of(0));

    // `APPLIED2_CONT` is the panel's only wake-up: word 0 has no bit left
    // (`core.rs`), so a container change that did not raise it would be
    // invisible to a client that is not polling. Both halves must raise
    // it — the sync and the close.
    open(&mut core, 0, CONT_BAG, bag);
    let mut sync_flags = 0u32;
    for _ in 0..3 {
        let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
        core.tick(&stats, |lane, slot, bytes| {
            if let Lane::Event = lane {
                events.push((slot, bytes.to_vec()));
            }
            true
        });
        for (slot, bytes) in events {
            if slot == 0 {
                clients[0].1.on_stream(&bytes).expect("decodes");
                sync_flags |= clients[0].1.applied2();
            } else {
                clients[1].1.on_stream(&bytes).expect("decodes");
            }
        }
    }
    assert!(
        sync_flags & APPLIED2_CONT != 0,
        "a contents sync must announce itself in word 1"
    );

    open(&mut core, 0, CONT_SELF, 0);
    let mut close_flags = 0u32;
    for _ in 0..3 {
        let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
        core.tick(&stats, |lane, slot, bytes| {
            if let Lane::Event = lane {
                events.push((slot, bytes.to_vec()));
            }
            true
        });
        for (slot, bytes) in events {
            if slot == 0 {
                clients[0].1.on_stream(&bytes).expect("decodes");
                close_flags |= clients[0].1.applied2();
            } else {
                clients[1].1.on_stream(&bytes).expect("decodes");
            }
        }
    }
    assert!(
        close_flags & APPLIED2_CONT != 0,
        "and so must a close — a panel that is never told it shut is a \
         panel still drawing a container it may not read"
    );
}
