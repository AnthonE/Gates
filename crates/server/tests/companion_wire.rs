//! The companion's sim half (`companion.rs`) against a real `ShardCore`: a
//! team line said in the game reaches the companion with every teammate's
//! wallet and nobody else's; a line typed on the page is said on the team
//! channel from the wallet's body and reaches the team in the game only;
//! and the snapshot carries each body's wallet and team.

use protocol::{
    ActionMsg, ChatMsg, ChatText, EventMsg, ItemCatalog, TEAM_OP_ACCEPT, TEAM_OP_INVITE,
};
use server::companion::{ToWeb, WebLine};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use server::store::PlayerKey;
use sim_core::gather::GatherContent;

const SEED: u64 = 20_260_731;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn wallet(slot: usize) -> String {
    format!("0x{:040x}", 0xa0 + slot)
}

fn pump(core: &mut ShardCore, stats: &ShardStats, seen: &mut Vec<(usize, EventMsg)>) {
    for _ in 0..10 {
        let mut events: Vec<(usize, Vec<u8>)> = Vec::new();
        core.tick_bare(stats, |lane, slot, bytes| {
            if lane == Lane::Event {
                events.push((slot, bytes.to_vec()));
            }
            true
        });
        for (slot, bytes) in events {
            seen.push((slot, protocol::decode_event(&bytes).expect("decodes")));
        }
    }
}

fn team_chats(seen: &[(usize, EventMsg)]) -> Vec<(usize, u32)> {
    seen.iter()
        .filter_map(|(slot, m)| match m {
            EventMsg::Chat { from, channel, .. } if *channel == protocol::Channel::Team => {
                Some((*slot, *from))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn team_chat_crosses_between_the_game_and_the_companion() {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.dev_spawn = Some((2048.0, 2048.0));
    core.catalog = ItemCatalog::EMPTY;
    let (companion, mut from_sim, mut to_sim) = server::companion::pair();
    core.companion = companion;
    for slot in 0..3 {
        let key = PlayerKey::new(wallet(slot).as_bytes()).unwrap();
        assert!(core
            .connect_as(slot, id_of(slot), Some(key), None)
            .is_some());
    }
    let mut seen = Vec::new();
    pump(&mut core, &stats, &mut seen);
    core.push_action(
        0,
        ActionMsg::Team {
            op: TEAM_OP_INVITE,
            target: id_of(1),
        },
    );
    pump(&mut core, &stats, &mut seen);
    core.push_action(
        1,
        ActionMsg::Team {
            op: TEAM_OP_ACCEPT,
            target: id_of(0),
        },
    );
    pump(&mut core, &stats, &mut seen);

    // The snapshot: every body with its wallet; 0 and 1 share a team.
    core.publish_companion();
    let mut snap = None;
    while let Ok(m) = from_sim.pop() {
        if let ToWeb::Snap(s) = m {
            snap = Some(s);
        }
    }
    let snap = snap.expect("a snapshot");
    let team_of = |slot: usize| {
        snap.people
            .iter()
            .find(|p| p.wallet == wallet(slot))
            .map(|p| p.team)
            .expect("a body for each wallet")
    };
    assert_eq!(team_of(0), id_of(0));
    assert_eq!(team_of(1), id_of(0));
    assert_eq!(team_of(2), 0);

    // Said in the game: the companion hears it with the team's wallets.
    core.push_chat(
        0,
        ChatMsg {
            channel: protocol::Channel::Team,
            text: ChatText::sanitize(b"wood").unwrap(),
        },
    );
    pump(&mut core, &stats, &mut seen);
    let lines: Vec<_> = std::iter::from_fn(|| from_sim.pop().ok())
        .filter_map(|m| match m {
            ToWeb::Line(l) => Some(l),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(
        (lines[0].from.as_str(), lines[0].text.as_str(), lines[0].web),
        (wallet(0).as_str(), "wood", false)
    );
    assert_eq!(lines[0].hearers, vec![wallet(0), wallet(1)]);

    // Typed on the page by slot 1's wallet: the team hears it in the game,
    // from slot 1's body, and slot 2 does not.
    seen.clear();
    to_sim
        .push(WebLine {
            wallet: PlayerKey::new(wallet(1).as_bytes()).unwrap(),
            text: ChatText::sanitize(b"on my way").unwrap(),
        })
        .unwrap();
    pump(&mut core, &stats, &mut seen);
    let mut heard = team_chats(&seen);
    heard.sort();
    assert_eq!(heard, vec![(0, id_of(1)), (1, id_of(1))]);
    let web: Vec<_> = std::iter::from_fn(|| from_sim.pop().ok())
        .filter_map(|m| match m {
            ToWeb::Line(l) => Some(l),
            _ => None,
        })
        .collect();
    assert_eq!(web.len(), 1);
    assert!(web[0].web);
    assert_eq!(web[0].from, wallet(1));

    // A wallet with no team says nothing anywhere.
    seen.clear();
    to_sim
        .push(WebLine {
            wallet: PlayerKey::new(wallet(2).as_bytes()).unwrap(),
            text: ChatText::sanitize(b"hello?").unwrap(),
        })
        .unwrap();
    pump(&mut core, &stats, &mut seen);
    assert!(team_chats(&seen).is_empty());
}
