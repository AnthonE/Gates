//! Landmarks (`landmark.rs`): the big island has places worth walking to,
//! they keep off the roads, their crates are really in the scatter, and their
//! walls are walls.

use sim_core::landmark::{self, LANDMARK_R_M};
use sim_core::occupy::Scratch;
use sim_core::terrain::{self, Occupant, ScatterTable, CELL_SIZE};

const SEEDS: [u64; 3] = [20260731, 0x0047_4154_4553, 0xDEAD_BEEF];

#[test]
fn the_island_has_landmarks_of_several_kinds_off_the_roads() {
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let live: Vec<_> = haven.marks.iter().filter(|m| m.live).collect();
        assert!(
            live.len() >= 8,
            "seed {seed:#x}: only {} landmarks",
            live.len()
        );
        let mut kinds: Vec<_> = live.iter().map(|m| m.kind).collect();
        kinds.sort_by_key(|k| *k as u8);
        kinds.dedup();
        assert!(kinds.len() >= 3, "seed {seed:#x}: only {kinds:?}");
        for m in &live {
            let road = LANDMARK_R_M + terrain::ROAD_SHOULDER_HALF_W;
            assert!(
                haven.ring.dist2(m.x, m.z) >= road * road,
                "seed {seed:#x}: a {:?} on the ring road",
                m.kind
            );
            assert!(
                m.y > terrain::LAND_MIN_H,
                "seed {seed:#x}: a {:?} in the sea",
                m.kind
            );
        }
    }
}

#[test]
fn every_landmark_crate_is_a_scatter_slot() {
    let table = ScatterTable::alpha_default();
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let (mut want, mut got) = (0, 0);
        for m in haven.marks.iter().filter(|m| m.live) {
            let mut k = 0;
            while let Some((x, z, _, occ)) = landmark::anchor(m, k) {
                k += 1;
                want += 1;
                let (cx, cz) = ((x / CELL_SIZE) as i32, (z / CELL_SIZE) as i32);
                let s = terrain::scatter(seed, &table, &haven, cx, cz);
                if s.occupant == occ {
                    got += 1;
                }
            }
        }
        // Two anchors can share a cell on an unlucky bearing; nearly all
        // must land.
        assert!(
            got * 10 >= want * 9,
            "seed {seed:#x}: {got} of {want} landmark crates are in the scatter"
        );
    }
}

#[test]
fn a_landmark_wall_stops_a_body() {
    let seed = SEEDS[0];
    let mut scratch = Scratch::live(seed);
    let haven = terrain::haven(seed);
    let m = *haven
        .marks
        .iter()
        .find(|m| m.live)
        .expect("a landmark on the shipped seed");
    // The tallest part standing on the ground, at the landmark's base.
    let part = landmark::parts(m.kind)
        .iter()
        .filter(|p| p.b[1] <= 0.5)
        .max_by(|a, b| a.b[4].total_cmp(&b.b[4]))
        .unwrap();
    let (lx, lz) = ((part.b[0] + part.b[3]) * 0.5, (part.b[2] + part.b[5]) * 0.5);
    let (x, z) = landmark::to_world(&m, lx, lz);
    let mut occ = scratch.occupants();
    assert!(
        occ.blocks(seed, x, z, m.y),
        "a body walked into a {:?}'s tallest part",
        m.kind
    );
    let _ = Occupant::None;
}

/// The two kits with a terrain need of their own (`NOW.md` §0n2 item 1): a
/// relay stands within sight of a road, a quarry in the dry foothills. Over
/// a handful of seeds each turns up somewhere, and every relay is near a road.
#[test]
fn relays_stand_by_roads_and_quarries_turn_up() {
    use sim_core::landmark::{LandmarkKind, LANDMARK_R_M};
    let (mut relays, mut quarries) = (0, 0);
    for seed in [1u64, 7, 42, 99, 2026, 31337] {
        let h = sim_core::terrain::haven(seed);
        for m in h.marks.iter().filter(|m| m.live) {
            match m.kind {
                LandmarkKind::Relay => {
                    relays += 1;
                    let mut d2 = h.ring.dist2(m.x, m.z);
                    for r in h.roads.iter().filter(|r| r.live) {
                        d2 = d2.min(r.dist2(m.x, m.z));
                    }
                    let most = LANDMARK_R_M + 18.0 + 50.0;
                    assert!(d2 <= most * most, "a relay {} m from any road", d2.sqrt());
                }
                LandmarkKind::Quarry => {
                    quarries += 1;
                    assert!(m.y <= 42.0, "a quarry on a summit at {} m", m.y);
                }
                _ => {}
            }
        }
    }
    assert!(relays > 0, "no relay on six islands");
    assert!(quarries > 0, "no quarry on six islands");
}
