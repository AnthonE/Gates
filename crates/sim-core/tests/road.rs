//! The coast road, gated as arithmetic (TERRAIN.md §7's `test_terrain_gameplay`,
//! the "road ring is closed and walkable" half).
//!
//! Every assertion here is a count, a radius, a spacing or a grade — numbers
//! that fail in seconds. The gate re-derives the road from `terrain::height`
//! rather than trusting `road_band`, and it re-derives the clearance from the
//! slot list rather than trusting the veto, so a veto that stopped firing
//! reddens instead of passing. Same shape as `spawn_ring_lands_on_a_clear_beach`.

// The measurements ARE the gate's output — a floor is only readable next to
// the number it was set from, so this file prints them into the CI log. The
// L5 wall bans format/print in SIM code; a test harness is not sim code
// (same reasoning, same allow, as `examples/probe.rs`).
#![allow(clippy::disallowed_macros)]

use sim_core::fmath::fabs;
use sim_core::terrain::{
    self, Occupant, RoadBand, ScatterTable, CELLS_PER_SIDE, CELL_SIZE, CLIFF_SLOPE_RATIO,
    ISLAND_SIZE, ROAD_BARREL_PERMILLE, ROAD_BAY_BARREL_PERMILLE, ROAD_HALF_W, ROAD_INLAND_M,
    ROAD_OPEN_BARREL_PERMILLE, ROAD_R_MAX, ROAD_R_MIN, ROAD_SHOULDER_HALF_W, SEA_LEVEL,
};
use sim_core::yaw_dir;

const SEEDS: [u64; 4] = [0x0047_4154_4553, 0x1, 0xDEAD_BEEF, 0x5EED];
/// Bearings marched per seed. 64 of the yaw LUT's 256 entries, so each is an
/// exact table lookup — no interpolation, no trig (wall 1).
const BEARINGS: u16 = 64;
/// Radial march step, meters. Must be under the carriageway's full radial
/// width (2 × ROAD_HALF_W = 4 m) or a march could step over the road.
const MARCH_M: f32 = 1.0;

fn center() -> f32 {
    ISLAND_SIZE * 0.5
}

/// The ring is closed: every bearing off the island center crosses the
/// carriageway at least once inside the radial bracket. A gap here is a
/// circulation loop a player can be cut off from, which is the whole point
/// of the road (TERRAIN.md §5 "where do I go?").
#[test]
fn road_ring_is_closed_on_every_bearing() {
    let c = center();
    let mut worst_hits = usize::MAX;
    let mut r_lo = f32::MAX;
    let mut r_hi = 0.0f32;

    for seed in SEEDS {
        for b in 0..BEARINGS {
            let (ux, uz) = yaw_dir((b * (256 / BEARINGS)) << 8);
            let mut hits = 0usize;
            let mut d = ROAD_R_MIN;
            while d <= ROAD_R_MAX {
                if terrain::ring_band(seed, c + ux * d, c + uz * d) == RoadBand::Carriageway {
                    hits += 1;
                    r_lo = r_lo.min(d);
                    r_hi = r_hi.max(d);
                }
                d += MARCH_M;
            }
            assert!(
                hits > 0,
                "seed {seed:#x} bearing {b}: no carriageway anywhere in \
                 [{ROAD_R_MIN}, {ROAD_R_MAX}] m — the ring is open, so the \
                 loop does not circulate. Either the bracket no longer holds \
                 the shoreline or the window test stopped finding crossings."
            );
            worst_hits = worst_hits.min(hits);
        }
    }
    println!(
        "road ring: closed on {} bearings x {} seeds; thinnest crossing {worst_hits} m of \
         carriageway; ring radius spans {r_lo:.0}-{r_hi:.0} m",
        BEARINGS,
        SEEDS.len()
    );
    // The carriageway is 2 x ROAD_HALF_W wide radially by construction. A
    // bearing that reads much thinner than that is the window test degrading,
    // not a narrow road.
    assert!(
        worst_hits as f32 >= ROAD_HALF_W,
        "thinnest crossing {worst_hits} m is under ROAD_HALF_W {ROAD_HALF_W} m — \
         the radial window is not resolving to its stated width"
    );
    // The bracket must not be doing the work: a ring pinned to either end of
    // it means the shoreline moved outside and the road is being clamped.
    assert!(
        r_lo > ROAD_R_MIN && r_hi < ROAD_R_MAX,
        "ring radius {r_lo:.0}-{r_hi:.0} m touches the bracket \
         [{ROAD_R_MIN}, {ROAD_R_MAX}] — widen the bracket, do not clamp the road"
    );
}

