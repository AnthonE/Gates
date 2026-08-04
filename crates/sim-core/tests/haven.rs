//! The haven pad as arithmetic (TERRAIN.md §1 stage 8).
//!
//! Same standard as `tests/road.rs`: counts, distances and slopes that fail
//! in seconds, re-derived from the public surface rather than trusting the
//! selector's own return value. The one number this suite exists to publish
//! is `relief` — how flat the argmax could get *without* carving. That is
//! what says whether the carve (which needs the memo threaded through
//! `height`, ~50 call sites in four crates) is optional or mandatory.
//!
//! Every assert prints its measurement, so a regression reports how far it
//! moved and not merely that it moved.

// The measurements ARE the gate's output — same reasoning and same allow as
// `tests/road.rs`: the L5 wall bans format/print in SIM code, and a test
// harness is not sim code. `f32::abs` is walled everywhere, tests included,
// so the two places this needs a magnitude use `max` of the signed pair.
#![allow(clippy::disallowed_macros)]

use sim_core::terrain::{
    self, Haven, Occupant, RoadBand, ScatterTable, CELLS_PER_SIDE, CLIFF_SLOPE_RATIO,
    HAVEN_CANDIDATES, HAVEN_HEIGHT_W, HAVEN_RADIUS_M, ISLAND_SIZE, LAND_MIN_H, ROAD_INLAND_M,
    ROAD_R_MAX, ROAD_R_MIN, SEA_LEVEL,
};
use sim_core::yaw_dir;

/// Seeds for the cheap per-seed checks. TERRAIN.md §7: "a seed that fails
/// is a bug in the generator, not a reroll".
const SEEDS: [u64; 16] = [
    1,
    2,
    7,
    42,
    99,
    1337,
    20_260_731,
    20_260_804,
    555_555,
    8_675_309,
    31_337,
    4_294_967_291,
    123_456_789,
    999_999_937,
    0xDEAD_BEEF,
    0x0BAD_C0DE,
];

/// Seeds for the checks that sweep all 65,536 scatter cells.
const SWEEP_SEEDS: [u64; 4] = [1, 42, 20_260_804, 0xDEAD_BEEF];

/// A haven parked off the island, so `in_haven` is false everywhere — the
/// control for "the exclusion actually removed something".
fn no_haven() -> Haven {
    Haven {
        x: -1.0e6,
        z: -1.0e6,
        y: 0.0,
        relief: 0.0,
    }
}

/// Relief over a far denser footprint than the 48-tap selector used: two
/// rings of 32 and one of 16 at half radius, plus the center. An
/// independent flatness statement, not a re-run of the selector's rosette.
fn dense_relief(seed: u64, x: f32, z: f32) -> f32 {
    let h0 = terrain::height(seed, x, z);
    let (mut lo, mut hi) = (h0, h0);
    for k in 0..32u16 {
        let (dx, dz) = yaw_dir((k * 8) << 8);
        for r in [HAVEN_RADIUS_M, HAVEN_RADIUS_M * 0.5] {
            let h = terrain::height(seed, x + dx * r, z + dz * r);
            lo = lo.min(h);
            hi = hi.max(h);
        }
    }
    hi - lo
}

fn radius_from_center(x: f32, z: f32) -> f32 {
    let c = ISLAND_SIZE * 0.5;
    let (dx, dz) = (x - c, z - c);
    (dx * dx + dz * dz).sqrt()
}

/// The pad is a pure function of the seed: same answer twice, a different
/// answer per seed, and every field finite. Without this the rest of the
/// suite could be measuring a coincidence.
#[test]
fn the_pad_is_a_pure_function_of_the_seed() {
    let mut seen: Vec<(u64, f32, f32)> = Vec::new();
    for seed in SEEDS {
        let a = terrain::haven(seed);
        let b = terrain::haven(seed);
        assert_eq!(
            (
                a.x.to_bits(),
                a.z.to_bits(),
                a.y.to_bits(),
                a.relief.to_bits()
            ),
            (
                b.x.to_bits(),
                b.z.to_bits(),
                b.y.to_bits(),
                b.relief.to_bits()
            ),
            "seed {seed}: haven() is not deterministic"
        );
        assert!(
            a.x.is_finite() && a.z.is_finite() && a.y.is_finite() && a.relief.is_finite(),
            "seed {seed}: haven {a:?} has a non-finite field"
        );
        seen.push((seed, a.x, a.z));
    }
    // Distinct seeds must not all land on one site — that would mean the
    // search collapsed to a constant and every assert below is vacuous.
    let distinct = {
        let mut v: Vec<(u32, u32)> = seen
            .iter()
            .map(|s| (s.1.to_bits(), s.2.to_bits()))
            .collect();
        v.sort_unstable();
        v.dedup();
        v.len()
    };
    println!(
        "haven: {distinct} distinct sites over {} seeds",
        SEEDS.len()
    );
    assert_eq!(
        distinct,
        SEEDS.len(),
        "the pad site repeats across seeds — the argmax is not reading the terrain"
    );
}

