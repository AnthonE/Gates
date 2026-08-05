//! The container-contents gate (M1): a player opens a box, is told what
//! is inside it, and is told when it stops being theirs to look at —
//! `ShardCore` ↔ `ClientCore` through real encoded bytes, no sockets.
//!
//! `box_container` proved the sim can move an item into a box.
//! `backpack_wire` proved a bag crosses as a position. Neither could
//! prove the thing both judges ranked first, because until v19 **no
//! container's contents crossed at all** — the panel had nothing to draw
//! for a box or for a bag, and `Command::Loot`'s take-all was the only
//! container verb a client could express.
//!
//! Three things a contents lane gets wrong that a broadcast lane cannot,
//! and every one of them is asserted here against `seen` — the decoded
//! bytes the server actually put on the wire — rather than against a
//! client mirror that can agree with the world for reasons the encoder
//! never earned:
//!
//! 1. **It goes to the opener and to nobody else.** Contents are the one
//!    fact a player has to walk up and open something to learn. An AOI
//!    fan-out would be an ESP feed, and it would read as working.
//! 2. **It stops.** Reach is re-asked every tick, not once when the panel
//!    opened, so walking away closes the box — the same posture the move
//!    verb already takes, through the same two `World` methods.
//! 3. **It is a mirror of the world, not a log of your own moves.** A
//!    second player moving something into the box you have open has to
//!    show up in your panel, and no message exists that says so.

use client_wasm::core::{ClientCore, APPLIED2_CONT};
use protocol::{decode_action, encode_action_open, EventMsg, ItemCatalog};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::backpack::BackpackContent;
use sim_core::build::{foundation_terrain_ok, BuildContent, BUILD_CELL_M, LOC_PLANE};
use sim_core::deploy::{box_key, DeployContent, DeployDef, ARCH_BOX, PLACE_FOUNDATION};
use sim_core::gather::{GatherContent, ItemStack};
use sim_core::inventory::{CONT_BOX, CONT_SELF};
use sim_core::limits::BOX_SLOTS;
use sim_core::movement::Body;
use sim_core::world::Command;

const SEED: u64 = 20_260_805;
/// Row 4 of the deploy fixture is the box, as in `box_container`.
const BOX_ROW: u16 = 4;
const BOX_ITEM: u16 = 6;
const FOUNDATION_ROW: u16 = 0;
/// Loot with no weapon row — what a box is for.
const JUNK: u16 = 7;
const OTHER: u16 = 5;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

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

/// Ring order, so the cell is deterministic for a seed rather than a
/// function of scan direction (`box_container`'s helper).
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

fn world_slot(core: &ShardCore, id: u32) -> usize {
    core.world
        .players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("player in world")
}