/// The carriageway is clear, and it is clear because `scatter` vetoed it —
/// re-derived from the slot list, not from the veto's own return value.
#[test]
fn carriageway_is_clear_and_the_shoulder_carries_barrels() {
    let table = ScatterTable::alpha_default();
    let mut worst_shoulder_barrels = usize::MAX;
    let mut worst_ratio = f32::MAX;

    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let mut on_carriageway = 0usize;
        let mut shoulder_barrels = 0usize;
        let mut shoulder_cells = 0usize;
        let mut live = 0usize;

        for cx in 0..CELLS_PER_SIDE {
            for cz in 0..CELLS_PER_SIDE {
                let s = terrain::scatter(seed, &table, &haven, cx, cz);
                if s.occupant == Occupant::None {
                    continue;
                }
                live += 1;
                // `road_band`, not `ring_band`: this is classifying what
                // `scatter` actually emitted, and a barrel on a side road's
                // shoulder is a real barrel on a real road.
                match terrain::road_band(seed, &haven, s.x, s.z) {
                    RoadBand::Carriageway => on_carriageway += 1,
                    RoadBand::Shoulder => {
                        shoulder_cells += 1;
                        if s.occupant == Occupant::BarrelSlot {
                            shoulder_barrels += 1;
                        }
                    }
                    RoadBand::Off => {}
                }
            }
        }

        assert_eq!(
            on_carriageway, 0,
            "seed {seed:#x}: {on_carriageway} slots stand on the carriageway — \
             the road surface is not clear, so the loop is not walkable"
        );
        println!(
            "seed {seed:#x}: {live} live slots, {shoulder_barrels} barrels on {shoulder_cells} \
             occupied shoulder cells"
        );
        worst_shoulder_barrels = worst_shoulder_barrels.min(shoulder_barrels);
        // Barrels should dominate what stands on the shoulder: the road draw
        // fires at ROAD_BARREL_PERMILLE before the biome table is consulted.
        if shoulder_cells > 0 {
            worst_ratio = worst_ratio.min(shoulder_barrels as f32 / shoulder_cells as f32);
        }
    }

    // A loot route needs enough barrels to be worth walking. Both floors sit
    // one margin under the MEASURED worst seed, not far under it: at the
    // original 40 / 20% a 60% collapse in the route stayed green, which is a
    // floor that records an intention rather than guarding a number.
    // Measured worst: 103 barrels, 52.2% of occupied shoulder cells; the
    // margin is ~20% for coastline variation (DECISIONS.md §open: coast road
    // v0), so 80 and 42%.
    //
    // **The ratio was re-measured at forest density v1 (2026-09-14) and the
    // count was not, which is the point of keeping both.** The road draw
    // fires before the biome table is consulted, so the barrel COUNT is
    // invariant to the table — measured identical across the change (111 /
    // 123 / 149 / 96 → 95 per seed, the one cell a blended forest-edge row
    // now hands to a tree ahead of its barrel entry). The RATIO is not: a
    // shoulder cell whose road roll fails draws the biome row, and a forest
    // section's row now puts a tree on ~86% of its cells where it put one on
    // a quarter, so what stands beside the barrels tripled. Measured 46 / 37
    // / 49 / 39% after (56 / 50 / 64 / 52% before); the same ~20% margin
    // under the worst is 30%. A road through a forest is lined with trees.
    // What this floor still catches is the mechanism: a barrel branch that
    // ran AFTER the table would read ~10% here, not 37.
    assert!(
        worst_shoulder_barrels >= 80,
        "worst seed puts only {worst_shoulder_barrels} barrels on the road \
         shoulder (measured 103 when this floor was set) — the route does not \
         pay, so nobody walks it"
    );
    assert!(
        worst_ratio >= 0.30,
        "barrels are only {:.0}% of what stands on the shoulder (measured 37% \
         at the worst seed when this floor was set, 52% before forest density \
         v1) — the road draw is being outvoted by the biome table",
        worst_ratio * 100.0
    );
}

/// The road is walkable: it is a coastal contour at a fixed inland offset, so
/// it should be near-flat by construction. This asserts that it actually is,
/// which is what makes it a circulation loop rather than a line on a cliff.
#[test]
fn the_road_is_walkable_along_its_length() {
    let c = center();
    let mut sampled = 0usize;
    let mut cliffed = 0usize;
    let mut worst_slope = 0.0f32;

    for seed in SEEDS {
        for b in 0..BEARINGS {
            let (ux, uz) = yaw_dir((b * (256 / BEARINGS)) << 8);
            let mut d = ROAD_R_MIN;
            while d <= ROAD_R_MAX {
                let (x, z) = (c + ux * d, c + uz * d);
                if terrain::ring_band(seed, x, z) == RoadBand::Carriageway {
                    let s = terrain::slope(seed, x, z);
                    sampled += 1;
                    worst_slope = worst_slope.max(s);
                    if s > CLIFF_SLOPE_RATIO {
                        cliffed += 1;
                    }
                }
                d += MARCH_M;
            }
        }
    }

    assert!(
        sampled > 0,
        "no road sampled — the closure test should have caught this first"
    );
    let frac = cliffed as f32 / sampled as f32;
    println!(
        "road walkability: {sampled} carriageway samples, {cliffed} over the cliff ratio \
         ({:.1}%), worst slope {worst_slope:.2}",
        frac * 100.0
    );
    // Not zero: a coastline can turn a headland into a cliff, and clamping the
    // road off it would open the ring. The bound is that cliff is the
    // exception — a road that is mostly cliff is not a route.
    assert!(
        frac < 0.10,
        "{:.1}% of the road is steeper than the cliff ratio {CLIFF_SLOPE_RATIO} — \
         a player cannot walk the loop",
        frac * 100.0
    );
}

