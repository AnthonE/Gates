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
//! Five things it holds:
//!
//!   1. the list reaches the dying player and **nobody else** — it is an
//!      own-fact, and readiness in particular is exactly what an attacker
//!      would like to know;
//!   2. it carries **their** bags and not the island's;
//!   3. it arrives **before** the `Death` that raises the screen, which is
//!      what makes reading it in `OnEnter(Screen::Dead)` sound;
//!   4. a spent bag says so, so a second death inside the cooldown draws a
//!      hollow marker instead of promising an anchor that will not answer;
//!   5. it follows the bags while the owner lives — told at a join, a
//!      placement, a pickup, a resync and a wake that spends one, not only
//!      at a death (NOW §0die 2).

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
const SPAWN: (f32, f32) = (2048.0, 2048.0);
const CX: u16 = 682;
const CZ: u16 = 682;
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
/// Returns every event the tick of the placement produced.
fn place_bag(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    net_slot: usize,
    cx: u16,
    cz: u16,
) -> Vec<(usize, EventMsg)> {
    let w = world_slot(core, id_of(net_slot));
    core.world.players[w].inv[10] = ItemStack {
        item: BAG_ITEM,
        count: 1,
        // Durability v0 landed on the trunk while this branch was open: a
        // bag is not a tool and carries no condition, and `..default()`
        // would hide that from a reader rather than say it.
        cond: 0,
        skin: 0,
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
            pose: sim_core::footprint::Pose::CENTRE,
        },
    );
    let mut seen = Vec::new();
    pump(core, stats, clients, &mut seen);
    assert_eq!(
        core.world.deploys.len(),
        before + 1,
        "the bag did not place at ({cx}, {cz}) — the fixture, not the mechanic"
    );
    seen
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

/// How many `Bags` messages went to `slot`.
fn lists_to(seen: &[(usize, EventMsg)], slot: usize) -> usize {
    seen.iter()
        .filter(|(s, m)| *s == slot && matches!(m, EventMsg::Bags { .. }))
        .count()
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
    // The bystander was told its own bag when it placed it (NOW §0die 2),
    // so its mirror holds that one — and still only that one.
    let mine: Vec<_> = clients[1]
        .1
        .own_bags()
        .iter()
        .map(|b| (b.cx, b.cz))
        .collect();
    assert_eq!(
        mine,
        vec![(CX + 1, CZ)],
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
    let mut waking = Vec::new();
    for _ in 0..4 {
        let mut seen = Vec::new();
        pump(&mut core, &stats, &mut clients, &mut seen);
        for (slot, m) in &seen {
            if *slot == 0 && matches!(m, EventMsg::Respawn { .. }) {
                woke[0] |= APPLIED_RESPAWN;
            }
        }
        waking.extend(seen);
    }
    assert_ne!(woke[0], 0, "the respawn never came back");
    assert!(!clients[0].1.dead, "the body never woke");
    assert!(
        clients[0].1.woke_on_bag,
        "the wake did not land on the bag — the cooldown below would be \
         measuring nothing"
    );
    // The wake spent the bag, so the living owner is told at once, not at
    // the next death: the map's bed mark must not promise a ready anchor.
    assert_eq!(
        bags_for(&waking, 0),
        vec![(CX, CZ, 0u8, false)],
        "a wake on the bag left the owner's list saying it is ready"
    );
    assert_eq!(
        lists_to(&waking, 1),
        0,
        "the bystander heard the owner's list"
    );
    assert!(!clients[0].1.any_bag_ready(), "the living owner's mirror");

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

/// **The list follows the bags while the owner lives** (NOW §0die 2), not
/// only at a death: the map tags your beds off it, so a list from your
/// last death misses every bed placed since and still tags one a raider
/// took. A join is told (empty), a placement is told, a pickup is told,
/// and a resync is told again — each to the owner and nobody else.
///
/// A stranger's pickup is refused — a bag answers to its owner, even on
/// unclaimed ground (`deploy::pick_up`), since lifting it took away its
/// owner's respawn — and the owner's own lift is told to the owner alone.
#[test]
fn a_bag_placed_or_taken_is_told_to_its_living_owner_and_nobody_else() {
    let (mut core, stats, mut clients) = a_shard();
    let mut joined = Vec::new();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut joined);
    }
    // 1 · a join is told, explicitly, that nothing is there.
    for slot in 0..2 {
        assert_eq!(lists_to(&joined, slot), 1, "slot {slot}'s join list");
    }
    assert_eq!(bags_for(&joined, 0), vec![]);

    // 2 · a placement reaches its owner alive, and only its owner.
    let placed = place_bag(&mut core, &stats, &mut clients, 0, CX, CZ);
    assert!(!clients[0].1.dead, "nobody died — this is the living case");
    assert_eq!(
        lists_to(&placed, 0),
        1,
        "the owner was not told its new bag"
    );
    assert_eq!(bags_for(&placed, 0), vec![(CX, CZ, 0u8, true)]);
    assert_eq!(
        lists_to(&placed, 1),
        0,
        "the bystander was told somebody else's bags"
    );
    assert_eq!(clients[0].1.own_bags().len(), 1, "the owner's mirror");
    assert!(clients[1].1.own_bags().is_empty(), "the bystander's mirror");

    // 3 · a resync re-tells it whole, from the store.
    core.clients[0].ev_resync();
    let mut resynced = Vec::new();
    pump(&mut core, &stats, &mut clients, &mut resynced);
    assert_eq!(bags_for(&resynced, 0), vec![(CX, CZ, 0u8, true)]);
    assert_eq!(lists_to(&resynced, 1), 0);

    // 4 · the stranger may not lift it; the owner does, and is told.
    let lift = ActionMsg::Demolish {
        deploy: true,
        cx: CX,
        cz: CZ,
        level: 0,
        loc: LOC_PLANE,
    };
    assert!(core.wants_action(1), "hand should be open");
    core.push_action(1, lift);
    let mut refused = Vec::new();
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut refused);
    }
    assert_eq!(
        core.world.deploys.len(),
        1,
        "a stranger pocketed somebody's bag"
    );
    assert!(core.wants_action(0), "hand should be open");
    core.push_action(0, lift);
    let mut taken = Vec::new();
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut taken);
        if core.world.deploys.is_empty() {
            break;
        }
    }
    assert_eq!(
        core.world.deploys.len(),
        0,
        "the owner's pickup did not land — the fixture, not the mechanic"
    );
    assert_eq!(
        lists_to(&taken, 0),
        1,
        "the owner was not told its bag is gone"
    );
    assert_eq!(bags_for(&taken, 0), vec![]);
    assert_eq!(
        lists_to(&taken, 1),
        0,
        "the bystander was sent somebody else's list"
    );
    assert!(
        clients[0].1.own_bags().is_empty(),
        "the owner's map still tags a bed that is gone"
    );
}

