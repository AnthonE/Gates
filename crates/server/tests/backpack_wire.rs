//! The backpack-on-the-wire gate (M1): `ShardCore` ↔ `ClientCore` through
//! real encoded bytes — a kill drops a bag both clients see, a late joiner
//! is handed the standing set by the sync walk, and the loot action moves
//! the loot into the killer's inventory and takes the bag off every
//! client's map; a despawn storm leaves a joiner's bag walk standing, and
//! bags are aimed by class-S interest with the owner exempt.
//! Deterministic, no sockets; asserts are structural and exact (the
//! deploy_wire shape).
//!
//! The composition this closes is the one the sim tests cannot: a bag
//! exists in the sim whether or not the wire says so, and a client can
//! agree with the world for reasons the *encoder* never earned. So every
//! claim here is made twice — once against the client mirror, once
//! against `seen`, the decoded bytes the server actually put on the lane.

use client_core::core::{ClientCore, APPLIED_BAGS};
use protocol::{ActionMsg, EventMsg, ItemCatalog};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::backpack::{BackpackContent, BAG_GONE_EMPTIED};
use sim_core::combat::CombatContent;
use sim_core::gather::{GatherContent, ItemStack, SWING_INTERVAL_TICKS};
use sim_core::input::BTN_PRIMARY;
use sim_core::movement::{Body, POS_XZ_Q};

