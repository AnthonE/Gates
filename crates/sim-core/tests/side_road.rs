//! Gate: the road to the interior exists, goes somewhere, and is walkable.
//!
//! `reference/ROADS.md` §8 gates 1–4, which that doc wrote as proposals and
//! this file is. They are the four things a second road tier can be wrong
//! about, and every one of them is arithmetic over `terrain` — no clock, no
//! pixels, and nothing that needs the road to be drawn.
//!
//! **The reach gate is a DOSE-RESPONSE and not a bound**, which is the one
//! design decision here worth stating. §8 gate 1 proposes "assert the p90
//! walk stays under a stated bound", and a bound is the shape this repo has
//! been burned by twice: it is satisfied by an island that got smaller, it
//! drifts every time worldgen moves, and it cannot tell "the road works" from
//! "the land near the coast is what we measured". So the test measures the
//! island BOTH WAYS — with the side road and with it switched off — and
//! asserts the difference. The mutant §8 asks for ("delete a road tier; it
//! must redden") is then not a mutant at all: it is the control arm, run
//! every time.

// The measurements are this file's output, as in `tests/forest.rs`.
#![allow(clippy::disallowed_macros)]

use sim_core::fmath::fabs;
use sim_core::terrain::{
    self, Haven, RoadBand, CLIFF_SLOPE_RATIO, ISLAND_SIZE, LAND_MIN_H, ROAD_HALF_W, ROAD_R_MAX,
    ROAD_R_MIN, ROAD_SHOULDER_HALF_W,
};

/// The cheap per-seed checks run the sixteen `tests/haven.rs` sweeps.
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

/// The island-walk checks run three, because each one floods 65,536 cells
/// twice and this suite runs in a debug build.
const SWEEP_SEEDS: [u64; 3] = [20_260_731, 42, 0xDEAD_BEEF];

/// A millimetre, `tests/carve.rs`'s tolerance and its reason: these are
/// physical claims and the positions are rebuilt from a bearing and a radius.
const MM: f32 = 1.0e-3;

// ─────────────────────────── §8 gate 3: it goes somewhere ─────────────────

/// **Every inland site has a road, and both of its ends are what they claim.**
///
/// §8 gate 3: "assert every side road's endpoints are a ring point and a site
/// port". Both halves are exact — the port is `sim_core::depot::PORT_Z` from the
/// site's centre on the bearing the road carries, and the ring end is on the
/// carriageway — so both are asserted at a millimetre rather than a band.
///
/// **The `live` assert is the load-bearing one.** `solve_side_roads` leaves
/// `SideRoad::NONE` when no bearing reaches the ring, which is the honest
/// empty a short tier gets — and an island with no road is exactly what this
/// whole file would otherwise pass over in silence, because every claim below
/// is vacuous on a dead road.
#[test]
fn every_inland_site_has_a_road_and_both_ends_are_what_they_claim() {
    let mut shortest = f32::MAX;
    let mut longest: f32 = 0.0;
    for seed in SEEDS {
        let h = terrain::haven(seed);
        let sites: Vec<_> = h
            .minor
            .iter()
            .filter(|w| w.live && w.kind == terrain::SiteKind::Inland)
            .collect();
        assert_eq!(
            sites.len() * 2,
            terrain::SIDE_ROADS,
            "seed {seed:#x}: {} inland sites against {} roads — the two are \
             two-to-one by construction and `solve_side_roads` indexes on it",
            sites.len(),
            terrain::SIDE_ROADS
        );
        for (i, r) in h.roads.iter().enumerate() {
            assert!(
                r.live,
                "seed {seed:#x}: side road {i} is dead — no bearing off the \
                 site reached the ring on walkable ground, which is a finding \
                 about the solve rather than a seed to shrug at"
            );
            let site = sites[i / 2];

            // The site end is the port: on the rim, on the carried bearing.
            let (dx, dz) = sim_core::yaw_dir((r.port as u16) << 8);
            let want = (
                site.x + dx * sim_core::depot::PORT_Z,
                site.z + dz * sim_core::depot::PORT_Z,
            );
            let off =
                ((r.px - want.0) * (r.px - want.0) + (r.pz - want.1) * (r.pz - want.1)).sqrt();
            assert!(
                off < MM,
                "seed {seed:#x}: road {i}'s port is {off:.4} m off the rim \
                 point its own bearing names — the stored bearing and the \
                 stored position disagree"
            );

            // The ring end is on the ring's carriageway, asked of the ring
            // alone: a side road that ended on ANOTHER side road would be a
            // road to a road.
            assert_eq!(
                terrain::ring_band(&h.ring, r.rx, r.rz),
                RoadBand::Carriageway,
                "seed {seed:#x}: road {i}'s ring end at ({:.0}, {:.0}) is not \
                 on the coast road's surface",
                r.rx,
                r.rz
            );
            let c = ISLAND_SIZE * 0.5;
            let rr = ((r.rx - c) * (r.rx - c) + (r.rz - c) * (r.rz - c)).sqrt();
            assert!(
                (ROAD_R_MIN..=ROAD_R_MAX).contains(&rr),
                "seed {seed:#x}: road {i} meets the ring at radius {rr:.0} m, \
                 outside the ring's own bracket"
            );

            let len = ((r.rx - r.px) * (r.rx - r.px) + (r.rz - r.pz) * (r.rz - r.pz)).sqrt();
            shortest = shortest.min(len);
            longest = longest.max(len);
            // It cannot be shorter than the gap between the site's rim and
            // the ring's innermost radius, which is `ROAD_REACH_M` less the
            // rim — the tier's own definition, arriving as a length.
            let floor = terrain::ROAD_REACH_M - sim_core::depot::PORT_Z;
            assert!(
                len >= floor,
                "seed {seed:#x}: road {i} is {len:.0} m long against a floor \
                 of {floor:.0} m — either the site is not inland or the road \
                 did not reach the ring"
            );
        }
    }
    println!(
        "side roads over {} seeds: {shortest:.0}–{longest:.0} m, every port on \
         its rim and every ring end on the carriageway",
        SEEDS.len()
    );
}

