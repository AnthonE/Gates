//! The desert's cactus (`terrain::Occupant::Cactus`): drawn only out of a
//! desert shrub, solid, and it pricks a body pressed against it once a second
//! (`World::prick_cacti`) while one standing clear of it is left alone.

use sim_core::collide::CAPSULE_RADIUS_M;
use sim_core::gather::GatherContent;
use sim_core::terrain::{self, Biome, Occupant, ScatterTable, Slot, CELLS_PER_SIDE};
use sim_core::world::{Command, World, CACTUS_HP, CACTUS_PERIOD_TICKS, EV_HURT};

const SEED: u64 = 0x0047_4154_4553; // the golden seed

/// Every cactus on a stride over the island, with the biome it stands in.
fn cacti(seed: u64) -> Vec<(Slot, Biome)> {
    let table = ScatterTable::alpha_default();
    let haven = terrain::haven(seed);
    let mut out = Vec::new();
    for cz in (0..CELLS_PER_SIDE).step_by(2) {
        for cx in (0..CELLS_PER_SIDE).step_by(2) {
            let s = terrain::scatter(seed, &table, &haven, cx, cz);
            if s.occupant == Occupant::Cactus {
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

#[test]
fn cacti_stand_in_the_desert_and_nowhere_else() {
    let found = cacti(SEED);
    assert!(
        found.len() > 20,
        "only {} cacti on the golden seed's stride — the desert grows none",
        found.len()
    );
    for (s, b) in &found {
        assert_eq!(*b, Biome::Desert, "a cactus at ({}, {}) in {b:?}", s.x, s.z);
    }
}

/// A body against the trunk loses `CACTUS_HP` a second; the same body two
/// metres off loses nothing.
#[test]
fn a_cactus_pricks_whoever_is_pressed_against_it() {
    let (s, _) = *cacti(SEED)
        .iter()
        .find(|(s, _)| {
            // Flat ground on the west side, so the spawn stands where it is put.
            let d = terrain::height(SEED, s.x - 1.0, s.z) - s.y;
            d * d < 0.09
        })
        .expect("a cactus on flat ground");
    let touching = CAPSULE_RADIUS_M + terrain::CACTUS_R_M * s.scale + 0.02;
    for (gap, pricked) in [(touching, true), (touching + 2.0, false)] {
        let mut w = Box::new(World::new(SEED));
        w.gather = GatherContent::probe_fixture();
        w.dev_spawn = Some((s.x - gap, s.z));
        w.tick(&[Command::Join { id: 1 }]);
        // The bare world loads no combat content, so no health to lose:
        // give the body a naked player's, outside any safe zone.
        (w.players[0].hp, w.players[0].hp_max) = (100, 100);
        w.players[0].safe = false;
        let hp = w.players[0].hp;
        let mut hurt = 0;
        for _ in 0..CACTUS_PERIOD_TICKS * 2 {
            w.tick(&[]);
            hurt += w
                .events
                .entries()
                .iter()
                .filter(|e| e.code == EV_HURT && e.a == 1)
                .count();
        }
        if pricked {
            assert!(
                hurt >= 1,
                "pressed against a cactus for two seconds, unhurt"
            );
            assert!(
                w.players[0].hp <= hp - CACTUS_HP,
                "hp {} → {}: the prick did not land",
                hp,
                w.players[0].hp
            );
        } else {
            assert_eq!(hurt, 0, "pricked from {gap:.2} m off");
            assert_eq!(w.players[0].hp, hp);
        }
    }
}