/// The road sits where TERRAIN.md stage 7 says: inland of the shoreline by
/// ROAD_INLAND_M, on land, above water. Re-derived from `height` alone.
#[test]
fn the_road_runs_inland_of_the_shoreline() {
    let c = center();
    let mut checked = 0usize;
    let mut min_h = f32::MAX;
    let mut worst_inland_lo = f32::MAX;
    let mut worst_inland_hi = 0.0f32;
    let mut worst_nearest = f32::MAX;

    for seed in SEEDS {
        for b in 0..BEARINGS {
            let (ux, uz) = yaw_dir((b * (256 / BEARINGS)) << 8);
            let mut d = ROAD_R_MIN;
            while d <= ROAD_R_MAX {
                let (x, z) = (c + ux * d, c + uz * d);
                if terrain::ring_band(seed, x, z) == RoadBand::Carriageway {
                    let h = terrain::height(seed, x, z);
                    min_h = min_h.min(h);
                    // Walk seaward from the road and find the water, rather
                    // than re-testing the window `road_band` already tested.
                    // First crossing, so a sandbar further out cannot flatter
                    // the answer — this is the distance a player actually
                    // walks from the road to the sea.
                    // Two different questions, and until world structure v1
                    // gave the coast headlands and coves they had one answer.
                    //
                    // `road_band` promises ONE thing: the shoreline crossing
                    // that lies ~`ROAD_INLAND_M` out along this radial is
                    // inside the carriageway window. On a near-circular coast
                    // the first water a seaward walk meets IS that crossing,
                    // so the old single walk tested the promise. A cove cuts
                    // the same radial closer in — the road is then on a spit
                    // with water on two sides — and the first crossing stops
                    // being the one `road_band` placed against. Reading it as
                    // the offset reports a road that "moved", which it did
                    // not: what appeared is a second shore.
                    //
                    // So: `inland` is the crossing NEAREST the stated offset,
                    // which is exactly what `road_band` claims and is held to
                    // the same tolerance it always was; `nearest` is the
                    // first water in any direction along the radial, held to
                    // a floor that only says the loop is not in the surf.
                    let mut inland = f32::MAX;
                    let mut nearest = f32::MAX;
                    let mut step = 0.0f32;
                    while step <= ROAD_INLAND_M + 20.0 {
                        let rr = d + step;
                        if terrain::height(seed, c + ux * rr, c + uz * rr) <= SEA_LEVEL {
                            if nearest == f32::MAX {
                                nearest = step;
                            }
                            if fabs(step - ROAD_INLAND_M) < fabs(inland - ROAD_INLAND_M) {
                                inland = step;
                            }
                        }
                        step += MARCH_M;
                    }
                    worst_nearest = worst_nearest.min(nearest);
                    assert!(
                        inland < f32::MAX,
                        "seed {seed:#x} bearing {b} r {d:.0}: no water within \
                         {} m seaward — this is not a coast road",
                        ROAD_INLAND_M + 20.0
                    );
                    worst_inland_lo = worst_inland_lo.min(inland);
                    worst_inland_hi = worst_inland_hi.max(inland);
                    checked += 1;
                }
                d += MARCH_M;
            }
        }
    }

    println!(
        "road placement: {checked} carriageway samples; the crossing it was \
         placed against is {worst_inland_lo:.0}-{worst_inland_hi:.0} m seaward \
         (target {ROAD_INLAND_M} m); nearest water on any radial \
         {worst_nearest:.0} m; lowest road ground {min_h:.2} m"
    );
    assert!(
        min_h > SEA_LEVEL,
        "the road dips to {min_h:.2} m — part of the loop is underwater"
    );
    // The band is the shoulder width either side of the stated offset: that
    // is the tolerance `road_band`'s window buys, and nothing wider is
    // "offset ~40 m inland". A drift outside it means the road stopped
    // tracking the coastline and started tracking the radial bracket.
    assert!(
        worst_inland_lo >= ROAD_INLAND_M - ROAD_SHOULDER_HALF_W
            && worst_inland_hi <= ROAD_INLAND_M + ROAD_SHOULDER_HALF_W,
        "the sea is {worst_inland_lo:.0}-{worst_inland_hi:.0} m seaward of the road, \
         outside {ROAD_INLAND_M} ± {ROAD_SHOULDER_HALF_W} m — the ring is no longer \
         a fixed offset from the coastline"
    );
    // CLAIM 2 — and a cove on the other side is still not the surf. Measured
    // 24 m at the worst of 1,036 carriageway samples over four seeds once the
    // coast had coves in it (world structure v1); the floor is under that
    // with room, and its job is to catch a road that starts hugging an inlet
    // rather than to pin the number.
    assert!(
        worst_nearest >= ROAD_NEAREST_WATER_MIN_M,
        "the nearest water to the road is {worst_nearest:.0} m — under the \
         {ROAD_NEAREST_WATER_MIN_M} m floor the loop is running along the \
         inside of an inlet, not inland of the coast"
    );
}

/// Floor on the nearest water to the carriageway, metres, in ANY direction
/// along its own radial — as opposed to the crossing `road_band` placed the
/// road against, which `ROAD_INLAND_M ± ROAD_SHOULDER_HALF_W` above still
/// pins exactly. A cove reaching across a radial is why the two numbers are
/// not the same one (world structure v1, 2026-09-15); measured worst 24 m.
const ROAD_NEAREST_WATER_MIN_M: f32 = 18.0;

