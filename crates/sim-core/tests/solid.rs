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
//! What it does not test is anybody calling this — and that half now exists.
//! `movement::step` asks `occupy::Occupants::blocks` on every candidate move,
//! and `tests/walk.rs` drives real bodies into real trunks, boulders and the
//! shelter's walls. The split stands: this suite is the geometry, that one is
//! the wiring, and a failure here means the shape is wrong rather than the
//! caller.

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

/// Two occupants have no volume and each is a different KIND of zero. A
/// test that only counted them would pass just as well if the wrong two
/// were zero, so this names each one and the reason it is what it is.
///
/// There were three until the shelter got its box list; the third is now the
/// broad-phase check at the bottom, because a bounding circle that does not
/// contain the building is the same failure the zero was.
const OCCUPANT_CORNER_SQ: f32 = terrain::SHELTER_CORNER_R_M * terrain::SHELTER_CORNER_R_M;

#[test]
fn the_two_zeros_are_the_two_stated_zeros() {
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

    // The shelter's zero is GONE — it had a third one until the box list
    // landed, and this is the assert that used to pin it. Its radius is now a
    // broad phase and the real volume is `SHELTER_BOXES`, so the check that
    // replaces it is that the two agree: a bounding circle smaller than the
    // building it guards would reject bodies before the boxes ever saw them,
    // which is the silent-passable failure the old zero was honest about.
    assert!(
        OCCUPANT_R_M[Occupant::HavenShelter as usize] > 0.0,
        "the shelter is back to passable"
    );
    for b in terrain::SHELTER_BOXES {
        let ex = b[0].max(-b[0]) + b[3] * 0.5;
        let ez = b[2].max(-b[2]) + b[5] * 0.5;
        assert!(
            ex * ex + ez * ez <= OCCUPANT_CORNER_SQ,
            "a shelter box reaches {:.3} m but the broad phase rejects past {:.3} m — \
             bodies would be waved through the narrow phase",
            (ex * ex + ez * ez).sqrt(),
            terrain::SHELTER_CORNER_R_M
        );
    }

    // And nothing else is zero. This is the half that makes the two above
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
/// failure — wrongly.
///
/// The sentence that used to sit here said "a pine trunk is 0.26 m of radius
/// and 0.21 m² of footprint, and 7,589 of them still leave 99.6% of the
/// ground standable", which is false and worth keeping the corpse of: 7,589
/// is the printed count of blockers of EVERY kind — trunks, three node kinds,
/// rocks, barrels, crates — and the sentence quietly re-labelled it as
/// trunks. Trunks at that count block about a seventh of the measured area,
/// because a 1.5 m rock is 7.1 m² against a trunk's 0.21 m² and the wide
/// occupants dominate the sum. `trunk_share` below asserts that ratio rather
/// than asserting the prose, which is the only way a sentence like this stays
/// true.
///
/// The point the false sentence was reaching for survives intact, and it is
/// the reason the floor was wrong: footprint is not what cover is made of. A
/// trunk blocks sight for 5.7 m of height off a fifth of a square metre.
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
        // The correction to the docstring above, as arithmetic rather than as
        // a sentence. If every one of these blockers were a pine trunk they
        // would cover a fraction of what the real mix covers, because the sum
        // is over r² and a 1.5 m rock is 50 trunks' worth of it. This is what
        // makes "footprint is not what cover is made of" a measurement.
        let (trunk_r, _) = terrain::occupant_volume(Occupant::Tree);
        let all_trunk_area = blockers as f32 * core::f32::consts::PI * trunk_r * trunk_r;
        let trunk_share = all_trunk_area / blocked_area;
        println!(
            "seed {seed}: were all {blockers} blockers trunks they would cover \
             {:.3}% of the land — {:.1}x less than the {:.2}% the real mix covers",
            all_trunk_area / land_area * 100.0,
            1.0 / trunk_share,
            frac * 100.0
        );
        assert!(
            trunk_share < 0.5,
            "seed {seed}: trunks alone would account for {:.0}% of the blocked \
             area, so the wide occupants no longer dominate it — the docstring \
             above is about to become false again",
            trunk_share * 100.0
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
        let mut shelter_arc = 0u32;
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
                // Which occupant blocked matters: the pad's OWN furniture is
                // the expected answer, and anything else on it would mean the
                // exclusion zone has stopped clearing the pad.
                //
                // The shelter joined that set when it got its box list. It is
                // not an intruder — it stands at `HAVEN_SHELTER_R_M` = 6.5 m
                // with a 4.95 m bounding radius, so its far corner reaches
                // past the 10 m ring by construction and always did. What
                // changed is that it is solid enough to be noticed. The arc it
                // takes is bounded below so "expected" cannot quietly become
                // "the building ate the ring".
                assert!(
                    blocked == Occupant::CrateSlot || blocked == Occupant::HavenShelter,
                    "seed {seed}: the crate ring is blocked at bearing {yaw} by \
                     {blocked:?}, which is not the pad's own furniture — the \
                     exclusion zone is not clearing the pad"
                );
                if blocked == Occupant::HavenShelter {
                    shelter_arc += 1;
                }
            }
        }
        // The building stands ON the ring, so it takes an arc of it; a body
        // walks round. Sealing a third of the circumference would mean the
        // shelter had grown or moved outward onto the containers.
        assert!(
            shelter_arc < 85,
            "seed {seed}: the shelter blocks {shelter_arc}/256 of the crate \
             ring — it has grown out over the containers rather than standing \
             inside them"
        );
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