// ─────────────────────────── §8 gate 4: over neither water nor cliff ──────

/// **No road crosses water or a cliff** — Devblog 189's fix as an assertion.
///
/// The solve tests this at `SIDE_ROAD_SAMPLE_M` (8 m, one sample per scatter
/// cell). This walks the same line at **one metre**, which is the point: a
/// solve that sampled coarsely enough to step over a 6 m inlet would pass its
/// own check and fail here. `tests/road.rs::the_road_is_walkable_along_its_
/// length` is the ring's version of this and this is the second tier's.
#[test]
fn a_side_road_crosses_neither_water_nor_cliff() {
    let mut worst_slope: f32 = 0.0;
    let mut lowest = f32::MAX;
    let mut samples = 0u32;
    for seed in SEEDS {
        let h = terrain::haven(seed);
        for (i, r) in h.roads.iter().enumerate() {
            if !r.live {
                continue;
            }
            let len = ((r.rx - r.px) * (r.rx - r.px) + (r.rz - r.pz) * (r.rz - r.pz)).sqrt();
            let n = len as i32;
            for k in 0..=n {
                let t = k as f32 / n as f32;
                let (x, z) = (r.px + (r.rx - r.px) * t, r.pz + (r.rz - r.pz) * t);
                let y = terrain::ground(seed, &h, x, z);
                let s = terrain::ground_slope(seed, &h, x, z);
                samples += 1;
                lowest = lowest.min(y);
                worst_slope = worst_slope.max(s);
                assert!(
                    y >= LAND_MIN_H,
                    "seed {seed:#x}: road {i} is under water at ({x:.0}, \
                     {z:.0}) — {y:.2} m against the land line {LAND_MIN_H}. \
                     The solve samples every {} m; this walks every metre",
                    terrain::SIDE_ROAD_SAMPLE_M
                );
                assert!(
                    s <= CLIFF_SLOPE_RATIO,
                    "seed {seed:#x}: road {i} crosses a cliff at ({x:.0}, \
                     {z:.0}) — slope {s:.3} against {CLIFF_SLOPE_RATIO}"
                );
            }
        }
    }
    println!(
        "{samples} metre-samples along every side road: lowest {lowest:.2} m, \
         steepest {worst_slope:.3} (cliff at {CLIFF_SLOPE_RATIO:.3})"
    );
}