/// The solved authored sites for `seed` — what `terrain::ground` needs in order
/// to know where the carve is.
///
/// Memoized per seed, and that is not premature: `terrain::haven` is a few
/// thousand `height` taps (a shoreline march, a bisect and a rosette per
/// candidate bearing), these suites call it from inside assertion loops, and
/// the first draft of this helper resolved it per call and took the workspace
/// test run past five minutes. It is a pure function of the seed, so caching
/// cannot change a result.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    // A thread-local rather than a `Mutex`: `std::sync::Mutex` is on
    // `sim-core/clippy.toml`'s disallowed list (wall 3), and that list is
    // crate-scoped, so it binds this suite too. Per-thread is the right shape
    // anyway — the cache exists to stop a per-assertion recompute, not to be
    // shared.
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static sim_core::terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static sim_core::terrain::Haven = Box::leak(Box::new(sim_core::terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

const SEED: u64 = 20_260_802;
/// The canonical dev spawn point, guarded walkable in sim-core
/// `world::tests`.
const SPAWN: (f32, f32) = (2048.0, 2048.0);
/// The gather fixture's item 0 — also the combat fixture's 34-damage
/// weapon, so three hits kill.
const SPEAR: u16 = 0;
/// A fixture item with no weapon row: pure loot.
const FILLER: u16 = 7;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn pump(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
    seen: &mut Vec<(usize, EventMsg)>,
) -> [u32; 4] {
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
    let mut flags = [0u32; 4];
    for (slot, bytes) in events {
        seen.push((
            slot,
            protocol::decode_event(&bytes).expect("server events decode"),
        ));
        if let Some(c) = clients.iter_mut().find(|(s, _)| *s == slot).map(|(_, c)| c) {
            flags[slot] |= c.on_stream(&bytes).expect("server events decode");
        }
    }
    for (slot, bytes) in snaps {
        if let Some(c) = clients.iter_mut().find(|(s, _)| *s == slot).map(|(_, c)| c) {
            c.on_datagram(&bytes);
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

fn armed_core() -> Box<ShardCore> {
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.combat = CombatContent::probe_fixture();
    core.world.backpack = BackpackContent::probe_fixture();
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = ItemCatalog::EMPTY;
    core
}

/// Hold slot 0's swing on slot 1 until the kill lands, standing them
/// coincident every tick so the aim cone has no bearing to fail on
/// (point-blank, the same arrangement `test_alloc_zero` uses). Returns
/// the events the wire carried while it happened.
fn fight_to_a_kill(
    core: &mut ShardCore,
    stats: &ShardStats,
    clients: &mut [(usize, ClientCore)],
) -> Vec<(usize, EventMsg)> {
    let w1 = world_slot(core, id_of(1));
    let deaths_before = core.world.players[w1].deaths;
    let mut seen = Vec::new();
    clients[0].1.set_input(BTN_PRIMARY, 0, 128, 0, 0, 0);
    clients[1].1.set_input(0, 0, 128, 0, 0, 0);
    for _ in 0..(SWING_INTERVAL_TICKS * 8) {
        // Stand the victim inside the attacker every tick: point-blank
        // has no bearing to test, so the aim cone cannot make the fight
        // flaky (`test_alloc_zero`'s arrangement).
        let (w0, w1) = (world_slot(core, id_of(0)), world_slot(core, id_of(1)));
        core.world.players[w1].body = core.world.players[w0].body;
        pump(core, stats, clients, &mut seen);
        let w1 = world_slot(core, id_of(1));
        if core.world.players[w1].deaths > deaths_before {
            clients[0].1.set_input(0, 0, 128, 0, 0, 0);
            return seen;
        }
    }
    panic!("three fixture spear hits must kill inside eight swing intervals");
}

#[test]
fn a_kill_puts_a_bag_on_every_client_and_the_loot_takes_it_off() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    // Let the join drips settle so nothing below is racing a catalog.
    let mut warm = Vec::new();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut warm);
    }
    for (_, c) in &clients {
        assert!(c.bags.is_empty(), "nobody has died yet");
    }

    // Arm both, and give the victim something worth taking.
    let (w0, w1) = (world_slot(&core, id_of(0)), world_slot(&core, id_of(1)));
    core.world.players[w0].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
        cond: 0,
        skin: 0,
    };
    core.world.players[w1].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
        cond: 0,
        skin: 0,
    };
    core.world.players[w1].inv[1] = ItemStack {
        item: FILLER,
        count: 42,
        cond: 0,
        skin: 0,
    };

    let seen = fight_to_a_kill(&mut core, &stats, &mut clients);
    assert_eq!(core.world.backpacks.len(), 1, "the death dropped one bag");
    let bag = core.world.backpacks.entries()[0];

    // The wire itself said so — to both clients, at the position the sim
    // holds, and not merely "a client that agrees".
    let dropped: Vec<usize> = seen
        .iter()
        .filter_map(|(slot, m)| match m {
            EventMsg::BagDropped { id, qx, qy, qz, .. } if *id == bag.id => {
                assert_eq!(
                    (*qx, *qy, *qz),
                    (bag.qx, bag.qy, bag.qz),
                    "bag moved on the wire"
                );
                Some(*slot)
            }
            _ => None,
        })
        .collect();
    assert_eq!(dropped, vec![0, 1], "a bag is a broadcast, like a death");
    assert!(
        seen.iter()
            .any(|(_, m)| matches!(m, EventMsg::Death { .. })),
        "the death rides the same lane"
    );
    for (_, c) in &clients {
        assert_eq!(c.bags.len(), 1, "both clients hold the bag");
        let mirrored = c.bags.entries()[0];
        assert_eq!(mirrored.id, bag.id);
        assert_eq!(
            (mirrored.qx, mirrored.qy, mirrored.qz),
            (bag.qx, bag.qy, bag.qz)
        );
    }

    // The killer reaches for it. They never moved, so it is at their feet.
    assert!(core.wants_action(0), "hand should be open");
    core.push_action(0, ActionMsg::Loot);
    let mut seen = Vec::new();
    pump(&mut core, &stats, &mut clients, &mut seen);

    assert!(core.world.backpacks.is_empty(), "an emptied bag leaves");
    let w0 = world_slot(&core, id_of(0));
    let held: Vec<(u16, u16)> = core.world.players[w0]
        .inv
        .iter()
        .filter(|s| s.count > 0)
        .map(|s| (s.item, s.count))
        .collect();
    assert_eq!(
        held,
        vec![(SPEAR, 2), (FILLER, 42)],
        "the kill paid: the victim's kit is in the killer's pockets"
    );
    let removed: Vec<usize> = seen
        .iter()
        .filter_map(|(slot, m)| match m {
            EventMsg::BagRemoved { id, why } if *id == bag.id => {
                assert_eq!(*why as u32, BAG_GONE_EMPTIED, "the reason is on the wire");
                Some(*slot)
            }
            _ => None,
        })
        .collect();
    assert_eq!(removed, vec![0, 1], "the removal is a broadcast too");
    for (_, c) in &clients {
        assert!(c.bags.is_empty(), "and no client is still drawing it");
    }
}