// ── the shelter: the one occupant whose volume is a hole ───────────────────

/// Feet height a body walks the building at: standing on the pad's own
/// ground, which is the slot's `y`. The plinth's 0.2 m top is a kerb nothing
/// yet steps onto (`SHELTER_FLOOR_IX` says so), so the honest test is a body
/// at pad ground, which is also the worst case for the floor sealing the
/// doorway.
const FEET: f32 = 0.0;

/// Is a body at local (lx, lz) stopped by the shelter standing at the origin
/// with the given yaw? Takes LOCAL coordinates and rotates them out to world,
/// so the test drives the predicate through the same frame change it performs
/// and a sign error there cannot cancel itself.
fn shelter_hits(yaw: u8, lx: f32, lz: f32) -> bool {
    let mut slot = at_origin(Occupant::HavenShelter);
    slot.yaw = yaw;
    let (s, c) = yaw_dir((yaw as u16) << 8);
    // local +X is (c, -s), local +Z is (s, c)
    let x = lx * c + lz * s;
    let z = -lx * s + lz * c;
    terrain::slot_blocks(&slot, x, z, FEET, CAPSULE_RADIUS_M, CAPSULE_HEIGHT_M)
}

/// The doorway is a doorway: a body walks in along local +Z and is never
/// stopped, at every yaw.
///
/// This is the test the whole box list exists for. A single radius passes
/// every other check in this file and fails this one, which is why the
/// shelter could not have a radius.
#[test]
fn a_body_walks_in_through_the_door() {
    for yaw in [0u8, 17, 64, 128, 200, 255] {
        // From outside the door (+Z, clear of the bounding circle) inward to
        // the middle of the room, and no further: the back wall is a wall and
        // stopping at it is the NEXT test's business. Marching past it was
        // this test's first draft and it "failed" on the back wall's inner
        // face at local z = −2.30, which is the wall doing its job.
        let mut lz = terrain::SHELTER_CORNER_R_M + 2.0;
        let mut blocked_at: Option<f32> = None;
        while lz >= 0.0 {
            if shelter_hits(yaw, 0.0, lz) && blocked_at.is_none() {
                blocked_at = Some(lz);
            }
            lz -= 0.05;
        }
        assert!(
            blocked_at.is_none(),
            "yaw {yaw}: the centreline through the doorway is blocked at local \
             z = {:.2} m — the door is walled, or the yaw frame is inverted and \
             the body is walking into the back wall",
            blocked_at.unwrap_or(0.0)
        );
        // The paired half, here rather than apart from it: keep going and the
        // back wall MUST stop the same body. Together these say the hole is
        // in exactly one end of the building.
        assert!(
            shelter_hits(yaw, 0.0, -2.5),
            "yaw {yaw}: the body walked out through the back of the building"
        );
    }
}

/// And the other three sides are walls. The same march on −Z and on ±X is
/// stopped every time, at every yaw.
///
/// Paired with the test above deliberately: "nothing blocks" passes if the
/// predicate always returns false, and "everything blocks" passes if it always
/// returns true. Only the pair says the shape has a hole in one known place.
#[test]
fn the_other_three_sides_are_walls() {
    for yaw in [0u8, 17, 64, 128, 200, 255] {
        for (name, ux, uz) in [
            ("back", 0.0, -1.0),
            ("left", -1.0, 0.0),
            ("right", 1.0, 0.0),
        ] {
            let mut hit = false;
            let mut d = terrain::SHELTER_CORNER_R_M + 2.0;
            while d >= 0.0 {
                if shelter_hits(yaw, ux * d, uz * d) {
                    hit = true;
                    break;
                }
                d -= 0.05;
            }
            assert!(
                hit,
                "yaw {yaw}: a body walks straight through the {name} wall — \
                 the box list is not sealing, or the broad phase rejects before \
                 the narrow phase runs"
            );
        }
    }
}

