//! The parity/golden probe surface: `extern "C"` exports that build to
//! wasm32-unknown-unknown with no bindgen, driven from node by
//! `ci/parity.mjs` and from native by `examples/probe.rs`. Same functions,
//! both targets, byte-equal answers — that is `test_parity_wasm`
//! (DESIGN.md §12) and the wasm half of `test_terrain_golden`
//! (TERRAIN.md §0).

use crate::bots::bot_frame;
use crate::rng::{splitmix64, Pcg32};
use crate::terrain::{self, ScatterTable};
use crate::world::{Command, World};
use xxhash_rust::xxh3::Xxh3;

/// Golden terrain fingerprint: 64×64 heights sampled on a 32 m grid, then
/// the 256 scatter cells of the island-center 16×16 block (cells 120..136;
/// the corner block would be all sea and pin nothing). Hashes f32 bit
/// patterns — bit-identical or bust.
#[no_mangle]
pub extern "C" fn probe_terrain(seed: u64) -> u64 {
    let mut h = Xxh3::new();
    for gz in 0..64i32 {
        for gx in 0..64i32 {
            let x = gx as f32 * 32.0 + 16.0;
            let z = gz as f32 * 32.0 + 16.0;
            h.update(&terrain::height(seed, x, z).to_bits().to_le_bytes());
        }
    }
    let table = ScatterTable::alpha_default();
    for cz in 120..136i32 {
        for cx in 120..136i32 {
            let s = terrain::scatter(seed, &table, cx, cz);
            h.update(&[s.occupant as u8, s.yaw]);
            h.update(&s.x.to_bits().to_le_bytes());
            h.update(&s.y.to_bits().to_le_bytes());
            h.update(&s.z.to_bits().to_le_bytes());
            h.update(&s.scale.to_bits().to_le_bytes());
        }
    }
    h.digest()
}

/// Movement + gather parity: `sequences` independent random input
/// sequences, each a fresh world + 2 bots × `ticks` ticks; the
/// per-sequence state hashes fold into one digest (DESIGN.md §4: 10,000
/// sequences through both builds). Bots hold the primary button in
/// bursts and the world carries the synthetic gather fixture, so slot
/// life, yields, and inventories are inside the parity surface.
#[no_mangle]
pub extern "C" fn probe_parity(master_seed: u64, sequences: u32, ticks: u32) -> u64 {
    let mut h = Xxh3::new();
    for s in 0..sequences {
        let seq_seed = splitmix64(master_seed ^ (s as u64));
        let mut world = World::new(seq_seed);
        world.gather = crate::gather::GatherContent::probe_fixture();
        world.craft = crate::craft::CraftContent::probe_fixture();
        world.tick(&[Command::Join { id: 1 }, Command::Join { id: 2 }]);
        let mut rng = Pcg32::new(seq_seed, 7);
        let mut yaws = [0u16; 2];
        for t in 0..ticks {
            let f1 = bot_frame(&mut rng, yaws[0], t as u16);
            let f2 = bot_frame(&mut rng, yaws[1], t as u16);
            yaws = [f1.yaw, f2.yaw];
            // Bots poke the craft verb on a fixed cadence so enqueue,
            // completion, refusal, and cancel are all inside the parity/
            // replay/alloc surface (fixture recipes, gathered inputs).
            let craft = Command::Craft {
                id: 1,
                recipe: (t % 4) as u16, // 3 = out of range: refusal path
                count: 1 + (t % 2) as u16,
            };
            let cancel = Command::CraftCancel {
                id: 2,
                index: (t % 5) as u16,
            };
            if t % 16 == 7 {
                world.tick(&[
                    Command::Input { id: 1, frame: f1 },
                    Command::Input { id: 2, frame: f2 },
                    craft,
                    Command::Craft {
                        id: 2,
                        recipe: 0,
                        count: 1,
                    },
                ]);
            } else if t % 64 == 20 {
                world.tick(&[
                    Command::Input { id: 1, frame: f1 },
                    Command::Input { id: 2, frame: f2 },
                    cancel,
                ]);
            } else {
                world.tick(&[
                    Command::Input { id: 1, frame: f1 },
                    Command::Input { id: 2, frame: f2 },
                ]);
            }
        }
        h.update(&world.state_hash().to_le_bytes());
    }
    h.digest()
}
