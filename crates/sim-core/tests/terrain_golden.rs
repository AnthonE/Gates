//! `test_terrain_golden` (TERRAIN.md §0/§7): fixed seed → pinned hash of
//! 64×64 sampled heights + 256 scatter results. The wasm half of the same
//! assertion runs in ci/gates.sh via ci/parity.mjs against the same pin.
//! Plus shape sanity: the generator must produce an island, not a puddle.

use sim_core::probe::probe_terrain;
use sim_core::terrain::{self, Occupant, ScatterTable, CELLS_PER_SIDE};

const GOLDEN_SEED: u64 = 0x0047_4154_4553; // "GATES"

/// Pinned fingerprint for GOLDEN_SEED. Regenerates only with an intentional
/// worldgen change, in the same commit (CLAUDE.md walls 5/6 discipline).
const GOLDEN_TERRAIN_HASH: u64 = 0xFB51_6278_08A9_150E;

#[test]
fn test_terrain_golden() {
    assert_eq!(
        probe_terrain(GOLDEN_SEED),
        GOLDEN_TERRAIN_HASH,
        "worldgen output drifted from the pinned golden; if intentional, \
         regenerate the golden in this same commit"
    );
    // A different seed produces a different island.
    assert_ne!(probe_terrain(GOLDEN_SEED ^ 1), GOLDEN_TERRAIN_HASH);
}

#[test]
fn test_terrain_shape_sanity() {
    let mut min_h = f32::INFINITY;
    let mut max_h = f32::NEG_INFINITY;
    for gz in 0..64i32 {
        for gx in 0..64i32 {
            let h = terrain::height(
                GOLDEN_SEED,
                gx as f32 * 32.0 + 16.0,
                gz as f32 * 32.0 + 16.0,
            );
            min_h = min_h.min(h);
            max_h = max_h.max(h);
        }
    }
    assert!(min_h < -5.0, "no sea floor: min sampled height {min_h}");
    assert!(max_h > 40.0, "no relief: max sampled height {max_h}");

    // TERRAIN.md §6: ~8–12k live slots per seed. The band is the doc's; a
    // seed outside it means the scatter weights drifted, not the seed.
    let table = ScatterTable::alpha_default();
    let mut live = 0u32;
    let mut trees = 0u32;
    let mut barrels = 0u32;
    let mut ore = 0u32;
    for cz in 0..CELLS_PER_SIDE {
        for cx in 0..CELLS_PER_SIDE {
            let s = terrain::scatter(GOLDEN_SEED, &table, cx, cz);
            match s.occupant {
                Occupant::None => {}
                Occupant::Tree => {
                    live += 1;
                    trees += 1;
                }
                Occupant::BarrelSlot => {
                    live += 1;
                    barrels += 1;
                }
                Occupant::StoneNode | Occupant::MetalNode | Occupant::SulfurNode => {
                    live += 1;
                    ore += 1;
                }
                _ => live += 1,
            }
        }
    }
    assert!(
        (8_000..=12_000).contains(&live),
        "live slots {live} outside TERRAIN.md §6's 8–12k band (trees {trees}, ore {ore}, barrels {barrels})"
    );
    assert!(trees > 1_000, "island needs wood: {trees} trees");
    assert!(ore > 300, "island needs ore: {ore} nodes");
    assert!(barrels > 50, "the loot route needs barrels: {barrels}");
}
