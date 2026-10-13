//! Teams on the wire (v108): `ShardCore` ↔ `ClientCore` through real
//! encoded bytes. An invite within reach makes a team and reaches the
//! invitee as a waiting invite; accepting joins it and both clients learn
//! the roster; the team channel reaches the team wherever they stand and
//! nobody else; a refusal is said to the asker; and the last one out
//! leaves no team behind.

use client_core::core::ClientCore;
use protocol::{
    ActionMsg, ChatMsg, ChatText, EventMsg, ItemCatalog, TEAM_AWAKE, TEAM_OP_ACCEPT,
    TEAM_OP_INVITE, TEAM_OP_LEAVE,
};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::gather::GatherContent;

const SEED: u64 = 20_260_731;
const SPAWN: (f32, f32) = (2048.0, 2048.0);
/// Position quantum in centimeters (DESIGN.md §5.5: 3 cm x/z).
const Q_CM: i32 = 3;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn pump(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    seen: &mut Vec<(usize, EventMsg)>,
) {
    let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
    core.tick_bare(stats, |lane, slot, bytes| {
        if lane == Lane::Event {
            events.push((slot, bytes.to_vec()));
        }
        true
    });
    for (slot, bytes) in events {
        seen.push((slot, protocol::decode_event(&bytes).expect("decodes")));
        if let Some((_, c)) = clients.iter_mut().find(|(s, _)| *s == slot) {
            c.on_stream(&bytes).expect("decodes");
        }
    }
}

fn settle(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    seen: &mut Vec<(usize, EventMsg)>,
) {
    // Past the pace's gap for a team verb (`pace::Kind::Use`).
    for _ in 0..10 {
        pump(core, stats, clients, seen);
    }
}

fn world_slot(core: &ShardCore, id: u32) -> usize {
    core.world
        .players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("player in world")
}

fn chats_to(seen: &[(usize, EventMsg)], slot: usize) -> Vec<(u32, protocol::Channel)> {
    seen.iter()
        .filter(|(s, _)| *s == slot)
        .filter_map(|(_, m)| match m {
            EventMsg::Chat { from, channel, .. } => Some((*from, *channel)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_team_forms_talks_and_parts_on_the_wire() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    for slot in 0..3 {
        assert!(core.connect(slot, id_of(slot)));
    }
    let mut clients: Vec<(usize, ClientCore)> = (0..3)
        .map(|slot| (slot, ClientCore::new(SEED, id_of(slot), 0)))
        .collect();
    let mut seen = Vec::new();
    settle(&mut core, &stats, &mut clients, &mut seen);

    // Too far: slot 2 walks off and is refused, and no team is made.
    let w2 = world_slot(&core, id_of(2));
    core.world.players[w2].body.qx += 3_000 / Q_CM; // 30 m east
    seen.clear();
    core.push_action(
        0,
        ActionMsg::Team {
            op: TEAM_OP_INVITE,
            target: id_of(2),
        },
    );
    settle(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(chats_to(&seen, 0), vec![(0, protocol::Channel::Global)]);
    assert_eq!(
        clients[0].1.team().count,
        0,
        "a refused invite makes no team"
    );

    // Within reach: the invite founds a team and waits on slot 1.
    core.push_action(
        0,
        ActionMsg::Team {
            op: TEAM_OP_INVITE,
            target: id_of(1),
        },
    );
    settle(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(clients[0].1.team().leader, id_of(0));
    assert_eq!(clients[1].1.team().invite, id_of(0));
    assert_eq!(clients[1].1.team().count, 0);

    core.push_action(
        1,
        ActionMsg::Team {
            op: TEAM_OP_ACCEPT,
            target: id_of(0),
        },
    );
    settle(&mut core, &stats, &mut clients, &mut seen);
    for (slot, c) in clients.iter().take(2) {
        let t = c.team();
        assert_eq!(
            (t.leader, t.count, t.invite),
            (id_of(0), 2, 0),
            "slot {slot}"
        );
        assert!(t.members().iter().all(|m| m.state == TEAM_AWAKE));
        assert_eq!(t.members()[1].x_m, 2048, "a member's whole metres");
    }
    assert!(clients[0].1.is_teammate(id_of(1)) && clients[1].1.is_teammate(id_of(0)));
    assert!(!clients[0].1.is_teammate(id_of(0)), "never yourself");
    assert_eq!(
        clients[2].1.team().count,
        0,
        "the stranger is shown nothing"
    );

    // The team channel reaches the team, however far, and nobody else.
    let w1 = world_slot(&core, id_of(1));
    core.world.players[w1].body.qx -= 50_000 / Q_CM; // 500 m west
    seen.clear();
    let text = ChatText::sanitize(b"wood at the north rock").unwrap();
    core.push_chat(
        0,
        ChatMsg {
            channel: protocol::Channel::Team,
            text,
        },
    );
    pump(&mut core, &stats, &mut clients, &mut seen);
    let team = vec![(id_of(0), protocol::Channel::Team)];
    assert_eq!(chats_to(&seen, 0), team, "the speaker's own receipt");
    assert_eq!(chats_to(&seen, 1), team, "a teammate 500 m off hears it");
    assert!(
        chats_to(&seen, 2).is_empty(),
        "a stranger 30 m off does not"
    );
    // The far member's position followed them on the drip.
    settle(&mut core, &stats, &mut clients, &mut seen);
    let far = clients[0].1.team().members()[1];
    assert_eq!((far.id, far.x_m), (id_of(1), 2048 - 500));

    // A player with no team is told, and nobody hears them.
    seen.clear();
    core.push_chat(
        2,
        ChatMsg {
            channel: protocol::Channel::Team,
            text,
        },
    );
    pump(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(chats_to(&seen, 2), vec![(0, protocol::Channel::Global)]);
    assert!(chats_to(&seen, 0).is_empty() && chats_to(&seen, 1).is_empty());

    // The leader leaves: the team passes to slot 1, alone, by `/team`.
    let leave = ChatText::sanitize(b"/team leave").unwrap();
    core.push_chat(
        0,
        ChatMsg {
            channel: protocol::Channel::Local,
            text: leave,
        },
    );
    settle(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(clients[0].1.team().count, 0);
    let t = clients[1].1.team();
    assert_eq!((t.leader, t.count), (id_of(1), 1));
    core.push_action(
        1,
        ActionMsg::Team {
            op: TEAM_OP_LEAVE,
            target: 0,
        },
    );
    settle(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(*clients[1].1.team(), protocol::TeamView::NONE);
}