/// One tick, both directions, with every event the server emitted
/// recorded as `(slot, decoded)`. The slot is the point: it is what makes
/// "to the opener and to nobody else" an assertion rather than a hope.
fn pump(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    seen: &mut Vec<(usize, EventMsg)>,
) {
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
    let mut snaps: Vec<(usize, Vec<u8>)> = Vec::new();
    let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
    core.tick(stats, |lane, slot, bytes| {
        match lane {
            Lane::Snapshot => snaps.push((slot, bytes.to_vec())),
            Lane::Event => events.push((slot, bytes.to_vec())),
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
    for (slot, bytes) in snaps {
        if let Some(c) = clients.iter_mut().find(|(s, _)| *s == slot).map(|(_, c)| c) {
            c.on_datagram(&bytes);
        }
    }
}

/// Hand the server an open (or close) request the way the net thread
/// would: encoded by the *client's* encoder, decoded by the server's
/// decoder. Nothing here constructs an `ActionMsg` by hand — the point of
/// a wire test is that the bytes are the interface.
fn send_open(core: &mut ShardCore, slot: usize, kind: u8, cont: u32) {
    let mut buf = [0u8; 64];
    let n = encode_action_open(kind, cont, &mut buf).expect("open encodes");
    let act = decode_action(&buf[..n]).expect("open decodes");
    assert!(core.wants_action(slot), "the action hand must be free");
    core.push_action(slot, act);
}

/// Every `ContSync` the server sent to `slot`, in order.
fn syncs_to(seen: &[(usize, EventMsg)], slot: usize) -> Vec<EventMsg> {
    seen.iter()
        .filter(|(s, m)| *s == slot && matches!(m, EventMsg::ContSync { .. }))
        .map(|(_, m)| *m)
        .collect()
}

/// Two connected clients and a box standing on a foundation with both
/// players next to it. Returns the box's packed address.
///
/// Fills a caller-owned `ShardCore` rather than returning one: `World` is
/// ~416 kB of fixed capacity, and handing it back inside a tuple is one
/// more stack copy than a test thread's 2 MB has room for. It overflows,
/// in release, with no hint that size is what did it (`NOW.md` has the
/// same cause under `cargo test --workspace` in debug).
fn armed(stats: &ShardStats, core: &mut ShardCore) -> (Vec<(usize, ClientCore)>, u32) {
    core.world.gather = GatherContent::probe_fixture();
    core.world.build = BuildContent::probe_fixture();
    core.world.deploy = box_fixture();
    core.world.backpack = BackpackContent::probe_fixture();
    core.catalog = ItemCatalog::EMPTY;

    let (cx, cz) = buildable_cell(SEED);
    let (x, z) = cell_center(cx, cz);
    core.world.dev_spawn = Some((x, z));

    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    // Let the join drips settle so nothing below races a catalog or a
    // deploy walk for the tick's send.
    let mut warm = Vec::new();
    for _ in 0..8 {
        pump(core, stats, &mut clients, &mut warm);
    }

    let w0 = world_slot(core, id_of(0));
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
    assert_eq!(core.world.deploys.len(), 1, "the fixture needs its box");

    // Stand both players on the box. Reach is planar, so this is the
    // arrangement that has no bearing to fail on.
    for slot in [0usize, 1] {
        let w = world_slot(core, id_of(slot));
        core.world.players[w].body = Body::at(SEED, x, z);
    }
    (clients, box_key(cx, cz, 0))
}

/// Put `n` of `item` in box slot `s`, straight into the store — this file
/// is about the *lane*, and routing every fixture item through the move
/// verb would make a contents failure look like a move failure.
fn stock(core: &mut ShardCore, s: usize, item: u16, n: u16) {
    core.world
        .deploys
        .set_box_slot(0, s, ItemStack { item, count: n });
}

#[test]
fn opening_a_box_sends_its_contents_to_the_opener_and_to_nobody_else() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    let (mut clients, addr) = armed(&stats, &mut core);
    stock(&mut core, 2, JUNK, 31);
    stock(&mut core, 11, OTHER, 4);

    let mut seen = Vec::new();
    send_open(&mut core, 0, CONT_BOX, addr);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }

    // 1. The wire carried it, to slot 0.
    let mine = syncs_to(&seen, 0);
    assert!(
        !mine.is_empty(),
        "the opener was told nothing — contents never crossed"
    );
    match mine[0] {
        EventMsg::ContSync {
            kind,
            cont,
            reset,
            slots,
            count,
        } => {
            assert_eq!(kind, CONT_BOX, "the first batch names the wrong kind");
            assert_eq!(cont, addr, "the first batch names the wrong box");
            assert!(reset, "the first batch after an open must clear the panel");
            assert_eq!(count, 2, "two stocked slots, two rows");
            let rows: Vec<(u8, u16, u16)> = slots[..count as usize]
                .iter()
                .map(|s| (s.slot, s.stack.item, s.stack.count))
                .collect();
            assert_eq!(
                rows,
                vec![(2, JUNK, 31), (11, OTHER, 4)],
                "the rows are not the box's contents"
            );
        }
        other => panic!("wrong variant {other:?}"),
    }

    // 2. And to NOBODY else. This is the ESP assertion and it is the
    //    reason the lane is per-client at all: a fan-out would satisfy
    //    every other check in this file.
    assert!(
        syncs_to(&seen, 1).is_empty(),
        "a player who opened nothing was told a box's contents"
    );

    // 3. The client landed it where the panel reads from.
    let c = &clients[0].1;
    assert_eq!(c.cont_kind, CONT_BOX, "client did not record the open");
    assert_eq!(c.cont_handle, addr, "client recorded the wrong handle");
    assert_eq!(
        c.cont_slots as usize, BOX_SLOTS,
        "the panel would draw an inventory-sized grid for a box"
    );
    assert_eq!(
        c.cont[2],
        ItemStack {
            item: JUNK,
            count: 31
        }
    );
    assert_eq!(
        c.cont[11],
        ItemStack {
            item: OTHER,
            count: 4
        }
    );
    // The other client's panel stayed shut, in its own state and not just
    // in the bytes it never received.
    assert_eq!(clients[1].1.cont_kind, CONT_SELF);
    assert_eq!(clients[1].1.cont_slots, 0);
}

#[test]
fn another_players_move_into_the_open_box_reaches_the_panel() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    let (mut clients, addr) = armed(&stats, &mut core);
    let mut seen = Vec::new();
    send_open(&mut core, 0, CONT_BOX, addr);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    let before = seen.len();

    // Slot 1 stashes something. No message says "someone else moved" —
    // the panel updates because the mirror is diffed against the world,
    // which is the whole reason it is a shadow and not an event log.
    let w1 = world_slot(&core, id_of(1));
    core.world.players[w1].inv[3] = ItemStack {
        item: JUNK,
        count: 8,
    };
    core.world.tick(&[Command::Move {
        id: id_of(1),
        cont: addr,
        from_kind: CONT_SELF,
        from_slot: 3,
        to_kind: CONT_BOX,
        to_slot: 0,
        count: 8,
    }]);
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }

    let after: Vec<EventMsg> = seen[before..]
        .iter()
        .filter(|(s, m)| *s == 0 && matches!(m, EventMsg::ContSync { .. }))
        .map(|(_, m)| *m)
        .collect();
    assert!(
        !after.is_empty(),
        "the opener was never told about a move made by someone else"
    );
    match after[0] {
        EventMsg::ContSync {
            reset,
            slots,
            count,
            ..
        } => {
            assert!(!reset, "an update is a diff, not a rebuild");
            assert_eq!(count, 1, "one slot changed, one row");
            assert_eq!(slots[0].slot, 0);
            assert_eq!(
                slots[0].stack,
                ItemStack {
                    item: JUNK,
                    count: 8
                }
            );
        }
        other => panic!("wrong variant {other:?}"),
    }
    assert_eq!(
        clients[0].1.cont[0],
        ItemStack {
            item: JUNK,
            count: 8
        },
        "the panel did not see another player's stash"
    );
}

