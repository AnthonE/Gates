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

use sim_core::terrain::{
    self, Occupant, RoadBand, ScatterTable, CELLS_PER_SIDE, CELL_SIZE, CLIFF_SLOPE_RATIO,
    ISLAND_SIZE, ROAD_BARREL_PERMILLE, ROAD_HALF_W, ROAD_INLAND_M, ROAD_R_MAX, ROAD_R_MIN,
    ROAD_SHOULDER_HALF_W, SEA_LEVEL,
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
                if terrain::road_band(seed, c + ux * d, c + uz * d) == RoadBand::Carriageway {
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
                match terrain::road_band(seed, s.x, s.z) {
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
    assert!(
        worst_shoulder_barrels >= 80,
        "worst seed puts only {worst_shoulder_barrels} barrels on the road \
         shoulder (measured 103 when this floor was set) — the route does not \
         pay, so nobody walks it"
    );
    assert!(
        worst_ratio >= 0.42,
        "barrels are only {:.0}% of what stands on the shoulder (measured 52% \
         when this floor was set) — the road draw is being outvoted by the \
         biome table",
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
                if terrain::road_band(seed, x, z) == RoadBand::Carriageway {
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

    for seed in SEEDS {
        for b in 0..BEARINGS {
            let (ux, uz) = yaw_dir((b * (256 / BEARINGS)) << 8);
            let mut d = ROAD_R_MIN;
            while d <= ROAD_R_MAX {
                let (x, z) = (c + ux * d, c + uz * d);
                if terrain::road_band(seed, x, z) == RoadBand::Carriageway {
                    let h = terrain::height(seed, x, z);
                    min_h = min_h.min(h);
                    // Walk seaward from the road and find the water, rather
                    // than re-testing the window `road_band` already tested.
                    // First crossing, so a sandbar further out cannot flatter
                    // the answer — this is the distance a player actually
                    // walks from the road to the sea.
                    let mut inland = f32::MAX;
                    let mut step = 0.0f32;
                    while step <= ROAD_INLAND_M + 20.0 {
                        let rr = d + step;
                        if terrain::height(seed, c + ux * rr, c + uz * rr) <= SEA_LEVEL {
                            inland = step;
                            break;
                        }
                        step += MARCH_M;
                    }
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
        "road placement: {checked} carriageway samples; sea is {worst_inland_lo:.0}-\
         {worst_inland_hi:.0} m seaward (target {ROAD_INLAND_M} m); lowest road ground {min_h:.2} m"
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
}

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
