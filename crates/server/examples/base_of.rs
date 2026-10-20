//! Which hearths and locks a body id is on, in a COPY of a world file, and
//! where that body lies. With `--grant <from> <to>` it also puts `to` on
//! every list `from` is on and writes the copy back (shard stopped, then
//! swap the copy in): the recovery for a session played as a guest by
//! mistake.
//!
//! cargo run --release -p server --example base_of -- <copy of island.world> <content dir> <seed> <id>... [--grant FROM TO]

use sim_core::movement::POS_XZ_Q;
use std::path::Path;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (path, content_dir, seed) = (&a[0], &a[1], a[2].parse::<u64>().expect("seed"));
    let grant = a.iter().position(|s| s == "--grant").map(|i| {
        (
            a[i + 1].parse::<u32>().expect("from"),
            a[i + 2].parse::<u32>().expect("to"),
        )
    });
    let end = a.iter().position(|s| s == "--grant").unwrap_or(a.len());
    let ids: Vec<u32> = a[3..end].iter().map(|s| s.parse().expect("id")).collect();
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
    println!("tick {} bodies {}", found.tick, found.bodies);
    let town = &w.haven.town;
    println!("THE GATE at ({:.0}, {:.0})", town.x, town.z);
    for (key, id) in &boot.idents {
        let k = String::from_utf8_lossy(key.as_bytes()).to_lowercase();
        if k.contains("b1ab332993") {
            println!("wallet {k} -> body {id}");
        }
    }
    for p in w.players.iter().filter(|p| p.id != 0) {
        if ids.contains(&p.id) || p.id == 0 {
            println!(
                "body {} active {} sleeping {} hp {} at ({:.1}, {:.1})",
                p.id,
                p.active,
                p.sleeping,
                p.hp,
                p.body.qx as f32 * POS_XZ_Q,
                p.body.qz as f32 * POS_XZ_Q
            );
        }
    }
    for h in w.deploys.hearths() {
        let (x, z) = w.deploys.hearth_xz(h);
        let mine = h.crew.members().iter().any(|m| ids.contains(m)) || ids.contains(&h.owner);
        let near = ((x - town.x).powi(2) + (z - town.z).powi(2)).sqrt();
        if mine {
            println!(
                "HEARTH at ({x:.1}, {z:.1}) level {} owner {} crew {:?} stock {:?} · {near:.0} m from the gate",
                h.level,
                h.owner,
                h.crew.members(),
                h.stock
            );
        }
    }
    for l in w.deploys.locks() {
        if l.auth.members().iter().any(|m| ids.contains(m)) || ids.contains(&l.owner) {
            println!(
                "LOCK cell ({}, {}) level {} loc {} owner {} key {} locked {} auth {:?}",
                l.cx,
                l.cz,
                l.level,
                l.loc,
                l.owner,
                l.is_key(),
                l.locked,
                l.auth.members()
            );
        }
    }
    if let Some((from, to)) = grant {
        let (h, l) = w.deploys.grant_like(from, to);
        let mut buf = vec![0u8; sim_core::worldsave::WORLD_SAVE_MAX_BYTES];
        let n = w.save_world(&mut buf).expect("encode");
        boot.file
            .write(found.tick, &buf[..n], &boot.idents)
            .expect("write");
        println!("granted {to} on {h} hearth(s) and {l} lock(s) beside {from}; wrote {path}");
    }
}
