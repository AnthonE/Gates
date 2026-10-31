//! Sweep the camp fires a list of wallets left on open ground out of a COPY
//! of a world file (`SCALE.md`: agents' fires filled `MAX_BOXES`). Unlit
//! fires that no stocked hearth covers go, with what was in them. Without
//! `--write` it only counts. Stop the shard, sweep a copy, swap it in.
//!
//! cargo run --release -p server --example sweep_fires -- <copy of island.world> <content dir> <seed> <wallets file> [--write]

use std::path::Path;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (path, content_dir, seed) = (&a[0], &a[1], a[2].parse::<u64>().expect("seed"));
    let wallets: std::collections::BTreeSet<String> = std::fs::read_to_string(&a[3])
        .expect("wallets file")
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .collect();
    let write = a.iter().any(|s| s == "--write");
    let content = content::Content::load_dir(Path::new(content_dir)).expect("content");
    let tables = server::net::bake_all(&content).expect("bake");
    let mut w = sim_core::world::World::new(seed);
    w.gather = tables.gather;
    w.craft = tables.craft;
    w.build = tables.build;
    w.deploy = tables.deploy;
    w.combat = tables.combat;
    w.backpack = tables.backpack;
    w.survival = tables.survival;
    w.cook = tables.cook;
    w.loot = tables.loot;
    let digest = sim_core::probe::probe_terrain(seed);
    let (mut boot, found) = server::worldfile::open(
        Path::new(path),
        &mut w,
        seed,
        content.hash(),
        content.layout_hash(),
        digest,
        1800,
    )
    .expect("open");
    let ids: std::collections::BTreeSet<u32> = boot
        .idents
        .iter()
        .filter(|(key, _)| {
            wallets.contains(&String::from_utf8_lossy(key.as_bytes()).to_lowercase())
        })
        .map(|(_, id)| *id)
        .collect();
    println!(
        "tick {} · {} of {} wallets have a body id · containers {} / {}",
        found.tick,
        ids.len(),
        wallets.len(),
        w.deploys.boxes().len(),
        sim_core::limits::MAX_BOXES
    );
    let n = w
        .deploys
        .sweep_loose_fires(&w.deploy, &w.pieces, |owner| ids.contains(&owner));
    println!(
        "swept {n} fires · containers now {} / {}",
        w.deploys.boxes().len(),
        sim_core::limits::MAX_BOXES
    );
    if write {
        let mut buf = vec![0u8; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
        let len = w.save_world(&mut buf).expect("encode");
        boot.file
            .write(found.tick, &buf[..len], &boot.idents)
            .expect("write");
        println!("wrote {path}");
    }
}
