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
    let hp0 = core.world.players[s].hp;
    for _ in 0..30 * 10 {
        core.world.tick(&[]);
    }
    assert!(
        core.world.players[s].hp < hp0 || core.world.players[s].hp == 0,
        "{said}: the turret never hit the stranger (hp {hp0} → {})",
        core.world.players[s].hp
    );
}