/// The pad stands on the coast road, on land, inside the ring's bracket,
/// and the sea is where the road's own offset says it should be. Together
/// these say the two fallbacks in `haven` were not taken.
#[test]
fn the_pad_stands_on_the_road_it_terminates() {
    let mut worst_sea_err = 0.0f32;
    let mut lowest = f32::MAX;
    for seed in SEEDS {
        let h = terrain::haven(seed);

        assert!(
            terrain::road_band(seed, h.x, h.z) != RoadBand::Off,
            "seed {seed}: pad at ({}, {}) is off the road — haven() took its \
             relaxed fallback, which this gate asserts is unreachable",
            h.x,
            h.z
        );
        assert!(
            h.y >= LAND_MIN_H,
            "seed {seed}: pad ground {} m is below the land line {LAND_MIN_H} m",
            h.y
        );
        assert_eq!(
            h.y.to_bits(),
            terrain::height(seed, h.x, h.z).to_bits(),
            "seed {seed}: pad y {} disagrees with height() at its own site",
            h.y
        );

        let d = radius_from_center(h.x, h.z);
        assert!(
            (ROAD_R_MIN..=ROAD_R_MAX).contains(&d),
            "seed {seed}: pad at radius {d} m is outside the ring bracket \
             [{ROAD_R_MIN}, {ROAD_R_MAX}]"
        );

        // March seaward along the pad's own radial to the first water. The
        // road puts its center line ROAD_INLAND_M inland of the shoreline,
        // so the pad — which sits on that line — must agree.
        let c = ISLAND_SIZE * 0.5;
        let (ux, uz) = ((h.x - c) / d, (h.z - c) / d);
        let mut sea_at = f32::MAX;
        let mut step = 0.0f32;
        while step <= ROAD_INLAND_M * 2.0 {
            if terrain::height(seed, h.x + ux * step, h.z + uz * step) <= SEA_LEVEL {
                sea_at = step;
                break;
            }
            step += 0.5;
        }
        assert!(
            sea_at.is_finite(),
            "seed {seed}: no water within {} m seaward of the pad",
            ROAD_INLAND_M * 2.0
        );
        let err = (sea_at - ROAD_INLAND_M).max(ROAD_INLAND_M - sea_at);
        worst_sea_err = worst_sea_err.max(err);
        lowest = lowest.min(h.y);
    }
    println!(
        "haven: worst |sea distance - {ROAD_INLAND_M}| = {worst_sea_err:.2} m, \
         lowest pad ground {lowest:.2} m"
    );
    // The bisection resolves the crossing to under 0.1 m, but the pad's
    // radial and the crossing's are the same line only to first order on a
    // wobbled coast. Bounded by the shoulder, not by the bisection.
    // Measured worst 2.50 m; bar one margin above it, not an order of
    // magnitude above it (DECISIONS.md §open: haven pad v0).
    assert!(
        worst_sea_err <= 4.0,
        "the pad drifted {worst_sea_err:.2} m off the road's stated \
         {ROAD_INLAND_M} m offset (measured 2.50 m when this bar was set)"
    );
}