#[test]
fn a_late_joiner_is_handed_the_standing_bags_by_the_sync_walk() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    let mut warm = Vec::new();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut warm);
    }
    let (w0, w1) = (world_slot(&core, id_of(0)), world_slot(&core, id_of(1)));
    core.world.players[w0].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
        cond: 0,
        skin: 0,
    };
    core.world.players[w1].inv[0] = ItemStack {
        item: FILLER,
        count: 9,
        cond: 0,
        skin: 0,
    };
    let seen = fight_to_a_kill(&mut core, &stats, &mut clients);
    assert!(
        seen.iter()
            .any(|(_, m)| matches!(m, EventMsg::Death { .. })),
        "somebody died"
    );
    assert_eq!(core.world.backpacks.len(), 1);
    let bag = core.world.backpacks.entries()[0];

    // A third client arrives after the fact and hears nothing about it —
    // the broadcast is long gone. The walk is what must repair them.
    assert!(core.connect(2, id_of(2)));
    clients.push((2usize, ClientCore::new(SEED, id_of(2), 0)));
    let mut late = 0u32;
    let mut seen = Vec::new();
    for _ in 0..8 {
        late |= pump(&mut core, &stats, &mut clients, &mut seen)[2];
    }
    assert_ne!(late & APPLIED_BAGS, 0, "the bag walk never reached slot 2");
    assert!(
        seen.iter()
            .any(|(slot, m)| *slot == 2
                && matches!(m, EventMsg::BagSync { count, .. } if *count > 0)),
        "and it arrived as a sync batch, not as a replayed broadcast"
    );
    let c2 = &clients.iter().find(|(s, _)| *s == 2).unwrap().1;
    assert_eq!(c2.bags.len(), 1, "the late joiner sees the standing bag");
    assert_eq!(c2.bags.entries()[0].id, bag.id);
}

#[test]
fn a_bag_out_of_reach_is_refused_by_distance_alone() {
    let stats = ShardStats::default();
    let mut core = armed_core();
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];
    let mut warm = Vec::new();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut warm);
    }
    let (w0, w1) = (world_slot(&core, id_of(0)), world_slot(&core, id_of(1)));
    core.world.players[w0].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
        cond: 0,
        skin: 0,
    };
    core.world.players[w1].inv[1] = ItemStack {
        item: FILLER,
        count: 42,
        cond: 0,
        skin: 0,
    };
    fight_to_a_kill(&mut core, &stats, &mut clients);
    let bag = core.world.backpacks.entries()[0];

    // Walk the killer 20 m off and press loot. The action lane carries no
    // target, so there is nothing here to forge — distance is the whole
    // check, and it must hold.
    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].body = Body::at(
        SEED,
        hv(SEED),
        bag.qx as f32 * POS_XZ_Q + 20.0,
        bag.qz as f32 * POS_XZ_Q,
    );
    core.push_action(0, ActionMsg::Loot);
    let mut seen = Vec::new();
    pump(&mut core, &stats, &mut clients, &mut seen);
    assert_eq!(core.world.backpacks.len(), 1, "the bag is untouched");
    let w0 = world_slot(&core, id_of(0));
    assert!(
        core.world.players[w0].inv.iter().all(|s| s.item != FILLER),
        "and nothing crossed twenty metres into a pocket"
    );
    assert!(
        !seen
            .iter()
            .any(|(_, m)| matches!(m, EventMsg::BagRemoved { .. })),
        "nor did the wire announce a removal that never happened"
    );
}

