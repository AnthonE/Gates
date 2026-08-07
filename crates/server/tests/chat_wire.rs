//! The chat-on-the-wire gate (M1, ALPHA.md §1): `ShardCore` ↔
//! `ClientCore` through real encoded bytes — a local line reaches the
//! speaker and their neighbours and stops at 20 m, a global line reaches
//! everyone, the speaker's own line comes back as the delivery receipt,
//! each recipient gets it exactly once, and none of it moves the state
//! hash, because chat is not sim state.
//!
//! Deterministic, no sockets; asserts are structural and exact (the
//! deploy_wire shape). The wire's own refusals (bad UTF-8, control
//! characters, a forged length, a non-canonical line) are the protocol
//! crate's gate; what this file owns is the *routing*.

use client_wasm::core::{ClientCore, APPLIED_CHAT};
use protocol::{ChatMsg, ChatText, EventMsg, ItemCatalog};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::gather::GatherContent;
use sim_core::limits::CHAT_LOCAL_CM;

const SEED: u64 = 20_260_731;
/// The canonical dev spawn point, guarded walkable in sim-core
/// `world::tests`. `dev_spawn` pins every join here, so the test starts
/// with everyone in one place and moves them apart deliberately.
const SPAWN: (f32, f32) = (1024.0, 1024.0);
/// Position quantum in centimeters (DESIGN.md §5.5: 3 cm x/z).
const Q_CM: i32 = 3;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

/// One lockstep pump. Returns per-slot APPLIED flags and appends every
/// event message the server actually put on the lane to `seen`, decoded
/// from those bytes — routing is a claim about what the *wire* carried,
/// so it gets asserted there and not only on the client mirrors.
fn pump(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    seen: &mut Vec<(usize, EventMsg)>,
) -> [u32; 3] {
    let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
    core.tick(stats, |lane, slot, bytes| {
        if lane == Lane::Event {
            events.push((slot, bytes.to_vec()));
        }
        true
    });
    let mut flags = [0u32; 3];
    for (slot, bytes) in events {
        seen.push((
            slot,
            protocol::decode_event(&bytes).expect("server events decode"),
        ));
        if let Some(c) = clients.iter_mut().find(|(s, _)| *s == slot).map(|(_, c)| c) {
            flags[slot] |= c.on_stream(&bytes).expect("server events decode");
        }
    }
    flags
}

fn world_slot(core: &ShardCore, id: u32) -> usize {
    core.world
        .players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("player in world")
}

fn line(s: &str) -> ChatText {
    ChatText::sanitize(s.as_bytes()).expect("a clean line")
}

/// Chat lines this tick's events carried to `slot`, as (speaker, global,
/// text) — the wire's own account, not a client's.
fn chats_to(seen: &[(usize, EventMsg)], slot: usize) -> Vec<(u32, bool, ChatText)> {
    seen.iter()
        .filter(|(s, _)| *s == slot)
        .filter_map(|(_, msg)| match msg {
            EventMsg::Chat { from, global, text } => Some((*from, *global, *text)),
            _ => None,
        })
        .collect()
}

/// Three clients on one shard, joined and pinned to the same spawn.
/// Takes the core by reference: `ShardCore` is far too big to hand back
/// by value in a debug build, which is a stack overflow, not a style
/// note.
fn setup(core: &mut ShardCore) -> Vec<(usize, ClientCore)> {
    core.world.gather = GatherContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    for slot in 0..3 {
        assert!(core.connect(slot, id_of(slot)));
    }
    (0..3)
        .map(|slot| (slot, ClientCore::new(SEED, id_of(slot), 0)))
        .collect()
}

#[test]
fn chat_rides_the_wire() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    let mut clients = setup(&mut core);
    let mut seen = Vec::new();
    // Land the joins so every speaker has a position to measure from.
    pump(&mut core, &stats, &mut clients, &mut seen);
    pump(&mut core, &stats, &mut clients, &mut seen);

    // Slot 2 walks out of earshot, far past the 20 m radius.
    let w2 = world_slot(&core, id_of(2));
    core.world.players[w2].body.qx += 10_000 / Q_CM; // 100 m east

    // --- local: the speaker and the neighbour, never the distant one ---
    seen.clear();
    let hello = line("anyone got stone?");
    core.push_chat(
        0,
        ChatMsg {
            global: false,
            text: hello,
        },
    );
    let flags = pump(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(
        chats_to(&seen, 0),
        vec![(id_of(0), false, hello)],
        "the speaker must hear their own line exactly once — the echo is \
         the delivery receipt"
    );
    assert_eq!(
        chats_to(&seen, 1),
        vec![(id_of(0), false, hello)],
        "the neighbour at the same spawn must hear it exactly once"
    );
    assert!(
        chats_to(&seen, 2).is_empty(),
        "100 m away is outside the 20 m local radius"
    );
    assert_ne!(flags[0] & APPLIED_CHAT, 0, "speaker's client missed it");
    assert_ne!(flags[1] & APPLIED_CHAT, 0, "neighbour's client missed it");
    assert_eq!(flags[2] & APPLIED_CHAT, 0, "distant client heard it");
    assert_eq!(
        clients[1].1.pop_chat(),
        Some((id_of(0), false, hello)),
        "the client mirror must carry the same bytes the wire did"
    );
    assert_eq!(clients[2].1.pop_chat(), None);

    // --- global: everyone, distance irrelevant ---
    seen.clear();
    let shout = line("server wipes at 6");
    core.push_chat(
        2,
        ChatMsg {
            global: true,
            text: shout,
        },
    );
    pump(&mut core, &stats, &mut clients, &mut seen);
    for slot in 0..3 {
        assert_eq!(
            chats_to(&seen, slot),
            vec![(id_of(2), true, shout)],
            "global line missed slot {slot} (or doubled)"
        );
    }

    // --- one line per tick, and nothing lingers ---
    seen.clear();
    pump(&mut core, &stats, &mut clients, &mut seen);
    for slot in 0..3 {
        assert!(
            chats_to(&seen, slot).is_empty(),
            "a said line repeated on the next tick to slot {slot}"
        );
    }
}