/// The flatness the argmax actually achieved, measured on a denser
/// footprint than it scored, and the ground a player would walk on it.
/// **This is the number that decides whether the carve is mandatory.**
#[test]
fn the_pad_is_flat_enough_to_stand_a_monument_on() {
    let mut worst_relief = 0.0f32;
    let mut worst_dense = 0.0f32;
    let mut worst_slope = 0.0f32;
    let mut worst_seed = 0u64;
    for seed in SEEDS {
        let h = terrain::haven(seed);
        let dense = dense_relief(seed, h.x, h.z);
        if dense > worst_dense {
            worst_dense = dense;
            worst_seed = seed;
        }
        worst_relief = worst_relief.max(h.relief);

        // Nothing on the pad may be ground scatter itself refuses to stand
        // on — a pad that straddles a cliff is not a site, carved or not.
        for k in 0..16u16 {
            let (dx, dz) = yaw_dir((k * 16) << 8);
            let s = terrain::slope(seed, h.x + dx * HAVEN_RADIUS_M, h.z + dz * HAVEN_RADIUS_M);
            worst_slope = worst_slope.max(s);
        }
        println!(
            "haven seed {seed:>12}: ({:>7.1}, {:>7.1}) y {:>5.2} m  relief {:>5.2} m  \
             dense {dense:>5.2} m",
            h.x, h.z, h.y, h.relief
        );
    }
    println!(
        "haven: worst carried relief {worst_relief:.2} m, worst dense relief \
         {worst_dense:.2} m (seed {worst_seed}), worst rim slope {worst_slope:.2}"
    );
    assert!(
        worst_slope <= CLIFF_SLOPE_RATIO,
        "the pad rim reaches slope {worst_slope:.2}, over the cliff ratio \
         {CLIFF_SLOPE_RATIO} — scatter refuses to stand there and so would a player"
    );
    // The cliff ratio is the MEANING bar (can a player stand here); this is
    // the REGRESSION bar. Measured worst rim slope 0.21, so 0.45 is ~2x.
    assert!(
        worst_slope <= 0.45,
        "the pad rim reaches slope {worst_slope:.2} (measured 0.21 when this \
         bar was set) — the argmax is settling for rougher ground"
    );
    // Floor set from the measurement below, with margin stated in
    // DECISIONS.md §open (haven pad v0). It is a REGRESSION bar, not a
    // design target: the design target is the carve, which this slice does
    // not land, and this number is the argument for it.
    assert!(
        worst_dense <= HAVEN_RELIEF_BAR_M,
        "the flattest site on the ring reads {worst_dense:.2} m of relief across \
         the {HAVEN_RADIUS_M} m pad (bar {HAVEN_RELIEF_BAR_M} m) — the argmax got worse"
    );
}

/// Measured worst dense relief (3.76 m, seed 1337) plus ~33% margin
/// (DECISIONS.md §open: haven pad v0). Local to the gate on purpose — it is
/// an observation about the generator, not a knob the generator reads.
const HAVEN_RELIEF_BAR_M: f32 = 5.0;

/// The argmax is the argmax. Re-derives the candidate ring with a fine
/// linear march instead of the selector's coarse-march-plus-bisection, and
/// asserts **no candidate scores better than the shipped site**.
///
/// Scores, not coordinates, on purpose: the two searches resolve the
/// shoreline crossing by different arithmetic, so their sites differ by a
/// step width and comparing positions would test the arithmetic twice
/// rather than the choice once. What must hold is that nothing on the ring
/// beats what shipped — which is exactly what an argmax claims.
#[test]
fn the_shipped_site_is_the_best_candidate_on_the_ring() {
    let c = ISLAND_SIZE * 0.5;
    let mut worst_accepted = usize::MAX;
    let mut worst_margin = 0.0f32;
    for seed in SEEDS {
        let shipped = terrain::haven(seed);
        let shipped_score = shipped.relief + HAVEN_HEIGHT_W * (shipped.y - LAND_MIN_H);
        let mut accepted = 0usize;
        let mut best_score = f32::MAX;
        let mut best = (0.0f32, 0.0f32);

        for i in 0..HAVEN_CANDIDATES {
            let step = (256 / HAVEN_CANDIDATES) as u16;
            let (dx, dz) = yaw_dir((i as u16 * step) << 8);
            let (inner, outer) = (ROAD_R_MIN + ROAD_INLAND_M, ROAD_R_MAX + ROAD_INLAND_M);
            if terrain::height(seed, c + dx * inner, c + dz * inner) <= SEA_LEVEL {
                continue;
            }
            // First water going seaward, at 0.05 m — 80× the selector's
            // coarse step, so a crossing it brackets wrong shows up here.
            let mut cross = f32::MAX;
            let mut r = inner;
            while r <= outer {
                if terrain::height(seed, c + dx * r, c + dz * r) <= SEA_LEVEL {
                    cross = r;
                    break;
                }
                r += 0.05;
            }
            if !cross.is_finite() {
                continue;
            }
            let (x, z) = (
                c + dx * (cross - ROAD_INLAND_M),
                c + dz * (cross - ROAD_INLAND_M),
            );
            let y = terrain::height(seed, x, z);
            if y < LAND_MIN_H || terrain::road_band(seed, x, z) == RoadBand::Off {
                continue;
            }
            accepted += 1;
            let (mut lo, mut hi) = (y, y);
            for k in 0..8u16 {
                let (rx, rz) = yaw_dir((k * 32) << 8);
                let h = terrain::height(seed, x + rx * HAVEN_RADIUS_M, z + rz * HAVEN_RADIUS_M);
                lo = lo.min(h);
                hi = hi.max(h);
            }
            let score = (hi - lo) + HAVEN_HEIGHT_W * (y - LAND_MIN_H);
            if score < best_score {
                best_score = score;
                best = (x, z);
            }
        }

        assert!(
            accepted > 0,
            "seed {seed}: no candidate bearing is road-legal"
        );
        worst_accepted = worst_accepted.min(accepted);
        let margin = shipped_score - best_score;
        worst_margin = worst_margin.max(margin);
        assert!(
            margin <= HAVEN_SCORE_SLACK_M,
            "seed {seed}: shipped pad ({}, {}) scores {shipped_score:.2}, but a \
             candidate at ({}, {}) scores {best_score:.2} — {margin:.2} m better \
             than what the argmax returned",
            shipped.x,
            shipped.z,
            best.0,
            best.1
        );
    }
    println!(
        "haven: fewest road-legal candidates on any seed = {worst_accepted} of \
         {HAVEN_CANDIDATES}; worst score margin vs. the fine march = {worst_margin:.2} m"
    );
    // Measured worst 60 of 64; floor one margin below (DECISIONS.md §open).
    assert!(
        worst_accepted >= 48,
        "only {worst_accepted} of {HAVEN_CANDIDATES} bearings were road-legal \
         (measured 60 when this floor was set) — the argmax is choosing from \
         almost nothing"
    );
}

