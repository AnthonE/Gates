//! The lockstep island (`server::lockstep`, `jev-bot --bench --warp`): a
//! run is a function of its inputs. The same fleet on the same variant ends
//! on the same world and the same standings, tick for tick; another variant
//! plays out otherwise.

mod common;

use server::explorer::{Survivor, SurvivorOpts};
use server::lockstep::{wallet, Island};
use server::mind::{Mind, MindConfig, Scripted};
use sim_core::limits::TICK_HZ;
use std::time::Instant;

/// Two scripted agents for `minutes` of island time on `variant`: the
/// world's hash and each seat's alive ticks.
fn play(variant: u64, minutes: u64, t0: Instant) -> (u64, Vec<u32>) {
    let content = common::content();
    let cfg = server::agent_demo::local_config().unwrap();
    assert_eq!(
        cfg.seed,
        common::SEED,
        "the fixture seed is the local island's"
    );
    let mut island = Island::new(&content, &cfg, common::haven(), 48, t0, variant).unwrap();
    for i in 0..2 {
        let mind = Mind::inline(Scripted::default(), MindConfig::default()).unwrap();
        island
            .seat(Survivor::with(mind, SurvivorOpts::default()), &wallet(i))
            .unwrap();
    }
    island.run(minutes * 60 * u64::from(TICK_HZ));
    for i in 0..2 {
        assert!(
            island.row(i).is_some(),
            "seat {i} has no standings row after {minutes} minutes"
        );
    }
    let alive = island.seats.iter().map(|s| s.alive_ticks).collect();
    (island.core.world.state_hash(), alive)
}

#[test]
fn a_lockstep_island_replays_and_another_variant_does_not() {
    let t0 = Instant::now();
    let a = play(0, 2, t0);
    assert!(
        a.1.iter().all(|&t| t > 0),
        "a seated agent never stood: {:?}",
        a.1
    );
    assert_eq!(play(0, 2, t0), a, "the same island played out otherwise");
    assert_ne!(play(1, 2, t0).0, a.0, "another variant played the same");
}
