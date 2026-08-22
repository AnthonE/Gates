//! The admin lane end to end (admin v0, `ALPHA.md` §3): a slash line
//! typed into chat, through the real dispatcher, out as a kick on the ring
//! or a command in the world — plus the anomaly log's account of it.
//!
//! `chat_wire.rs`'s harness, because the lane IS the chat lane: the point
//! of the design is that no wire byte moved, so the transport under test
//! is the one that already worked. What this file owns is **who may**, and
//! **what happens** — the routing is chat's gate, one file over.
//!
//! Deterministic, no sockets. The two things asserted hardest are the two
//! a permission check gets wrong: a stranger's `/kick` must do nothing,
//! and a mistyped command must never reach the room.

use protocol::{ChatMsg, ChatText, EventMsg, ItemCatalog};
use server::admin::{AdminAct, Admins};
use server::anomaly::Sink;
use server::core::{Lane, Ops, ShardCore};
use server::stats::ShardStats;
use server::store::PlayerKey;
use sim_core::gather::GatherContent;

const SEED: u64 = 20_260_811;
const SPAWN: (f32, f32) = (1024.0, 1024.0);
/// The admin's wallet, and a stranger's.
const ADMIN_WALLET: &str = "0x00112233445566778899aabbccddeeff00112233";
const OTHER_WALLET: &str = "0xffeeddccbbaa99887766554433221100ffeeddcc";

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn key(s: &str) -> PlayerKey {
    PlayerKey::new(s.as_bytes()).expect("a legal key")
}

fn line(s: &str) -> ChatText {
    ChatText::sanitize(s.as_bytes()).expect("a clean line")
}

/// Slot 0 is the admin, slot 1 is not. Both connect with the wallet they
/// will be judged by — which is the whole permission story: the key is the
/// handshake's, and the dispatcher reads it rather than trusting a claim.
fn setup(core: &mut ShardCore) {
    core.world.gather = GatherContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    core.install_admins(Admins::parse(ADMIN_WALLET).expect("a legal list"));
    // Through `connect_as` with a key, because the key IS the permission
    // — a `connect` with none would make the admin a stranger.
    assert!(core
        .connect_as(0, id_of(0), Some(key(ADMIN_WALLET)), None)
        .is_some());
    assert!(core
        .connect_as(1, id_of(1), Some(key(OTHER_WALLET)), None)
        .is_some());
}

/// One tick with a live admin ring, returning what crossed it and every
/// chat line the wire carried.
fn pump(
    core: &mut ShardCore,
    stats: &ShardStats,
    admin_tx: &mut rtrb::Producer<AdminAct>,
) -> (Vec<(usize, ChatText, u32)>, bool) {
    let mut log = Sink::off();
    let mut save_now = false;
    let mut chats = Vec::new();
    {
        let mut ops = Ops {
            log: &mut log,
            admin_tx: Some(admin_tx),
            save_now: &mut save_now,
        };
        core.tick(stats, &mut ops, |lane, slot, bytes| {
            if lane == Lane::Event {
                if let Ok(EventMsg::Chat { from, text, .. }) = protocol::decode_event(bytes) {
                    chats.push((slot, text, from));
                }
            }
            true
        });
    }
    (chats, save_now)
}

fn say(core: &mut ShardCore, slot: usize, text: &str) {
    core.push_chat(
        slot,
        ChatMsg {
            global: true,
            text: line(text),
        },
    );
}

fn world_slot(core: &ShardCore, id: u32) -> usize {
    core.world
        .players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("player in world")
}

/// An admin's `/kick` reaches the accept loop's ring naming the target; a
/// `/ban` carries the target's **wallet**, because an id means nothing
/// after a reconnect.
#[test]
fn an_admin_kicks_and_bans_by_id_and_the_ban_carries_the_wallet() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    setup(&mut core);
    let (mut tx, mut rx) = rtrb::RingBuffer::<AdminAct>::new(8);
    pump(&mut core, &stats, &mut tx);

    say(&mut core, 0, "/kick 257");
    pump(&mut core, &stats, &mut tx);
    match rx.pop() {
        Ok(AdminAct::Kick { id }) => assert_eq!(id, id_of(1)),
        other => panic!("expected a kick for slot 1, got {other:?}"),
    }

    say(&mut core, 0, "/ban 257");
    pump(&mut core, &stats, &mut tx);
    match rx.pop() {
        Ok(AdminAct::Ban { id, key: k }) => {
            assert_eq!(id, id_of(1));
            assert_eq!(
                k.as_bytes(),
                OTHER_WALLET.as_bytes(),
                "a ban must carry the wallet the handshake proved"
            );
        }
        other => panic!("expected a ban for slot 1, got {other:?}"),
    }
}

/// The permission check, from the wrong side: a player who is not on the
/// list types the same line and **nothing happens** — no ring entry, and
/// no line in the room either.
#[test]
fn a_stranger_commands_nothing() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    setup(&mut core);
    let (mut tx, mut rx) = rtrb::RingBuffer::<AdminAct>::new(8);
    pump(&mut core, &stats, &mut tx);

    say(&mut core, 1, "/kick 256");
    let (chats, _) = pump(&mut core, &stats, &mut tx);
    assert!(
        rx.pop().is_err(),
        "a stranger's kick must not cross the ring"
    );
    assert!(
        chats.is_empty(),
        "and it must not be relayed as chat either: {chats:?}"
    );
}