#[test]
fn walking_away_closes_the_box_and_the_lane_goes_quiet() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    let (mut clients, addr) = armed(&stats, &mut core);
    stock(&mut core, 0, JUNK, 12);
    let mut seen = Vec::new();
    send_open(&mut core, 0, CONT_BOX, addr);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    assert_eq!(clients[0].1.cont_kind, CONT_BOX, "it must open first");
    let before = seen.len();

    // Out of reach — far enough that no reach rule could call it near.
    let w0 = world_slot(&core, id_of(0));
    let b = core.world.players[w0].body;
    core.world.players[w0].body = Body::at(SEED, 1500.0, 1500.0);
    assert!(
        (core.world.players[w0].body.qx - b.qx).abs() > 100,
        "the fixture must actually move the player"
    );
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }

    let closed = syncs_to(&seen[before..], 0);
    assert!(
        !closed.is_empty(),
        "the server kept mirroring a box the player walked away from"
    );
    match closed[0] {
        EventMsg::ContSync {
            kind, cont, count, ..
        } => {
            assert_eq!(kind, CONT_SELF, "walking away must CLOSE, not resend");
            assert_eq!(cont, 0);
            assert_eq!(count, 0);
        }
        other => panic!("wrong variant {other:?}"),
    }
    assert_eq!(
        clients[0].1.cont_kind, CONT_SELF,
        "the panel would still be drawing a box across the map"
    );
    assert_eq!(clients[0].1.cont_slots, 0);

    // And it stays quiet: a closed container costs no further bytes, so a
    // player who never opens anything pays nothing for this lane at all.
    let quiet = seen.len();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    assert!(
        syncs_to(&seen[quiet..], 0).is_empty(),
        "a closed container is still sending"
    );
}