/// **The death screen lives on the server** (wire v116), not in the client's
/// memory. The victim alone is told where it fell, ahead of the `Death`. A
/// player who closes the game on the screen and comes back gets it again —
/// its facts, its marker and its bags, to them alone and with no line in
/// anybody's kill feed — and the respawn that answers it keeps the marker.
#[test]
fn the_death_screen_and_its_marker_survive_a_reconnect() {
    use server::core::Admitted;
    use server::store::PlayerKey;
    use sim_core::world::DEATH_BY_CLOCK;

    let me = PlayerKey::new(b"comes-back-dead").expect("fits");
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.deploy = DeployContent::probe_fixture();
    core.world.survival = SurvivalContent::probe_fixture();
    core.world.combat = CombatContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    assert!(core.connect_as(0, id_of(0), Some(me), None).is_some());
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut Vec::new());
    }
    place_bag(&mut core, &stats, &mut clients, 0, CX, CZ);

    let seen = die(&mut core, &stats, &mut clients);
    let w = world_slot(&core, id_of(0));
    let body = core.world.players[w].body;
    let at = Some((body.qx, body.qz));
    let first = |slot: usize, pred: fn(&EventMsg) -> bool| {
        seen.iter().position(|(s, m)| *s == slot && pred(m))
    };
    let mark = first(0, |m| matches!(m, EventMsg::DeathMark { .. }))
        .expect("the victim was never told where it fell");
    let death = first(0, |m| matches!(m, EventMsg::Death { .. })).expect("no Death");
    assert!(mark < death, "the marker came after the screen it pins");
    assert_eq!(seen[mark].1, EventMsg::DeathMark { at });
    assert!(
        first(1, |m| matches!(m, EventMsg::DeathMark { .. })).is_none(),
        "the bystander was told where the victim fell"
    );
    let q = sim_core::movement::POS_XZ_Q;
    let fell = (body.qx as f32 * q, body.qz as f32 * q);
    assert_eq!(clients[0].1.death_marker, Some(fell));

    // Closed the game on the death screen: the corpse sleeps where it fell.
    assert!(core.disconnect(0).is_some());
    clients.retain(|(s, _)| *s != 0);
    pump(&mut core, &stats, &mut clients, &mut Vec::new());
    assert!(core.world.players[w].dead && core.world.players[w].sleeping);

    // And comes back to it.
    let (how, _) = core
        .connect_as(0, id2_of(0), Some(me), None)
        .expect("admitted");
    assert_eq!(how, Admitted::TookOver);
    clients.push((0, ClientCore::new(SEED, id2_of(0), 0)));
    let mut back = Vec::new();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut back);
    }
    assert!(core.world.players[w].dead, "the takeover woke the corpse");
    let owner = &clients.iter().find(|(s, _)| *s == 0).unwrap().1;
    assert!(owner.dead, "the death screen did not come back");
    assert_eq!(owner.own_death_cause, DEATH_BY_CLOCK);
    assert_eq!(
        owner.own_death_killer,
        id2_of(0),
        "a death of your own doing must name you by your new id"
    );
    assert!(
        owner.own_death_alive_s.is_some(),
        "this shard timed that life"
    );
    assert_eq!(owner.death_marker, Some(fell), "the marker was lost");
    // The bag is the wallet's, not the old connection's: the screen offers it.
    assert_eq!(owner.own_bags().len(), 1, "the reconnect orphaned the bag");
    assert!(
        !back
            .iter()
            .any(|(_, m)| matches!(m, EventMsg::Death { .. })),
        "a reconnect announced a killing in the kill feed"
    );
    assert!(
        !back.iter().any(
            |(s, m)| *s == 1 && matches!(m, EventMsg::Dead { .. } | EventMsg::DeathMark { .. })
        ),
        "the bystander heard somebody else's death screen"
    );

    // The answer stands the body up somewhere else; the marker stays.
    assert!(core.wants_action(0), "hand should be open");
    core.push_action(0, ActionMsg::Respawn { on_bag: false });
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut Vec::new());
    }
    let owner = &clients.iter().find(|(s, _)| *s == 0).unwrap().1;
    assert!(!owner.dead, "the respawn did not close the screen");
    assert_eq!(
        owner.death_marker,
        Some(fell),
        "the respawn took the marker"
    );
}

