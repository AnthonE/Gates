//! Occupant volume as arithmetic — what a slot does to a body that walks
//! into it (`TERRAIN.md` §1 stage 6, "wood, cover, low visibility").
//!
//! Same standard as `tests/road.rs` and `tests/haven.rs`: counts, distances
//! and areas that fail in seconds, re-derived from the public surface rather
//! than trusting the table's own numbers back. The two things this suite
//! exists to publish are the **cover fraction** — how much of the island's
//! land a body cannot stand in, which is the number that says whether a
//! forest is cover or decoration — and the **road's clearance**, because a
//! circulation loop that a boulder blocks is not a loop.
//!
//! What it deliberately does NOT test is anybody calling this. `collide.rs`
//! and `movement.rs` are the systems lane's files and contain zero references
//! to `Occupant` today; this suite gates the shapes and the predicate so that
//! wiring them is a one-line call against a surface that is already proven.

// The measurements ARE the gate's output — same reasoning and same allow as
// `tests/road.rs` and `tests/haven.rs`: the L5 wall bans format/print in SIM
// code, and a test harness is not sim code. `f32::abs` is walled everywhere,
// tests included, so anything needing a magnitude uses `max` of the signed
// pair.
#![allow(clippy::disallowed_macros)]

use sim_core::collide::{CAPSULE_HEIGHT_M, CAPSULE_RADIUS_M};
use sim_core::terrain::{
    self, Occupant, RoadBand, ScatterTable, Slot, CELLS_PER_SIDE, CELL_SIZE, ISLAND_SIZE,
    OCCUPANT_PROBE_CELLS, OCCUPANT_R_M, OCCUPANT_TOP_M, ROAD_R_MAX, ROAD_R_MIN, SLOT_SCALE_MAX,
};
use sim_core::yaw_dir;

/// Seeds every statistical check runs over. TERRAIN.md §7: "a seed that
/// fails is a bug in the generator, not a reroll" — so a band that holds on
/// one seed and not the next is a finding, not a reroll.
const SEEDS: [u64; 4] = [0, 1, 7, 12345];

fn center() -> f32 {
    ISLAND_SIZE * 0.5
}

/// Every occupant a scatter cell can actually hand back.
const SCATTERED: [Occupant; 8] = [
    Occupant::Tree,
    Occupant::StoneNode,
    Occupant::MetalNode,
    Occupant::SulfurNode,
    Occupant::Bush,
    Occupant::Rock,
    Occupant::BarrelSlot,
    Occupant::CrateSlot,
];

/// A slot standing at the origin at unit scale, for the predicate's own
/// unit tests — built by hand rather than drawn, so the geometry under test
/// is the geometry written down.
fn at_origin(occupant: Occupant) -> Slot {
    Slot {
        occupant,
        x: 0.0,
        y: 0.0,
        z: 0.0,
        yaw: 0,
        scale: 1.0,
    }
}

// ── the table ──────────────────────────────────────────────────────────────

/// Three occupants have no volume and each is a different KIND of zero. A
/// test that only counted them would pass just as well if the wrong three
/// were zero, so this names each one and the reason it is what it is.
#[test]
fn the_three_zeros_are_the_three_stated_zeros() {
    // Deliberately passable: a bush you cannot push through is a wall you
    // cannot see over.
    assert_eq!(
        OCCUPANT_R_M[Occupant::Bush as usize],
        0.0,
        "the bush has grown a radius — it is authored passable on purpose"
    );

    // Not a sim occupant at all: index 8 is the client's felled-pine stump,
    // and the enum skips the discriminant so the two tables stay aligned by
    // construction. If something ever takes 8, this fails before a body can
    // start bouncing off a stump the sim does not believe in.
    assert_eq!(OCCUPANT_R_M[8], 0.0, "index 8 is the client-only stump");
    assert_eq!(OCCUPANT_TOP_M[8], 0.0, "index 8 is the client-only stump");

    // DEFERRED, not authored. Pinned so the zero cannot be read as a tuned
    // value: the shelter's volume is a wall list with a doorway in it, which
    // is the one shape a radius cannot express, and giving it one is its own
    // slice. When that slice lands it deletes this assert on its way past.
    assert_eq!(
        OCCUPANT_R_M[Occupant::HavenShelter as usize],
        0.0,
        "the shelter has a radius — a cylinder either seals its doorway or \
         blocks nothing; it needs a box list, see NOW.md"
    );

    // And nothing else is zero. This is the half that makes the three above
    // mean something.
    for o in SCATTERED {
        if o == Occupant::Bush {
            continue;
        }
        assert!(
            OCCUPANT_R_M[o as usize] > 0.0 && OCCUPANT_TOP_M[o as usize] > 0.0,
            "{o:?} is scattered into the world with no volume: r={} top={}",
            OCCUPANT_R_M[o as usize],
            OCCUPANT_TOP_M[o as usize]
        );
    }
}