/// The floor does not seal the building.
///
/// The regression `SHELTER_FLOOR_IX` exists for, and it is worth its own test
/// because it fails in the most misleading way available: include the plinth
/// and every wall test above still passes, the doorway test fails, and the
/// building reads as a solid 7 x 7 block that a player bounces off from the
/// outside. Here it is asserted from the other end — the plinth's own
/// footprint, at a point that is inside it and outside every wall, is clear.
#[test]
fn the_plinth_is_floor_and_not_a_fourteenth_wall() {
    let f = terrain::SHELTER_BOXES[terrain::SHELTER_FLOOR_IX];
    let half_x = f[3] * 0.5;
    let half_z = f[5] * 0.5;
    // Just inside the plinth's rim, dead center of the doorway side: inside
    // the floor's footprint, outside the jambs, and on the threshold.
    let probe = half_z - 0.05;
    assert!(
        !shelter_hits(0, 0.0, probe),
        "the plinth blocks at the threshold — it is being treated as a wall, \
         and the doorway is sealed from outside"
    );
    // It really is the floor's footprint being probed and not thin air.
    assert!(
        probe < half_z && half_x > 2.0,
        "the plinth is no longer the wide low box this test assumes"
    );
    // And the walls that stand ON it are still solid at the same height,
    // which is what makes the exception narrow rather than a hole in the
    // predicate.
    assert!(
        shelter_hits(0, 0.0, -probe),
        "the back wall went passable along with the floor — the floor \
         exception is skipping more than one box"
    );
}

/// Headroom and overhead clearance: the lintel is over a doorway, not across
/// it, and the roof is something you walk under.
///
/// The interval-overlap half of the predicate, applied to the boxes. A
/// vertical test written as a ceiling rather than an overlap passes the
/// horizontal tests above and makes the roof a floor-to-sky wall.
#[test]
fn the_lintel_and_the_roof_are_overhead() {
    // A body on the threshold is clear, because the lintel starts above it.
    assert!(!shelter_hits(0, 0.0, 2.9));
    // Standing in the middle of the building: the roof is 3.8 m up and a
    // 1.7 m body is under it.
    assert!(
        !shelter_hits(0, 0.0, 0.0),
        "the interior is blocked — the roof or the floor is being met by a \
         body standing under it"
    );
    // Raise the same body until its feet are at the roof and it meets it.
    let mut slot = at_origin(Occupant::HavenShelter);
    slot.yaw = 0;
    assert!(
        terrain::slot_blocks(&slot, 0.0, 0.0, 3.9, CAPSULE_RADIUS_M, CAPSULE_HEIGHT_M),
        "feet at 3.9 m are inside the roof slab and pass through it"
    );
    // And above the tower cap there is nothing left to hit.
    assert!(
        !terrain::slot_blocks(
            &slot,
            0.0,
            0.0,
            terrain::SHELTER_PEAK_M + 0.01,
            CAPSULE_RADIUS_M,
            CAPSULE_HEIGHT_M
        ),
        "something blocks above the shelter's own stated peak"
    );
}

