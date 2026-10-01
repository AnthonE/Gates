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

/// A dry standing point with open sea inside the drink reach.
pub fn shore() -> (f32, f32) {
    use sim_core::terrain;
    let haven = terrain::haven(SEED);
    let reach = sim_core::survival::DRINK_REACH_M;
    let c = terrain::ISLAND_SIZE * 0.5;
    for (dx, dz) in [(1.0f32, 0.0f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        for step in 0..(c as usize) {
            let (x, z) = (c + dx * step as f32, c + dz * step as f32);
            let wet = [
                (0.0, 0.0),
                (reach, 0.0),
                (-reach, 0.0),
                (0.0, reach),
                (0.0, -reach),
            ]
            .iter()
            .any(|(ox, oz)| terrain::height(SEED, x + ox, z + oz) < terrain::SEA_LEVEL);
            if wet && terrain::ground(SEED, &haven, x, z) > 0.3 {
                return (x, z);
            }
        }
    }
    panic!("fixture seed has no shore");
}

/// A level, dry clearing: no tree, rock or node within `radius_cells`
/// scatter cells of it, and the ground within a few metres of level over
/// it. Where two bodies meet in the arena with nothing between them.
pub fn clearing(radius_cells: i32) -> (f32, f32) {
    use sim_core::terrain::{self, Occupant, ScatterTable, CELL_SIZE};
    let haven = terrain::haven(SEED);
    let table = ScatterTable::alpha_default();
    let r = radius_cells;
    for cz in (20 + r..236 - r).step_by(2) {
        for cx in (20 + r..236 - r).step_by(2) {
            let (x, z) = ((cx as f32 + 0.5) * CELL_SIZE, (cz as f32 + 0.5) * CELL_SIZE);
            let h0 = terrain::ground(SEED, &haven, x, z);
            if h0 < 2.0 {
                continue;
            }
            let clear = (-r..=r).all(|dz| {
                (-r..=r).all(|dx| {
                    let s = terrain::scatter(SEED, &table, &haven, cx + dx, cz + dz);
                    let h = terrain::ground(
                        SEED,
                        &haven,
                        x + dx as f32 * CELL_SIZE,
                        z + dz as f32 * CELL_SIZE,
                    );
                    s.occupant == Occupant::None && h > 1.0 && (h - h0).abs() < 3.0
                })
            });
            if clear {
                return (x, z);
            }
        }
    }
    panic!("fixture seed has no clearing {r} cells wide");
}

/// A shard with the shipped tables, the dev spawn at `at`, and a player
/// `id` joined on slot 0.
pub fn shard(
    content: &content::Content,
    at: (f32, f32),
    wildlife: bool,
    id: u32,
) -> Box<ShardCore> {
    shard_with(content, at, wildlife, &[(0, id)])
}

/// The same, with these `(slot, id)` joins in this order. The order is the
/// order the world steps bodies in (the first free row), so two swings on
/// one tick land in it.
pub fn shard_with(
    content: &content::Content,
    at: (f32, f32),
    wildlife: bool,
    joins: &[(usize, u32)],
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
    for &(slot, id) in joins {
        assert!(core.connect(slot, id));
    }
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

/// A loot container of this kind (`Occupant::BarrelSlot`, `CrateSlot` or
/// `CacheSlot`) and a dry, level standing point a few metres from it with
/// nothing scattered between: `(cell, stand)`.
pub fn container(kind: sim_core::terrain::Occupant) -> ((u16, u16), (f32, f32)) {
    use sim_core::terrain::{self, Occupant, ScatterTable};
    let haven = terrain::haven(SEED);
    let table = ScatterTable::alpha_default();
    for cz in 16..terrain::CELLS_PER_SIDE - 16 {
        for cx in 16..terrain::CELLS_PER_SIDE - 16 {
            let slot = terrain::scatter(SEED, &table, &haven, cx, cz);
            if slot.occupant != kind {
                continue;
            }
            for (dx, dz) in [(0.0f32, -4.0f32), (0.0, 4.0), (-4.0, 0.0), (4.0, 0.0)] {
                let at = (slot.x + dx, slot.z + dz);
                let floor = terrain::ground(SEED, &haven, at.0, at.1);
                let open = (1..=3).all(|k| {
                    let f = k as f32 / 4.0;
                    let (x, z) = (slot.x + dx * f, slot.z + dz * f);
                    let c = terrain::scatter(
                        SEED,
                        &table,
                        &haven,
                        (x / terrain::CELL_SIZE).floor() as i32,
                        (z / terrain::CELL_SIZE).floor() as i32,
                    );
                    c.occupant == Occupant::None || c.occupant == kind
                });
                if floor > 1.0 && (floor - slot.y).abs() < 0.4 && open {
                    return ((cx as u16, cz as u16), at);
                }
            }
        }
    }
    panic!("fixture seed has no {kind:?} with open ground beside it");
}