/// The radii are read off `web/src/props.js`'s authored geometry, and the
/// rule is the mesh's maximum horizontal extent so a body never reaches a
/// triangle the client is drawing. That rule has an observable consequence
/// this can check without parsing JavaScript: a node and a rock are the same
/// family of mesh at different sizes, a barrel is narrower than either, and
/// a trunk is the narrowest thing on the island.
#[test]
fn the_radii_keep_the_order_the_meshes_have() {
    let r = |o: Occupant| OCCUPANT_R_M[o as usize];
    assert!(
        r(Occupant::Tree) < r(Occupant::BarrelSlot),
        "a pine trunk ({}) is not narrower than a barrel ({})",
        r(Occupant::Tree),
        r(Occupant::BarrelSlot)
    );
    assert!(
        r(Occupant::BarrelSlot) < r(Occupant::CrateSlot),
        "the road's barrel ({}) is not narrower than the pad's crate ({})",
        r(Occupant::BarrelSlot),
        r(Occupant::CrateSlot)
    );
    assert!(
        r(Occupant::CrateSlot) < r(Occupant::StoneNode),
        "the crate ({}) is not narrower than an ore node ({})",
        r(Occupant::CrateSlot),
        r(Occupant::StoneNode)
    );
    assert!(
        r(Occupant::StoneNode) < r(Occupant::Rock),
        "a node ({}) is not narrower than a boulder ({})",
        r(Occupant::StoneNode),
        r(Occupant::Rock)
    );
    // The three ore nodes share one mesh, so they must share one radius —
    // a body that stops closer to sulfur than to stone is a bug nobody
    // would ever look for.
    assert_eq!(r(Occupant::StoneNode), r(Occupant::MetalNode));
    assert_eq!(r(Occupant::StoneNode), r(Occupant::SulfurNode));

    // A trunk you stop at is 5.7 m of trunk, not 6.6 m of tree: you walk
    // UNDER a canopy. The distinction is free at a 1.7 m body and correct
    // the moment anything leaves the ground.
    assert!(
        OCCUPANT_TOP_M[Occupant::Tree as usize] > 5.0,
        "the tree's blocking height collapsed to {}",
        OCCUPANT_TOP_M[Occupant::Tree as usize]
    );
    // Everything a body can see over is under its own eyeline; everything
    // else is cover. The crate is the low one and that is the point of it.
    assert!(
        OCCUPANT_TOP_M[Occupant::CrateSlot as usize] < 1.0,
        "the crate is now taller than a metre: {}",
        OCCUPANT_TOP_M[Occupant::CrateSlot as usize]
    );
}

// ── the predicate ──────────────────────────────────────────────────────────

/// The reach is `r * scale + capsule`, and the test is strict on the far
/// side: a body exactly at the reach is clear. Checked either side of the
/// boundary rather than at it, because "touching counts as blocked" and
/// "touching counts as clear" are both defensible and only one is written.
#[test]
fn a_body_stops_at_the_reach_and_not_before() {
    for o in SCATTERED {
        let s = at_origin(o);
        let reach = OCCUPANT_R_M[o as usize] + CAPSULE_RADIUS_M;
        if OCCUPANT_R_M[o as usize] <= 0.0 {
            assert!(
                !terrain::slot_blocks(&s, 0.0, 0.0, 0.0, CAPSULE_RADIUS_M, CAPSULE_HEIGHT_M),
                "{o:?} has no radius but blocked a body standing in it"
            );
            continue;
        }
        assert!(
            terrain::slot_blocks(&s, 0.0, 0.0, 0.0, CAPSULE_RADIUS_M, CAPSULE_HEIGHT_M),
            "{o:?} did not block a body standing on its own axis"
        );
        assert!(
            terrain::slot_blocks(
                &s,
                reach - 0.01,
                0.0,
                0.0,
                CAPSULE_RADIUS_M,
                CAPSULE_HEIGHT_M
            ),
            "{o:?} let a body inside its reach of {reach}"
        );
        assert!(
            !terrain::slot_blocks(
                &s,
                reach + 0.01,
                0.0,
                0.0,
                CAPSULE_RADIUS_M,
                CAPSULE_HEIGHT_M
            ),
            "{o:?} blocked a body past its reach of {reach}"
        );
        // Radial, not axis-aligned: the diagonal has to agree with the
        // cardinal or the shape is a square wearing a radius.
        let d = (reach + 0.01) * core::f32::consts::FRAC_1_SQRT_2;
        assert!(
            !terrain::slot_blocks(&s, d, d, 0.0, CAPSULE_RADIUS_M, CAPSULE_HEIGHT_M),
            "{o:?} blocked past its reach on the diagonal"
        );
    }
}

