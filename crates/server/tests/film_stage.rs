//! The footage turret (`server::film`) stands on the shipped content and
//! fires: built, powered, loaded, and a stranger in reach takes rounds.

mod common;

use common::{content, haven, SEED};
use server::core::ShardCore;
use sim_core::movement::Body;
use sim_core::world::Command;

#[test]
fn the_film_turret_stands_powered_and_shoots_a_stranger() {
    const STRANGER: u32 = 77;
    let content = content();
    let t = server::net::bake_all(&content).unwrap();
    let mut core = Box::new(ShardCore::with_haven(SEED, haven()));
    server::net::install_tables(&mut core, t);
    let (x, z) = common::clearing(3);
    core.world.dev_spawn = Some((x, z));
    let staged = server::film::stage_turret(&mut core.world, x, z, 6).unwrap();
    let (said, (tx, tz)) = (staged.said, staged.at);
    core.world.tick(&[Command::Join { id: STRANGER }]);
    let s = core
        .world
        .players
        .iter()
        .position(|p| p.id == STRANGER)
        .unwrap();
    // Two cells east of the turret: 6 m, well inside its 30.
    core.world.players[s].body = Body::at(SEED, &core.world.haven, tx + 6.0, tz);
    // Rounds in the turret's box: the turret's own spending is the proof,
    // not the stranger's health, which the cold and hunger take too.
    let rounds = |w: &sim_core::world::World| -> u32 {
        let i = w
            .deploys
            .boxes()
            .iter()
            .position(|b| {
                let (x, z) = b.xz();
                (x - tx).abs() < 2.0 && (z - tz).abs() < 2.0
            })
            .expect("the turret's box");
        (1..=6).map(|k| w.deploys.box_slot(i, k).count as u32).sum()
    };
    let before = rounds(&core.world);
    for _ in 0..30 * 10 {
        core.world.tick(&[]);
    }
    let after = rounds(&core.world);
    assert!(
        after < before,
        "{said}: the turret spent no rounds ({before} → {after})"
    );
}

/// Ground heights for a shot list: `FILM_POINTS="x,z x,z …" cargo test -p
/// server --test film_stage heights -- --ignored --nocapture`.
#[test]
#[ignore]
fn heights() {
    let haven = haven();
    for p in std::env::var("FILM_POINTS")
        .unwrap_or_default()
        .split_whitespace()
    {
        let (x, z) = p.split_once(',').unwrap();
        let (x, z) = (x.parse::<f32>().unwrap(), z.parse::<f32>().unwrap());
        println!(
            "{x},{z} ground {:.1}",
            sim_core::terrain::ground(SEED, &haven, x, z)
        );
    }
}