/// The local radius is a hard edge at `CHAT_LOCAL_CM`, inclusive — worth
/// pinning, because "roughly 20 m" is how a radius becomes 24 m.
#[test]
fn the_local_radius_is_exact() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    let mut clients = setup(&mut core);
    let mut seen = Vec::new();
    pump(&mut core, &stats, &mut clients, &mut seen);
    pump(&mut core, &stats, &mut clients, &mut seen);

    let w0 = world_slot(&core, id_of(0));
    let w1 = world_slot(&core, id_of(1));
    let base = core.world.players[w0].body.qx;

    // Exactly on the radius: heard.
    core.world.players[w1].body.qx = base + (CHAT_LOCAL_CM as i32) / Q_CM;
    seen.clear();
    let at = line("on the line");
    core.push_chat(
        0,
        ChatMsg {
            global: false,
            text: at,
        },
    );
    pump(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(
        chats_to(&seen, 1),
        vec![(id_of(0), false, at)],
        "a listener exactly at the radius must hear it"
    );

    // One quantum past it: not heard.
    core.world.players[w1].body.qx = base + (CHAT_LOCAL_CM as i32) / Q_CM + 1;
    seen.clear();
    let past = line("one step further");
    core.push_chat(
        0,
        ChatMsg {
            global: false,
            text: past,
        },
    );
    pump(&mut core, &stats, &mut clients, &mut seen);
    assert!(
        chats_to(&seen, 1).is_empty(),
        "one 3 cm quantum past the radius must be silence"
    );
}

/// Chat is not sim state: it never becomes a `Command`, never enters
/// `World`, and so cannot move the state hash the replay gate compares.
/// If this ever fails, chat has leaked into the sim and determinism now
/// depends on what someone typed.
#[test]
fn chat_never_touches_the_state_hash() {
    // Two whole shards at once overflows a debug test thread's 2 MiB
    // (a `ShardCore` carries the piece, deploy and slot-life stores
    // inline), so this one runs on a stack sized for the comparison.
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(chat_never_touches_the_state_hash_body)
        .expect("spawn")
        .join()
        .expect("the comparison thread");
}

fn chat_never_touches_the_state_hash_body() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    let mut clients = setup(&mut core);
    let mut seen = Vec::new();
    for _ in 0..3 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    // A quiet reference shard, driven the same number of ticks.
    let quiet_stats = ShardStats::default();
    let mut quiet = ShardCore::new(SEED);
    let mut quiet_clients = setup(&mut quiet);
    let mut quiet_seen = Vec::new();
    for _ in 0..3 {
        pump(
            &mut quiet,
            &quiet_stats,
            &mut quiet_clients,
            &mut quiet_seen,
        );
    }
    assert_eq!(
        core.world.state_hash(),
        quiet.world.state_hash(),
        "the two shards must start identical"
    );

    for slot in 0..3 {
        core.push_chat(
            slot,
            ChatMsg {
                global: slot % 2 == 0,
                text: line("talking through the whole tick"),
            },
        );
    }
    for _ in 0..5 {
        pump(&mut core, &stats, &mut clients, &mut seen);
        pump(
            &mut quiet,
            &quiet_stats,
            &mut quiet_clients,
            &mut quiet_seen,
        );
    }
    assert_eq!(
        core.world.state_hash(),
        quiet.world.state_hash(),
        "chat moved the state hash — it has leaked into the sim"
    );
}

/// The fan-out measures a radius from the speaker's body, so a speaker
/// the world no longer holds — a leave that landed between the line
/// being ringed and the tick that would say it, or a join whose command
/// is still queued behind a full buffer — has nothing to measure from.
/// That line is dropped and counted, never guessed at, and never panics
/// on the stale slot.
#[test]
fn a_line_from_a_speaker_the_world_has_lost_is_dropped() {
    let stats = ShardStats::default();
    let mut core = ShardCore::new(SEED);
    let mut clients = setup(&mut core);
    let mut seen = Vec::new();
    pump(&mut core, &stats, &mut clients, &mut seen);
    pump(&mut core, &stats, &mut clients, &mut seen);

    // The world drops the speaker's entity while the connection lives on.
    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].active = false;
    seen.clear();
    core.push_chat(
        0,
        ChatMsg {
            global: true,
            text: line("still here?"),
        },
    );
    pump(&mut core, &stats, &mut clients, &mut seen);
    assert!(
        seen.iter()
            .all(|(_, m)| !matches!(m, EventMsg::Chat { .. })),
        "a line from a speaker with no body must not be relayed"
    );
    assert_eq!(
        ShardStats::get(&stats.chat_undelivered),
        1,
        "the drop must be counted, not silent"
    );

    // The guard is not sticky: with a body again, the next line goes.
    core.world.players[w0].active = true;
    seen.clear();
    let there = line("here now");
    core.push_chat(
        0,
        ChatMsg {
            global: true,
            text: there,
        },
    );
    pump(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(chats_to(&seen, 0), vec![(id_of(0), true, there)]);
}
