//! The solved road must carry a full-width lap, not just good node samples.
use sim_core::terrain::{self, RING_BEARINGS};

const SEEDS: [u64; 16] = [
    1, 2, 7, 42, 99, 1337, 20260731, 20260804, 555555, 8675309, 31337, 4294967291, 123456789,
    999999937, 0xDEADBEEF, 0x0BADC0DE,
];

#[test]
fn the_whole_carriageway_is_dry_and_walkable_including_every_joint() {
    for seed in SEEDS.into_iter().chain((0..32u64).map(|n| {
        n.wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407)
    })) {
        let h = terrain::haven(seed);
        for i in 0..RING_BEARINGS {
            let (ax, az) = h.ring.node(i as i32);
            let (bx, bz) = h.ring.node(i as i32 + 1);
            let (dx, dz) = (bx - ax, bz - az);
            let len = (dx * dx + dz * dz).sqrt();
            let steps = (len / 0.5) as usize + 1;
            for k in 0..=steps {
                for off in [-terrain::ROAD_HALF_W, 0.0, terrain::ROAD_HALF_W] {
                    let t = k as f32 / steps as f32;
                    let (x, z) = (ax + dx * t - dz / len * off, az + dz * t + dx / len * off);
                    let g = terrain::ground(seed, &h, x, z);
                    let slope = terrain::ground_slope(seed, &h, x, z);
                    assert!(
                        g >= terrain::LAND_MIN_H - 1e-4,
                        "seed {seed}, segment {i}, at ({x},{z}): wet road {g}"
                    );
                    assert!(
                        slope <= terrain::CLIFF_SLOPE_RATIO,
                        "seed {seed}, segment {i}, at ({x},{z}): cliff {slope}"
                    );
                    // The movement step's rise/run rule, in both directions,
                    // at a quarter metre: central slope alone can hide a step.
                    let next = terrain::ground(seed, &h, x + dx / len * 0.25, z + dz / len * 0.25);
                    assert!(
                        sim_core::fmath::fabs(next - g) <= 0.25 * terrain::CLIFF_SLOPE_RATIO + 1e-3,
                        "seed {seed} segment {i}: road steps from {g} to {next}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_bench_leaves_raw_bits_outside_its_declared_band() {
    // Independent all-segment distance, so an omitted segment in the query
    // window cannot make this test skip the same area as the implementation.
    for seed in [20260731, 42, 0xDEADBEEF] {
        let h = terrain::haven(seed);
        let reach = terrain::ROAD_SHOULDER_HALF_W + terrain::RING_BLEND_M;
        let mut changed = 0;
        for x in (0..=2048).step_by(8) {
            for z in (0..=2048).step_by(8) {
                let (x, z) = (x as f32, z as f32);
                let mut d2 = f32::MAX;
                for i in 0..RING_BEARINGS {
                    let (ax, az) = h.ring.node(i as i32);
                    let (bx, bz) = h.ring.node(i as i32 + 1);
                    let (dx, dz) = (bx - ax, bz - az);
                    let t = (((x - ax) * dx + (z - az) * dz) / (dx * dx + dz * dz)).clamp(0.0, 1.0);
                    let (px, pz) = (x - ax - t * dx, z - az - t * dz);
                    d2 = d2.min(px * px + pz * pz);
                }
                let raw = terrain::height(seed, x, z);
                let bench = h.ring.ground(raw, x, z);
                if d2 >= reach * reach {
                    assert_eq!(bench.to_bits(), raw.to_bits());
                } else if bench != raw {
                    changed += 1;
                }
                let mut ungraded = h;
                ungraded.ring.graded = false;
                if d2 >= reach * reach {
                    assert_eq!(
                        terrain::ground(seed, &h, x, z).to_bits(),
                        terrain::ground(seed, &ungraded, x, z).to_bits()
                    );
                }
            }
        }
        assert!(changed > 0, "the bench must actually repair terrain");
    }
}