#[test]
fn the_close_verb_is_the_open_verb_with_cont_self() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    let (mut clients, addr) = armed(&stats, &mut core);
    stock(&mut core, 0, JUNK, 3);
    let mut seen = Vec::new();
    send_open(&mut core, 0, CONT_BOX, addr);
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    assert_eq!(clients[0].1.cont_kind, CONT_BOX);
    let before = seen.len();

    // The claim the fixture pins in bytes, made here in behaviour: this
    // is the *open* verb, and it closes. Nothing else in the suite would
    // notice a reader that decided CONT_SELF opens a backpack.
    send_open(&mut core, 0, CONT_SELF, 0);
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    let closed = syncs_to(&seen[before..], 0);
    assert!(!closed.is_empty(), "the close was never answered");
    match closed[0] {
        EventMsg::ContSync { kind, count, .. } => {
            assert_eq!(kind, CONT_SELF);
            assert_eq!(count, 0);
        }
        other => panic!("wrong variant {other:?}"),
    }
    assert_eq!(clients[0].1.cont_kind, CONT_SELF);
    assert_eq!(
        core.clients[0].open_kind, CONT_SELF,
        "the server kept the handle after being told to drop it"
    );
}

#[test]
fn opening_a_box_that_is_not_there_answers_closed_rather_than_nothing() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    let (mut clients, addr) = armed(&stats, &mut core);
    let mut seen = Vec::new();

    // A handle that resolves to no box. Silence would leave the panel
    // spinning forever; the answer is the same "closed" a walk-away gets,
    // because from the player's side they are the same fact.
    send_open(&mut core, 0, CONT_BOX, addr ^ 0x0001_0000);
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    let answered = syncs_to(&seen, 0);
    assert!(
        !answered.is_empty(),
        "an impossible open was never answered"
    );
    match answered[0] {
        EventMsg::ContSync { kind, .. } => assert_eq!(kind, CONT_SELF),
        other => panic!("wrong variant {other:?}"),
    }
    assert_eq!(clients[0].1.cont_kind, CONT_SELF);
    assert_eq!(core.clients[0].open_kind, CONT_SELF);
}

#[test]
fn the_contents_flag_is_word_one_and_never_the_error_bit() {
    // The trap `APPLIED2_MOVE` was born from: word 0 is exactly full and
    // bit 31 is the stream-error sentinel, so a flag added there reads as
    // a dead connection. Asserted at the crate that owns the words in
    // `applied_word_is_full_and_bit_31_is_the_error_sentinel`; asserted
    // here through the real lane, because that is where it would bite.
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    let (mut clients, addr) = armed(&stats, &mut core);
    stock(&mut core, 0, JUNK, 1);
    let mut seen = Vec::new();
    send_open(&mut core, 0, CONT_BOX, addr);

    let mut buf = [0u8; 1100];
    let mut saw = false;
    for _ in 0..4 {
        let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
        for (slot, c) in clients.iter_mut() {
            c.advance(1000.0 / 30.0);
            let n = c.poll_input(&mut buf);
            if n > 0 {
                core.push_input(*slot, &protocol::decode_input(&buf[..n]).unwrap());
            }
        }
        core.tick(&stats, |lane, slot, bytes| {
            if lane == Lane::Event {
                events.push((slot, bytes.to_vec()));
            }
            true
        });
        for (slot, bytes) in events {
            let decoded = protocol::decode_event(&bytes).unwrap();
            seen.push((slot, decoded));
            if slot != 0 {
                continue;
            }
            let word0 = clients[0].1.on_stream(&bytes).expect("decodes");
            let word1 = clients[0].1.applied2();
            if matches!(decoded, EventMsg::ContSync { .. }) {
                assert_ne!(
                    word1 & APPLIED2_CONT,
                    0,
                    "a contents message set no word-1 flag — the panel never repaints"
                );
                assert_eq!(
                    word0 & (1 << 31),
                    0,
                    "a contents message read as a stream error"
                );
                saw = true;
            } else {
                assert_eq!(
                    word1 & APPLIED2_CONT,
                    0,
                    "a message that carried no contents claimed to — a stale \
                     verdict an unconditional read cannot distinguish"
                );
            }
        }
    }
    assert!(saw, "no contents message crossed, so nothing was proved");
}