/// The road's own numbers stay coherent with the scatter grid it vetoes
/// against. A carriageway narrower than a scatter cell would let slots
/// straddle it; a shoulder wider than the cell grid would swamp the biomes.
/// These are relations between constants, so they hold at compile time —
/// a `const` block fails the build rather than a test run, which is strictly
/// earlier than the gate that would have caught it.
#[test]
fn road_widths_stay_inside_the_scatter_grid() {
    const {
        assert!(
            ROAD_SHOULDER_HALF_W > ROAD_HALF_W,
            "the shoulder must lie outside the carriageway"
        );
        assert!(
            ROAD_SHOULDER_HALF_W * 2.0 <= CELL_SIZE * 2.0,
            "the shoulder band is wider than two scatter cells — the barrel \
             draw would displace whole biomes, not line a road"
        );
        assert!(
            ROAD_BARREL_PERMILLE <= 1000,
            "ROAD_BARREL_PERMILLE is not a per-mille"
        );
        assert!(
            ROAD_INLAND_M > ROAD_SHOULDER_HALF_W,
            "the road would overlap the water it is offset from"
        );
    }
}

// ── Bays: the route stops being uniform (TERRAIN.md §1 stage 7) ────────────
//
// Three separable claims, because the item can fail three ways and only one
// of them is "the numbers are wrong":
//
//   1. the classification is COHERENT — bays are arcs a player can learn,
//      not per-cell speckle (`reference/SPAWN.md` §9.3, applied to the road)
//   2. both populations EXIST on every seed — a classifier that answers
//      constant is the failure that looks most like a working gate
//   3. the bay PAYS MORE while the route's total pay is CONSERVED
//
// Claim 3's second half is the load-bearing one. `tests/haven.rs`'s
// HAVEN_PRIZE_RATIO_MIN prices the pad against the shoulder it replaces and
// its own doc says the floor sits high enough that doubling the shoulder
// rate trips it, so a bay bought by inflating the road would have been paid
// for out of the destination's lead. Asserting the mean here is what stops a
// later pass from tuning ROAD_BAY_BARREL_PERMILLE up on its own.

/// Bearings swept for the coherence claim. The same 64 the ring-closure test
/// above proves a crossing on — deliberately not finer. A first draft swept
/// 128 and died on seed 0xDEADBEEF bearing 53, a radial with no carriageway
/// crossing at any radius in the bracket. That is a real, newly measured
/// fact about the ring and it is recorded in `NOW.md`; it is NOT this test's
/// to assert, because a radial that misses the road does not prove a break a
/// player could walk into — inside an inlet the ring doubles back, and one
/// radial can miss a road that continues. Diagnosing that wants its own
/// pass. Sweeping the proven resolution keeps this test measuring bays.
const BAY_BEARINGS: u16 = 64;

/// A bay is an arc, not a speckle: walking the ring you cross the boundary a
/// handful of times, not on every other step.
///
/// This is the property that separates "the coast decides" from "the hash
/// decides". `in_bay` reads the coastline through two `height` taps, and a
/// coastline is smooth at the ~900 m wavelength COAST_FREQ gives it — so a
/// correct classifier MUST come out in runs. One that came out speckled
/// would still pass a density ratio (half the cells would still be denser);
/// it would just mean nothing to a player, who cannot learn a pattern with
/// no extent. Counting transitions is the cheapest way to say that.
#[test]
fn bays_are_arcs_of_coast_not_speckle() {
    let c = center();
    let mut worst_runs = 0usize;
    let mut worst_share = (f32::MAX, f32::MIN);

    for seed in SEEDS {
        // Classify one point per bearing: the first carriageway crossing on
        // that radial, so the sweep tracks the ring's wobble rather than a
        // circle drawn through it.
        let mut ring: Vec<bool> = Vec::with_capacity(BAY_BEARINGS as usize);
        for b in 0..BAY_BEARINGS {
            let (ux, uz) = yaw_dir((b * (256 / BAY_BEARINGS)) << 8);
            let mut d = ROAD_R_MIN;
            let mut found = false;
            while d <= ROAD_R_MAX {
                let (px, pz) = (c + ux * d, c + uz * d);
                if terrain::ring_band(seed, px, pz) == RoadBand::Carriageway {
                    ring.push(terrain::in_bay(seed, px, pz));
                    found = true;
                    break;
                }
                d += MARCH_M;
            }
            assert!(
                found,
                "seed {seed:#x} bearing {b}: no carriageway on this radial, \
                 which `road_ring_is_closed_on_every_bearing` asserts against \
                 on these same 64 bearings — so that test reddens too, and it \
                 is the one that owns this claim. The sweep below would be \
                 classifying a gap as coast"
            );
        }

        // Transitions around the closed ring, so the wrap is counted once.
        let n = ring.len();
        let mut transitions = 0usize;
        for i in 0..n {
            if ring[i] != ring[(i + 1) % n] {
                transitions += 1;
            }
        }
        // Transitions come in pairs on a closed loop (every arc you enter
        // you leave), so arcs = transitions / 2.
        let arcs = transitions / 2;
        let bays = ring.iter().filter(|b| **b).count();
        let share = bays as f32 / n as f32;
        println!(
            "bay arcs seed {seed:#x}: {bays}/{n} bearings sheltered ({:.0}%), \
             {arcs} arc(s), {transitions} transitions",
            share * 100.0
        );

        // CLAIM 2 — both populations exist. A classifier stuck at either rail
        // makes claim 3 vacuous (one of its two samples would be empty) and
        // is exactly what a broken probe returns.
        assert!(
            bays > 0 && bays < n,
            "seed {seed:#x}: `in_bay` answers a constant {} on all {n} \
             bearings — the coast probes are not reading the coastline, so \
             the whole redistribution is a no-op wearing two constants",
            bays > 0
        );
        worst_share.0 = worst_share.0.min(share);
        worst_share.1 = worst_share.1.max(share);

        // CLAIM 1 — coherence. The coast wobble is a 2-octave fBm at
        // COAST_FREQ = 1/900 m, so around a ~6 km ring it makes a handful of
        // lobes; half of each is a bay. The cap is set BETWEEN two
        // measurements rather than from theory: the shipped classifier gives
        // 2-5 arcs across these seeds, and `in_bay` mutated to per-cell
        // parity — the cheapest speckle there is — gives 14, 14, 17, 15.
        // n/4 = 16 was the first draft and it caught only the third of those,
        // which is a gate that passes the defect it names. n/8 = 8 is 60%
        // above the worst real seed and 43% under the weakest speckle one.
        assert!(
            arcs > 0 && arcs <= (n / 8),
            "seed {seed:#x}: {arcs} bay arcs over {n} bearings — the \
             classification is speckle, not coastline. A bay a player cannot \
             walk the length of is a per-cell dice roll with a nautical name"
        );
        worst_runs = worst_runs.max(arcs);
    }
    println!(
        "bay arcs: worst {worst_runs} arcs; sheltered share spans \
         {:.0}-{:.0}% across {} seeds",
        worst_share.0 * 100.0,
        worst_share.1 * 100.0,
        SEEDS.len()
    );
}