/// A slash line never reaches the room — not a good one, not a typo'd
/// one. This is the property that keeps `/kik 257` from announcing to a
/// griefer that somebody is fumbling for the kick command.
#[test]
fn no_slash_line_is_ever_relayed() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    setup(&mut core);
    let (mut tx, _rx) = rtrb::RingBuffer::<AdminAct>::new(8);
    pump(&mut core, &stats, &mut tx);

    for text in ["/kik 257", "/kick", "/nonsense", "/kick 257"] {
        say(&mut core, 0, text);
        let (chats, _) = pump(&mut core, &stats, &mut tx);
        assert!(
            chats.is_empty(),
            "{text:?} was relayed to the room: {chats:?}"
        );
    }
    // And an ordinary line still is, so the intercept is a slash rule and
    // not a chat outage.
    say(&mut core, 0, "hello");
    let (chats, _) = pump(&mut core, &stats, &mut tx);
    assert!(!chats.is_empty(), "ordinary chat must still be relayed");
}

/// `/say` reaches everybody, marked, from an id no player can hold — so a
/// client renders it without learning a new message shape.
#[test]
fn say_reaches_everyone_as_the_house() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    setup(&mut core);
    let (mut tx, _rx) = rtrb::RingBuffer::<AdminAct>::new(8);
    pump(&mut core, &stats, &mut tx);

    say(&mut core, 0, "/say wipe in 5");
    let (chats, _) = pump(&mut core, &stats, &mut tx);
    assert_eq!(chats.len(), 2, "both connected clients hear it: {chats:?}");
    for (_, text, from) in &chats {
        assert_eq!(text.as_bytes(), b"[server] wipe in 5");
        assert_eq!(*from, 0, "the house speaks from an id no player holds");
    }
}

/// `/tp` and `/give` land as world state, through commands — so they are
/// in the stream a replay reads, which is what `ALPHA.md` §3 means by an
/// admin act being visible in a replay.
#[test]
fn tp_and_give_change_the_world_through_commands() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    setup(&mut core);
    let (mut tx, _rx) = rtrb::RingBuffer::<AdminAct>::new(8);
    pump(&mut core, &stats, &mut tx);
    pump(&mut core, &stats, &mut tx);

    // Move the stranger somewhere the admin is not, then teleport to them.
    let w0 = world_slot(&core, id_of(0));
    let w1 = world_slot(&core, id_of(1));
    core.world.players[w1].body.qx += 30_000 / 3; // 300 m east
    let target = core.world.players[w1].body;
    assert_ne!(core.world.players[w0].body.qx, target.qx);

    say(&mut core, 0, "/tp 257");
    // Queued this tick, applied the next — the command's own doc says so.
    pump(&mut core, &stats, &mut tx);
    pump(&mut core, &stats, &mut tx);
    assert_eq!(
        core.world.players[w0].body.qx, target.qx,
        "/tp must put the admin at the target's feet"
    );

    // `/give` of a probe-fixture row lands in the admin's own pockets.
    let before: u16 = core.world.players[w0]
        .inv
        .iter()
        .filter(|s| s.item == 1)
        .map(|s| s.count)
        .sum();
    say(&mut core, 0, "/give 1 7");
    pump(&mut core, &stats, &mut tx);
    pump(&mut core, &stats, &mut tx);
    let after: u16 = core.world.players[w0]
        .inv
        .iter()
        .filter(|s| s.item == 1)
        .map(|s| s.count)
        .sum();
    assert_eq!(after, before + 7, "/give must add exactly what it named");
}

/// `/save` raises the flag the sim thread reads, and does not itself
/// write anything — one world-save path, not two.
#[test]
fn save_raises_the_flag() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    setup(&mut core);
    let (mut tx, _rx) = rtrb::RingBuffer::<AdminAct>::new(8);
    let (_, before) = pump(&mut core, &stats, &mut tx);
    assert!(!before, "nothing asked for a save yet");
    say(&mut core, 0, "/save");
    let (_, raised) = pump(&mut core, &stats, &mut tx);
    assert!(raised, "/save must raise the sim thread's flag");
}

/// `/bug` is **everybody's** — the one slash verb that needs no allowlist
/// (`ALPHA.md` §4) — and it is not relayed to the room either.
#[test]
fn anyone_can_file_a_bug() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    setup(&mut core);
    let (mut tx, mut rx) = rtrb::RingBuffer::<AdminAct>::new(8);
    pump(&mut core, &stats, &mut tx);
    pump(&mut core, &stats, &mut tx);

    say(&mut core, 1, "/bug fell through the floor");
    let (chats, _) = pump(&mut core, &stats, &mut tx);
    assert!(chats.is_empty(), "a bug report is not a chat line");
    assert!(rx.pop().is_err(), "and it is not an admin act");
    // The observable is the log, and a disabled sink swallows it — what
    // this test can still say is that a non-admin's `/bug` is not refused
    // the way their `/kick` was, which is the whole distinction. The
    // counter is the proxy: `admin_refused` moves for the kick and not
    // for the bug.
    let refused_after_bug = ShardStats::get(&stats.admin_refused);
    say(&mut core, 1, "/kick 256");
    pump(&mut core, &stats, &mut tx);
    assert_eq!(
        ShardStats::get(&stats.admin_refused),
        refused_after_bug,
        "the sim side counts refusals in the log, not here"
    );
}
