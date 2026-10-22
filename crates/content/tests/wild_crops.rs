//! The wild crops by the lakes pay what `gatherables.toml` says: a wild corn
//! pick is 2 corn and a corn seed, a wild pumpkin 1 pumpkin and its seed.

use std::path::PathBuf;

use content::Content;
use sim_core::gather::cell_key;
use sim_core::terrain::{self, Occupant, ScatterTable, CELLS_PER_SIDE, SEA_LEVEL};
use sim_core::world::{Command, World, EV_GATHER, EV_SLOT_HARVESTED};

const SEED: u64 = 42;

fn content_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content")
}

/// The first `want` on the island with dry ground a metre off it, that
/// ground, and its cell.
fn find(want: Occupant) -> ((f32, f32), (i32, i32)) {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(SEED);
    for cz in 0..CELLS_PER_SIDE {
        for cx in 0..CELLS_PER_SIDE {
            let s = terrain::scatter(SEED, &table, &haven, cx, cz);
            if s.occupant != want {
                continue;
            }
            for (dx, dz) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
                let (px, pz) = (s.x + dx, s.z + dz);
                let py = terrain::ground(SEED, &haven, px, pz);
                if py > SEA_LEVEL + 0.5 && (py - s.y).abs() < 0.5 {
                    return ((px, pz), (cx, cz));
                }
            }
        }
    }
    panic!("seed {SEED} has no {want:?} on dry ground");
}

/// What one pick at `want` pays, `(item, count)` per `EV_GATHER`.
fn pick(c: &Content, want: Occupant) -> Vec<(u16, u32)> {
    let (pos, (cx, cz)) = find(want);
    let mut w = Box::new(World::new(SEED));
    w.gather = c.bake_gather().expect("shipped gather table must bake");
    w.dev_spawn = Some(pos);
    w.tick(&[Command::Join { id: 1 }]);
    w.tick(&[Command::Pick {
        id: 1,
        cell: cell_key(cx as u16, cz as u16),
    }]);
    let mut paid = Vec::new();
    let mut harvested = 0;
    for e in w.events.entries() {
        match e.code {
            EV_GATHER => paid.push(((e.b >> 16) as u16, e.b & 0xFFFF)),
            EV_SLOT_HARVESTED => {
                assert_eq!(e.b, want as u32, "the event names the crop");
                harvested += 1;
            }
            _ => {}
        }
    }
    assert_eq!(harvested, 1, "one pick spends the {want:?}");
    paid
}

#[test]
fn a_wild_corn_pick_pays_two_corn_and_a_seed() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let corn = c.item_index("item.corn").unwrap();
    let seed = c.item_index("item.corn_seed").unwrap();
    assert_eq!(pick(&c, Occupant::WildCorn), vec![(corn, 2), (seed, 1)]);
}

#[test]
fn a_wild_pumpkin_pick_pays_a_pumpkin_and_a_seed() {
    let c = Content::load_dir(&content_dir()).expect("shipped content must load");
    let pumpkin = c.item_index("item.pumpkin").unwrap();
    let seed = c.item_index("item.pumpkin_seed").unwrap();
    assert_eq!(
        pick(&c, Occupant::WildPumpkin),
        vec![(pumpkin, 1), (seed, 1)]
    );
}
