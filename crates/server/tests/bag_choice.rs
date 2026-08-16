//! `bag_choice` — **the death screen is told which bags are yours**, on the
//! wire, at the moment it needs to know (bag choice v0, `SUB_BAGS`).
//!
//! The screen decides its own shape from this list: two rows and a map with
//! your beds on it, or one row and a beach. Everything that could make that
//! wrong lives between two crates, so it is gated here, through real encoded
//! bytes — `deploy_wire`'s shape.
//!
//! **Why a message at all**, restated because it is the part a later pass
//! will want to undo: `DeployRec::owner` is deliberately not on the wire, so
//! the island-wide deploy mirror every client already holds says where every
//! bed is and cannot say whose. The alternative to this message is an owner
//! id on every deployable anybody can see, which hands a raider the census
//! of a base they have not opened yet.
//!
//! Four things it holds:
//!
//!   1. the list reaches the dying player and **nobody else** — it is an
//!      own-fact, and readiness in particular is exactly what an attacker
//!      would like to know;
//!   2. it carries **their** bags and not the island's;
//!   3. it arrives **before** the `Death` that raises the screen, which is
//!      what makes reading it in `OnEnter(Screen::Dead)` sound;
//!   4. a spent bag says so, so a second death inside the cooldown draws a
//!      hollow marker instead of promising an anchor that will not answer.

use client_core::core::{ClientCore, APPLIED2_BAGS, APPLIED_RESPAWN};
use protocol::{ActionMsg, EventMsg, ItemCatalog};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::build::LOC_PLANE;
use sim_core::combat::CombatContent;
use sim_core::deploy::DeployContent;
use sim_core::gather::ItemStack;
use sim_core::survival::SurvivalContent;

const SEED: u64 = 20_260_731;
/// The canonical dev spawn point, guarded walkable in sim-core
/// `world::tests` — walkable terrain also takes ground-class deploys.
const SPAWN: (f32, f32) = (1024.0, 1024.0);
const CX: u16 = 341;
const CZ: u16 = 341;
/// Row 3 of `DeployContent::probe_fixture` is the ground-class bag and it
/// costs one unit of fixture item 5 — `sim-core`'s `bag_respawn` fixture,
/// restated rather than re-derived so the two files place the same thing.
const BAG_ROW: u16 = 3;
const BAG_ITEM: u16 = 5;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

/// One lockstep pump, keeping every event the server sent, decoded from the
/// bytes it actually put on the lane — `deploy_wire::pump_seen`.
///
/// The `seen` log is the point here rather than a convenience: what this
/// file asserts is *audience* and *order*, and a client mirror cannot show
/// either. A `Bags` delivered to both players leaves both mirrors correct.
fn pump(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    seen: &mut Vec<(usize, EventMsg)>,
) -> [u32; 2] {
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
    core.tick_bare(stats, |lane, slot, bytes| {
        match lane {
            Lane::Snapshot => snaps.push((slot, bytes.to_vec())),
            Lane::Event => events.push((slot, bytes.to_vec())),
        }
        true
    });
    let mut applied2 = [0u32; 2];
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
    for (slot, bytes) in snaps {
        if let Some(c) = clients.iter_mut().find(|(s, _)| *s == slot).map(|(_, c)| c) {
            c.on_datagram(&bytes);
        }
    }
    applied2
}

fn world_slot(core: &ShardCore, id: u32) -> usize {
    core.world
        .players
        .iter()
        .position(|p| p.active && p.id == id)
        .expect("player in world")
}

/// Place a bag through the real verb, standing where the fixture stands.
fn place_bag(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    net_slot: usize,
    cx: u16,
    cz: u16,
) {
    let w = world_slot(core, id_of(net_slot));
    core.world.players[w].inv[10] = ItemStack {
        item: BAG_ITEM,
        count: 1,
        // Durability v0 landed on the trunk while this branch was open: a
        // bag is not a tool and carries no condition, and `..default()`
        // would hide that from a reader rather than say it.
        cond: 0,
    };
    let before = core.world.deploys.len();
    assert!(core.wants_action(net_slot), "hand should be open");
    core.push_action(
        net_slot,
        ActionMsg::Deploy {
            row: BAG_ROW,
            cx,
            cz,
            level: 0,
            loc: LOC_PLANE,
        },
    );
    pump(core, stats, clients, &mut Vec::new());
    assert_eq!(
        core.world.deploys.len(),
        before + 1,
        "the bag did not place at ({cx}, {cz}) — the fixture, not the mechanic"
    );
}