/// The bay pays more than the open coast, and the road pays what it always
/// paid. Both halves, or the item is a raise with a story.
#[test]
fn bays_concentrate_the_route_without_enriching_it() {
    let table = ScatterTable::alpha_default();
    let mut worst_ratio = f32::MAX;
    let mut agg_barrels = 0usize;
    let mut agg_cells = 0usize;
    let mut agg_bay_cells = 0usize;

    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let (mut bay_cells, mut bay_barrels) = (0usize, 0usize);
        let (mut open_cells, mut open_barrels) = (0usize, 0usize);

        for cx in 0..CELLS_PER_SIDE {
            for cz in 0..CELLS_PER_SIDE {
                // Cell centers, like `the_pad_outpays_the_road_that_leads_to_it`
                // and for the same reason: the band a cell is counted in must
                // not depend on where its jitter landed. Both populations pay
                // the same distortion, so the ratio between them is clean.
                let x = cx as f32 * CELL_SIZE + CELL_SIZE * 0.5;
                let z = cz as f32 * CELL_SIZE + CELL_SIZE * 0.5;
                if terrain::ring_band(seed, x, z) != RoadBand::Shoulder {
                    continue;
                }
                let barrel =
                    terrain::scatter(seed, &table, &haven, cx, cz).occupant == Occupant::BarrelSlot;
                if terrain::in_bay(seed, x, z) {
                    bay_cells += 1;
                    bay_barrels += usize::from(barrel);
                } else {
                    open_cells += 1;
                    open_barrels += usize::from(barrel);
                }
            }
        }

        assert!(
            bay_cells > 0 && open_cells > 0,
            "seed {seed:#x}: {bay_cells} bay / {open_cells} open shoulder \
             cells — one population is empty, so the comparison below is not \
             a comparison"
        );
        let bay_rate = bay_barrels as f32 / bay_cells as f32;
        let open_rate = open_barrels as f32 / open_cells as f32;
        assert!(
            open_rate > 0.0,
            "seed {seed:#x}: the open coast carries no barrels at all — the \
             redistribution emptied the route instead of shaping it"
        );
        let ratio = bay_rate / open_rate;
        println!(
            "bay slots seed {seed:#x}: bay {bay_barrels}/{bay_cells} = \
             {:.0} per-mille; open {open_barrels}/{open_cells} = {:.0} \
             per-mille; ratio {ratio:.2}x",
            bay_rate * 1000.0,
            open_rate * 1000.0
        );
        worst_ratio = worst_ratio.min(ratio);
        agg_barrels += bay_barrels + open_barrels;
        agg_cells += bay_cells + open_cells;
        agg_bay_cells += bay_cells;
    }

    // CLAIM 3a — the bay is worth walking to. The knobs are set 430/170
    // per-mille = 2.53x, and the floor is under the measured worst seed with
    // room for coastline variance, not under the knob ratio: what a player
    // meets is the realized rate, and the two differ because the shoulder's
    // biome draw fires on cells the road draw declines.
    assert!(
        worst_ratio >= BAY_RATIO_MIN,
        "the bay carries only {worst_ratio:.2}x the open coast's barrels — \
         under the {BAY_RATIO_MIN}x floor a player cannot tell the two \
         stretches apart, which is the uniform road this replaced"
    );

    // CLAIM 3b — and the route as a whole was not enriched to pay for it.
    // This is the assert that keeps the item honest: raising
    // ROAD_BAY_BARREL_PERMILLE alone reddens here, because the pad's lead
    // over the shoulder (tests/haven.rs, HAVEN_PRIZE_RATIO_MIN) is priced
    // off exactly this mean.
    //
    // Stated twice, because the two halves fail for different reasons.
    //
    // NOMINAL: the two knobs weighted by the sheltered share this island
    // actually has must come back to ROAD_BARREL_PERMILLE. This is the
    // design claim, in the knobs' own units, and it is what reddens if a
    // later pass raises one rate without lowering the other.
    let share = agg_bay_cells as f32 / agg_cells as f32;
    let nominal =
        share * ROAD_BAY_BARREL_PERMILLE as f32 + (1.0 - share) * ROAD_OPEN_BARREL_PERMILLE as f32;
    // `f32::abs` is off the wall-1 float list, in the test crate too
    // (`0555b07`). max of the two differences is the same number.
    let nominal_drift =
        (nominal - ROAD_BARREL_PERMILLE as f32).max(ROAD_BARREL_PERMILLE as f32 - nominal);

    // REALIZED: and what the island actually grows must not move either.
    // The realized rate is well under the nominal one — 116 against 261 —
    // because the shoulder branch fires on the slot's JITTERED position while
    // this sweep counts by cell center, and because the land, slope and pad
    // vetoes take cells before the road draw is reached. That leakage is not
    // this item's to fix; it applies identically to both arms, which is why
    // the ratio above is clean. What matters is that it did not move:
    // measured 2026-08-05 over these same four seeds, with BOTH rates pinned
    // to ROAD_BARREL_PERMILLE, the shoulder grew 239 barrels over 2,033 cells
    // = 117.6 per-mille. The split grows 236 = 116.1. That 1.3% is the whole
    // evidence for the word "redistribution" and it is why this constant is a
    // measurement rather than an intention.
    let realized = agg_barrels as f32 / agg_cells as f32 * 1000.0;
    let realized_drift =
        (realized - BAY_FLAT_REALIZED_PERMILLE).max(BAY_FLAT_REALIZED_PERMILLE - realized);
    println!(
        "bay slots: sheltered share {:.0}%; nominal {nominal:.0} per-mille vs \
         ROAD_BARREL_PERMILLE {ROAD_BARREL_PERMILLE} (drift {nominal_drift:.0}); \
         realized {realized:.0} vs flat-rate {BAY_FLAT_REALIZED_PERMILLE:.0} \
         (drift {realized_drift:.0}); worst bay:open ratio {worst_ratio:.2}x",
        share * 100.0
    );
    assert!(
        nominal_drift <= BAY_MEAN_DRIFT_PERMILLE,
        "the two rates weighted by the {:.0}% sheltered share come to \
         {nominal:.0} per-mille against the {ROAD_BARREL_PERMILLE} they are \
         supposed to conserve (drift {nominal_drift:.0} > \
         {BAY_MEAN_DRIFT_PERMILLE}) — the bay was bought by inflating the \
         road, which spends the haven pad's lead over it",
        share * 100.0
    );
    assert!(
        realized_drift <= BAY_MEAN_DRIFT_PERMILLE,
        "the shoulder actually grows {realized:.0} per-mille against the \
         {BAY_FLAT_REALIZED_PERMILLE:.0} measured under a flat rate (drift \
         {realized_drift:.0} > {BAY_MEAN_DRIFT_PERMILLE}) — whatever moved, \
         the route's total pay is no longer the number every gate downstream \
         of it was tuned against"
    );
}

