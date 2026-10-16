//! The Verdigris (`terrain::fel`): where it stands, how much of the island it
//! takes, and the spires it raises (`boulder.rs`).

use sim_core::boulder::{self, BLOCK_SPIRE, FEL_SPIRE_AT};
use sim_core::terrain::{self, Biome};

const SEEDS: [u64; 3] = [20260731, 0x0047_4154_4553, 0xDEAD_BEEF];

/// Every point of a square grid of `step` over the island.
fn grid(step: f32, mut f: impl FnMut(f32, f32)) {
    let n = (terrain::ISLAND_SIZE / step) as i32;
    for iz in 0..n {
        for ix in 0..n {
            f((ix as f32 + 0.5) * step, (iz as f32 + 0.5) * step);
        }
    }
}

#[test]
fn the_verdigris_keeps_clear_of_the_town_the_desert_and_the_fixtures() {
    let c = terrain::ISLAND_SIZE * 0.5;
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        // The town and every metre of ground it blends.
        let keep = sim_core::town::TOWN_FOOTPRINT.blend_m + 20.0;
        let (tx, tz) = (haven.town.x, haven.town.z);
        grid(16.0, |x, z| {
            let (dx, dz) = (x - tx, z - tz);
            if dx * dx + dz * dz < keep * keep {
                assert_eq!(
                    terrain::fel(seed, x, z),
                    0.0,
                    "seed {seed:#x}: the town at ({x},{z})"
                );
            }
            // The desert and the Verdigris never meet.
            if terrain::aridity(seed, x, z) > 0.0 {
                assert_eq!(
                    terrain::fel(seed, x, z),
                    0.0,
                    "seed {seed:#x}: desert at ({x},{z})"
                );
            }
        });
        // The centre's golden window, the server tests' +x shore ray, and the
        // capture rig's spawn in the south.
        let mut x = c - 64.0;
        while x < terrain::ISLAND_SIZE {
            assert_eq!(
                terrain::fel(seed, x, c),
                0.0,
                "seed {seed:#x}: shore ray at x {x}"
            );
            x += 4.0;
        }
        assert_eq!(terrain::fel(seed, 1500.0, 600.0), 0.0);
    }
}

#[test]
fn the_heart_is_whole_and_the_zone_is_a_place_not_a_province() {
    for seed in SEEDS {
        assert_eq!(
            terrain::fel(seed, terrain::FEL_HEART.0, terrain::FEL_HEART.1),
            1.0,
            "seed {seed:#x}: the heart is not wholly corrupted"
        );
        let (mut land, mut fel) = (0u32, 0u32);
        grid(16.0, |x, z| {
            let h = terrain::height(seed, x, z);
            if h < terrain::LAND_MIN_H {
                return;
            }
            land += 1;
            let b = terrain::biome_at(seed, h, x, z);
            if b == Biome::Fel {
                fel += 1;
                assert!(
                    h >= terrain::BEACH_MAX_H,
                    "seed {seed:#x}: a Verdigris beach"
                );
            }
        });
        let share = fel as f32 / land as f32;
        assert!(
            (0.02..=0.09).contains(&share),
            "seed {seed:#x}: the Verdigris is {:.1}% of the land",
            share * 100.0
        );
    }
}

#[test]
fn spires_stand_only_in_the_verdigris_and_stand_tall() {
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let n = boulder::cells_per_side();
        let mut spires = 0;
        for bz in 0..n {
            for bx in 0..n {
                for b in boulder::formation(seed, &haven, bx, bz).iter() {
                    if b.kind != BLOCK_SPIRE {
                        continue;
                    }
                    spires += 1;
                    assert!(
                        terrain::fel(seed, b.x, b.z) >= FEL_SPIRE_AT,
                        "seed {seed:#x}: a spire outside the Verdigris: {b:?}"
                    );
                    let over = b.y1 - terrain::height(seed, b.x, b.z);
                    assert!(
                        over >= 3.0 * b.hx.max(b.hz),
                        "seed {seed:#x}: a spire {over:.1} m tall on a {:.1} m footprint",
                        2.0 * b.hx.max(b.hz)
                    );
                }
            }
        }
        assert!(spires > 30, "seed {seed:#x}: only {spires} spires");
    }
}
