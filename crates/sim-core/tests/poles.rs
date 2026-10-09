//! The ring's power poles: most of the ring carries them, and the bearing
//! lookup that answers collision agrees with asking every pole.
#![allow(clippy::disallowed_types)]

use sim_core::poles::{self, POLE_R_M};
use sim_core::terrain::{self, RING_BEARINGS};

#[test]
fn the_ring_is_strung_and_collision_finds_every_pole() {
    let seed = 20260731;
    let haven = terrain::haven(seed);
    let standing: Vec<(f32, f32)> = (0..RING_BEARINGS)
        .filter_map(|k| poles::at(&haven, k))
        .collect();
    assert!(
        standing.len() > RING_BEARINGS * 3 / 4,
        "only {} poles",
        standing.len()
    );
    let brute = |x: f32, z: f32, r: f32| {
        standing.iter().any(|&(px, pz)| {
            (x - px) * (x - px) + (z - pz) * (z - pz) < (r + POLE_R_M) * (r + POLE_R_M)
        })
    };
    for &(px, pz) in &standing {
        assert!(poles::blocks(&haven, px + 0.2, pz - 0.1, 0.3));
        // And a sweep round each pole, both answers agreeing.
        for (ux, uz) in [
            (1.0f32, 0.0f32),
            (0.7, 0.7),
            (0.0, 1.0),
            (-0.7, 0.7),
            (-1.0, 0.0),
            (-0.7, -0.7),
            (0.0, -1.0),
            (0.7, -0.7),
        ] {
            for d in [0.3, 0.45, 0.6, 1.5] {
                let (x, z) = (px + d * ux, pz + d * uz);
                assert_eq!(
                    poles::blocks(&haven, x, z, 0.3),
                    brute(x, z, 0.3),
                    "at ({x}, {z})"
                );
            }
        }
    }
    // Nothing stands at the island's middle or out at sea.
    assert!(!poles::blocks(&haven, 2048.0, 2048.0, 5.0));
    assert!(!poles::blocks(&haven, 10.0, 10.0, 5.0));
}