/// Floor on the bay:open barrel-rate ratio. Knobs give 430/170 = 2.53x
/// (DECISIONS.md §open: bay slots v0); what is asserted is the REALIZED
/// ratio, which is lower because a shoulder cell that declines the road's own
/// draw can still roll a barrel off its biome row.
///
/// **Re-measured 2026-09-15 for world structure v1, which moved the
/// coastline.** `in_bay` classifies off the shore's own shape, so a second
/// wobble term at 260 m re-sorts which stretches are sheltered. The four gate
/// seeds now read **3.20 / 2.41 / 1.92 / 3.39** where the worst had been
/// 2.29 — the mechanism is untouched (on the worst seed the bay still pays
/// 197‰ against the open coast's 102‰), the spread widened. The floor keeps
/// its old derivation, ~15% under the measured worst, so it is still low
/// enough that coastline variance cannot trip it and high enough that halving
/// the gap between the two knobs does.
const BAY_RATIO_MIN: f32 = 1.63;

/// How far either conservation claim may drift, in per-mille. The two rates
/// are set against the MEASURED sheltered share, which moves with the seed's
/// coastline, so exact conservation is not available: worst nominal drift
/// across the four gate seeds is 19 (seed 0x5EED, 23.4% sheltered) and worst
/// realized drift is 2. 25 of 250 is 10%, inside the ~15% headroom
/// HAVEN_PRIZE_RATIO_MIN was given (DECISIONS.md §open: bay slots v0).
const BAY_MEAN_DRIFT_PERMILLE: f32 = 25.0;

/// What the shoulder grows per-mille of its cells under a FLAT rate —
/// measured 2026-08-05 on these four seeds with both bay rates pinned to
/// ROAD_BARREL_PERMILLE: 239 barrels over 2,033 shoulder cells. The baseline
/// the redistribution must not move (DECISIONS.md §open: bay slots v0).
const BAY_FLAT_REALIZED_PERMILLE: f32 = 117.6;

