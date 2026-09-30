//! Lockstep scenery shared by the agent's integration tests: the fixture
//! seed, a standing point with a tree and a stone node, and a shard with
//! every table a real one installs. Test code, so it may name the world.

#![allow(dead_code)]

use server::core::ShardCore;

pub const SEED: u64 = 20260731;

pub fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn content() -> content::Content {
    content::Content::load_dir(&root().join("content")).unwrap()
}

/// A level standing point near a stone node with a tree close by.
pub fn scene() -> (f32, f32) {
    use sim_core::terrain::{self, Occupant, ScatterTable};
    let haven = terrain::haven(SEED);
    let table = ScatterTable::alpha_default();
    for cz in 20..236 {
        for cx in 20..236 {
            let stone = terrain::scatter(SEED, &table, &haven, cx, cz);
            if stone.occupant != Occupant::StoneNode {
                continue;
            }
            let tree = (-3..=3).any(|dz| {
                (-3..=3).any(|dx| {
                    terrain::scatter(SEED, &table, &haven, cx + dx, cz + dz).occupant
                        == Occupant::Tree
                })
            });
            let at = (stone.x, stone.z - 6.0);
            let floor = terrain::ground(SEED, &haven, at.0, at.1);
            if tree && floor > 1.0 && (floor - stone.y).abs() < 0.5 {
                return at;
            }
        }
    }
    panic!("fixture seed has no stone node beside a tree");
}

/// A shard with the shipped tables, the dev spawn at `at`, and a player
/// `id` joined on slot 0.
pub fn shard(
    content: &content::Content,
    at: (f32, f32),
    wildlife: bool,
    id: u32,
) -> Box<ShardCore> {
    let t = server::net::bake_all(content).unwrap();
    let mut core = Box::new(ShardCore::new(SEED));
    core.world.gather = t.gather;
    core.world.craft = t.craft;
    core.world.build = t.build;
    core.world.deploy = t.deploy;
    core.world.combat = t.combat;
    core.world.backpack = t.backpack;
    core.world.survival = t.survival;
    core.world.cook = t.cook;
    core.world.spawn_kit = t.spawn_kit;
    core.world.loot = t.loot;
    core.world.research = t.research;
    if wildlife {
        core.world.mob = t.mobs;
    }
    core.catalog = t.catalog;
    core.world.dev_spawn = Some(at);
    assert!(core.connect(0, id));
    core
}

/// A stack of one of the named item, as a fresh one comes off the bench.
pub fn stack(content: &content::Content, id: &str) -> sim_core::gather::ItemStack {
    let item = content.item_index(id).unwrap();
    let catalog = server::net::bake_all(content).unwrap().catalog;
    sim_core::gather::ItemStack {
        item,
        count: 1,
        cond: catalog.cond_max(item as usize),
        skin: 0,
    }
}