/// The interior is enclosed and reachable, by flood fill rather than by
/// inspection — the same question `ci/haven_shelter.mjs` asked of the mesh
/// before it went with the browser client, asked here of the sim's volume.
/// This side is the one that survived, so it is now the only side.
///
/// Reachable from outside AND enclosed except through the door is one
/// property, not two: seal the door and the fill must fail; leave a gap in a
/// wall and the fill leaks out the side, which the perimeter check catches.
#[test]
fn the_interior_is_reachable_only_through_the_door() {
    const STEP: f32 = 0.25;
    const SPAN: i32 = 24; // ±6 m about the origin at 0.25 m
    let idx = |ix: i32, iz: i32| ((iz + SPAN) * (2 * SPAN + 1) + (ix + SPAN)) as usize;
    let n = ((2 * SPAN + 1) * (2 * SPAN + 1)) as usize;
    let mut seen = vec![false; n];
    // Flood from a point outside the building, in front of the door.
    let mut stack = vec![(0i32, SPAN)];
    seen[idx(0, SPAN)] = true;
    let mut reached_center = false;
    while let Some((ix, iz)) = stack.pop() {
        if ix == 0 && iz == 0 {
            reached_center = true;
        }
        for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
            let (nx, nz) = (ix + dx, iz + dz);
            if !(-SPAN..=SPAN).contains(&nx) || !(-SPAN..=SPAN).contains(&nz) || seen[idx(nx, nz)] {
                continue;
            }
            if shelter_hits(0, nx as f32 * STEP, nz as f32 * STEP) {
                continue;
            }
            seen[idx(nx, nz)] = true;
            stack.push((nx, nz));
        }
    }
    assert!(
        reached_center,
        "the interior cannot be reached from outside — the doorway does not \
         admit a {CAPSULE_RADIUS_M} m capsule on the grid this walks"
    );
    // Enclosure: the four cells just inside each wall must NOT have been
    // reached by going around — they are reached, but only via the door, so
    // the check that means something is that the fill did not leak through a
    // wall. Re-run it with the door sealed and the centre must become
    // unreachable.
    let sealed = |lx: f32, lz: f32| shelter_hits(0, lx, lz) || (lz > 2.5 && lz < 3.3);
    let mut seen2 = vec![false; n];
    let mut stack2 = vec![(0i32, SPAN)];
    seen2[idx(0, SPAN)] = true;
    let mut leaked = false;
    while let Some((ix, iz)) = stack2.pop() {
        if ix == 0 && iz == 0 {
            leaked = true;
        }
        for (dx, dz) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
            let (nx, nz) = (ix + dx, iz + dz);
            if !(-SPAN..=SPAN).contains(&nx) || !(-SPAN..=SPAN).contains(&nz) || seen2[idx(nx, nz)]
            {
                continue;
            }
            if sealed(nx as f32 * STEP, nz as f32 * STEP) {
                continue;
            }
            seen2[idx(nx, nz)] = true;
            stack2.push((nx, nz));
        }
    }
    assert!(
        !leaked,
        "with the doorway plugged the interior is STILL reachable — there is a \
         gap in the walls somewhere and the door is not the only way in"
    );
}

/// The shelter that worldgen actually places is solid, on every seed.
///
/// Everything above drives a hand-built slot at the origin. This drives the
/// one the island really has: resolved through `scatter`, at its own position,
/// at its own yaw, on its own ground. A box list that is right at the origin
/// and wrong once placed is the failure the road and pad suites both had to
/// learn to catch separately.
#[test]
fn the_placed_shelter_is_solid_and_faces_its_door_at_the_pad() {
    let table = ScatterTable::alpha_default();
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let (sx, sz, syaw) = terrain::haven_shelter(&haven);
        let slot = terrain::scatter(
            seed,
            &table,
            &haven,
            (sx / CELL_SIZE) as i32,
            (sz / CELL_SIZE) as i32,
        );
        assert_eq!(
            slot.occupant,
            Occupant::HavenShelter,
            "seed {seed}: the shelter's own cell does not hand back the shelter"
        );
        assert_eq!(
            slot.scale, 1.0,
            "seed {seed}: the shelter is authored, not drawn"
        );

        // Its back wall stops a body: walk in from the far side.
        let (s, c) = yaw_dir((syaw as u16) << 8);
        let feet = slot.y;
        let mut hit = false;
        let mut d = terrain::SHELTER_CORNER_R_M;
        while d >= 0.0 {
            // local −Z, out to world
            let (x, z) = (slot.x - s * d, slot.z - c * d);
            if terrain::slot_blocks(&slot, x, z, feet, CAPSULE_RADIUS_M, CAPSULE_HEIGHT_M) {
                hit = true;
                break;
            }
            d -= 0.05;
        }
        assert!(
            hit,
            "seed {seed}: the placed shelter's back wall is passable"
        );

        // And its doorway faces the pad centre, which is what makes it a
        // shelter a player walking off the road arrives at rather than a box
        // with its back to them. `haven_shelter` states it; this measures it.
        let door_x = slot.x + s * 4.0;
        let door_z = slot.z + c * 4.0;
        let to_pad = (haven.x - slot.x) * s + (haven.z - slot.z) * c;
        assert!(
            to_pad > 0.0,
            "seed {seed}: the doorway points away from the pad centre"
        );
        assert!(
            !terrain::slot_blocks(
                &slot,
                door_x,
                door_z,
                feet,
                CAPSULE_RADIUS_M,
                CAPSULE_HEIGHT_M
            ),
            "seed {seed}: the placed shelter's doorway is blocked at 4 m in"
        );

        // Two authored solids may not occupy the same ground. `haven_shelter`
        // picks a bearing in a gap between containers and `haven_ring_phase`
        // picks the gaps, but both were written when the shelter had no volume
        // and neither has ever been asked this question. A crate standing
        // inside a wall is a container a player can see and never open.
        for k in 0..terrain::HAVEN_CRATES {
            let (ax, az, _) = terrain::haven_crate(&haven, k);
            let (crate_r, _) = terrain::occupant_volume(Occupant::CrateSlot);
            assert!(
                !terrain::slot_blocks(&slot, ax, az, feet, crate_r, CAPSULE_HEIGHT_M),
                "seed {seed}: container {k} stands inside the shelter's walls"
            );
        }
    }
}