/// How much worse than the fine march's best the shipped site may score,
/// in meters of footprint relief. Absorbs the crossing-resolution
/// difference between the two searches and nothing else — measured worst
/// plus margin (DECISIONS.md §open: haven pad v0).
const HAVEN_SCORE_SLACK_M: f32 = 0.5;

/// The exclusion zone is real and it is not a no-op: nothing scatters
/// inside the pad, and the pad is somewhere that would otherwise have been
/// full. Re-derived from the slot list, never from the veto's return value.
#[test]
fn the_pad_is_clear_and_would_not_have_been() {
    let table = ScatterTable::alpha_default();
    let control = no_haven();
    let mut worst_cleared = usize::MAX;
    let mut total_cleared = 0usize;
    for seed in SWEEP_SEEDS {
        let haven = terrain::haven(seed);
        let mut inside = 0usize;
        let mut cleared = 0usize;
        let mut live = 0usize;

        for cx in 0..CELLS_PER_SIDE {
            for cz in 0..CELLS_PER_SIDE {
                let s = terrain::scatter(seed, &table, &haven, cx, cz);
                if s.occupant != Occupant::None {
                    live += 1;
                    if terrain::in_haven(&haven, s.x, s.z) {
                        inside += 1;
                    }
                }
                // The same cell with the pad parked off-island: what stage
                // 9 would have placed had stage 8 never run.
                let u = terrain::scatter(seed, &table, &control, cx, cz);
                if u.occupant != Occupant::None && terrain::in_haven(&haven, u.x, u.z) {
                    cleared += 1;
                }
            }
        }
        println!(
            "haven seed {seed:>12}: {inside} slot(s) inside the pad, {cleared} cleared \
             by it, {live} live islandwide"
        );
        assert_eq!(
            inside, 0,
            "seed {seed}: {inside} scatter slot(s) stand inside the {HAVEN_RADIUS_M} m pad"
        );
        worst_cleared = worst_cleared.min(cleared);
        total_cleared += cleared;
        assert!(
            cleared >= 1,
            "seed {seed}: the pad cleared nothing — an exclusion zone over \
             empty ground proves nothing"
        );
    }
    println!(
        "haven: {total_cleared} slots cleared across {} swept seeds \
         (fewest on any one seed: {worst_cleared})",
        SWEEP_SEEDS.len()
    );
    // The floor is on the TOTAL, not the per-seed minimum. A 16 m pad covers
    // ~12.6 scatter cells at ~15% occupancy, so the per-seed count is a
    // single-digit draw and a floor on it would measure variance rather than
    // regression. Measured total 15 (4+5+4+2), floor one ~20% margin under
    // it (DECISIONS.md §open: haven pad v0).
    assert!(
        total_cleared >= HAVEN_MIN_CLEARED_TOTAL,
        "the pad cleared {total_cleared} slot(s) across all swept seeds \
         (measured 15 when this floor was set) — the exclusion zone is \
         drifting onto ground that was empty anyway"
    );
}

/// Measured total clearance minus margin (DECISIONS.md §open: haven pad v0).
const HAVEN_MIN_CLEARED_TOTAL: usize = 12;

/// The pad's footprint fits the constants it is defined by: it clears whole
/// scatter cells, and it cannot reach off the island. Compile-time, so it
/// costs nothing and cannot be skipped.
#[test]
fn the_pad_fits_inside_the_world_it_is_placed_in() {
    const _: () = {
        // A pad narrower than a scatter cell could clear nothing at all.
        assert!(HAVEN_RADIUS_M > terrain::CELL_SIZE);
        // The ring's outer bracket plus the pad radius stays on the map.
        assert!(ROAD_R_MAX + HAVEN_RADIUS_M < ISLAND_SIZE * 0.5);
    };
}