/// Scale is not decoration: `scatter` draws it in [0.9, 1.1] and a body has
/// to stop where the mesh actually is. Both the radius and the height carry
/// it, and the vertical test is a half-open interval against the slot's own
/// ground so that standing on top of something is the ground query's job and
/// not this one's.
#[test]
fn scale_moves_the_reach_and_the_top_together() {
    let o = Occupant::Rock;
    let r = OCCUPANT_R_M[o as usize];
    let top = OCCUPANT_TOP_M[o as usize];

    let mut small = at_origin(o);
    small.scale = 0.9;
    let mut large = at_origin(o);
    large.scale = SLOT_SCALE_MAX;

    // A point outside the small boulder and inside the large one separates
    // them; if scale were ignored both would answer the same.
    let between = r * 1.0 + CAPSULE_RADIUS_M;
    assert!(
        !terrain::slot_blocks(
            &small,
            between,
            0.0,
            0.0,
            CAPSULE_RADIUS_M,
            CAPSULE_HEIGHT_M
        ),
        "a 0.9-scale boulder blocked at the unit reach"
    );
    assert!(
        terrain::slot_blocks(
            &large,
            between,
            0.0,
            0.0,
            CAPSULE_RADIUS_M,
            CAPSULE_HEIGHT_M
        ),
        "a 1.1-scale boulder cleared at the unit reach"
    );

    // Feet at or above the top walk over; below it, stopped.
    assert!(
        !terrain::slot_blocks(
            &small,
            0.0,
            0.0,
            top * 0.9,
            CAPSULE_RADIUS_M,
            CAPSULE_HEIGHT_M
        ),
        "a body standing on the small boulder was still blocked by it"
    );
    assert!(
        terrain::slot_blocks(
            &large,
            0.0,
            0.0,
            top * 0.9,
            CAPSULE_RADIUS_M,
            CAPSULE_HEIGHT_M
        ),
        "the large boulder's extra height did not block"
    );
    // A slot's ground is its own, not sea level: a node on a hill blocks at
    // the hill's height and nothing about the test may assume y = 0.
    let mut high = at_origin(o);
    high.y = 40.0;
    assert!(
        terrain::slot_blocks(&high, 0.0, 0.0, 40.0, CAPSULE_RADIUS_M, CAPSULE_HEIGHT_M),
        "a slot 40 m up did not block a body standing at its own base"
    );
    assert!(
        !terrain::slot_blocks(&high, 0.0, 0.0, 0.0, CAPSULE_RADIUS_M, CAPSULE_HEIGHT_M),
        "a slot 40 m up blocked a body down at sea level"
    );
}

// ── the neighbourhood ──────────────────────────────────────────────────────