/// The id the slot table mints for `slot`'s second tenant: a reconnect.
fn id2_of(slot: usize) -> u32 {
    (2 << 8) | slot as u32
}

/// **Evicted while away, still your bag.** A full shard evicts the
/// longest-asleep body to seat a newcomer; its owner coming back later gets
/// a new body from their record, under a new id — and the bag they placed is
/// still theirs (`Command::Rekey`), not orphaned under the id the evicted
/// body wore.
#[test]
fn a_bag_survives_its_owner_being_evicted_and_coming_back() {
    use server::core::Admitted;
    use server::store::PlayerKey;
    const MAX: usize = sim_core::limits::MAX_PLAYERS;

    let me = PlayerKey::new(b"evicted-while-away").expect("fits");
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.deploy = DeployContent::probe_fixture();
    core.world.combat = CombatContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    assert!(core.connect_as(0, id_of(0), Some(me), None).is_some());
    let mut clients = vec![(0usize, ClientCore::new(SEED, id_of(0), 0))];
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut Vec::new());
    }
    place_bag(&mut core, &stats, &mut clients, 0, CX, CZ);
    let bags = |core: &ShardCore, id: u32| {
        let mut out = [sim_core::deploy::BagAnchor::default(); sim_core::deploy::BAG_CAP];
        core.world
            .deploys
            .own_bags(&core.world.deploy, id, core.world.tick, &mut out)
    };
    assert_eq!(bags(&core, id_of(0)), 1);

    // Log off, and the shard fills up behind the sleeper.
    assert!(core.disconnect(0).is_some());
    core.tick_bare(&stats, |_, _, _| true);
    for s in 1..MAX {
        assert!(core.connect(s, id_of(s)), "filler {s}");
    }
    core.tick_bare(&stats, |_, _, _| true);
    // A newcomer needs a body: the longest asleep, ours, is evicted.
    let (_, evicted) = core
        .connect_as(
            0,
            id2_of(0),
            Some(PlayerKey::new(b"newcomer").unwrap()),
            None,
        )
        .expect("admitted");
    assert_eq!(evicted.map(|(k, _)| k), Some(me), "ours is the pick");
    core.tick_bare(&stats, |_, _, _| true);
    assert!(core
        .world
        .players
        .iter()
        .all(|p| !p.active || p.id != id_of(0)));

    // Room to come back: a filler logs off, and its sleeper makes the room.
    core.disconnect(5);
    core.tick_bare(&stats, |_, _, _| true);
    let (how, _) = core
        .connect_as(5, id2_of(5), Some(me), None)
        .expect("admitted");
    assert_eq!(how, Admitted::Restored);
    core.tick_bare(&stats, |_, _, _| true);
    assert_eq!(
        bags(&core, id2_of(5)),
        1,
        "the bag stayed with the evicted id"
    );
    assert_eq!(bags(&core, id_of(0)), 0);
}