/// Empty both meters and pump until the death screen is up on slot 0's
/// client. Returns every event the tick of the death produced, in order.
fn die(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
) -> Vec<(usize, EventMsg)> {
    let w = world_slot(core, id_of(0));
    let before = core.world.players[w].deaths;
    core.world.players[w].food = 0;
    core.world.players[w].water = 0;
    for _ in 0..120 * 30 {
        let mut seen = Vec::new();
        pump(core, stats, clients, &mut seen);
        if core.world.players[w].deaths > before {
            return seen;
        }
    }
    panic!("the probe clock never killed the body — the survival fixture changed");
}

fn bags_for(seen: &[(usize, EventMsg)], slot: usize) -> Vec<(u16, u16, u8, bool)> {
    seen.iter()
        .filter(|(s, _)| *s == slot)
        .filter_map(|(_, m)| match m {
            EventMsg::Bags { bags, count } => Some(
                bags[..*count as usize]
                    .iter()
                    .map(|b| (b.cx, b.cz, b.level, b.ready))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .next_back()
        .unwrap_or_default()
}

fn a_shard() -> (Box<ShardCore>, ShardStats, Vec<(usize, ClientCore)>) {
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.deploy = DeployContent::probe_fixture();
    // The clock is the murder weapon and nothing else — probe spans are
    // seconds, so a whole death fits inside a test (`bag_respawn`'s note).
    // `combat` is what gives the body hp to lose; an inert table grants
    // zero and a body at zero can never reach zero.
    core.world.survival = SurvivalContent::probe_fixture();
    core.world.combat = CombatContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    (core, stats, clients)
}

/// **The whole item.** Two players with a bag each; one dies, and the list
/// that reaches them names one bag — theirs.
///
/// Both halves proven red rather than argued. Sending the list to every
/// connected client instead of `client_slot_of(ev.a)` fails the audience
/// assertion; moving the send below the death broadcast, still inside the
/// same arm, fails the ordering one — and only that one, which is what
/// makes the two separate assertions worth having.
#[test]
fn a_dying_player_is_told_their_own_bags_and_only_their_own() {
    let (mut core, stats, mut clients) = a_shard();
    // Let the join drip settle so the tick of the death is not also the
    // tick of a def batch.
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut Vec::new());
    }
    place_bag(&mut core, &stats, &mut clients, 0, CX, CZ);
    place_bag(&mut core, &stats, &mut clients, 1, CX + 1, CZ);
    assert_eq!(core.world.deploys.len(), 2, "two bags stand");

    let seen = die(&mut core, &stats, &mut clients);

    // 1 · the audience. The stranger standing next to the corpse learns
    // nothing about where its owner wakes.
    assert!(
        !seen
            .iter()
            .any(|(s, m)| *s == 1 && matches!(m, EventMsg::Bags { .. })),
        "somebody else's bag list went to the bystander — readiness is \
         precisely what an attacker would like to know"
    );

    // 2 · the contents: one bag, at the owner's address, ready.
    assert_eq!(
        bags_for(&seen, 0),
        vec![(CX, CZ, 0u8, true)],
        "the dying player's own-bag list is wrong — either it picked up the \
         stranger's bed or it lost their own"
    );

    // 3 · the order. The screen reads the list in `OnEnter(Screen::Dead)`,
    // and what raises that state is the `Death`. The event lane is ordered
    // and reliable, so "before" is a guarantee — but only if the server
    // actually writes it first.
    let pos = |pred: fn(&EventMsg) -> bool| {
        seen.iter()
            .position(|(s, m)| *s == 0 && pred(m))
            .unwrap_or_else(|| panic!("the owner never received the message"))
    };
    assert!(
        pos(|m| matches!(m, EventMsg::Bags { .. })) < pos(|m| matches!(m, EventMsg::Death { .. })),
        "the bag list arrived after the death that raises the screen — the \
         first frame of the death screen would offer a beach to a player \
         with a bag"
    );

    // 4 · the client mirror agrees, which is what the screen actually reads.
    let owner = &clients[0].1;
    assert!(owner.dead, "the owner's client is not on the death screen");
    assert_eq!(owner.own_bags().len(), 1);
    assert!(owner.any_bag_ready());
    assert!(
        clients[1].1.own_bags().is_empty(),
        "the bystander's mirror grew a bag from somebody else's death"
    );
}

/// A player who never placed one is told **nothing is there** — and told it
/// explicitly, as an empty list rather than as silence.
///
/// The distinction is the whole reason `encode_event_bags` accepts an empty
/// body where every other batch encoder in `protocol` refuses one: silence
/// and "you have no bags" are the same bytes if emptiness is not sendable,
/// and a client cannot tell a message it did not get from a message that
/// said no.
#[test]
fn a_player_with_no_bag_is_told_so_rather_than_left_guessing() {
    let (mut core, stats, mut clients) = a_shard();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut Vec::new());
    }

    let seen = die(&mut core, &stats, &mut clients);
    let sent: Vec<_> = seen
        .iter()
        .filter(|(s, m)| *s == 0 && matches!(m, EventMsg::Bags { .. }))
        .collect();
    assert_eq!(
        sent.len(),
        1,
        "a bagless death sent {} bag lists — the screen has nothing to \
         shape itself from",
        sent.len()
    );
    assert_eq!(bags_for(&seen, 0), vec![]);
    assert!(clients[0].1.own_bags().is_empty());
    assert!(!clients[0].1.any_bag_ready());
}