/// `OCCUPANT_PROBE_CELLS` claims a 3×3 cell scan is COMPLETE, not usual, and
/// the whole claim rests on every slot lying strictly inside its own cell.
/// That is a property of `scatter`'s jitter (±3 m about a cell center on an
/// 8 m cell) and of the pad's authored anchors, which are a different code
/// path entirely — so it is walked over real seeds rather than argued.
#[test]
fn every_slot_lies_inside_its_own_cell() {
    let table = ScatterTable::alpha_default();
    let mut checked = 0u32;
    let mut worst_margin = CELL_SIZE;
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        for cz in 0..CELLS_PER_SIDE {
            for cx in 0..CELLS_PER_SIDE {
                let s = terrain::scatter(seed, &table, &haven, cx, cz);
                if s.occupant == Occupant::None {
                    continue;
                }
                checked += 1;
                let lo_x = cx as f32 * CELL_SIZE;
                let lo_z = cz as f32 * CELL_SIZE;
                let mx = (s.x - lo_x).min(lo_x + CELL_SIZE - s.x);
                let mz = (s.z - lo_z).min(lo_z + CELL_SIZE - s.z);
                let m = mx.min(mz);
                worst_margin = worst_margin.min(m);
                assert!(
                    m >= 0.0,
                    "seed {seed}: {:?} at ({}, {}) is outside cell ({cx}, {cz})",
                    s.occupant,
                    s.x,
                    s.z
                );
            }
        }
    }
    assert!(checked > 0, "no slots at all — the scan is not scanning");
    println!(
        "slots {checked}, worst distance from a slot to its own cell edge {worst_margin:.3} m"
    );

    // With that established, the 3×3 is complete iff the widest reach fits
    // in a cell. Re-derived here from the table rather than trusting the
    // const block, because the const block is the thing under test.
    let mut widest: f32 = 0.0;
    for r in OCCUPANT_R_M {
        widest = widest.max(r);
    }
    let reach = widest * SLOT_SCALE_MAX + CAPSULE_RADIUS_M;
    assert!(
        reach < CELL_SIZE * OCCUPANT_PROBE_CELLS as f32,
        "widest reach {reach:.3} m does not fit in {OCCUPANT_PROBE_CELLS} cell(s) of \
         {CELL_SIZE} m — the probe neighbourhood is no longer complete"
    );
    println!(
        "widest reach {reach:.3} m against a {} m probe radius",
        CELL_SIZE * OCCUPANT_PROBE_CELLS as f32
    );
}

// ── what it means for the game ─────────────────────────────────────────────

/// The numbers this suite exists to publish. There are TWO of them and the
/// first draft of this test conflated them, which is worth recording because
/// the conflation is the intuitive one.
///
/// **Blocked area** answers "can I cross the island" and nothing else. It
/// came out at 0.43% of the land on seed 0, and a floor under it read as a
/// failure — wrongly. A pine trunk is 0.26 m of radius and 0.21 m² of
/// footprint, and 7,589 of them still leave 99.6% of the ground standable.
/// Footprint is simply not what cover is made of: a trunk blocks sight for
/// 5.7 m of height off a fifth of a square metre.
///
/// **Blocker density** is the cover metric — how far you walk between things
/// you can put between yourself and someone else. That is the one with a
/// floor, and the floor is what fails if scatter ever thins back into
/// decoration.
///
/// Both bands are wide on purpose: they are regression tripwires on numbers
/// nobody had measured here before this pass, not tuning targets, and the
/// printed values are the point.
#[test]
fn the_island_has_cover_and_stays_crossable() {
    let table = ScatterTable::alpha_default();
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let mut blocked_area = 0.0f32;
        let mut land_cells = 0u32;
        let mut blockers = 0u32;
        for cz in 0..CELLS_PER_SIDE {
            for cx in 0..CELLS_PER_SIDE {
                let s = terrain::scatter(seed, &table, &haven, cx, cz);
                // Land is what a body could stand on at all. A cell that
                // produced any slot is land by construction; an empty cell
                // is tested directly so the denominator is not the count of
                // occupied cells (which would make the ratio meaningless).
                let x = cx as f32 * CELL_SIZE + 4.0;
                let z = cz as f32 * CELL_SIZE + 4.0;
                if terrain::height(seed, x, z) < terrain::LAND_MIN_H {
                    continue;
                }
                land_cells += 1;
                let r = OCCUPANT_R_M[s.occupant as usize] * s.scale;
                if r > 0.0 {
                    blockers += 1;
                    blocked_area += core::f32::consts::PI * r * r;
                }
            }
        }
        let land_area = land_cells as f32 * CELL_SIZE * CELL_SIZE;
        let frac = blocked_area / land_area;
        let density = blockers as f32 / land_cells as f32;
        // Mean spacing if the blockers were on a square lattice — the
        // player-facing form of the density, in metres between things you
        // can stand behind.
        let spacing = CELL_SIZE / density.max(1.0e-6).sqrt();
        println!(
            "seed {seed}: {blockers} blockers over {land_cells} land cells \
             ({:.2} km²) — {:.1}% of cells carry cover, ~{spacing:.1} m apart, \
             {:.2}% of the land is solid",
            land_area * 1.0e-6,
            density * 100.0,
            frac * 100.0
        );
        // Cover: something to break line of sight on, often enough that
        // walking is a decision. A collapse to a twentieth of the measured
        // 19.6% is a generator regression, not weather.
        assert!(
            density > 0.05,
            "seed {seed}: only {:.2}% of land cells carry anything solid \
             (~{spacing:.0} m apart) — there is nothing on this island to \
             break line of sight on",
            density * 100.0
        );
        // Crossability: the other question entirely, and the one footprint
        // actually answers.
        assert!(
            frac < 0.25,
            "seed {seed}: {:.2}% of the land is solid — the island is a maze",
            frac * 100.0
        );
    }
}