// ── the canopy: the other occupant whose volume is a hole, the other way ────

/// Is a body at local (lx, lz) stopped by the canopy standing at the origin
/// with the given yaw? `shelter_hits` with the other table, and the same
/// local→world rotation for the same reason: the test drives the predicate
/// through the frame change it performs, so a sign error there cannot cancel
/// itself.
fn canopy_hits(yaw: u8, lx: f32, lz: f32) -> bool {
    let mut slot = at_origin(Occupant::WaystationCanopy);
    slot.yaw = yaw;
    let (s, c) = yaw_dir((yaw as u16) << 8);
    // local +X is (c, -s), local +Z is (s, c)
    let x = lx * c + lz * s;
    let z = -lx * s + lz * c;
    terrain::slot_blocks(&slot, x, z, FEET, CAPSULE_RADIUS_M, CAPSULE_HEIGHT_M)
}

/// **A body walks clean through the canopy on three of its four sides, and
/// the parapet stops it on the fourth.**
///
/// The shelter's pair of tests inverted, which is the whole claim the second
/// box list makes: the pad's greybox is a room with one hole and this is a
/// roof with one wall. Asserting only that a body passes would be satisfied
/// by a predicate that never blocks, and asserting only the parapet by one
/// that always does — the pair is what says the shape is a canopy.
///
/// Local −Z is the parapet, because `WAYSTATION_CANOPY_BOXES`' parapet row is
/// centred at z = −2.2. The march runs between the posts on each open side:
/// the posts stand at local (±2.2, ±2.2) and are 0.36 m square, so a lane
/// down local x = 0 or z = 0 clears them by 1.84 m against a body radius the
/// assert below prints if it ever stops being enough.
#[test]
fn a_body_walks_under_the_canopy_and_the_parapet_stops_it() {
    let span = terrain::WAYSTATION_CANOPY_R_M + 2.0;
    for yaw in [0u8, 17, 64, 128, 200, 255] {
        // The three open sides: in along +Z, and across on ±X, all the way
        // through and out the other side. Nothing may touch the body.
        for (ux, uz) in [(0.0f32, 1.0f32), (1.0, 0.0), (-1.0, 0.0)] {
            let mut t = span;
            let mut blocked_at: Option<f32> = None;
            while t >= -span {
                // Stop at the parapet's own face, grown by the body — walking
                // OUT through −Z is the parapet's business, asserted next, and
                // marching into it here would fail on the wall doing its job
                // (`a_body_walks_in_through_the_door`'s first draft made
                // exactly that mistake). Derived from the parapet row and the
                // capsule, so thickening either moves this with it.
                let p = &terrain::WAYSTATION_CANOPY_BOXES[terrain::WAYSTATION_CANOPY_PARAPET_IX];
                let face = p[2] + p[5] * 0.5 + CAPSULE_RADIUS_M;
                let lz = uz * t;
                if lz < face {
                    break;
                }
                if canopy_hits(yaw, ux * t, lz) && blocked_at.is_none() {
                    blocked_at = Some(t);
                }
                t -= 0.05;
            }
            assert!(
                blocked_at.is_none(),
                "yaw {yaw}: the lane along local ({ux}, {uz}) is blocked at \
                 {:.2} m — a body {CAPSULE_RADIUS_M} m wide cannot walk under \
                 the canopy, so it is a shed and not a canopy",
                blocked_at.unwrap_or(0.0)
            );
        }
        // And the fourth side is solid: the parapet stops the same body.
        assert!(
            canopy_hits(yaw, 0.0, -2.2),
            "yaw {yaw}: the body walked out through the parapet, so the canopy \
             has no solid side at all and the silhouette is four posts"
        );
    }
}

