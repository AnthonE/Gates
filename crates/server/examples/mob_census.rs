//! Where the animals are:
//! `cargo run -p server --example mob_census [SEED] [SECONDS] [ARC_HOUR] [lit]`.
//!
//! Boots the shipped content into a world, runs it, and prints every live
//! animal by species with its position, then the densest 200 m cells for
//! each. It is how a shot list (`ci/film/`) finds a meadow with pigs in it.
//! `ARC_HOUR` and `lit` are the shard's `dev_arc_hour` and `dev_works_lit`:
//! the animals hatch off the world clock, so a late wipe puts them elsewhere.

use sim_core::mob::{MOB_PIG, MOB_STAG, MOB_WOLF};
use sim_core::movement::POS_XZ_Q;
use std::collections::BTreeMap;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let seed: u64 = args
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20260731);
    let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30);
    let content = content::Content::load_dir(std::path::Path::new("content")).expect("content");
    let t = server::net::bake_all(&content).expect("bake");
    let hour: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut w = sim_core::world::World::new(seed);
    w.tick = hour * sim_core::blight::TICKS_PER_HOUR;
    w.works_def = t.works;
    if args.get(3).map(String::as_str) == Some("lit") {
        for d in w.works_def.defs.iter_mut() {
            d.opens_at = 0;
            d.fallback_at = 0;
        }
    }
    w.works.refresh(&w.works_def);
    w.mob = t.mobs;
    w.gather = t.gather;
    w.combat = t.combat;
    w.loot = t.loot;
    w.heli_def = t.heli;
    for _ in 0..secs * u64::from(sim_core::limits::TICK_HZ) {
        w.tick(&[]);
    }
    let h = &w.haven;
    println!("haven pad {:.0},{:.0}", h.x, h.z);
    println!(
        "town (THE GATE) {:.0},{:.0} live {}",
        h.town.x, h.town.z, h.town.live
    );
    println!(
        "ziggurat {:.0},{:.0} live {}",
        h.ziggurat.x, h.ziggurat.z, h.ziggurat.live
    );
    for (i, s) in h.minor.iter().enumerate() {
        println!("minor site {i} {:.0},{:.0}", s.x, s.z);
    }
    for m in h.marks.iter().filter(|m| m.live) {
        println!("landmark {:?} {:.0},{:.0}", m.kind, m.x, m.z);
    }
    let mut cells: [BTreeMap<(i32, i32), u32>; 3] = Default::default();
    for (slot, m) in w.mobs.m.iter().enumerate() {
        if !m.alive || m.kind > MOB_STAG {
            continue;
        }
        let (x, z) = (m.body.qx as f32 * POS_XZ_Q, m.body.qz as f32 * POS_XZ_Q);
        let name = ["pig", "wolf", "stag"][m.kind as usize];
        println!("{name:5} slot {slot:3}  {x:7.1},{z:7.1}");
        *cells[m.kind as usize]
            .entry(((x / 200.0) as i32, (z / 200.0) as i32))
            .or_default() += 1;
    }
    for (kind, name) in [(MOB_PIG, "pig"), (MOB_WOLF, "wolf"), (MOB_STAG, "stag")] {
        let mut v: Vec<_> = cells[kind as usize].iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        let top: Vec<String> = v
            .iter()
            .take(6)
            .map(|((cx, cz), n)| format!("{n}@{},{}", cx * 200 + 100, cz * 200 + 100))
            .collect();
        println!("{name}: {}", top.join("  "));
    }
}
