//! The vertical build lattice (`build::column_floor_y`) — the gate on the
//! 2026-08-15 playtest defect "pieces don't line up".
//!
//! Before `BUILD_BASE_Q_M`, every column's base was its own raw cell-center
//! terrain sample, so two foundations side by side on any ground that was
//! not perfectly flat stood at two different heights: a row of them was a
//! staircase of arbitrary ~0..3 m steps with cracks, and the wall between
//! two cells based itself on its canonical cell's sample and met neither
//! neighbour flush. The lattice makes flushness an *identity*: columns
//! whose ground falls in one band share one bit-equal base, and a band
//! step is an exact multiple of the quantum.
//!
//! **Proven red under the old formula**: with the raw sample restored,
//! `neighbouring_columns_in_one_band_are_bit_equal_flush` fails on the
//! shipped seed at the first sloped pair (observed 2026-08-15), and
//! `a_lone_foundation_is_always_steppable`'s constant half fails the
//! moment any of its three constants moves against the other two. The
//! window below is centred on `ISLAND_SIZE/2` — the same centre
//! `World::spawn_pos` starts from — per the relief-sweep trap
//! (`CLAUDE.md`): a sweep window is cross-checked against something that
//! already knows where the world is.
//!
//! Float vocabulary: `fmath` only — the walls' clippy list reaches test
//! targets too, and a test written outside the wall would redden the gate
//! it rides in.

use sim_core::build::{column_floor_y, BUILD_BASE_Q_M, BUILD_CELL_M};
use sim_core::collide::PIECE_LIFT_M;
use sim_core::fmath::{fabs, floor_i32};
use sim_core::movement::STEP_UP;
use sim_core::terrain::{self, ISLAND_SIZE};

/// The solved authored sites for `seed` — what `terrain::ground` needs in order
/// to know where the carve is.
///
/// Memoized per seed, and that is not premature: `terrain::haven` is a few
/// thousand `height` taps (a shoreline march, a bisect and a rosette per
/// candidate bearing), these suites call it from inside assertion loops, and
/// the first draft of this helper resolved it per call and took the workspace
/// test run past five minutes. It is a pure function of the seed, so caching
/// cannot change a result.
fn hv(seed: u64) -> &'static sim_core::terrain::Haven {
    use std::cell::RefCell;
    // A thread-local rather than a `Mutex`: `std::sync::Mutex` is on
    // `sim-core/clippy.toml`'s disallowed list (wall 3), and that list is
    // crate-scoped, so it binds this suite too. Per-thread is the right shape
    // anyway — the cache exists to stop a per-assertion recompute, not to be
    // shared.
    thread_local! {
        static CACHE: RefCell<Vec<(u64, &'static sim_core::terrain::Haven)>> =
            const { RefCell::new(Vec::new()) };
    }
    let hit = CACHE.with(|c| c.borrow().iter().find(|(s, _)| *s == seed).map(|&(_, h)| h));
    if let Some(h) = hit {
        return h;
    }
    let h: &'static sim_core::terrain::Haven = Box::leak(Box::new(sim_core::terrain::haven(seed)));
    CACHE.with(|c| c.borrow_mut().push((seed, h)));
    h
}

/// The shipped island (`shard.toml`), plus one arbitrary other seed so the
/// lattice is a property of the rule and not of one world.
const SEEDS: [u64; 2] = [20260731, 7];

/// The scan: an 80×80-cell window centred on the island's own centre.
fn cells() -> impl Iterator<Item = (u16, u16)> {
    let c = (ISLAND_SIZE * 0.5 / BUILD_CELL_M) as u16;
    (c - 40..c + 40).flat_map(move |cx| (c - 40..c + 40).map(move |cz| (cx, cz)))
}

fn center_h(seed: u64, cx: u16, cz: u16) -> f32 {
    let half = BUILD_CELL_M * 0.5;
    terrain::height(
        seed,
        cx as f32 * BUILD_CELL_M + half,
        cz as f32 * BUILD_CELL_M + half,
    )
}

/// The band index, computed the way the lattice computes it (lift folded
/// into the snap — `column_floor_y`'s doc says why). Land only — the sea
/// floor cannot hold a foundation and its bands are not the claim.
fn band(seed: u64, cx: u16, cz: u16) -> Option<i32> {
    let h = center_h(seed, cx, cz);
    if h < 1.5 {
        return None;
    }
    Some(floor_i32((h + PIECE_LIFT_M) / BUILD_BASE_Q_M + 0.5))
}