/// The coast road's whole job is circulation (`TERRAIN.md` §5, "where do I
/// go?"), and `tests/road.rs` already keeps the carriageway clear of SLOTS.
/// Volume is the other half of that claim and it is not implied by it: a
/// boulder standing beside the road reaches 1.65 m past its own center at
/// full scale, which is most of the way across a 4 m carriageway.
///
/// Walked as a body would walk it — the 3×3 probe at points across the
/// carriageway — so this exercises `OCCUPANT_PROBE_CELLS` against the real
/// world rather than asserting the neighbourhood is right in the abstract.
///
/// **What is measured is the widest CLEAR RUN across the carriageway, not
/// whether every point of it is clear**, and the difference is a finding
/// rather than a convenience. The first draft sampled the single point where
/// a radial march first entered the carriageway — its inner EDGE — and
/// demanded that be clear. On seed 0 it found a pine at (1773.2, 1271.5)
/// reaching 0.69 m over that edge from the shoulder, and called the ring
/// blocked. It is not: the tree is legally on the shoulder (`tests/road.rs`
/// owns that and is green), a road lined with trees is a road, and a body
/// crossing at that bearing has 3-odd metres of clear carriageway beside it.
/// A circulation loop is walkable when a body FITS, so the gate is the clear
/// width against the body's own diameter.
#[test]
fn the_road_is_walkable_with_volume_on() {
    // What has to fit through: a capsule is `CAPSULE_RADIUS_M` in radius, so
    // its diameter is the narrowest gap it can pass. Derived from the body,
    // never a chosen clearance.
    let body_w = 2.0 * CAPSULE_RADIUS_M;
    // Fine enough that a run measured in steps resolves the body width to
    // better than a fifth of itself.
    const STEP: f32 = 0.125;
    let table = ScatterTable::alpha_default();
    let c = center();
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let mut sampled = 0u32;
        let mut narrowest = f32::MAX;
        let mut narrowest_at = 0u16;
        for yaw in 0..256u16 {
            let (ux, uz) = yaw_dir(yaw << 8);
            // Find the carriageway's inner edge on this bearing, the same
            // coarse march `tests/road.rs` uses.
            let mut hit = None;
            let mut d = ROAD_R_MIN;
            while d <= ROAD_R_MAX {
                if terrain::road_band(seed, c + ux * d, c + uz * d) == RoadBand::Carriageway {
                    hit = Some(d);
                    break;
                }
                d += 0.5;
            }
            let Some(entry) = hit else {
                // A bearing with no carriageway is `tests/road.rs`'s
                // business, not this one's — it owns ring closure.
                continue;
            };
            sampled += 1;

            // Scan across the carriageway from just inside the entry, and
            // measure the longest unbroken clear run. Starting half a coarse
            // step back so the fine scan cannot miss the edge the coarse
            // march stepped over.
            let mut best = 0.0f32;
            let mut run = 0.0f32;
            let mut d = entry - 0.5;
            while d <= entry + 2.0 * terrain::ROAD_HALF_W + 1.0 {
                let (x, z) = (c + ux * d, c + uz * d);
                d += STEP;
                if terrain::road_band(seed, x, z) != RoadBand::Carriageway {
                    run = 0.0;
                    continue;
                }
                let feet = terrain::height(seed, x, z);
                let bx = (x * (1.0 / CELL_SIZE)) as i32;
                let bz = (z * (1.0 / CELL_SIZE)) as i32;
                let mut blocked = false;
                for dz in -OCCUPANT_PROBE_CELLS..=OCCUPANT_PROBE_CELLS {
                    for dx in -OCCUPANT_PROBE_CELLS..=OCCUPANT_PROBE_CELLS {
                        let s = terrain::scatter(seed, &table, &haven, bx + dx, bz + dz);
                        blocked |= terrain::slot_blocks(
                            &s,
                            x,
                            z,
                            feet,
                            CAPSULE_RADIUS_M,
                            CAPSULE_HEIGHT_M,
                        );
                    }
                }
                run = if blocked { 0.0 } else { run + STEP };
                best = best.max(run);
            }
            if best < narrowest {
                narrowest = best;
                narrowest_at = yaw;
            }
        }
        assert!(
            sampled > 200,
            "seed {seed}: only {sampled} of 256 bearings carried a carriageway"
        );
        println!(
            "seed {seed}: {sampled} bearings; narrowest clear carriageway \
             {narrowest:.2} m at bearing {narrowest_at}, against a {body_w:.2} m body"
        );
        assert!(
            narrowest >= body_w,
            "seed {seed}: bearing {narrowest_at} leaves only {narrowest:.2} m of clear \
             carriageway for a {body_w:.2} m body — the circulation loop does not circulate"
        );
    }
}

