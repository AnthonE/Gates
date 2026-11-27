//! Map marks shared with the team on the wire (v157): `ShardCore` ↔
//! `ClientCore` through real encoded bytes. A set of marks reaches the
//! sender's teammates and nobody else — not the sender, not a stranger; one
//! from a player with no team goes nowhere; and a mate who leaves takes
//! their marks off the map of those who stay.

use client_core::core::ClientCore;
use protocol::{
    ActionMsg, EventMsg, ItemCatalog, PinSet, WirePin, TEAM_OP_ACCEPT, TEAM_OP_INVITE,
    TEAM_OP_LEAVE,
};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::gather::GatherContent;

const SEED: u64 = 20_261_125;
const SPAWN: (f32, f32) = (2048.0, 2048.0);

fn id_of(slot: usize) -> u32 {
    protocol::player_id(1, slot)
}

/// A connection's id on its second session.
fn id2_of(slot: usize) -> u32 {
    protocol::player_id(2, slot)
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

/// Past the pace's gap for a team verb (`pace::Kind::Use`) and a set of
/// marks (`pace::Kind::Pins`).
fn settle(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    seen: &mut Vec<(usize, EventMsg)>,
) {
    for _ in 0..14 {
        pump(core, stats, clients, seen);
    }
}

fn pins_to(seen: &[(usize, EventMsg)], slot: usize) -> Vec<(u32, PinSet)> {
    seen.iter()
        .filter(|(s, _)| *s == slot)
        .filter_map(|(_, m)| match m {
            EventMsg::TeamPins { from, pins } => Some((*from, *pins)),
            _ => None,
        })
        .collect()
}

fn set_of(labels: &[&str]) -> PinSet {
    let mut set = PinSet::NONE;
    for (i, l) in labels.iter().enumerate() {
        assert!(set.push(WirePin::new(1000 + i as u16, 2000, 2, 1, l).unwrap()));
    }
    set
}

#[test]
fn a_set_of_marks_reaches_the_team_and_nobody_else() {
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

    // No team yet: a set goes nowhere, and nobody is told anything.
    seen.clear();
    core.push_action(2, ActionMsg::Pins(set_of(&["ALONE"])));
    settle(&mut core, &stats, &mut clients, &mut seen);
    for slot in 0..3 {
        assert!(pins_to(&seen, slot).is_empty(), "slot {slot}");
    }

    // Slots 0 and 1 make a team; slot 2 stays out of it.
    core.push_action(
        0,
        ActionMsg::Team {
            op: TEAM_OP_INVITE,
            target: id_of(1),
        },
    );
    settle(&mut core, &stats, &mut clients, &mut seen);
    core.push_action(
        1,
        ActionMsg::Team {
            op: TEAM_OP_ACCEPT,
            target: id_of(0),
        },
    );
    settle(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(clients[1].1.team().count, 2);

    // Slot 0's marks reach slot 1, once, and neither the sender nor the
    // stranger.
    seen.clear();
    let raid = set_of(&["RAID", "MEET"]);
    core.push_action(0, ActionMsg::Pins(raid));
    settle(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(pins_to(&seen, 1), vec![(id_of(0), raid)]);
    assert!(pins_to(&seen, 0).is_empty(), "no echo to the sender");
    assert!(pins_to(&seen, 2).is_empty(), "a stranger sees none");
    let shown: Vec<_> = clients[1].1.team_pins().map(|(id, s)| (id, *s)).collect();
    assert_eq!(shown, vec![(id_of(0), raid)]);
    assert_eq!(clients[2].1.team_pins().count(), 0);

    // An empty set takes them off the mate's map.
    core.push_action(0, ActionMsg::Pins(PinSet::NONE));
    settle(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(clients[1].1.team_pins().count(), 0);

    // Slot 1's marks reach slot 0; slot 1 leaves, and slot 0's map loses
    // them with the next team view.
    core.push_action(1, ActionMsg::Pins(set_of(&["HOME"])));
    settle(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(clients[0].1.team_pins().count(), 1);
    core.push_action(
        1,
        ActionMsg::Team {
            op: TEAM_OP_LEAVE,
            target: 0,
        },
    );
    settle(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(clients[0].1.team_pins().count(), 0, "a leaver's marks go");

    // And once apart, a set reaches neither.
    seen.clear();
    core.push_action(1, ActionMsg::Pins(set_of(&["GONE"])));
    settle(&mut core, &stats, &mut clients, &mut seen);
    for slot in 0..3 {
        assert!(pins_to(&seen, slot).is_empty(), "slot {slot}");
    }
}

fn client(clients: &[(usize, ClientCore)], slot: usize) -> &ClientCore {
    &clients
        .iter()
        .find(|(s, _)| *s == slot)
        .expect("connected")
        .1
}

/// **The shard keeps a player's marks while they play** (batch 2's
/// follow-up): a mate who comes back, or a player who comes onto the team,
/// sees marks set before they arrived though nobody changed one since, and
/// a player who leaves takes theirs out of what the shard hands on.
#[test]
fn a_joiner_sees_marks_set_before_they_came() {
    use server::store::PlayerKey;
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    let key = |slot: usize| PlayerKey::new(format!("pins-{slot}").as_bytes()).expect("fits");
    for slot in 0..3 {
        assert!(core
            .connect_as(slot, id_of(slot), Some(key(slot)), None)
            .is_some());
    }
    let mut clients: Vec<(usize, ClientCore)> = (0..3)
        .map(|slot| (slot, ClientCore::new(SEED, id_of(slot), 0)))
        .collect();
    let mut seen = Vec::new();
    settle(&mut core, &stats, &mut clients, &mut seen);

    // Slot 2 marks the map with no team; slots 0 and 1 make one.
    core.push_action(2, ActionMsg::Pins(set_of(&["LOOT"])));
    core.push_action(
        0,
        ActionMsg::Team {
            op: TEAM_OP_INVITE,
            target: id_of(1),
        },
    );
    settle(&mut core, &stats, &mut clients, &mut seen);
    core.push_action(
        1,
        ActionMsg::Team {
            op: TEAM_OP_ACCEPT,
            target: id_of(0),
        },
    );
    settle(&mut core, &stats, &mut clients, &mut seen);
    let raid = set_of(&["RAID", "MEET"]);
    core.push_action(0, ActionMsg::Pins(raid));
    core.push_action(1, ActionMsg::Pins(set_of(&["HOME"])));
    settle(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(client(&clients, 1).team_pins().count(), 1);

    // Slot 1 closes the game and comes back to its sleeping body, a new
    // session with a new id: slot 0's marks are on its map without slot 0
    // sending them again.
    assert!(core.disconnect(1).is_some());
    clients.retain(|(s, _)| *s != 1);
    settle(&mut core, &stats, &mut clients, &mut seen);
    let (how, _) = core
        .connect_as(1, id2_of(1), Some(key(1)), None)
        .expect("admitted");
    assert_eq!(how, server::core::Admitted::TookOver);
    clients.push((1, ClientCore::new(SEED, id2_of(1), 0)));
    seen.clear();
    settle(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(client(&clients, 1).team().count, 2, "still on the team");
    let shown: Vec<_> = client(&clients, 1)
        .team_pins()
        .map(|(id, s)| (id, *s))
        .collect();
    assert_eq!(
        shown,
        vec![(id_of(0), raid)],
        "a joiner sees a mate's marks"
    );
    assert_eq!(pins_to(&seen, 1).len(), 1, "told once");
    // Slot 1's HOME left with its session: slot 0 is not sent it again.
    assert!(
        pins_to(&seen, 0).is_empty(),
        "a leaver's marks are not kept"
    );

    // Slot 2 comes onto the team: it sees slot 0's marks, and both of them
    // see the marks slot 2 set while it had no team.
    core.push_action(
        0,
        ActionMsg::Team {
            op: TEAM_OP_INVITE,
            target: id_of(2),
        },
    );
    settle(&mut core, &stats, &mut clients, &mut seen);
    seen.clear();
    core.push_action(
        2,
        ActionMsg::Team {
            op: TEAM_OP_ACCEPT,
            target: id_of(0),
        },
    );
    settle(&mut core, &stats, &mut clients, &mut seen);
    let to_2: Vec<_> = client(&clients, 2)
        .team_pins()
        .map(|(id, s)| (id, *s))
        .collect();
    assert_eq!(to_2, vec![(id_of(0), raid)], "a new member sees the team's");
    for slot in [0, 1] {
        let loot: Vec<_> = pins_to(&seen, slot)
            .into_iter()
            .filter(|(from, _)| *from == id_of(2))
            .collect();
        assert_eq!(loot, vec![(id_of(2), set_of(&["LOOT"]))], "slot {slot}");
    }
    assert_eq!(pins_to(&seen, 2).len(), 1, "slot 1 has no marks to send");
}