/// The plates overhead are overhead: a body standing under them is not
/// stopped, and the same body lifted to the eave is.
///
/// `the_lintel_and_the_roof_are_overhead` for the other table. Without it
/// `OCCUPANT_TOP_M[12]` is a number that agrees with the const block and with
/// nothing a player does — the canopy's whole point is that you stand under
/// it, and its top is 4.1 m for the same reason the shelter's is 9.2.
#[test]
fn the_canopy_plates_are_overhead() {
    // Under the middle of the deck, feet on the ground: clear.
    let slot = {
        let mut s = at_origin(Occupant::WaystationCanopy);
        s.yaw = 0;
        s
    };
    assert!(
        !terrain::slot_blocks(&slot, 0.0, 0.0, FEET, CAPSULE_RADIUS_M, CAPSULE_HEIGHT_M),
        "the centre of the canopy is blocked at head height — the eave hangs \
         into the body, so nobody can shelter under it"
    );
    // The same body with its feet at the eave's underside is inside the
    // plate and must be stopped. The eave row is centred 3.15 with a 0.3
    // slab, so its underside is 3.0.
    assert!(
        terrain::slot_blocks(&slot, 0.0, 0.0, 3.0, CAPSULE_RADIUS_M, CAPSULE_HEIGHT_M),
        "a body at the eave's underside is not stopped — the roof is not solid, \
         so the box list draws a plate the sim does not have"
    );
    // And above the finial nothing blocks, which is what OCCUPANT_TOP_M says.
    assert!(
        !terrain::slot_blocks(
            &slot,
            0.0,
            0.0,
            terrain::WAYSTATION_CANOPY_PEAK_M + 0.01,
            CAPSULE_RADIUS_M,
            CAPSULE_HEIGHT_M
        ),
        "the canopy blocks above its own published peak"
    );
}

/// The placed canopy is solid where it stands, faces its site, and shares no
/// ground with either cache.
///
/// `the_placed_shelter_is_solid_and_faces_its_door_at_the_pad` for the lesser
/// tier, and the same reason it exists: every test above stands the structure
/// at the origin at an authored yaw, and none of them asks whether the thing
/// worldgen actually placed is that structure. Two authored solids sharing
/// ground is a cache a player can see and never reach.
#[test]
fn the_placed_canopy_is_solid_and_clears_its_caches_at_every_site() {
    for seed in [1u64, 42, 20_260_804, 0xDEAD_BEEF] {
        let haven = terrain::haven(seed);
        for w in 0..terrain::WAYSTATIONS {
            let ws = haven.minor[w];
            assert!(ws.live, "seed {seed:#x}: site {w} is not live");
            let (kx, kz, kyaw) = terrain::waystation_canopy(&ws);
            let feet = terrain::height(seed, kx, kz);
            let slot = Slot {
                occupant: Occupant::WaystationCanopy,
                x: kx,
                y: feet,
                z: kz,
                yaw: kyaw,
                scale: 1.0,
            };
            // A post is where a post should be: the body at a post's local
            // corner is stopped, which is the cheapest proof the frame the
            // slot carries is the frame the boxes are read in.
            let (s, c) = yaw_dir((kyaw as u16) << 8);
            let (px, pz) = (2.2f32, 2.2f32);
            let wx = kx + px * c + pz * s;
            let wz = kz - px * s + pz * c;
            assert!(
                terrain::slot_blocks(&slot, wx, wz, feet, CAPSULE_RADIUS_M, CAPSULE_HEIGHT_M),
                "seed {seed:#x} site {w}: the placed canopy has no post at its \
                 own local (2.2, 2.2) — the yaw frame is not the frame the \
                 boxes are in"
            );
            // The caches stand outside it. This is the assert the first draft
            // of this structure could not have passed: it stood at the site
            // centre, 6.5 m from each cache, and the eave reached 2.8 m of
            // that on the diagonal.
            let (cache_r, _) = terrain::occupant_volume(Occupant::CacheSlot);
            for k in 0..terrain::WAYSTATION_CRATES {
                let (ax, az, _) = terrain::waystation_crate(&ws, k);
                assert!(
                    !terrain::slot_blocks(&slot, ax, az, feet, cache_r, CAPSULE_HEIGHT_M),
                    "seed {seed:#x} site {w}: cache {k} stands inside the canopy"
                );
            }
        }
    }
}
