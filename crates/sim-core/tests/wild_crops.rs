//! Wild corn and pumpkin (`terrain::Occupant::WildCorn`, `WildPumpkin`):
//! drawn only out of a shrub with a lake's fresh water near, so the two
//! crops have a wild source the way hemp does. A lake's shore lies under
//! `BEACH_MAX_H`, so they stand almost all on lake shores in the Beach
//! biome, never by the sea alone and never in the dry biomes.

use sim_core::terrain::{self, Biome, Occupant, ScatterTable, Slot, CELLS_PER_SIDE, SEA_LEVEL};

const SEED: u64 = 42;

/// Every wild crop on the island, with the biome it stands in.
fn wild(seed: u64) -> Vec<(Slot, Biome)> {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);
    let mut out = Vec::new();
    for cz in 0..CELLS_PER_SIDE {
        for cx in 0..CELLS_PER_SIDE {
            let s = terrain::scatter(seed, &table, &haven, cx, cz);
            if matches!(s.occupant, Occupant::WildCorn | Occupant::WildPumpkin) {
                let b = terrain::biome(
                    s.y,
                    terrain::moisture(seed, s.x, s.z),
                    terrain::fel(seed, s.x, s.z),
                );
                out.push((s, b));
            }
        }
    }
    out
}

/// Fresh water somewhere within `r` metres: a 1 m grid over the disc, any
/// point under the sea line whose water is a lake's.
fn fresh_within(seed: u64, x: f32, z: f32, r: f32) -> bool {
    let n = r as i32;
    for iz in -n..=n {
        for ix in -n..=n {
            let (dx, dz) = (ix as f32, iz as f32);
            if dx * dx + dz * dz > r * r {
                continue;
            }
            let (h, open) = terrain::height_open(seed, x + dx, z + dz);
            if h < SEA_LEVEL && open < 0.5 {
                return true;
            }
        }
    }
    false
}

#[test]
fn wild_crops_grow_by_fresh_water_and_nowhere_dry() {
    let found = wild(SEED);
    let corn = found
        .iter()
        .filter(|(s, _)| s.occupant == Occupant::WildCorn)
        .count();
    let pumpkin = found
        .iter()
        .filter(|(s, _)| s.occupant == Occupant::WildPumpkin)
        .count();
    assert!(corn > 0, "seed {SEED} grows no wild corn");
    assert!(pumpkin > 0, "seed {SEED} grows no wild pumpkin");
    for (s, b) in &found {
        assert!(
            !matches!(b, Biome::Desert | Biome::Fel | Biome::Highland),
            "a {:?} at ({}, {}) in {b:?}",
            s.occupant,
            s.x,
            s.z
        );
        assert!(
            fresh_within(SEED, s.x, s.z, 24.0),
            "a {:?} at ({}, {}) with no fresh water within 24 m",
            s.occupant,
            s.x,
            s.z
        );
    }
}

/// **The census, pinned**: how many of each grow on seed 42's island, and
/// where. Measured (2026-10-21, three seeds): a lake's shore lies under
/// `BEACH_MAX_H`, so nearly every wild crop stands in the Beach biome on a
/// lake's edge with fresh water the nearest water to it, and the meadow
/// behind starts past the reach. A change to the rule, the shares or the
/// lakes moves these numbers; if it was meant, re-measure and re-pin.
#[test]
fn the_wild_crop_census_is_pinned() {
    let found = wild(SEED);
    let corn = found
        .iter()
        .filter(|(s, _)| s.occupant == Occupant::WildCorn)
        .count();
    let mut by_biome = [0usize; terrain::BIOMES];
    for (_, b) in &found {
        by_biome[*b as usize] += 1;
    }
    assert_eq!(
        (corn, found.len() - corn, by_biome),
        (15, 8, [23, 0, 0, 0, 0, 0]),
        "(corn, pumpkin, by biome: beach meadow forest highland desert fel)"
    );
}