/// **And the band it produces is the band a road has** — the query side of
/// the same claim.
///
/// The two are different code: the walk above tests the TERRAIN the solve
/// chose, and this tests `side_band`, the point-to-segment arithmetic every
/// consumer actually calls. A road whose stored ends were right and whose
/// distance function was wrong would pass the first and fail this.
/// Point-to-polyline distance, written from the PUBLISHED nodes and sharing
/// no code with the thing it checks.
///
/// `CLAUDE.md`'s `lattice.rs` entry: a naive rebuild that calls the function
/// under test is a rebuild of nothing, and `SideRoad::node` is `pub` for
/// exactly this — the shape is published, the arithmetic over it is this
/// file's own. Run the mutant: point `side_band_of` at the chord
/// (`seg_dist2(r.px, r.pz, r.rx, r.rz, ..)`) and this reddens on every seed.
fn naive_dist(r: &terrain::SideRoad, x: f32, z: f32) -> f32 {
    let mut best = f32::MAX;
    for k in 1..terrain::SIDE_ROAD_POINTS {
        let (ax, az) = r.node(k - 1);
        let (bx, bz) = r.node(k);
        let (ex, ez) = (bx - ax, bz - az);
        let len2 = ex * ex + ez * ez;
        let t = if len2 > 0.0 {
            (((x - ax) * ex + (z - az) * ez) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (qx, qz) = (x - ax - ex * t, z - az - ez * t);
        best = best.min((qx * qx + qz * qz).sqrt());
    }
    best
}

/// What the two widths say a distance means.
fn band_at(d: f32) -> RoadBand {
    if d <= ROAD_HALF_W {
        RoadBand::Carriageway
    } else if d <= ROAD_SHOULDER_HALF_W {
        RoadBand::Shoulder
    } else {
        RoadBand::Off
    }
}

#[test]
fn the_side_band_is_a_road_on_the_line_and_off_it_beside() {
    // Two float paths cannot be trusted to agree on which side of a threshold
    // a point exactly ON it falls, so samples within this of either width are
    // not asserted. It is 1 mm against widths of 2 m and 5 m, so what it
    // excuses is rounding and not a band.
    const BAND_EDGE_EPS_M: f32 = 0.001;
    let mut checked = 0u64;
    let mut skipped = 0u64;
    for seed in SEEDS {
        let h = terrain::haven(seed);
        for (i, r) in h.roads.iter().filter(|r| r.live).enumerate() {
            // ON THE LINE — every point between two nodes is this road's
            // carriageway. Distance-free by construction, so it holds
            // whatever any distance function does with a point off the road.
            for k in 1..terrain::SIDE_ROAD_POINTS {
                let (ax, az) = r.node(k - 1);
                let (bx, bz) = r.node(k);
                for j in 0..=32 {
                    let t = j as f32 / 32.0;
                    let (x, z) = (ax + (bx - ax) * t, az + (bz - az) * t);
                    assert_eq!(
                        terrain::side_band(&h, x, z),
                        RoadBand::Carriageway,
                        "seed {seed:#x}: road {i} is not its own carriageway \
                         on leg {k} at t={t:.2}"
                    );
                }
            }
            // BESIDE IT — a swept neighbourhood, each sample classified by
            // this file's own distance rather than by an offset the test
            // assumed would land in a band. `side_band_of` and not
            // `side_band`, so the other approach's own band is not this
            // road's failure.
            for k in 1..terrain::SIDE_ROAD_POINTS {
                let (ax, az) = r.node(k - 1);
                let (bx, bz) = r.node(k);
                let (ex, ez) = (bx - ax, bz - az);
                let len = (ex * ex + ez * ez).sqrt();
                let (nx, nz) = (-ez / len, ex / len);
                for j in 0..=24 {
                    let t = j as f32 / 24.0;
                    let (cx, cz) = (ax + ex * t, az + ez * t);
                    for step in -24i32..=24 {
                        let off = step as f32 * 0.5;
                        let (x, z) = (cx + nx * off, cz + nz * off);
                        let d = naive_dist(r, x, z);
                        if fabs(d - ROAD_HALF_W) < BAND_EDGE_EPS_M
                            || fabs(d - ROAD_SHOULDER_HALF_W) < BAND_EDGE_EPS_M
                        {
                            skipped += 1;
                            continue;
                        }
                        assert_eq!(
                            terrain::side_band_of(r, x, z),
                            band_at(d),
                            "seed {seed:#x}: road {i} leg {k} at {d:.3} m off \
                             the road reads as the wrong band"
                        );
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked > 100_000, "the sweep did not sweep: {checked}");
    println!(
        "side_band: {checked} samples agree with an independent \
         point-to-polyline distance ({skipped} within {BAND_EDGE_EPS_M} m of a width, \
         not asserted)"
    );
}

/// **Every live road actually bends**, and no further than it is allowed to.
///
/// The `water_carry.rs` lesson in `CLAUDE.md`: the suites above check that a
/// road is walkable and that its band is its band, and BOTH are satisfied by
/// a `bend_side_road` that returns its input. This is the one assertion that
/// is not — and the ceiling half of it is what
/// `scatter`'s bounding-box reject is allowed to assume.
#[test]
fn every_live_road_bends_and_stays_inside_its_ceiling() {
    // **The fallback is real and it is not hidden.** One road in this sweep
    // (seed 0x845fed) fails its corridor at 60, 30 AND 15 m and keeps its
    // chord. Extending the ladder two rungs does rescue it — measured — and
    // what it buys is a 5.2 m wander over 770 m, which is 0.7% and is
    // straight to anyone looking at it. A gate that went green on that would
    // be green for the wrong reason, so the ladder stops where a bend is
    // still worth having and this counts the roads it cannot help.
    //
    // The separation that makes it a gate: 1 of 32 here, 32 of 32 under a
    // `bend_side_road` that returns its input.
    /// What counts as bent, metres.
    ///
    /// ⚠ **Not `> 0.0`, and that was a real defect in this gate.** A road that
    /// fell back to its chord has its nodes placed ON that chord by
    /// `straighten`, and `bend_m` measures a perpendicular distance in f32 —
    /// over a 978 m chord the answer is **3.1e-5 m**, not zero. The first
    /// draft classified that as bent and then asserted `path_len() > chord`
    /// about it, which is false at the same precision. Any real bend is at
    /// least `SIDE_ROAD_BEND_MIN_SHARE` of the ladder's last rung — about
    /// 5.7 m — so a centimetre separates the two by three orders of
    /// magnitude and cannot be the thing that decides.
    const BENT_MIN_M: f32 = 0.01;

    let mut least = f32::MAX;
    let mut most = 0.0f32;
    let (mut roads, mut flat) = (0u32, 0u32);
    for seed in SEEDS {
        let h = terrain::haven(seed);
        for (i, r) in h.roads.iter().filter(|r| r.live).enumerate() {
            let bend = r.bend_m();
            let chord = ((r.rx - r.px) * (r.rx - r.px) + (r.rz - r.pz) * (r.rz - r.pz)).sqrt();
            // The ceiling is the load-bearing half: `scatter`'s bounding-box
            // reject assumes it, and a node past it is a prop left standing
            // in the carriageway.
            assert!(
                bend <= terrain::SIDE_ROAD_BEND_M,
                "seed {seed:#x}: road {i} bends {bend:.1} m, past the \
                 {:.1} m ceiling `scatter`'s reject assumes",
                terrain::SIDE_ROAD_BEND_M
            );
            if bend <= BENT_MIN_M {
                flat += 1;
            } else {
                // A bent road is longer than its chord — the same claim
                // stated as a cost rather than as a shape.
                assert!(
                    r.path_len() > chord,
                    "seed {seed:#x}: road {i} bends {bend:.3} m off a \
                     {chord:.1} m chord and yet is not longer than it \
                     ({:.3} m) — the nodes and the length disagree",
                    r.path_len()
                );
                least = least.min(bend);
                most = most.max(bend);
            }
            roads += 1;
        }
    }
    assert!(
        flat * 4 < roads,
        "{flat} of {roads} roads kept their chord — the bend is the exception \
         now, not the rule"
    );
    // And the island the shard ships is not allowed to be one of them: it is
    // the frame the operator is looking at.
    let shipped = terrain::haven(20_260_731);
    for (i, r) in shipped.roads.iter().filter(|r| r.live).enumerate() {
        assert!(
            r.bend_m() > BENT_MIN_M,
            "seed 20260731 road {i} is a chord — this is the island on the \
             screen when the straight road was reported"
        );
    }
    println!(
        "bend: {roads} roads, {} bent {least:.1}–{most:.1} m off their own \
         chords, {flat} kept the chord",
        roads - flat
    );
}

/// **Every node is inside the chord's box grown by the ceiling** — the
/// assumption `scatter` rejects against, asserted where a reader can find it.
///
/// A box that is too tight does not fail loudly: it leaves a boulder standing
/// in the carriageway on the outside of a bend. **This test is not what
/// catches that** — it holds the NODE claim the box is built on. The mutant
/// (drop the `+ SIDE_ROAD_BEND_M` in `scatter`) leaves `test_terrain_golden`
/// green and reddens only `tests/depot.rs`'s
/// `carriageways_clear_real_capsules_over_final_ground_including_both_edges`,
/// which walks a capsule down the road. Both halves are needed and neither
/// substitutes for the other, which is the `ring_handoff.rs` entry in
/// `CLAUDE.md`: a probe that stands where it spawned certifies the spawn.
#[test]
fn every_node_is_inside_the_chords_box_grown_by_the_bend_ceiling() {
    for seed in SEEDS {
        let h = terrain::haven(seed);
        for (i, r) in h.roads.iter().filter(|r| r.live).enumerate() {
            let (lo_x, hi_x) = (r.px.min(r.rx), r.px.max(r.rx));
            let (lo_z, hi_z) = (r.pz.min(r.rz), r.pz.max(r.rz));
            for k in 0..terrain::SIDE_ROAD_POINTS {
                let (x, z) = r.node(k);
                assert!(
                    x >= lo_x - terrain::SIDE_ROAD_BEND_M
                        && x <= hi_x + terrain::SIDE_ROAD_BEND_M
                        && z >= lo_z - terrain::SIDE_ROAD_BEND_M
                        && z <= hi_z + terrain::SIDE_ROAD_BEND_M,
                    "seed {seed:#x}: road {i} node {k} at ({x:.1}, {z:.1}) is \
                     outside its own chord box grown by {:.1} m",
                    terrain::SIDE_ROAD_BEND_M
                );
            }
        }
    }
}

/// **The two approaches are not one straight line across the island** —
/// the defect this slice exists to retire, as arithmetic.
///
/// Operator, 2026-09-17, on seed 20260731: *"just a road going straight
/// across the world. it did not look good at all"*. Both ring junctions and
/// both gates sat on `z = 899.3`, an axis-aligned chord 1,760 m long across
/// a 2,048 m island — and they still WILL sit on one line, because
/// `depot.rs`'s two gates share the yard's local Z axis and the pairing is
/// the building rather than a choice. What may not happen again is the road
/// following it.
#[test]
fn the_pair_is_never_one_line_across_the_island() {
    // A road's own width. Under this the two approaches read as one stroke
    // with a yard in the middle, which is what was on the screen.
    let floor = ROAD_SHOULDER_HALF_W * 2.0;
    let mut worst = f32::MAX;
    let mut worst_seed = 0;
    for seed in SEEDS {
        let h = terrain::haven(seed);
        let live: Vec<_> = h.roads.iter().filter(|r| r.live).collect();
        if live.len() < 2 {
            continue;
        }
        let (a, b) = (live[0], live[1]);
        // The through-line: junction to junction, which is what a player
        // standing at the depot sees running away in both directions.
        let (ex, ez) = (b.rx - a.rx, b.rz - a.rz);
        let len2 = ex * ex + ez * ez;
        assert!(len2 > 0.0, "seed {seed:#x}: both junctions at one point");
        let mut off = 0.0f32;
        for r in [a, b] {
            for k in 0..terrain::SIDE_ROAD_POINTS {
                let (x, z) = r.node(k);
                let (px, pz) = (x - a.rx, z - a.rz);
                let t = (px * ex + pz * ez) / len2;
                let (qx, qz) = (px - ex * t, pz - ez * t);
                off = off.max((qx * qx + qz * qz).sqrt());
            }
        }
        assert!(
            off > floor,
            "seed {seed:#x}: the two approaches stay within {off:.1} m of one \
             line between their junctions — under {floor:.1} m that is the \
             ruler across the island again"
        );
        if off < worst {
            worst = off;
            worst_seed = seed;
        }
    }
    println!(
        "pair: the flattest island's approaches still leave their \
         through-line by {worst:.1} m (seed {worst_seed:#x}, floor {floor:.1})"
    );
}

// ─────────────────────────── §8 gates 1 and 2: reach, and one piece ───────

/// The island's walkable cells, and which of them are road.
///
/// 4 m — the carriageway's full width, and **not a free choice.** The first
/// draft used `CELL_SIZE` (8 m) on the argument that a walk is measured at
/// the scatter grid's resolution, and read the SHIPPED coast ring as **7.6%
/// in one piece** against the 79% `examples/second_road.rs` measures at 2 m.
/// Nothing was wrong with the ring: a 4 m ribbon sampled every 8 m along a
/// curve gives consecutive hits that are diagonal neighbours, and a
/// 4-neighbour flood cannot join them. That is the same sampling artifact
/// that read every second-road candidate as confetti before that example was
/// corrected, and it is worth the extra 196,608 cells to not re-learn.
///
/// Walkable means what `scatter` and the clutter population mean by it: above
/// the land line and under the cliff ratio. Filling through cliff cells is
/// what flattered every candidate in that same example to ~100%.
const GRID: f32 = 4.0;
const N: usize = (ISLAND_SIZE / GRID) as usize;

struct Walk {
    /// Sorted walk distances in cells, over reachable walkable land.
    d: Vec<u32>,
    /// Walkable cells no road reaches at all.
    stranded: usize,
    /// Cells that are road.
    road: usize,
    /// The largest connected component of the road set, in cells.
    largest: usize,
    /// Of the component holding each side road: how many of its cells are
    /// that road's own, and how many are ring. Empty when no side road is in
    /// the arm.
    junction: Vec<(usize, usize)>,
}

fn walk(seed: u64, h: &Haven, with_side: bool) -> Walk {
    let mut ok = vec![false; N * N];
    let mut on = vec![false; N * N];
    // Which road a cell belongs to, so the junction's composition can be read
    // off the component rather than guessed: 0 = ring only, 1+ = side road
    // `n-1` (a cell both roads cover counts as the side road's, since the
    // question below is how much RING the road reaches).
    let mut owner = vec![0u8; N * N];
    let mut dist = vec![u32::MAX; N * N];
    let mut q: Vec<usize> = Vec::new();
    for iz in 0..N {
        for ix in 0..N {
            let i = iz * N + ix;
            let (x, z) = (ix as f32 * GRID, iz as f32 * GRID);
            if terrain::height(seed, x, z) < LAND_MIN_H
                || terrain::slope(seed, x, z) > CLIFF_SLOPE_RATIO
            {
                continue;
            }
            ok[i] = true;
            let ring = terrain::ring_band(&h.ring, x, z) != RoadBand::Off;
            on[i] = ring;
            if with_side {
                for (n, r) in h.roads.iter().enumerate() {
                    if r.live && terrain::side_band_of(r, x, z) != RoadBand::Off {
                        on[i] = true;
                        owner[i] = n as u8 + 1;
                    }
                }
            }
            if on[i] {
                dist[i] = 0;
                q.push(i);
            }
        }
    }
    let nbr = |p: usize| {
        let (px, pz) = (p % N, p / N);
        [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)]
            .into_iter()
            .filter_map(move |(dx, dz)| {
                let (jx, jz) = (px as i32 + dx, pz as i32 + dz);
                if jx < 0 || jz < 0 || jx >= N as i32 || jz >= N as i32 {
                    None
                } else {
                    Some(jz as usize * N + jx as usize)
                }
            })
    };
    let mut head = 0usize;
    while head < q.len() {
        let p = q[head];
        head += 1;
        for j in nbr(p) {
            if ok[j] && dist[j] == u32::MAX {
                dist[j] = dist[p] + 1;
                q.push(j);
            }
        }
    }
    // The road set's own components, over walkable road cells — §8 gate 2's
    // "the network is one piece", to the bar the ring already sets.
    //
    // **Eight-neighbour here and four-neighbour above, deliberately.** The
    // walk is a player and a player crossing a cliff corner diagonally is not
    // a walk, so that flood stays orthogonal. This one is asking whether the
    // road is ONE THING, and a ribbon crossing the lattice at an angle steps
    // diagonally whatever its width — so orthogonal adjacency would be
    // measuring the sampling and calling it the network.
    let diag = |p: usize| {
        let (px, pz) = (p % N, p / N);
        [
            (1i32, 0i32),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ]
        .into_iter()
        .filter_map(move |(dx, dz)| {
            let (jx, jz) = (px as i32 + dx, pz as i32 + dz);
            if jx < 0 || jz < 0 || jx >= N as i32 || jz >= N as i32 {
                None
            } else {
                Some(jz as usize * N + jx as usize)
            }
        })
    };
    let road: usize = (0..N * N).filter(|&i| on[i] && ok[i]).count();
    let mut seen = vec![false; N * N];
    let mut largest = 0usize;
    let mut junction = vec![(0usize, 0usize); h.roads.len()];
    for s in 0..N * N {
        if !on[s] || !ok[s] || seen[s] {
            continue;
        }
        let mut st = vec![s];
        seen[s] = true;
        let mut n = 0usize;
        // Per road in this component: its own cells, and the ring cells it
        // can reach without leaving the road.
        let mut mine = vec![0usize; h.roads.len()];
        let mut ring_here = 0usize;
        while let Some(p) = st.pop() {
            n += 1;
            if owner[p] == 0 {
                ring_here += 1;
            } else {
                mine[owner[p] as usize - 1] += 1;
            }
            for j in diag(p) {
                if on[j] && ok[j] && !seen[j] {
                    seen[j] = true;
                    st.push(j);
                }
            }
        }
        largest = largest.max(n);
        for (k, m) in mine.iter().enumerate() {
            if *m > 0 {
                junction[k] = (*m, ring_here);
            }
        }
    }
    let mut d: Vec<u32> = (0..N * N)
        .filter(|&i| ok[i] && dist[i] != u32::MAX)
        .map(|i| dist[i])
        .collect();
    let stranded = (0..N * N).filter(|&i| ok[i] && dist[i] == u32::MAX).count();
    d.sort_unstable();
    Walk {
        d,
        stranded,
        road,
        largest,
        junction,
    }
}

fn pct(w: &Walk, f: f32) -> f32 {
    w.d[((w.d.len() - 1) as f32 * f) as usize] as f32 * GRID
}

/// Share of walkable land more than `ROAD_REACH_M` of walking from a road.
fn unserved(w: &Walk) -> f32 {
    w.d.iter()
        .filter(|&&v| v as f32 * GRID > terrain::ROAD_REACH_M)
        .count() as f32
        * 100.0
        / w.d.len() as f32
}

/// **§8 gate 1: the road tier moves the walk, measured against itself.**
///
/// Both arms in one run — the island with the side road and the same island
/// with it switched off — so the mutant §8 asks for ("delete a road tier; it
/// must redden") is the control, not an edit someone has to remember to make.
/// A bound would have been satisfied by an island that merely got smaller;
/// this cannot be, because the two arms share every other term.
///
/// The floors are stated as a MARGIN over the control rather than as a
/// destination. `reference/ROADS.md` §7.2's dose-response says six spokes
/// reach 0% and one road cannot; what one road must do is move the number it
/// was built to move.
///
/// ⚠ **What this gate CANNOT see, measured rather than assumed: the
/// difference between a road and a dot.** Truncating the road to 1% of its
/// length — a 5 m stub at the site's own rim — still moves the unserved share
/// by 7.8 / 8.4 / 6.9 points against the full road's 11.9 / 12.3 / 7.8, and
/// the two sets OVERLAP (seed 0xDEADBEEF's whole road scores 7.8 and seed
/// 20260731's stub scores the same). So no floor can separate them, and
/// raising this one would only make it fail on a correct island. The reason
/// is the finding: most of the gain is from a road cell existing in the
/// INTERIOR at all, because the flood then spreads across the wasteland from
/// there — which is Devblog 189's own framing arriving as arithmetic. The
/// stub is caught two tests down (`a_side_road_joins_more_ring_than_it_is`)
/// and one up (the length floor), which is where it belongs.
#[test]
fn the_side_road_moves_the_walk_it_was_built_to_move() {
    // Measured 2026-09-16 over the three sweep seeds: unserved 36.7–40.6% ->
    // 26.8–32.8%, so the fall is 7.8–9.9 points and p90 improves 15–25%.
    // The floors sit under the worst of those with room, and the print is
    // what a later pass reads rather than these numbers.
    const UNSERVED_FALL_MIN: f32 = 5.0;
    const P90_GAIN_MIN: f32 = 0.10;

    for seed in SWEEP_SEEDS {
        let h = terrain::haven(seed);
        let before = walk(seed, &h, false);
        let after = walk(seed, &h, true);

        let (u0, u1) = (unserved(&before), unserved(&after));
        let (a90, b90) = (pct(&before, 0.9), pct(&after, 0.9));
        println!(
            "seed {seed:#x}: unserved {u0:.1}% -> {u1:.1}%, p50 {:.0} -> {:.0} m, \
             p90 {a90:.0} -> {b90:.0} m, stranded {} -> {}",
            pct(&before, 0.5),
            pct(&after, 0.5),
            before.stranded,
            after.stranded
        );
        assert!(
            u0 - u1 >= UNSERVED_FALL_MIN,
            "seed {seed:#x}: the side road moved the unserved share by only \
             {:.1} points ({u0:.1}% -> {u1:.1}%) against a floor of \
             {UNSERVED_FALL_MIN}. A road that changes nothing is a road to \
             somewhere the ring already reached",
            u0 - u1
        );
        assert!(
            (a90 - b90) / a90 >= P90_GAIN_MIN,
            "seed {seed:#x}: the p90 walk improved {:.1}% ({a90:.0} m -> \
             {b90:.0} m) against a floor of {:.0}%",
            100.0 * (a90 - b90) / a90,
            100.0 * P90_GAIN_MIN
        );
        // It may never make the island WORSE — a road cannot strand land,
        // and a cell that was reachable stays reachable.
        assert!(
            after.stranded <= before.stranded,
            "seed {seed:#x}: adding a road stranded {} more cells",
            after.stranded - before.stranded
        );
    }
}

/// **§8 gate 2: the road joins the network, and the network it joins is
/// bigger than the road.**
///
/// ⚠ **`reference/ROADS.md` §8 states this gate as "the largest walkable
/// component of the whole road set holds ≥ the share the coast ring alone
/// holds (79% measured)", and that statement is wrong** — not marginally, but
/// in a way that makes it fail on a correct road. Adding cells to any
/// component that is not the largest lowers the largest one's SHARE
/// arithmetically, whatever the road did; on the three sweep seeds the ring's
/// own largest component is 2,595 / 1,326 / 1,647 cells of 3,257 / 3,382 /
/// 3,173, so two of three side roads join a component that is not the biggest
/// and the share falls while the network strictly improves. The doc is
/// corrected; this is what the gate means.
///
/// Two claims, and the first is the one that caught a real defect. The road
/// must be in one component with RING — a junction that is not on the ring is
/// a road to a field — and that component must hold **more ring than road**,
/// which is what separates "it reaches the loop" from "it reaches 44 m of
/// loop". The first draft of `solve_side_roads` took the first carriageway
/// point on a walkable bearing and on seed 20260731 delivered the player to
/// an **11-cell island of ring**: 326 cells of road for 11 cells of
/// destination. `SIDE_ROAD_RING_RUN` is the fix and this is what holds it.
#[test]
fn a_side_road_joins_more_ring_than_it_is() {
    for seed in SWEEP_SEEDS {
        let h = terrain::haven(seed);
        let before = walk(seed, &h, false);
        let after = walk(seed, &h, true);
        assert!(
            after.road > before.road,
            "seed {seed:#x}: adding the side road added no road cells at all"
        );
        // A road cannot cut the network: the biggest piece of it may grow or
        // stay, never shrink.
        assert!(
            after.largest >= before.largest,
            "seed {seed:#x}: the largest road component shrank from {} cells \
             to {} — a road removed connectivity, which it cannot do",
            before.largest,
            after.largest
        );
        for (i, (mine, ring)) in after.junction.iter().enumerate() {
            println!(
                "seed {seed:#x}: road {i} is {mine} cells and reaches {ring} \
                 cells of ring without leaving the road ({:.1}x)",
                *ring as f32 / (*mine).max(1) as f32
            );
            assert!(
                *mine > 0,
                "seed {seed:#x}: road {i} contributed no cells — it is not on \
                 the grid this gate walks"
            );
            assert!(
                *ring > *mine,
                "seed {seed:#x}: road {i} is {mine} cells long and reaches \
                 only {ring} cells of ring. That is a road to a stub: the \
                 player walks the length of it to arrive at less road than \
                 they came down. `SIDE_ROAD_RING_RUN` is what refuses a \
                 junction on a fragment"
            );
        }
    }
}

/// At a junction, either road's cleared surface must beat the other's
/// shoulder. Returning the ring's Shoulder early left a transverse strip of
/// vegetation and shoulder slots across an otherwise valid side carriageway.
#[test]
fn a_carriageway_wins_over_the_other_roads_shoulder_at_a_junction() {
    let step = ROAD_HALF_W / 2.0;
    let span = (ROAD_SHOULDER_HALF_W / step) as i32;
    let mut side_over_ring = 0;
    let mut ring_over_side = 0;
    for seed in SEEDS {
        let h = terrain::haven(seed);
        let mut lattice = terrain::Lattice::new();
        for r in h.roads.iter().filter(|r| r.live) {
            let len = ((r.px - r.rx) * (r.px - r.rx) + (r.pz - r.rz) * (r.pz - r.rz)).sqrt();
            let (ux, uz) = ((r.px - r.rx) / len, (r.pz - r.rz) / len);
            for along in -span..=span * 2 {
                for across in -span..=span {
                    let x = r.rx + ux * along as f32 * step - uz * across as f32 * step;
                    let z = r.rz + uz * along as f32 * step + ux * across as f32 * step;
                    let ring = terrain::ring_band(&h.ring, x, z);
                    let side = terrain::side_band(&h, x, z);
                    let both = terrain::road_band(seed, &h, x, z);
                    assert_eq!(
                        terrain::road_band_memo(&mut lattice, seed, &h, x, z),
                        both,
                        "seed {seed} at ({x}, {z}): direct and memo junction masks disagree"
                    );
                    if ring == RoadBand::Shoulder && side == RoadBand::Carriageway {
                        side_over_ring += 1;
                        assert_eq!(both, RoadBand::Carriageway, "seed {seed} at ({x}, {z}): the ring shoulder interrupts the side carriageway");
                    }
                    if side == RoadBand::Shoulder && ring == RoadBand::Carriageway {
                        ring_over_side += 1;
                        assert_eq!(both, RoadBand::Carriageway, "seed {seed} at ({x}, {z}): the side shoulder interrupts the ring carriageway");
                    }
                }
            }
        }
    }
    assert!(
        side_over_ring > 0 && ring_over_side > 0,
        "the junction sweep must exercise both overlap directions"
    );
    println!("junction overlaps: {side_over_ring} side surfaces over ring shoulders, {ring_over_side} ring surfaces over side shoulders; direct and memo agree");
}