/// Two adjacent columns whose ground shares a band share ONE base — `==`,
/// not a tolerance — and such pairs are common enough on the shipped
/// island that the property is not vacuously true.
#[test]
fn neighbouring_columns_in_one_band_are_bit_equal_flush() {
    for seed in SEEDS {
        let mut flush_pairs = 0usize;
        let mut land_pairs = 0usize;
        for (cx, cz) in cells() {
            for (nx, nz) in [(cx + 1, cz), (cx, cz + 1)] {
                let (Some(a), Some(b)) = (band(seed, cx, cz), band(seed, nx, nz)) else {
                    continue;
                };
                land_pairs += 1;
                if a == b {
                    let ya = column_floor_y(seed, hv(seed), cx, cz);
                    let yb = column_floor_y(seed, hv(seed), nx, nz);
                    assert!(
                        ya == yb,
                        "seed {seed}: cells ({cx},{cz}) and ({nx},{nz}) share band {a} \
                         but base {ya} != {yb} — the plate is a staircase again"
                    );
                    flush_pairs += 1;
                }
            }
        }
        // The suite must not pass by checking nothing (CLAUDE.md: a pass it
        // did not earn). Under the OLD raw-sample formula zero pairs are
        // bit-equal. The SHARE is a claim about the shipped island — the
        // world players actually build on — so only SEEDS[0] carries a bar
        // (measured 2026-08-15: 71.5% there, 45.3% on the arbitrary seed,
        // whose relief is its own business); every seed must still produce
        // SOME flush pair or the identity above was tested against nothing.
        assert!(
            land_pairs > 500,
            "seed {seed}: the window found only {land_pairs} land pairs — it is off the island"
        );
        assert!(
            flush_pairs > 0,
            "seed {seed}: no adjacent land pair shares a band — the identity went untested"
        );
        if seed == SEEDS[0] {
            assert!(
                flush_pairs * 2 > land_pairs,
                "shipped seed: only {flush_pairs} of {land_pairs} adjacent land pairs are \
                 flush — the lattice is not doing its job on the island that ships"
            );
        }
    }
}

/// Where two plates meet, the step is an exact multiple of the quantum —
/// never an arbitrary terrain delta.
#[test]
fn a_band_step_is_an_exact_multiple_of_the_quantum() {
    for seed in SEEDS {
        let mut steps = 0usize;
        for (cx, cz) in cells() {
            for (nx, nz) in [(cx + 1, cz), (cx, cz + 1)] {
                let (Some(a), Some(b)) = (band(seed, cx, cz), band(seed, nx, nz)) else {
                    continue;
                };
                if a == b {
                    continue;
                }
                let d =
                    column_floor_y(seed, hv(seed), cx, cz) - column_floor_y(seed, hv(seed), nx, nz);
                let k = d / BUILD_BASE_Q_M;
                assert!(
                    k == floor_i32(k) as f32,
                    "seed {seed}: step {d} between ({cx},{cz}) and ({nx},{nz}) \
                     is not a whole number of quanta"
                );
                steps += 1;
            }
        }
        assert!(steps > 0, "seed {seed}: the window crossed no band at all");
    }
}

/// A lone foundation can always be stepped onto: its slab top sits at most
/// `q/2 + PIECE_LIFT_M` above the ground at its centre, and that sum is
/// under `STEP_UP` by construction — the inequality the quantum was picked
/// against, held here so a future retune of any of the three constants
/// reddens a gate instead of stranding players at their own first slab.
#[test]
fn a_lone_foundation_is_always_steppable() {
    // A const assertion, because the bound is on constants (`tests/ghost.rs`
    // §inset's own idiom): a retune of any of the three that breaks the
    // coupling fails the BUILD, not just the test run.
    const {
        assert!(
            BUILD_BASE_Q_M * 0.5 + PIECE_LIFT_M <= STEP_UP,
            "q/2 + PIECE_LIFT_M exceeds STEP_UP: a lone foundation can strand its builder"
        )
    };
    for seed in SEEDS {
        for (cx, cz) in cells() {
            if band(seed, cx, cz).is_none() {
                continue;
            }
            let lift = column_floor_y(seed, hv(seed), cx, cz) - center_h(seed, cx, cz);
            assert!(
                lift <= STEP_UP + 1e-4,
                "seed {seed}: cell ({cx},{cz}) floats {lift} above its own ground — unclimbable"
            );
            assert!(
                lift >= PIECE_LIFT_M - BUILD_BASE_Q_M * 0.5 - 1e-4,
                "seed {seed}: cell ({cx},{cz}) is buried {lift} relative to its own ground"
            );
        }
    }
}

/// The lattice never moves a floor more than half a quantum off the raw
/// sample — the bound on how far the ground can visibly disagree with the
/// slab that claims to sit on it.
#[test]
fn the_lattice_holds_the_ground_within_half_a_quantum() {
    for seed in SEEDS {
        for (cx, cz) in cells() {
            let h = center_h(seed, cx, cz);
            let snapped = column_floor_y(seed, hv(seed), cx, cz) - PIECE_LIFT_M;
            assert!(
                fabs(snapped - h) <= BUILD_BASE_Q_M * 0.5 + 1e-4,
                "seed {seed}: cell ({cx},{cz}) snapped {h} to {snapped}"
            );
        }
    }
}