/// **A despawn storm must not walk a joiner's bag walk back to the start**
/// (`NOW.md` §0n1 item 2).
///
/// The bag walk used to read upward and restart with a reset batch on every
/// removal that landed mid-walk; a fight's worth of bags timing out one
/// after another could hold a joiner at zero for as long as they kept
/// timing out. It reads tail-down now, like the piece and deploy walks.
///
/// Six batches of bags stood before anyone connects, their expiries
/// staggered six a tick, plus two batches that outlive the test so the
/// agreement at the end is not two empty sets. The second client joins
/// while the storm runs, and that is asserted rather than assumed.
#[test]
fn a_despawn_storm_leaves_every_bag_walk_standing() {
    use protocol::BAG_SYNC_BATCH;
    let stats = ShardStats::default();
    let mut core = armed_core();
    // A clock with room behind it, so a bag can be stood "earlier" than
    // now and expire a few ticks into the test.
    core.world.tick = 1_000;
    let now = core.world.tick;
    const DOOMED: usize = 6 * BAG_SYNC_BATCH;
    const STANDING: usize = 2 * BAG_SYNC_BATCH;
    const STRANGER: u32 = 0x0077_7777;
    let filler = {
        let mut items = [ItemStack::default(); sim_core::limits::INV_SLOTS];
        items[0] = ItemStack {
            item: FILLER,
            count: 1,
            cond: 0,
            skin: 0,
        };
        items
    };
    let rare = {
        let mut items = filler;
        items[0].item = SPEAR; // the fixture's long-lived half
        items
    };
    let short = core.world.backpack.lifetime_ticks(&filler) as u64;
    let long = core.world.backpack.lifetime_ticks(&rare) as u64;
    assert!(
        long > short + 64,
        "the fixture's two lifetimes are too close"
    );
    let (sx, sz) = ((SPAWN.0 / POS_XZ_Q) as i32, (SPAWN.1 / POS_XZ_Q) as i32);
    for k in 0..DOOMED + STANDING {
        let (qx, qz) = (sx + (k % 16) as i32 * 30, sz + (k / 16) as i32 * 30);
        let (items, stood) = if k < DOOMED {
            // Expires `3 + k / 6` ticks from now.
            (&filler, now + 3 + (k / 6) as u64 - short)
        } else {
            (&rare, now)
        };
        let w = &mut core.world;
        w.backpacks
            .stand_up(
                &w.backpack,
                qx,
                0,
                qz,
                STRANGER,
                items,
                stood,
                &mut w.events,
            )
            .expect("the bag stands");
    }
    let built = core.world.backpacks.len();
    assert_eq!(built, DOOMED + STANDING);

    assert!(core.connect(0, id_of(0)));
    let mut clients = vec![(0usize, ClientCore::new(SEED, id_of(0), 0))];
    const JOIN_AT: u64 = 2;
    const TICKS: u64 = 30;
    let mut mid_walk = 0usize;
    let mut mirror_was = [0usize; 2];
    let mut seen = Vec::new();
    for t in 0..TICKS {
        if t == JOIN_AT {
            assert!(core.connect(1, id_of(1)));
            clients.push((1usize, ClientCore::new(SEED, id_of(1), 0)));
        }
        let walking = t > JOIN_AT && {
            let c = &core.clients[1];
            c.bag_sync_reset || c.bag_sync_cursor > 0
        };
        let before = core.world.backpacks.len();
        pump(&mut core, &stats, &mut clients, &mut seen);
        let removed = before.saturating_sub(core.world.backpacks.len());
        if walking && removed > 0 {
            mid_walk += 1;
        }
        for (i, (slot, c)) in clients.iter().enumerate() {
            let held = c.bags.len();
            assert!(
                held + removed >= mirror_was[i],
                "client {slot} lost mirror ground at t={t}: {} → {held} with {removed} removed",
                mirror_was[i]
            );
            mirror_was[i] = held;
        }
    }

    assert!(
        mid_walk >= 2,
        "only {mid_walk} despawn ticks met the joiner mid-walk"
    );
    for (slot, _) in &clients {
        let resets = seen
            .iter()
            .filter(|(s, m)| s == slot && matches!(m, EventMsg::BagSync { reset: true, .. }))
            .count();
        assert_eq!(
            resets, 1,
            "client {slot} was sent {resets} bag reset batches"
        );
    }
    let mut world: Vec<u32> = core
        .world
        .backpacks
        .entries()
        .iter()
        .map(|b| b.id)
        .collect();
    world.sort_unstable();
    assert_eq!(world.len(), STANDING, "the storm took the wrong bags");
    for (slot, c) in &clients {
        let mut held: Vec<u32> = c.bags.entries().iter().map(|b| b.id).collect();
        held.sort_unstable();
        assert_eq!(held, world, "client {slot}'s bag mirror is not the world");
    }
    assert_eq!(
        ShardStats::get(&stats.bag_walk_completes),
        clients.len() as u64,
        "not every client's bag walk reached the end"
    );
    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}

