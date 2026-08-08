//! `mob_wire` — an animal reaches a client, and leaves when it dies.
//!
//! The slice's one end-to-end claim: `sim-core` moves a pig, the AOI pass
//! notices it, the priority fill puts it in a datagram, and a real client
//! view decodes it back to the exact quanta the sim holds. Everything here
//! runs `ShardCore` directly — no sockets, no sleeps, no clock (`CLAUDE.md`:
//! a gate that waits on a clock is not a gate on this box).
//!
//! What it is deliberately NOT testing: that a pig looks like a pig. That is
//! `client/tests/mob_mesh.rs`, and it is arithmetic there for the same
//! reason it is arithmetic here.

use server::core::{Lane, ShardCore};
use server::stats::ShardStats;
use server::view::{Applied, ClientView};
use sim_core::limits::{MOB_ID_TAG, SNAPSHOT_INTERVAL_TICKS};
use sim_core::mob::{self, MobContent};
use sim_core::movement::POS_XZ_Q;

const SEED: u64 = 0xB1D_6E75;

fn id_of(slot: usize) -> u32 {
    (1 << 8) | slot as u32
}

/// Run to the next snapshot cadence tick and return this client's datagram.
fn snapshot(core: &mut ShardCore, stats: &ShardStats, slot: usize) -> Vec<u8> {
    loop {
        let mut got: Option<Vec<u8>> = None;
        core.tick(stats, |lane, s, bytes| {
            if lane == Lane::Snapshot && s == slot {
                got = Some(bytes.to_vec());
            }
            true
        });
        if core.world.tick.is_multiple_of(SNAPSHOT_INTERVAL_TICKS) {
            if let Some(bytes) = got {
                return bytes;
            }
        }
    }
}

/// One client standing `away` metres from the first live animal on the
/// island. Returns the core and that animal's roster slot.
fn core_beside_a_pig(stats: &ShardStats, away: f32) -> (ShardCore, usize) {
    let mut core = ShardCore::new(SEED);
    core.world.mob = MobContent::probe_fixture();
    core.world.combat = sim_core::combat::CombatContent::probe_fixture();
    // Hatch the roster before seating anyone, so the spawn can be placed
    // relative to a real animal rather than the other way round.
    core.tick(stats, |_, _, _| true);
    let slot = core
        .world
        .mobs
        .m
        .iter()
        .position(|m| m.alive)
        .expect("armed content hatches the roster on the first tick");
    let m = &core.world.mobs.m[slot];
    core.world.dev_spawn = Some((
        m.body.qx as f32 * POS_XZ_Q + away,
        m.body.qz as f32 * POS_XZ_Q,
    ));
    assert!(core.connect(0, id_of(0)), "connect");
    core.tick(stats, |_, _, _| true);
    (core, slot)
}

#[test]
fn a_client_beside_an_animal_receives_it() {
    let stats = ShardStats::default();
    // 30 m: inside AOI (176 m), outside the fixture's 12 m spook radius, so
    // the animal is not bolting out of the frame while this runs.
    let (mut core, slot) = core_beside_a_pig(&stats, 30.0);
    let mut view = ClientView::new();

    let mut seen = false;
    for _ in 0..8 {
        let bytes = snapshot(&mut core, &stats, 0);
        match view.apply(&bytes).expect("server datagram decodes") {
            Applied::Ok { .. } => {}
            other => panic!("{other:?}"),
        }
        if let Some(e) = view.get(mob::mob_id(slot)) {
            seen = true;
            let m = &core.world.mobs.m[slot];
            // Quantize both sides: equality, never tolerance.
            assert_eq!(e.qx, m.body.qx);
            assert_eq!(e.qy, m.body.qy);
            assert_eq!(e.qz, m.body.qz);
            assert_eq!(e.yaw, m.yaw);
            // The two fields an animal does not have, answered rather than
            // defaulted: dormancy is not the `sleeping` bit, and nothing
            // about a pig looks up.
            assert!(!e.sleeping);
            assert_eq!(e.pitch, 0);
        }
    }
    assert!(seen, "a pig 30 m away never reached the client");

    // Every entity in the view is either a player or a roster slot, and the
    // tag is the only thing that says which. A client that guessed wrong
    // here draws a human being standing in a field — the failure `PROTO_VER`
    // 28 exists to make impossible.
    for (id, _) in view.entities.iter() {
        match mob::slot_of_id(*id) {
            Some(s) => assert!(core.world.mobs.m[s].alive, "a dead animal is still in view"),
            None => {
                assert_eq!(*id & MOB_ID_TAG, 0);
                assert!(core.world.players.iter().any(|p| p.active && p.id == *id));
            }
        }
    }
}

#[test]
fn a_killed_animal_leaves_the_clients_world() {
    let stats = ShardStats::default();
    let (mut core, slot) = core_beside_a_pig(&stats, 30.0);
    let mut view = ClientView::new();
    for _ in 0..8 {
        let bytes = snapshot(&mut core, &stats, 0);
        view.apply(&bytes).expect("decodes");
    }
    assert!(view.get(mob::mob_id(slot)).is_some(), "nothing to kill");

    // Killed in the sim rather than through a swing: this test is about the
    // *wire's* answer to a death, and arranging a melee connection would be
    // testing `mob::strike`, which `sim-core/tests/mob.rs` already owns.
    core.world.mobs.m[slot].alive = false;
    core.world.mobs.m[slot].hp = 0;
    core.world.mobs.m[slot].respawn_at = u64::MAX;

    let mut gone = false;
    for _ in 0..8 {
        let bytes = snapshot(&mut core, &stats, 0);
        view.apply(&bytes).expect("decodes");
        if view.get(mob::mob_id(slot)).is_none() {
            gone = true;
            break;
        }
    }
    assert!(
        gone,
        "a dead animal stayed in the client's entity set — a corpse that never moves again"
    );
}

#[test]
fn an_animal_past_the_aoi_band_is_never_sent() {
    let stats = ShardStats::default();
    // 400 m: past the 208 m exit radius by nearly double.
    let (mut core, slot) = core_beside_a_pig(&stats, 400.0);
    let mut view = ClientView::new();
    for _ in 0..8 {
        let bytes = snapshot(&mut core, &stats, 0);
        view.apply(&bytes).expect("decodes");
        assert!(
            view.get(mob::mob_id(slot)).is_none(),
            "an animal 400 m away was inside a 176 m interest set"
        );
    }
}