// ── The road's surface: `splat_road` ───────────────────────────────────────
//
// Stage 7 asked for a dirt material and shipped without one, so for months
// the ring's *population* was gravel (`clutter_kind_at` forces `Pebble` on
// the carriageway) standing on ground still painted meadow — two halves of
// one surface disagreeing. `splat_road` is the other half; these hold it to
// the same claim rather than to a colour, because no gate here can see a
// pixel.

/// The three bands do what they say, and the verge is a verge.
///
/// Written against a synthetic weight rather than a sampled one so the
/// arithmetic is exact and the assertion is about the LAW: a real splat would
/// make this a test of wherever the sampler happened to land.
#[test]
fn the_carriageway_is_grit_and_the_shoulder_is_between() {
    // A pure meadow vertex: all grass, no sand. The hardest case for the
    // override, because every byte it moves has to come from somewhere.
    let meadow = [0u8, 255, 0, 0];
    let off = terrain::splat_road(meadow, RoadBand::Off);
    let shoulder = terrain::splat_road(meadow, RoadBand::Shoulder);
    let road = terrain::splat_road(meadow, RoadBand::Carriageway);

    assert_eq!(off, meadow, "off the ring the ground is untouched");
    assert_eq!(
        road[0], 255,
        "the carriageway is not pure sand — the surface and the grit standing \
         on it come from one channel or they do not agree"
    );
    assert_eq!(
        [road[1], road[2], road[3]],
        [0, 0, 0],
        "the carriageway kept some meadow: {road:?}"
    );
    assert!(
        shoulder[0] > off[0] && shoulder[0] < road[0],
        "the shoulder's sand weight {} is not strictly between the wilderness's \
         {} and the carriageway's {} — a verge that matches either one of its \
         neighbours is not a verge",
        shoulder[0],
        off[0],
        road[0]
    );
    // The derivation, not the digit: 2/5 of the band beside the verge is road.
    let want = (255.0 * terrain::ROAD_WEAR_SHOULDER + 0.5) as u8;
    assert_eq!(
        shoulder[0], want,
        "the shoulder reads {} where ROAD_WEAR_SHOULDER = ROAD_HALF_W / \
         ROAD_SHOULDER_HALF_W asks for {want}",
        shoulder[0]
    );
    println!("splat_road on pure meadow: off {off:?} shoulder {shoulder:?} road {road:?}");
}

/// The coverage invariant survives the override — over the real island, not
/// over one hand-written weight.
///
/// `tests/clutter.rs::test_splat_weights_are_normalized_on_land` rests on the
/// four weights summing to 255, and the ground material divides by that sum.
/// The algebra says pushing `t` of every channel into channel 0 leaves the sum
/// at `S + t·(255 − S)`, i.e. unchanged when `S` is 255 — this is that claim
/// measured rather than believed, at the ±1 `splat_from`'s own rounding costs.
#[test]
fn the_road_override_keeps_the_weights_normalized() {
    let mut checked = 0u64;
    let mut on_road = 0u64;
    for seed in SEEDS {
        // March the ring the way every other test in this file does, so the
        // samples are on the road rather than near it.
        for b in 0..BEARINGS {
            let (ux, uz) = yaw_dir((b * (256 / BEARINGS)) << 8);
            let c = ISLAND_SIZE * 0.5;
            let mut r = ROAD_R_MIN;
            while r <= ROAD_R_MAX {
                let (x, z) = (c + ux * r, c + uz * r);
                r += MARCH_M;
                if terrain::height(seed, x, z) < SEA_LEVEL {
                    continue;
                }
                let band = terrain::ring_band(seed, x, z);
                let w = terrain::splat_road(terrain::splat(seed, x, z), band);
                let sum: u32 = w.iter().map(|v| *v as u32).sum();
                assert!(
                    (254..=256).contains(&sum),
                    "seed {seed:#x} at ({x:.1}, {z:.1}) band {band:?}: weights \
                     {w:?} sum to {sum}, not 255 ± 1 — the ground material \
                     divides by this"
                );
                checked += 1;
                if band != RoadBand::Off {
                    on_road += 1;
                    assert!(
                        w[0] >= (255.0 * terrain::ROAD_WEAR_SHOULDER) as u8,
                        "seed {seed:#x} at ({x:.1}, {z:.1}) is {band:?} and its \
                         sand weight is only {} — the road is not being worn in",
                        w[0]
                    );
                }
            }
        }
    }
    println!("splat_road normalization: {checked} land samples, {on_road} on the ring");
    // The ring is what this test is about; a march that met none of it is a
    // test of the wilderness. Same guard `a_near_chunk_on_the_coast_road...`
    // carries in the client, and for the same reason.
    assert!(
        on_road > 1_000,
        "only {on_road} of {checked} samples were on the road — this asserted \
         almost nothing"
    );
}

// ── The ring's walkable share (`reference/ROADS.md` §9.5 item 5) ───────────

