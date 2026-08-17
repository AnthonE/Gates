//! The craft-on-the-wire gate (M1): `ShardCore` ↔ `ClientCore` through
//! real encoded bytes — the recipe table drips to a joiner, a craft
//! request consumes inputs and announces the queue, a completed unit pays
//! the inventory mirror and toasts, a cancel refunds, a station-gated
//! recipe refuses with its reason, and none of it leaks to a bystander.
//! Deterministic, no sockets; asserts are structural and exact.

use client_core::core::{
    ClientCore, APPLIED_CRAFT_DONE, APPLIED_CRAFT_Q, APPLIED_CRAFT_REFUSED, APPLIED_RECIPES,
};
use protocol::{ActionMsg, ItemCatalog};
use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use sim_core::craft::{CraftContent, REFUSE_BLUEPRINT};
use sim_core::gather::GatherContent;

const SEED: u64 = 20_260_731;
/// The canonical dev spawn point, guarded walkable in sim-core
/// `world::tests` — craft needs standing room, not a tree.
const SPAWN: (f32, f32) = (1024.0, 1024.0);

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

fn probe_catalog() -> ItemCatalog {
    let mut cat = ItemCatalog::EMPTY;
    cat.count = 8;
    for i in 0..8usize {
        let name = [b'P', b'0' + i as u8];
        cat.set(i, &name).unwrap();
    }
    cat
}

/// One lockstep pump (the gather_wire shape): inputs in, tick, events and
/// snapshots back out through real bytes. Returns per-slot APPLIED flags.
fn pump(core: &mut ShardCore, stats: &ShardStats, clients: &mut [(usize, ClientCore)]) -> [u32; 4] {
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

/// Round-trip a decoded action the way the net loop does.
fn act(core: &mut ShardCore, slot: usize, a: ActionMsg) {
    assert!(core.wants_action(slot), "hand should be open");
    core.push_action(slot, a);
    assert!(
        !core.wants_action(slot),
        "hand should be holding the action"
    );
}

#[test]
fn craft_rides_the_wire() {
    let fixture = CraftContent::probe_fixture();
    let stats = ShardStats::default();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = GatherContent::probe_fixture();
    core.world.craft = fixture;
    core.world.dev_spawn = Some(SPAWN);
    core.catalog = probe_catalog();
    assert!(core.connect(0, id_of(0)));
    assert!(core.connect(1, id_of(1)));
    let mut clients = vec![
        (0usize, ClientCore::new(SEED, id_of(0), 0)),
        (1usize, ClientCore::new(SEED, id_of(1), 0)),
    ];

    // Join drip: catalog, recipes, sync reset. Three pumps cover the
    // one-message-per-kind-per-tick cadence with room to spare.
    let mut recipe_flags = 0u32;
    for _ in 0..4 {
        recipe_flags |= pump(&mut core, &stats, &mut clients)[0];
    }
    assert_ne!(recipe_flags & APPLIED_RECIPES, 0, "recipes never dripped");
    for (_, c) in &clients {
        assert_eq!(c.recipes.recipe_count, fixture.recipe_count);
        assert_eq!(c.recipes_have, fixture.recipe_count);
        for i in 0..fixture.recipe_count as usize {
            assert_eq!(c.recipes.recipes[i], fixture.recipes[i], "row {i} drifted");
        }
    }

    // Grant the crafter its inputs server-side (gather_wire covers how
    // resources are earned; this gate is about the craft lane).
    let w0 = world_slot(&core, id_of(0));
    core.world.players[w0].inv[0] = sim_core::gather::ItemStack {
        item: 0,
        count: 20,
        cond: 0,
    };

    // Craft 2 of recipe 0 (3 × item0 per unit, 2 ticks each).
    act(
        &mut core,
        0,
        ActionMsg::Craft {
            recipe: 0,
            count: 2,
        },
    );
    let flags = pump(&mut core, &stats, &mut clients);
    assert_ne!(flags[0] & APPLIED_CRAFT_Q, 0, "queue announce missing");
    let c0 = &clients[0].1;
    assert_eq!((c0.jobs_count, c0.jobs[0]), (1, (0, 2)), "queue mismatch");
    assert!(
        (1..=2).contains(&c0.craft_eta_ticks),
        "eta {} outside the head unit's window",
        c0.craft_eta_ticks
    );
    assert!(core.wants_action(0), "the tick should consume the action");

    // Two more ticks: the first unit completes — toast, queue decrement,
    // and the output lands in the mirrored inventory.
    let mut done_flags = 0u32;
    for _ in 0..2 {
        done_flags |= pump(&mut core, &stats, &mut clients)[0];
    }
    assert_ne!(done_flags & APPLIED_CRAFT_DONE, 0, "no completion toast");
    let c0 = &mut clients[0].1;
    assert_eq!(c0.pop_craft_toast(), Some((2, 2)), "item 2 × 2 landed");
    assert_eq!(c0.jobs[0], (0, 1), "one unit left");
    let inv_mirror = c0.inv;
    assert_eq!(inv_mirror, core.world.players[w0].inv, "mirror drifted");
    assert_eq!(
        sim_core::craft::inv_count(&inv_mirror, 0),
        14,
        "6 of item 0 consumed at enqueue"
    );
    assert_eq!(sim_core::craft::inv_count(&inv_mirror, 2), 2);

    // Cancel the rest: the remaining unit's inputs come back and the
    // empty-queue announce clears the client view.
    act(&mut core, 0, ActionMsg::CraftCancel { index: 0 });
    let flags = pump(&mut core, &stats, &mut clients);
    assert_ne!(flags[0] & APPLIED_CRAFT_Q, 0, "cancel announce missing");
    let c0 = &clients[0].1;
    assert_eq!(c0.jobs_count, 0, "queue should be empty");
    assert_eq!(
        sim_core::craft::inv_count(&c0.inv, 0),
        17,
        "3 of item 0 refunded"
    );

    // A gated recipe refuses with its reason, on the wire.
    //
    // Fixture row 2 is gated **twice** — a station and a blueprint — and
    // since research v0 the blueprint is what a player hears, because a
    // station refusal would send someone who needs a research table back
    // to the bench they are standing at (`craft::enqueue` states the
    // order; `sim-core/tests/research.rs` asserts it both ways). What this
    // test owns is the wire: a reason computed in the sim reaches the
    // right client as the same integer.
    act(
        &mut core,
        0,
        ActionMsg::Craft {
            recipe: 2,
            count: 1,
        },
    );
    let flags = pump(&mut core, &stats, &mut clients);
    assert_ne!(flags[0] & APPLIED_CRAFT_REFUSED, 0, "refusal never arrived");
    let c0 = &mut clients[0].1;
    assert_eq!(c0.pop_craft_refusal(), Some(REFUSE_BLUEPRINT as u8));
    assert_eq!(c0.jobs_count, 0, "refused request must not queue");

    // The bystander heard the recipe drip but none of the crafter's
    // private traffic.
    let c1 = &mut clients[1].1;
    assert_eq!(c1.jobs_count, 0, "queue leaked to a bystander");
    assert_eq!(c1.pop_craft_toast(), None, "toast leaked to a bystander");
    assert_eq!(
        c1.pop_craft_refusal(),
        None,
        "refusal leaked to a bystander"
    );
    assert!(
        c1.inv.iter().all(|s| s.count == 0),
        "inventory leaked to a bystander"
    );

    // Nothing in this run tripped an encoder range check.
    assert_eq!(ShardStats::get(&stats.encode_range_errors), 0);
}
