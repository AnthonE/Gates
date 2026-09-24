//! Gate: the interior ranges (`terrain` stage 4d) stay inside their window
//! and stay mountains a player can climb.
//!
//! **Two claims, and both are cheap to break by accident.**
//!
//! 1. **Outside the window the island is the island it was.** The ranges'
//!    whole justification for landing as one slice is that the coast, the
//!    ring, the haven, the waystations, every spawn and the inland site's near
//!    candidates keep their bits (`MASSIF_R_IN`/`MASSIF_R_OUT`'s doc). That
//!    rests on the lift being EXACTLY zero there — not small, zero — because
//!    `height_in` then never touches `land`, and every solve that samples out
//!    there sees the arithmetic it always saw. A term added outside the radial
//!    multiply (a gully octave that forgot `strength`, a spur that reaches past
//!    the fade) would move the ring on some seed without reddening anything
//!    that tests the interior.
//! 2. **A range is walkable in the main.** A range whose flanks are mostly
//!    cliff is scenery: the sim refuses a step past `CLIFF_SLOPE_RATIO`, and
//!    the gully filter's whole job is to add steepness. So the footprint's
//!    median slope and its cliff share are held, per seed, with the peak held
//!    from below so a "fix" that flattens the ranges away cannot pass.
//!
//! Bands set from `examples/massif_stats` (2026-09-23): footprint p50 slope
//! 0.64–0.71, cliff 4.9–10.7 %, peaks 94–141 m on the four seeds below.
//! **Proven red** under three mutants, 2026-09-23: `GULLY_AMP` 14 → 28 (the
//! shipped seed's cliff share 25.5 %), `MAJOR_AMP`/`MINOR_AMP` halved (a peak
//! of 77 m, and the window test's own floor), and the window's outer edge
//! pushed 60 m out in `massif_base` and `massif_lift` alike (caught at
//! (1492, 716) on the shipped seed, a lift of 1.3 mm).
//!
//! Headless, sim-core only, and `--release`-fast: sim-core is optimized in the
//! dev profile too (`Cargo.toml`).

// A test harness is not sim code: the print wall does not apply to it. The
// float walls are kept anyway — nothing below needs a transcendental.
#![allow(clippy::disallowed_macros)]

use sim_core::terrain::{self, CLIFF_SLOPE_RATIO, ISLAND_SIZE, MASSIF_R_IN, MASSIF_R_OUT};

/// The shipped seed, the golden seed, and two the rest of the suite uses.
const SEEDS: [u64; 4] = [20260731, 0x0047_4154_4553, 0x1, 0xDEAD_BEEF];

/// Metres of lift that count a sample as ON a range.
const FOOTPRINT_M: f32 = 5.0;

/// The footprint's median slope may not pass tan 40°.
const P50_MAX: f32 = 0.84;
/// The share of a footprint past the sim's cliff threshold.
const CLIFF_SHARE_MAX: f32 = 0.15;
/// The highest ground on the island's ranges, metres. The lowest island
/// measured reached 94 m; the floor sits under it with room for a re-tune.
const PEAK_MIN_M: f32 = 80.0;

#[test]
fn the_ranges_stop_exactly_at_their_window() {
    let c = ISLAND_SIZE * 0.5;
    for seed in SEEDS {
        let mut outside = 0u32;
        let mut peak_lift = 0.0f32;
        let mut z = 4.0;
        while z < ISLAND_SIZE {
            let mut x = 4.0;
            while x < ISLAND_SIZE {
                let (dx, dz) = (x - c, z - c);
                let r2 = dx * dx + dz * dz;
                let lift = terrain::massif_lift_at(seed, x, z);
                if r2 <= MASSIF_R_IN * MASSIF_R_IN || r2 >= MASSIF_R_OUT * MASSIF_R_OUT {
                    assert_eq!(
                        lift.to_bits(),
                        0.0f32.to_bits(),
                        "seed {seed}: a range lifts ({x}, {z}) by {lift} m, outside its window \
                         — the ring, the haven and the inland site's near candidates no \
                         longer keep their bits"
                    );
                    outside += 1;
                } else {
                    peak_lift = peak_lift.max(lift);
                }
                x += 8.0;
            }
            z += 8.0;
        }
        assert!(
            outside > 30_000,
            "seed {seed}: only {outside} samples outside — the sweep broke"
        );
        assert!(
            peak_lift > 50.0,
            "seed {seed}: the ranges lift nothing past {peak_lift} m — a window test over an \
             island with no ranges passes for free"
        );
    }
}

#[test]
fn the_ranges_are_mountains_a_player_can_climb() {
    for seed in SEEDS {
        let mut slopes = Vec::new();
        let mut peak = 0.0f32;
        let mut z = 2.0;
        while z < ISLAND_SIZE {
            let mut x = 2.0;
            while x < ISLAND_SIZE {
                if terrain::massif_lift_at(seed, x, z) > FOOTPRINT_M {
                    slopes.push(terrain::slope(seed, x, z));
                    peak = peak.max(terrain::height(seed, x, z));
                }
                x += 6.0;
            }
            z += 6.0;
        }
        assert!(
            slopes.len() > 2_000,
            "seed {seed}: a footprint of {} samples",
            slopes.len()
        );
        slopes.sort_by(|a, b| a.total_cmp(b));
        let p50 = slopes[slopes.len() / 2];
        let cliff =
            slopes.iter().filter(|s| **s > CLIFF_SLOPE_RATIO).count() as f32 / slopes.len() as f32;
        println!(
            "seed {seed:>12}: footprint {} samples, p50 slope {p50:.3}, cliff {:.1} %, peak {peak:.0} m",
            slopes.len(),
            cliff * 100.0
        );
        assert!(
            p50 < P50_MAX,
            "seed {seed}: half the ranges are steeper than {p50:.2} — past tan 40°, a range is a wall"
        );
        assert!(
            cliff < CLIFF_SHARE_MAX,
            "seed {seed}: {:.1} % of the ranges is past the sim's cliff threshold",
            cliff * 100.0
        );
        assert!(
            peak >= PEAK_MIN_M,
            "seed {seed}: the ranges top out at {peak:.0} m — they have been flattened away"
        );
    }
}