/// **Bags are aimed, and a bag's owner is exempt** (`NOW.md` §0n1 item 2).
///
/// A kill at spawn with a third client 400 m off: the drop reaches the
/// killer and the victim and not the stranger. Then the victim stands
/// where the stranger stands and both resync, which clears their mirrors
/// and re-walks the store from the far anchor. The victim's walk must
/// bring its own bag back — the client finds `own_bag` by id in this
/// mirror, so the map would lose it otherwise — and the stranger's must
/// not. The victim is moved rather than respawned: the subject is where
/// its walk is aimed from, and a respawn is `bag_choice.rs`'s.
#[test]
fn a_bag_reaches_who_is_near_and_its_owner_wherever_they_stand() {
    use server::interest::{body_cm, d2_cm, PIECE_INTEREST_CM};
    let stats = ShardStats::default();
    let mut core = armed_core();
    for s in 0..3 {
        assert!(core.connect(s, id_of(s)));
    }
    let mut clients: Vec<(usize, ClientCore)> = (0..3)
        .map(|s| (s, ClientCore::new(SEED, id_of(s), 0)))
        .collect();
    let mut warm = Vec::new();
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut warm);
    }
    let far = Body::at(SEED, hv(SEED), SPAWN.0 + 400.0, SPAWN.1);
    let w2 = world_slot(&core, id_of(2));
    core.world.players[w2].body = far;
    for _ in 0..4 {
        pump(&mut core, &stats, &mut clients, &mut warm);
    }

    let (w0, w1) = (world_slot(&core, id_of(0)), world_slot(&core, id_of(1)));
    core.world.players[w0].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
        cond: 0,
        skin: 0,
    };
    core.world.players[w1].inv[0] = ItemStack {
        item: SPEAR,
        count: 1,
        cond: 0,
        skin: 0,
    };
    let skipped_before = ShardStats::get(&stats.bag_events_skipped);
    let seen = fight_to_a_kill(&mut core, &stats, &mut clients);
    assert_eq!(core.world.backpacks.len(), 1, "the death dropped one bag");
    let bag = core.world.backpacks.entries()[0];
    assert_eq!(bag.owner, id_of(1), "the bag is the victim's");
    let dropped: Vec<usize> = seen
        .iter()
        .filter_map(|(slot, m)| match m {
            EventMsg::BagDropped { id, .. } if *id == bag.id => Some(*slot),
            _ => None,
        })
        .collect();
    assert_eq!(
        dropped,
        vec![0, 1],
        "the drop must reach the two at the kill only"
    );
    assert_eq!(
        ShardStats::get(&stats.bag_events_skipped),
        skipped_before + 1,
        "the stranger's copy was not skipped by the filter"
    );
    assert!(
        clients[2].1.bags.is_empty(),
        "a client 400 m off holds the bag"
    );

    // The victim joins the stranger out there, and both resync.
    let w1 = world_slot(&core, id_of(1));
    core.world.players[w1].body = far;
    let mut seen = Vec::new();
    pump(&mut core, &stats, &mut clients, &mut seen);
    assert!(
        d2_cm(body_cm(far.qx, far.qz), body_cm(bag.qx, bag.qz))
            > PIECE_INTEREST_CM * PIECE_INTEREST_CM,
        "the far spot is not out of the bag's interest"
    );
    core.clients[1].ev_resync();
    core.clients[2].ev_resync();
    seen.clear();
    for _ in 0..6 {
        pump(&mut core, &stats, &mut clients, &mut seen);
    }
    for slot in [1usize, 2] {
        assert!(
            seen.iter()
                .any(|(s, m)| *s == slot && matches!(m, EventMsg::BagSync { reset: true, .. })),
            "client {slot}'s resync never reached its bag walk"
        );
    }
    assert_eq!(
        clients[1]
            .1
            .bags
            .entries()
            .iter()
            .map(|b| b.id)
            .collect::<Vec<_>>(),
        vec![bag.id],
        "the owner's far resync dropped its own bag"
    );
    assert!(
        clients[2].1.bags.is_empty(),
        "a stranger 400 m off was walked a bag outside its interest"
    );
    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}
