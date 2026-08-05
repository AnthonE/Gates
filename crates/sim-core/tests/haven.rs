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
    self, Haven, Occupant, RoadBand, ScatterTable, CELLS_PER_SIDE, CELL_SIZE, CLIFF_SLOPE_RATIO,
    HAVEN_CANDIDATES, HAVEN_CRATES, HAVEN_CRATE_R_M, HAVEN_HEIGHT_W, HAVEN_PHASE_STEP,
    HAVEN_PHASE_TRIES, HAVEN_RADIUS_M, HAVEN_SHELTER_HALF_M, HAVEN_SHELTER_R_M,
    HAVEN_SHELTER_YAW_STEP, ISLAND_SIZE, LAND_MIN_H, ROAD_INLAND_M, ROAD_R_MAX, ROAD_R_MIN,
    SEA_LEVEL,
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
        phase: 0,
        shelter: 0,
    }
}

/// The scatter cell a world position falls in. The world is positive in
/// both axes over the road bracket, so the truncating cast is a floor —
/// the same one `scatter` uses to find the pad.
fn cell_of(x: f32, z: f32) -> (i32, i32) {
    ((x / CELL_SIZE) as i32, (z / CELL_SIZE) as i32)
}

/// The selector's ring-phase search, re-derived: the first rotation whose
/// every anchor is on land and off the carriageway, or `None`. Independent
/// of `haven`'s copy on purpose — `haven_ring_phase` is private, and a test
/// that called the function under test would only prove it is deterministic.
fn ring_phase(seed: u64, x: f32, z: f32) -> Option<u8> {
    for t in 0..HAVEN_PHASE_TRIES {
        let phase = (t * HAVEN_PHASE_STEP) as u8;
        let probe = Haven {
            x,
            z,
            y: 0.0,
            relief: 0.0,
            phase,
            shelter: 0,
        };
        let ok = (0..HAVEN_CRATES).all(|k| {
            let (ax, az, _) = terrain::haven_crate(&probe, k);
            terrain::height(seed, ax, az) >= LAND_MIN_H
                && terrain::road_band(seed, ax, az) != RoadBand::Carriageway
        });
        if ok {
            return Some(phase);
        }
    }
    None
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
            // The same check chain the selector applies. It has to be here:
            // a search that scores sites the selector is forbidden to pick
            // is not an independent re-derivation of the argmax, it is a
            // different argmax. Measured — with the ring rule on one side
            // only, seed 555555 "beats" the shipped pad by 2.06 m with a
            // site whose entire container ring is under the land line.
            if ring_phase(seed, x, z).is_none() {
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

/// The exclusion zone is real and it is not a no-op: nothing SCATTERS
/// inside the pad, the only things standing there are the ones the pad
/// itself placed, and the pad is somewhere that would otherwise have been
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
        let mut crates_inside = 0usize;
        let mut cleared = 0usize;
        let mut live = 0usize;

        for cx in 0..CELLS_PER_SIDE {
            for cz in 0..CELLS_PER_SIDE {
                let s = terrain::scatter(seed, &table, &haven, cx, cz);
                if s.occupant != Occupant::None {
                    live += 1;
                    if terrain::in_haven(&haven, s.x, s.z) {
                        if s.occupant == Occupant::CrateSlot || s.occupant == Occupant::HavenShelter
                        {
                            crates_inside += 1;
                        } else {
                            inside += 1;
                        }
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
            "haven seed {seed:>12}: {inside} scattered + {crates_inside} placed slot(s) \
             inside the pad, {cleared} cleared by it, {live} live islandwide"
        );
        assert_eq!(
            inside, 0,
            "seed {seed}: {inside} SCATTERED slot(s) stand inside the {HAVEN_RADIUS_M} m \
             pad — the exclusion zone leaks (the pad's own crates are counted \
             separately and asserted by `the_pad_carries_the_containers_it_placed`)"
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

/// Every container the pad placed is standing, exactly once, where it was
/// put — and the arithmetic that makes that true rather than lucky.
///
/// The failure this exists to catch is silent. A scatter cell holds one
/// slot, so two anchors sharing a cell means the second crate simply is not
/// there: no panic, no golden move (the crate that survives hashes the same
/// bytes), no clippy wall. Nothing in the codebase would say a word. So the
/// count is re-derived by sweeping all 65,536 cells and comparing against
/// `HAVEN_CRATES`, and the separation that guarantees the count is measured
/// against the longest distance that fits inside one cell.
#[test]
fn the_pad_carries_the_containers_it_placed() {
    let table = ScatterTable::alpha_default();
    // The longest line that fits inside one 8 m cell. Two anchors further
    // apart than this cannot share a cell whatever the pad's offset against
    // the grid — which is the whole proof, and it does not depend on a seed.
    let cell_diag = (CELL_SIZE * CELL_SIZE * 2.0).sqrt();
    let mut worst_sep = f32::MAX;
    let mut worst_sep_seed = 0u64;
    let mut worst_ring_err = 0.0f32;

    for seed in SWEEP_SEEDS {
        let haven = terrain::haven(seed);
        let mut found = 0usize;
        for cx in 0..CELLS_PER_SIDE {
            for cz in 0..CELLS_PER_SIDE {
                let s = terrain::scatter(seed, &table, &haven, cx, cz);
                if s.occupant != Occupant::CrateSlot {
                    continue;
                }
                found += 1;
                // It is inside the pad, and it is on the pad's own ground.
                assert!(
                    terrain::in_haven(&haven, s.x, s.z),
                    "seed {seed}: a crate stands outside the {HAVEN_RADIUS_M} m pad"
                );
                assert!(
                    s.y >= LAND_MIN_H,
                    "seed {seed}: a crate stands at y {} — below the land line \
                     {LAND_MIN_H}. The pad is the flattest site on the ring and \
                     placement does not veto, so this says the argmax picked \
                     ground the ring cannot support, not that the crate is wrong",
                    s.y
                );
                assert_eq!(
                    s.y,
                    terrain::height(seed, s.x, s.z),
                    "seed {seed}: a crate's y is not the terrain under it"
                );
                // Placed, not drawn: no jitter, no size wobble.
                assert_eq!(s.scale, 1.0, "seed {seed}: a placed crate was scaled");
            }
        }
        assert_eq!(
            found, HAVEN_CRATES as usize,
            "seed {seed}: {found} crate(s) stand islandwide against \
             HAVEN_CRATES = {HAVEN_CRATES}. Fewer means two anchors landed in \
             one scatter cell and the second was dropped without a word — \
             widen HAVEN_CRATE_R_M or cut the count until the separation \
             below clears one cell diagonal"
        );
    }

    // The ring itself, on every seed and not only the swept four: each anchor
    // sits at `HAVEN_CRATE_R_M` from the center, and adjacent anchors clear a
    // cell diagonal. Pure geometry, so it runs 16 seeds for the price of none.
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let mut pts = [(0.0f32, 0.0f32); 256];
        for k in 0..HAVEN_CRATES {
            let (ax, az, yaw) = terrain::haven_crate(&haven, k);
            pts[k as usize] = (ax, az);
            let (dx, dz) = (ax - haven.x, az - haven.z);
            let r = (dx * dx + dz * dz).sqrt();
            let err = (r - HAVEN_CRATE_R_M).max(HAVEN_CRATE_R_M - r);
            worst_ring_err = worst_ring_err.max(err);
            // Faces the center: the inward unit vector and the yaw's own
            // direction agree, so a crate never has its back to the pad.
            let (fx, fz) = yaw_dir((yaw as u16) << 8);
            let dot = fx * (-dx / r) + fz * (-dz / r);
            assert!(
                dot > 0.99,
                "seed {seed}: crate {k}'s yaw points {dot} at the pad center — \
                 the inward half-turn is off"
            );
        }
        for a in 0..HAVEN_CRATES as usize {
            for b in (a + 1)..HAVEN_CRATES as usize {
                let (dx, dz) = (pts[a].0 - pts[b].0, pts[a].1 - pts[b].1);
                let sep = (dx * dx + dz * dz).sqrt();
                if sep < worst_sep {
                    worst_sep = sep;
                    worst_sep_seed = seed;
                }
            }
        }
    }
    println!(
        "haven crates: {HAVEN_CRATES} on a {HAVEN_CRATE_R_M} m ring; closest pair \
         {worst_sep:.3} m (seed {worst_sep_seed}) against a {cell_diag:.3} m cell \
         diagonal; worst radius error {worst_ring_err:.4} m"
    );
    assert!(
        worst_sep > cell_diag,
        "the closest two anchors are {worst_sep:.3} m apart and one scatter cell \
         is {cell_diag:.3} m corner to corner — two crates can now land in one \
         cell, and the loser vanishes silently"
    );
    assert!(
        worst_ring_err < 0.01,
        "worst anchor is {worst_ring_err:.4} m off the {HAVEN_CRATE_R_M} m ring"
    );
}

/// The destination outpays the route to it — the defect the coast road left
/// behind, as a number.
///
/// `ROAD_BARREL_PERMILLE` was sourced from the beach row of the biome table,
/// so walking the loop paid exactly what standing on the spawn beach paid
/// (`findings/pass-20260804-205133-01-judge.md`, ranked gap 2). This
/// compares containers per square meter at the two ends of that walk. It
/// deliberately measures the shoulder's area by counting its cells rather
/// than by an annulus formula: the ring wobbles with the coastline, and an
/// idealized area would flatter whichever side the arithmetic was written
/// for.
#[test]
fn the_pad_outpays_the_road_that_leads_to_it() {
    let table = ScatterTable::alpha_default();
    let cell_area = CELL_SIZE * CELL_SIZE;
    let pad_area = core::f32::consts::PI * HAVEN_RADIUS_M * HAVEN_RADIUS_M;
    let mut worst_ratio = f32::MAX;

    for seed in SWEEP_SEEDS {
        let haven = terrain::haven(seed);
        let mut shoulder_cells = 0usize;
        let mut shoulder_barrels = 0usize;
        for cx in 0..CELLS_PER_SIDE {
            for cz in 0..CELLS_PER_SIDE {
                // Cell center, so the band a cell is counted in does not
                // depend on where its slot's jitter happened to land.
                let x = cx as f32 * CELL_SIZE + CELL_SIZE * 0.5;
                let z = cz as f32 * CELL_SIZE + CELL_SIZE * 0.5;
                if terrain::road_band(seed, x, z) != RoadBand::Shoulder {
                    continue;
                }
                shoulder_cells += 1;
                if terrain::scatter(seed, &table, &haven, cx, cz).occupant == Occupant::BarrelSlot {
                    shoulder_barrels += 1;
                }
            }
        }
        assert!(
            shoulder_barrels > 0,
            "seed {seed}: no barrels on the shoulder — the comparison below \
             would divide by the road not existing"
        );
        let road_density = shoulder_barrels as f32 / (shoulder_cells as f32 * cell_area);
        let pad_density = HAVEN_CRATES as f32 / pad_area;
        let ratio = pad_density / road_density;
        println!(
            "haven prize seed {seed:>12}: pad {HAVEN_CRATES} container(s) over \
             {pad_area:.0} m2 = 1 per {:.0} m2; shoulder {shoulder_barrels} over \
             {} m2 = 1 per {:.0} m2; ratio {ratio:.2}x",
            pad_area / HAVEN_CRATES as f32,
            shoulder_cells as f32 * cell_area,
            1.0 / road_density,
        );
        worst_ratio = worst_ratio.min(ratio);
    }
    println!("haven prize: worst container-density ratio {worst_ratio:.2}x pad over road");
    assert!(
        worst_ratio >= HAVEN_PRIZE_RATIO_MIN,
        "the pad concentrates containers only {worst_ratio:.2}x denser than the \
         road shoulder — below the {HAVEN_PRIZE_RATIO_MIN}x floor, the \
         destination is a stretch of road with a circle drawn on it"
    );
}

/// Containers per square meter on the pad, over the same on the road
/// shoulder. Measured worst 2.30 across the swept seeds; floored ~15% under
/// it, low enough that coastline variance cannot trip it and high enough
/// that losing a crate or doubling the shoulder rate does
/// (DECISIONS.md §open: haven crates v0).
const HAVEN_PRIZE_RATIO_MIN: f32 = 2.0;

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

/// The pad carries its shelter — one of it, at the center, facing a gap in
/// the ring, on a cell no container can take.
///
/// Same failure class as the containers and the same method: a scatter cell
/// holds one slot, so a structure that lost its cell would simply not be
/// there — no panic, no golden move, no clippy wall, and a destination that
/// is once again a circle where the trees stop. The count is re-derived by
/// sweeping all 65,536 cells rather than trusting the branch that writes it.
///
/// It also asserts the two arithmetic claims the greybox rests on, because
/// neither is visible from the mesh: the footprint clears every container it
/// stands among, and the whole of it stands on ground the exclusion zone
/// already cleared. `ci/haven_shelter.mjs` holds the client to the same two
/// numbers from the other side.
#[test]
fn the_pad_carries_the_shelter_at_its_center() {
    let table = ScatterTable::alpha_default();
    // Half the footprint's diagonal: the circle that circumscribes the
    // structure whatever yaw it is drawn at, which is what a distance test
    // against an unrotated square would miss.
    let corner = HAVEN_SHELTER_HALF_M * (2.0f32).sqrt();
    // Half an anchor gap, one LUT step in from it to absorb the truncation
    // an odd crate count forces — as a COSINE, read off the same 256-entry
    // LUT the bearings come from, so the bound needs no trig and no
    // second-hand degree value (wall 1). The cosine is the SECOND component:
    // `yaw_lut` documents index 0 as facing +Z, so an entry is (sin, cos).
    // Reading the first as a bound gave 0.556 for 33.75 degrees and would
    // have failed a structure that was correctly placed.
    let gap = 256 / HAVEN_CRATES;
    let (_, gap_cos_max) = yaw_dir(((gap / 2 - 1) as u16) << 8);
    let mut worst_clearance = f32::MAX;
    let mut worst_clearance_seed = 0u64;

    for seed in SWEEP_SEEDS {
        let haven = terrain::haven(seed);
        let (sx, sz, syaw) = terrain::haven_shelter(&haven);

        let mut found = 0usize;
        let mut standing: Option<terrain::Slot> = None;
        for cx in 0..CELLS_PER_SIDE {
            for cz in 0..CELLS_PER_SIDE {
                let s = terrain::scatter(seed, &table, &haven, cx, cz);
                if s.occupant == Occupant::HavenShelter {
                    found += 1;
                    standing = Some(s);
                }
            }
        }
        assert_eq!(
            found, 1,
            "seed {seed}: {found} shelter(s) islandwide, expected exactly 1 — \
             a scatter cell holds one slot, so anything but 1 is a silent \
             drop or a duplicate the pad never authored"
        );
        let s = standing.unwrap();

        // Placed, not drawn: the slot is the pure function's answer, bit for
        // bit, with no cell jitter and no scale wobble anywhere in it.
        assert_eq!(
            (s.x, s.z),
            (sx, sz),
            "seed {seed}: shelter is not standing where haven_shelter put it"
        );
        assert_eq!(
            s.yaw, syaw,
            "seed {seed}: shelter yaw is not the authored facing"
        );
        assert_eq!(
            s.scale, 1.0,
            "seed {seed}: the shelter is scaled — a monument is not weather"
        );
        assert_eq!(
            s.y,
            terrain::height(seed, s.x, s.z),
            "seed {seed}: the shelter is not standing on its own ground"
        );
        assert!(
            s.y >= LAND_MIN_H,
            "seed {seed}: shelter at {:.2} m is under the land line",
            s.y
        );

        // The structure stands beside the loop, not across it — the whole
        // reason it is off center at all, and the condition `tests/road.rs`
        // caught when this first shipped at the pad's middle.
        assert!(
            terrain::road_band(seed, sx, sz) != RoadBand::Carriageway,
            "seed {seed}: the shelter stands on the carriageway, blocking \
             the road the pad is reached by"
        );
        let off_center = ((sx - haven.x) * (sx - haven.x) + (sz - haven.z) * (sz - haven.z)).sqrt();
        assert!(
            (off_center - HAVEN_SHELTER_R_M).max(HAVEN_SHELTER_R_M - off_center) < 0.01,
            "seed {seed}: shelter stands {off_center:.3} m off center, not \
             {HAVEN_SHELTER_R_M} m"
        );

        // The whole footprint stands on ground the exclusion zone cleared,
        // so no tree can grow through a wall. Tested at the corners, which
        // is where a square first leaves a circle, and from the pad's own
        // center rather than the structure's.
        for k in 0..4u16 {
            let (dx, dz) = yaw_dir((k * 64) << 8);
            let (px, pz) = (sx + dx * corner, sz + dz * corner);
            assert!(
                terrain::in_haven(&haven, px, pz),
                "seed {seed}: a shelter corner at ({px:.1}, {pz:.1}) reaches \
                 outside the {HAVEN_RADIUS_M} m pad, onto ground scatter fills"
            );
        }

        // No container stands inside a wall, and none shares the shelter's
        // cell — the second is what `haven_ring_phase` refuses sites for,
        // re-derived here from the shipped anchors rather than believed.
        let scell = cell_of(sx, sz);
        for k in 0..HAVEN_CRATES {
            let (ax, az, _) = terrain::haven_crate(&haven, k);
            let d = ((ax - sx) * (ax - sx) + (az - sz) * (az - sz)).sqrt();
            assert!(
                d > corner,
                "seed {seed}: container {k} at {d:.2} m is inside the \
                 shelter's {corner:.2} m corner radius"
            );
            assert_ne!(
                cell_of(ax, az),
                scell,
                "seed {seed}: container {k} shares the shelter's scatter \
                 cell — one of the two is not in the world"
            );
            if d - corner < worst_clearance {
                worst_clearance = d - corner;
                worst_clearance_seed = seed;
            }

            // The structure stands in a GAP in the ring, not behind a
            // container. Measured as an ANGLE off the pad center between
            // the two shipped positions, deliberately: comparing the yaw
            // bytes is what put the first draft exactly on container 3 at
            // every seed, because `haven_crate`'s yaw faces IN and that
            // draft's faced OUT, and the collision is invisible in the
            // numbers. A dot product cannot be fooled by which end of a
            // heading a field means.
            let dot = ((ax - haven.x) * (sx - haven.x) + (az - haven.z) * (sz - haven.z))
                / (HAVEN_CRATE_R_M * HAVEN_SHELTER_R_M);
            assert!(
                dot < gap_cos_max,
                "seed {seed}: the shelter's bearing is within {dot:.3} \
                 cosine of container {k} (bound {gap_cos_max:.3}) — it \
                 should stand in a gap between two of them"
            );
        }
    }
    println!(
        "haven shelter: tightest container clearance {worst_clearance:.2} m \
         past the {corner:.2} m corner radius (seed {worst_clearance_seed})"
    );

    // Non-vacuity for the shared-cell rule, by construction rather than by
    // seed luck. The shelter and a container are ~6 m apart against an
    // 11.31 m cell diagonal, so a shared cell is not merely possible, it is
    // ordinary: place a pad so that a gap bearing and its neighbouring
    // anchor fall in one cell and the search must reject that bearing. If
    // this ever stops being reachable the rule is dead code, and a dead
    // guard reads exactly like a working one.
    let mut reachable = 0usize;
    for i in 0..64u32 {
        // Sub-cell offsets sweep the pad across its own grid alignment,
        // which is the variable the collision actually depends on.
        let probe = Haven {
            x: 8.0 * CELL_SIZE + (i % 8) as f32 * (CELL_SIZE / 8.0),
            z: 8.0 * CELL_SIZE + (i / 8) as f32 * (CELL_SIZE / 8.0),
            y: 0.0,
            relief: 0.0,
            phase: 0,
            shelter: HAVEN_SHELTER_YAW_STEP as u8,
        };
        let (px, pz, _) = terrain::haven_shelter(&probe);
        if (0..HAVEN_CRATES).any(|k| {
            let (ax, az, _) = terrain::haven_crate(&probe, k);
            cell_of(ax, az) == cell_of(px, pz)
        }) {
            reachable += 1;
        }
    }
    println!(
        "haven shelter: the shared-cell collision the search refuses is \
         reachable at {reachable} of 64 sub-cell alignments"
    );
    assert!(
        reachable > 0,
        "the collision the shared-cell rule exists to refuse is not \
         reachable at all — either the rule or this construction is stale"
    );
}