/// **What share of the coast ring is ground a player can stand on.**
///
/// `ROADS.md` §9.5 named "the ring's own fragments" as a bigger reach problem
/// than the interior was and said *"nothing gates it"*. This is that gate, and
/// it is **not** the metric that section proposed — the two instruments that
/// look obvious were both measured and thrown away, which is most of this
/// test's value:
///
/// 1. **An angular sweep built on `yaw_dir` measures 256 bearings however many
///    steps it claims.** The LUT has 256 entries (`yaw_lut.rs`), so a
///    4,096-step sweep samples every 20.9 m of arc at r=850 against a 4 m
///    carriageway, and adjacent "bearings" are mostly the same direction. A
///    first pass read the ring as 9.3% in one piece with *radial gaps* as the
///    dominant cause; at true resolution (`examples/ring_breaks.rs`, real
///    trig, 8,192 steps) it is 54.4% and radial gaps are **0–2 per seed**,
///    p99 gap 0.0 m. Both of the first numbers were the LUT.
/// 2. **A "largest connected component" count measures the GRID.** Over ring
///    cells with an 8-neighbour flood the same islands read **43.5% at a 2 m
///    grid and 13.3% at 4 m** — a 4 m ribbon sampled at 4 m is a broken chain
///    of cells whatever the terrain does. `ROADS.md` §8 gate 2 already carries
///    this warning in the other direction; it applies to the ring itself.
///
/// What IS stable across both grids, to a tenth of a point, is the share of
/// carriageway cells whose ground a player can stand on: 97.3% mean, 94.4%
/// worst, identical at 2 m and 4 m. So that is what this asserts. It is also
/// the direct measure of the CAUSE: `examples/ring_breaks.rs` ablates the
/// cliff cells out and the biggest arc goes 54.4% → **94.8%** mean, five of
/// eight seeds to 99.8–100%, while ablating the radial gaps moves nothing.
/// The ring is broken by unwalkable ground and by nothing else.
///
/// **The fix this floor is waiting for is a road bench**, and the measurement
/// that says so is `examples/ring_breaks.rs`'s gradient split: at a cliff cell
/// the ground climbs 3.8–35× more steeply ACROSS the road than along it (198
/// radial against 0 tangential on the shipped seed). The road's length is
/// walkable; the road is cut into a slope with no shelf under it. `NOW.md`
/// §0ring has what that costs.
#[test]
fn the_ring_is_ground_a_player_can_stand_on() {
    /// Measured 2026-09-17 over the twelve seeds below: **90.7–100.0%**, mean
    /// 96.2%, and the worst island is `0x5eed` rather than any of the eight
    /// `examples/ring_breaks.rs` sweeps (which bottom out at 94.4%) — which is
    /// why this gate runs a wider set than the rest of the file. The floor
    /// sits under the worst of those with room to move, because it is a
    /// REGRESSION guard and not a target; the print is what a later pass
    /// reads rather than this number.
    const WALKABLE_FLOOR: f32 = 0.85;
    /// 2 m, half the carriageway's radial width, so no cell of it is skipped.
    const GRID: f32 = 2.0;
    /// Wider than the file's `SEEDS` on purpose: the share moves 90.7–100.0%
    /// across islands, so four of them cannot say where the floor belongs.
    const RING_SEEDS: [u64; 12] = [
        0x0047_4154_4553,
        0x1,
        0xDEAD_BEEF,
        0x5EED,
        20_260_731,
        42,
        555_555,
        31_337,
        8_675_309,
        20_260_804,
        123_456_789,
        987_654_321,
    ];

    let n = (ISLAND_SIZE / GRID) as usize;
    let mut worst = 1.0f32;
    let mut worst_seed = 0u64;
    let mut sum = 0.0f32;
    for seed in RING_SEEDS {
        let (mut cells, mut walk) = (0u32, 0u32);
        for iz in 0..n {
            for ix in 0..n {
                let (x, z) = (ix as f32 * GRID, iz as f32 * GRID);
                if terrain::ring_band(seed, x, z) != RoadBand::Carriageway {
                    continue;
                }
                cells += 1;
                // Re-derived from `height`/`slope`, not from any road helper:
                // the question is whether the GROUND under the road holds a
                // player, and a road helper has no opinion about that.
                if terrain::height(seed, x, z) >= terrain::LAND_MIN_H
                    && terrain::slope(seed, x, z) <= CLIFF_SLOPE_RATIO
                {
                    walk += 1;
                }
            }
        }
        assert!(
            cells > 1_000,
            "seed {seed:#x}: only {cells} carriageway cells at a {GRID} m grid \
             — the sweep found no ring, so its share means nothing"
        );
        let share = walk as f32 / cells as f32;
        sum += share;
        println!(
            "seed {seed:#x}: {walk}/{cells} carriageway cells are standable \
             ({:.1}%)",
            100.0 * share
        );
        if share < worst {
            worst = share;
            worst_seed = seed;
        }
    }
    assert!(
        worst >= WALKABLE_FLOOR,
        "seed {worst_seed:#x}: only {:.1}% of the coast ring is ground a \
         player can stand on, against a floor of {:.0}%. The ring is the \
         circulation loop; a stretch of it on a cliff face cuts the loop",
        100.0 * worst,
        100.0 * WALKABLE_FLOOR
    );
    println!(
        "ring: worst island is {:.1}% standable (seed {worst_seed:#x}), mean \
         {:.1}% over {} islands, floor {:.0}%",
        100.0 * worst,
        100.0 * sum / RING_SEEDS.len() as f32,
        RING_SEEDS.len(),
        100.0 * WALKABLE_FLOOR
    );
}