/// **A spent bag says it is spent.** Wake on the bag, die again inside the
/// five minutes, and the second list carries the same address with `ready`
/// false — which is what draws it hollow instead of promising an anchor
/// that will hand you a beach.
///
/// It is also the assertion that the list is **restated whole** rather than
/// merged: a client that accumulated would still be holding the first
/// message's `ready: true` beside the second's `false`.
#[test]
fn a_bag_inside_its_cooldown_is_reported_spent() {
    let (mut core, stats, mut clients) = a_shard();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut Vec::new());
    }
    place_bag(&mut core, &stats, &mut clients, 0, CX, CZ);

    let first = die(&mut core, &stats, &mut clients);
    assert_eq!(bags_for(&first, 0), vec![(CX, CZ, 0u8, true)]);

    // Answer the screen with the bag, through the real action.
    assert!(core.wants_action(0), "hand should be open");
    core.push_action(0, ActionMsg::Respawn { on_bag: true });
    let mut woke = [0u32; 2];
    for _ in 0..4 {
        let mut seen = Vec::new();
        pump(&mut core, &stats, &mut clients, &mut seen);
        for (slot, m) in &seen {
            if *slot == 0 && matches!(m, EventMsg::Respawn { .. }) {
                woke[0] |= APPLIED_RESPAWN;
            }
        }
    }
    assert_ne!(woke[0], 0, "the respawn never came back");
    assert!(!clients[0].1.dead, "the body never woke");
    assert!(
        clients[0].1.woke_on_bag,
        "the wake did not land on the bag — the cooldown below would be \
         measuring nothing"
    );

    let second = die(&mut core, &stats, &mut clients);
    assert_eq!(
        bags_for(&second, 0),
        vec![(CX, CZ, 0u8, false)],
        "the second death inside the cooldown was offered a ready bag"
    );
    let owner = &clients[0].1;
    assert_eq!(
        owner.own_bags().len(),
        1,
        "the mirror merged two lists instead of replacing"
    );
    assert!(
        !owner.any_bag_ready(),
        "the client still thinks a spent bag will answer"
    );
}

/// The applied bit is raised, so a reader that wants "is this fresh" has
/// one. `APPLIED2_BAGS` is word 1 because word 0 is full — the bit itself
/// is cheap and the thing worth gating is that it is actually set, since a
/// flag nothing raises is the shape `APPLIED_MOVE` shipped in.
#[test]
fn the_bag_list_announces_itself() {
    let (mut core, stats, mut clients) = a_shard();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut Vec::new());
    }
    place_bag(&mut core, &stats, &mut clients, 0, CX, CZ);

    let w = world_slot(&core, id_of(0));
    core.world.players[w].food = 0;
    core.world.players[w].water = 0;
    let mut raised = 0u32;
    for _ in 0..120 * 30 {
        raised |= pump(&mut core, &stats, &mut clients, &mut Vec::new())[0];
        if clients[0].1.dead {
            break;
        }
    }
    assert_ne!(
        raised & APPLIED2_BAGS,
        0,
        "the bag list arrived and raised no applied flag"
    );
}