/// The pad's container ring is five crates on a 10 m circle, and a body has
/// to be able to walk between them to reach the building. Volume is what
/// makes that a real question: the gaps are wide by construction (the ring
/// is sized so no two anchors share a scatter cell) but "wide enough for a
/// cell" and "wide enough for a body" are different numbers, and only one of
/// them was ever checked.
#[test]
fn the_pad_ring_admits_a_body_between_its_crates() {
    let table = ScatterTable::alpha_default();
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        // Walk the ring's own circle and count how much of it is passable.
        let mut clear = 0u32;
        let mut total = 0u32;
        for yaw in 0..256u16 {
            let (ux, uz) = yaw_dir(yaw << 8);
            let x = haven.x + ux * terrain::HAVEN_CRATE_R_M;
            let z = haven.z + uz * terrain::HAVEN_CRATE_R_M;
            let feet = terrain::height(seed, x, z);
            let bx = (x * (1.0 / CELL_SIZE)) as i32;
            let bz = (z * (1.0 / CELL_SIZE)) as i32;
            let mut blocked = Occupant::None;
            for dz in -OCCUPANT_PROBE_CELLS..=OCCUPANT_PROBE_CELLS {
                for dx in -OCCUPANT_PROBE_CELLS..=OCCUPANT_PROBE_CELLS {
                    let s = terrain::scatter(seed, &table, &haven, bx + dx, bz + dz);
                    if terrain::slot_blocks(&s, x, z, feet, CAPSULE_RADIUS_M, CAPSULE_HEIGHT_M) {
                        blocked = s.occupant;
                    }
                }
            }
            total += 1;
            if blocked == Occupant::None {
                clear += 1;
            } else {
                // Which occupant blocked matters: the ring's own crates are
                // the expected answer, and anything else on it would mean
                // the exclusion zone has stopped clearing the pad.
                assert_eq!(
                    blocked,
                    Occupant::CrateSlot,
                    "seed {seed}: the crate ring is blocked at bearing {yaw} by \
                     {blocked:?}, which is not one of the pad's own containers — \
                     the exclusion zone is not clearing the pad"
                );
            }
        }
        let frac = clear as f32 / total as f32;
        println!(
            "seed {seed}: {clear}/{total} of the crate ring's circumference is \
             walkable ({:.1}%)",
            frac * 100.0
        );
        // Five crates of radius 0.68 plus a capsule on a 10 m ring cannot
        // physically cover much of it; the gate is that the gaps are the
        // majority and the crates are actually there.
        assert!(
            frac > 0.5,
            "seed {seed}: only {:.1}% of the ring is walkable — a body cannot \
             reach the building",
            frac * 100.0
        );
        assert!(
            frac < 1.0,
            "seed {seed}: the whole ring is walkable — the crates have no volume \
             at all, which means the pad's containers are still holograms"
        );
    }
}
