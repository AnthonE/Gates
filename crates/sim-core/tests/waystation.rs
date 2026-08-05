//! The lesser tier of destination, gated as arithmetic.
//!
//! `TERRAIN.md`'s numbers table read `monuments | 0 — haven pad only`, and both
//! judge reports of 2026-08-05 named the consequence rather than the count:
//! "there is one place on the island worth walking to". `terrain::WAYSTATIONS`
//! lesser sites answer that, and this file is what holds them to it.
//!
//! Same standard as `tests/haven.rs` and `tests/road.rs`, and the same reason:
//! counts, distances, separations and densities are numbers, and a number that
//! fails in two seconds beats a pixel that fails in nineteen minutes. Nothing
//! here reads a frame.
//!
//! The load-bearing test is `the_tiers_pay_in_order` — a destination that is
//! not the best place on the island is not a destination, and that is the one
//! property no count on its own can see.

// The measurements ARE this gate's output — same reasoning and same allow as
// `tests/haven.rs` and `tests/scatter.rs`: the L5 wall bans format/print in
// SIM code, and a test harness is not sim code. The float walls are NOT
// relaxed: `abs` stays banned here as everywhere, so every magnitude below is
// `max` of a signed pair.
#![allow(clippy::disallowed_macros)]

use sim_core::terrain::{
    self, Occupant, RoadBand, ScatterTable, Waystation, CELLS_PER_SIDE, CELL_SIZE,
    CLIFF_SLOPE_RATIO, HAVEN_CRATES, HAVEN_RADIUS_M, LAND_MIN_H, ROAD_R_MAX, ROAD_R_MIN,
    WAYSTATIONS, WAYSTATION_CRATES, WAYSTATION_CRATE_R_M, WAYSTATION_MIN_SEP_M,
    WAYSTATION_RADIUS_M,
};

/// The same sixteen `tests/haven.rs` sweeps, so a seed that is interesting to
/// one tier is interesting to both — 555555 in particular, the shore shelf
/// whose whole 10 m ring sits under the land line and which the pad refuses.
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

/// The four `tests/haven.rs` uses for its 65,536-cell island sweeps.
const SWEEP_SEEDS: [u64; 4] = [1, 42, 20_260_804, 0xDEAD_BEEF];

fn cell_of(x: f32, z: f32) -> (i32, i32) {
    (
        (x * (1.0 / CELL_SIZE)) as i32,
        (z * (1.0 / CELL_SIZE)) as i32,
    )
}

fn dist(ax: f32, az: f32, bx: f32, bz: f32) -> f32 {
    let dx = ax - bx;
    let dz = az - bz;
    (dx * dx + dz * dz).sqrt()
}

/// The tier is resolved, complete, and the same on every call.
///
/// "Complete" is the assert with teeth. `pick_minor` breaks out and leaves
/// `Waystation::NONE` behind when the ring has no room, which is the right
/// behaviour and the wrong thing to ship silently — a seed that quietly
/// generated one site instead of two would look exactly like a working island
/// until someone counted. So a short tier is a FAILURE here, not a shrug.
#[test]
fn every_seed_gets_a_full_tier() {
    let mut sites = Vec::new();
    for seed in SEEDS {
        let a = terrain::haven(seed);
        let b = terrain::haven(seed);
        assert_eq!(
            a.minor, b.minor,
            "seed {seed}: two calls disagreed — the tier is a pure function \
             of the seed or it is not replayable"
        );

        let live = a.minor.iter().filter(|w| w.live).count();
        assert_eq!(
            live, WAYSTATIONS,
            "seed {seed}: {live} of {WAYSTATIONS} waystations placed. The \
             search breaks out when no candidate clears \
             WAYSTATION_MIN_SEP_M = {WAYSTATION_MIN_SEP_M} m and a valid ring \
             phase; a short tier means the floor is too high for this \
             seed's ring, not that fewer sites are acceptable"
        );

        for w in a.minor.iter().filter(|w| w.live) {
            assert!(
                w.x.is_finite() && w.z.is_finite() && w.y.is_finite(),
                "seed {seed}: a waystation carries a non-finite coordinate"
            );
            sites.push((w.x, w.z));
        }
    }
    // Sixteen seeds, two sites each, no two in the same place: the tier is
    // seed-dependent and not a constant that happens to be on the ring.
    let mut distinct = 0usize;
    for (i, a) in sites.iter().enumerate() {
        if !sites[..i].iter().any(|b| b == a) {
            distinct += 1;
        }
    }
    assert_eq!(
        distinct,
        sites.len(),
        "{} of {} sites are duplicates — the tier is not seed-dependent",
        sites.len() - distinct,
        sites.len()
    );
}

/// Every site stands on the road ring it is reached by, on ground that can
/// hold it. The pad's own `the_pad_stands_on_the_road_it_terminates`, applied
/// one tier down — a waystation off the ring is a thing in a forest, and the
/// entire point of the tier is that the circulation loop has beads on it.
#[test]
fn every_site_stands_on_the_ring() {
    let c = terrain::ISLAND_SIZE * 0.5;
    let mut worst_y = f32::MAX;
    let mut worst_y_seed = 0u64;

    for seed in SEEDS {
        let haven = terrain::haven(seed);
        for w in haven.minor.iter().filter(|w| w.live) {
            assert_ne!(
                terrain::road_band(seed, w.x, w.z),
                terrain::RoadBand::Off,
                "seed {seed}: a waystation stands off the road entirely"
            );
            let r = dist(w.x, w.z, c, c);
            assert!(
                (ROAD_R_MIN..=ROAD_R_MAX).contains(&r),
                "seed {seed}: a waystation sits at radius {r:.1} m, outside \
                 the ring's own bracket {ROAD_R_MIN}..{ROAD_R_MAX}"
            );
            assert!(
                w.y >= LAND_MIN_H,
                "seed {seed}: a waystation stands at y {:.2}, below the land \
                 line {LAND_MIN_H}",
                w.y
            );
            assert_eq!(
                w.y,
                terrain::height(seed, w.x, w.z),
                "seed {seed}: a waystation's y is not the terrain under it"
            );
            if w.y < worst_y {
                worst_y = w.y;
                worst_y_seed = seed;
            }
        }
    }
    println!("lowest waystation: y {worst_y:.2} m (seed {worst_y_seed})");
}

/// The sites are separate PLACES, not a cluster with two names.
///
/// Every pair — pad to waystation and waystation to waystation — clears
/// `WAYSTATION_MIN_SEP_M`. This is what makes the tier a circulation loop
/// rather than a second ring of crates around the same destination, and it is
/// also what makes the "belongs to exactly one site" partition in
/// `tests/haven.rs` true by construction rather than by luck.
#[test]
fn the_sites_are_separate_places() {
    let mut worst = f32::MAX;
    let mut worst_seed = 0u64;

    for seed in SEEDS {
        let haven = terrain::haven(seed);
        let live: Vec<&Waystation> = haven.minor.iter().filter(|w| w.live).collect();

        for (i, w) in live.iter().enumerate() {
            let d = dist(w.x, w.z, haven.x, haven.z);
            assert!(
                d >= WAYSTATION_MIN_SEP_M,
                "seed {seed}: a waystation is {d:.0} m from the pad, inside \
                 the {WAYSTATION_MIN_SEP_M} m floor"
            );
            worst = worst.min(d);
            if worst == d {
                worst_seed = seed;
            }
            for other in &live[..i] {
                let e = dist(w.x, w.z, other.x, other.z);
                assert!(
                    e >= WAYSTATION_MIN_SEP_M,
                    "seed {seed}: two waystations are {e:.0} m apart, inside \
                     the {WAYSTATION_MIN_SEP_M} m floor"
                );
                worst = worst.min(e);
                if worst == e {
                    worst_seed = seed;
                }
            }
        }
    }
    println!(
        "closest pair of sites over {} seeds: {worst:.0} m (seed {worst_seed}), \
         floor {WAYSTATION_MIN_SEP_M} m",
        SEEDS.len()
    );
}

/// Every site carries the containers it placed, and the road stays walkable
/// past it.
///
/// The island sweep, so this counts what `scatter` actually yields rather than
/// what `waystation_crate` claims. The count assert is the silent-drop guard
/// the pad's carries for the same reason: one scatter cell holds one slot, so
/// two anchors in one cell lose one without a word, and nothing else in the
/// build would notice.
#[test]
fn every_site_carries_its_containers() {
    let table = ScatterTable::alpha_default();
    let cell_diag = (CELL_SIZE * CELL_SIZE * 2.0).sqrt();
    let mut worst_sep = f32::MAX;
    let mut worst_ring_err: f32 = 0.0;
    let mut worst_dot: f32 = 1.0;

    for seed in SWEEP_SEEDS {
        let haven = terrain::haven(seed);
        let live = haven.minor.iter().filter(|w| w.live).count();
        let mut found = 0usize;

        for cx in 0..CELLS_PER_SIDE {
            for cz in 0..CELLS_PER_SIDE {
                let s = terrain::scatter(seed, &table, &haven, cx, cz);
                // `CacheSlot`, the lesser tier's OWN container kind. It was
                // the pad's `CrateSlot` until 2026-08-05, and while it was,
                // this sweep could not tell the two tiers apart: a pad crate
                // that somehow landed inside a waystation zone would have
                // counted here as a waystation's own. The kind is what
                // selects the loot table, so counting it is counting the
                // price the site pays.
                if s.occupant != Occupant::CacheSlot || !terrain::in_waystation(&haven, s.x, s.z) {
                    continue;
                }
                found += 1;
                assert!(
                    s.y >= LAND_MIN_H,
                    "seed {seed}: a waystation crate stands at y {:.2}, below \
                     the land line. `waystation_ring_phase` tests every anchor \
                     against it, so this says the check chain was skipped, not \
                     that the site is marginal",
                    s.y
                );
                assert_eq!(
                    s.y,
                    terrain::height(seed, s.x, s.z),
                    "seed {seed}: a waystation crate's y is not the terrain \
                     under it"
                );
                // Placed, not drawn: no jitter, no size wobble.
                assert_eq!(
                    s.scale, 1.0,
                    "seed {seed}: a placed waystation crate was scaled"
                );
                assert_ne!(
                    terrain::road_band(seed, s.x, s.z),
                    terrain::RoadBand::Carriageway,
                    "seed {seed}: a waystation crate stands on the \
                     carriageway — the ring rotation search is what prevents \
                     this, and `tests/road.rs` requires the surface clear"
                );
            }
        }
        assert_eq!(
            found,
            live * WAYSTATION_CRATES as usize,
            "seed {seed}: {found} cache(s) stand across {live} waystation(s) \
             against {WAYSTATION_CRATES} apiece. Fewer means two anchors \
             landed in one scatter cell and the second was dropped without a \
             word — widen WAYSTATION_CRATE_R_M until the separation below \
             clears one cell diagonal"
        );
    }

    // The ring geometry itself, on all sixteen seeds: pure arithmetic off
    // `waystation_crate`, so it costs no sweep.
    for seed in SEEDS {
        let haven = terrain::haven(seed);
        for w in haven.minor.iter().filter(|w| w.live) {
            let mut pts = [(0.0f32, 0.0f32); 256];
            for k in 0..WAYSTATION_CRATES {
                let (ax, az, yaw) = terrain::waystation_crate(w, k);
                pts[k as usize] = (ax, az);

                // On the ring, to the millimetre.
                // Magnitude as `max` of the signed pair — `abs` is walled.
                let d = dist(ax, az, w.x, w.z);
                let err = (d - WAYSTATION_CRATE_R_M).max(WAYSTATION_CRATE_R_M - d);
                worst_ring_err = worst_ring_err.max(err);
                assert!(
                    err < 0.01,
                    "seed {seed}: an anchor is {err:.3} m off the \
                     {WAYSTATION_CRATE_R_M} m ring"
                );

                // Facing IN, the one convention both tiers use. Asserted as an
                // ANGLE and not as a number, because the shelter's yaw once
                // shipped wrong with the right arity, the right field and the
                // right type — see `terrain::haven_shelter`.
                let (sy, cy) = sim_core::yaw_dir((yaw as u16) << 8);
                let (ix, iz) = (w.x - ax, w.z - az);
                let inv = 1.0 / (ix * ix + iz * iz).sqrt();
                let dot = sy * ix * inv + cy * iz * inv;
                worst_dot = worst_dot.min(dot);
                assert!(
                    dot > 0.99,
                    "seed {seed}: an anchor faces {dot:.3} away from its own \
                     site centre — the yaw is the INWARD facing"
                );
            }
            // No two anchors in one cell, measured rather than argued. The
            // const block proves `2 * R > 1.5 * CELL_SIZE`; this is the same
            // claim against the real diagonal and the real positions.
            for i in 0..WAYSTATION_CRATES as usize {
                for j in 0..i {
                    let d = dist(pts[i].0, pts[i].1, pts[j].0, pts[j].1);
                    worst_sep = worst_sep.min(d);
                    assert!(
                        d > cell_diag,
                        "seed {seed}: two anchors are {d:.2} m apart against a \
                         {cell_diag:.2} m cell diagonal — they can share a cell \
                         and one would be dropped silently"
                    );
                    assert_ne!(
                        cell_of(pts[i].0, pts[i].1),
                        cell_of(pts[j].0, pts[j].1),
                        "seed {seed}: two anchors resolved to one scatter cell"
                    );
                }
            }
        }
    }
    println!(
        "ring: worst radius error {worst_ring_err:.4} m, worst inward dot \
         {worst_dot:.4}, closest anchors {worst_sep:.2} m against a \
         {cell_diag:.2} m cell diagonal"
    );
}

/// The exclusion zone is clear, and it would not have been.
///
/// The control arm is the half that matters: an empty zone proves nothing on
/// its own, because a site could have landed on bare rock. Running the same
/// sweep against a haven whose tier is switched off shows what the zone
/// actually displaced, so "cleared" is a measurement and not an assumption.
/// Straight from `tests/haven.rs::the_pad_is_clear_and_would_not_have_been`.
///
/// **Three claims, at the three scales each is true at.** The control's
/// per-seed form — every seed's zones displace at least one slot — was luck
/// rather than law, and this pass caught it holding a site in place: a
/// waystation zone is 380 m² of roadside, the carriageway inside it is vetoed
/// and the shoulder is thin, so zero is an ordinary draw. It read 3, 3, 3, 0
/// across the sweep once the canopy search moved seed `0xDEADBEEF`'s sites
/// onto emptier ground, and nothing about that is a defect.
///
/// So `cleared` is asserted over the SWEEP — the same one-per-seed claim, at
/// the scale it is true at — and the two things it was standing in for are
/// asked exactly, per seed, instead:
///
///   - `furnished`: every seed finds `WAYSTATIONS * (WAYSTATION_CRATES + 1)`
///     authored slots inside the zones, its caches and its canopy and nothing
///     else. A broken `in_waystation`, a dead site, a dropped anchor or a
///     canopy that stopped being emitted all read as a wrong count here.
///   - `opportunity`: the control arm HAD a chance to put something in the
///     zone — the zone covers at least one scatter cell whose ground the
///     control would not have vetoed outright. This is the half a bare
///     `cleared >= 1` was reaching for and could not state: it separates "the
///     zone displaced nothing because the exclusion is broken" from "the zone
///     displaced nothing because it is 380 m² of road and cliff", and unlike
///     the count it cannot be moved by a seed landing on bare ground.
///
/// `opportunity` is measured at the cell CENTRE while `scatter` draws at a
/// jittered point inside the cell, so it is a floor on the real opportunity
/// and never an overstatement of it.
#[test]
fn the_zones_are_clear_and_would_not_have_been() {
    let table = ScatterTable::alpha_default();
    let mut total_cleared = 0usize;
    let mut worst_opportunity = usize::MAX;
    let want_furnished = WAYSTATIONS * (WAYSTATION_CRATES as usize + 1);

    for seed in SWEEP_SEEDS {
        let haven = terrain::haven(seed);
        // Same pad, same everything, no lesser tier — so the only difference
        // between the two sweeps is the thing under test.
        let mut control = haven;
        control.minor = [Waystation::NONE; WAYSTATIONS];

        let mut inside = 0usize;
        let mut cleared = 0usize;
        let mut furnished = 0usize;
        let mut opportunity = 0usize;
        for cx in 0..CELLS_PER_SIDE {
            for cz in 0..CELLS_PER_SIDE {
                // Did the control arm have a chance? A cell is an opportunity
                // when its centre lies in a zone AND the ground there is not
                // something `scatter` refuses on sight — `terrain.rs`'s land,
                // cliff and carriageway vetoes, which run before any biome
                // draw. `in_waystation` is tested first and short-circuits, so
                // the two `height` fans cost nothing on the 65,000 cells that
                // are nowhere near a site.
                let (ccx, ccz) = (
                    cx as f32 * CELL_SIZE + CELL_SIZE * 0.5,
                    cz as f32 * CELL_SIZE + CELL_SIZE * 0.5,
                );
                if terrain::in_waystation(&haven, ccx, ccz)
                    && terrain::height(seed, ccx, ccz) >= LAND_MIN_H
                    && terrain::slope(seed, ccx, ccz) <= CLIFF_SLOPE_RATIO
                    && terrain::road_band(seed, ccx, ccz) != RoadBand::Carriageway
                {
                    opportunity += 1;
                }
                let s = terrain::scatter(seed, &table, &haven, cx, cz);
                // The site's own furniture — its two `CacheSlot` containers
                // and its one `WaystationCanopy` — is the only thing allowed
                // inside the zone, the same pair of exemptions
                // `tests/haven.rs` grants `CrateSlot` and `HavenShelter`.
                // `CrateSlot` is deliberately NOT exempted: the pad's kind
                // standing in a waystation would be the lesser tier drawing
                // `loot.crate`, which is the defect the two kinds were split
                // to remove, so it must count as an intruder here rather
                // than be waved past.
                if s.occupant != Occupant::None && terrain::in_waystation(&haven, s.x, s.z) {
                    if s.occupant == Occupant::CacheSlot || s.occupant == Occupant::WaystationCanopy
                    {
                        furnished += 1;
                    } else {
                        inside += 1;
                    }
                }
                let u = terrain::scatter(seed, &table, &control, cx, cz);
                if u.occupant != Occupant::None && terrain::in_waystation(&haven, u.x, u.z) {
                    cleared += 1;
                }
            }
        }
        assert_eq!(
            inside, 0,
            "seed {seed}: {inside} scattered slot(s) stand inside a waystation \
             zone — the exclusion is meant to run ahead of the biome draw"
        );
        assert_eq!(
            furnished, want_furnished,
            "seed {seed}: {furnished} authored slot(s) inside the zones against \
             {want_furnished} — every live site owes {WAYSTATION_CRATES} caches \
             and one canopy, so a wrong count here is a dead site, a dropped \
             anchor, or a zone predicate that is not on the site at all"
        );
        assert!(
            opportunity >= 1,
            "seed {seed}: the zones cover no scatter cell the control arm would \
             have drawn on, so this seed's control is vacuous and its 'cleared' \
             count means nothing either way. Either the zones are off the grid, \
             `in_waystation` is not on the site, or both sites stand on ground \
             scatter refuses outright"
        );
        worst_opportunity = worst_opportunity.min(opportunity);
        total_cleared += cleared;
    }
    assert!(
        total_cleared >= SWEEP_SEEDS.len(),
        "the zones displaced {total_cleared} slot(s) across {} seeds, so \
         'cleared' is not evidence of anything and the control arm is not a \
         control",
        SWEEP_SEEDS.len()
    );
    println!(
        "waystation zones displaced {total_cleared} slot(s) across {} seeds, \
         furnished {want_furnished} per seed, worst per-seed control \
         opportunity {worst_opportunity} cell(s)",
        SWEEP_SEEDS.len()
    );
}

/// **The tiers pay in order: pad > waystation > road shoulder.**
///
/// The one property that makes this a TIER rather than more crates. A
/// destination that is not the best place on the island is not a destination,
/// and the failure this catches is silent everywhere else: every count can be
/// right, every site on the ring, every container standing, and the haven
/// still be the second-best square metre on the map. That is not hypothetical
/// — at `WAYSTATION_RADIUS_M = 10.0` it was exactly true, two crates in 314 m²
/// beating five in 804 m², and only writing this assert found it.
///
/// Units are containers per square metre, the same units
/// `tests/haven.rs::the_pad_outpays_the_road_that_leads_to_it` states the
/// pad's 2.30× edge in, so all three tiers are one comparable number.
#[test]
fn the_tiers_pay_in_order() {
    const PI: f32 = std::f32::consts::PI;
    let table = ScatterTable::alpha_default();

    let pad_density = HAVEN_CRATES as f32 / (PI * HAVEN_RADIUS_M * HAVEN_RADIUS_M);
    let site_density = WAYSTATION_CRATES as f32 / (PI * WAYSTATION_RADIUS_M * WAYSTATION_RADIUS_M);

    // The aggregate rule, restated where a reader will see it: the whole
    // lesser tier put together still pays less than the one destination.
    assert!(
        WAYSTATIONS as i32 * WAYSTATION_CRATES < HAVEN_CRATES,
        "the lesser tier holds {} containers against the pad's {HAVEN_CRATES} \
         — added up it now outpays the destination it is subordinate to",
        WAYSTATIONS as i32 * WAYSTATION_CRATES
    );
    assert!(
        pad_density > site_density,
        "the pad pays {pad_density:.5} containers/m² against a waystation's \
         {site_density:.5} — the lesser tier is the better square metre, so a \
         player optimizing loot-per-walk skips the haven"
    );

    let mut worst_site_over_road = f32::MAX;
    let mut worst_seed = 0u64;

    for seed in SWEEP_SEEDS {
        let haven = terrain::haven(seed);
        let mut shoulder_cells = 0usize;
        let mut shoulder_barrels = 0usize;
        for cx in 0..CELLS_PER_SIDE {
            for cz in 0..CELLS_PER_SIDE {
                let x = cx as f32 * CELL_SIZE + 4.0;
                let z = cz as f32 * CELL_SIZE + 4.0;
                if terrain::road_band(seed, x, z) != terrain::RoadBand::Shoulder {
                    continue;
                }
                shoulder_cells += 1;
                let s = terrain::scatter(seed, &table, &haven, cx, cz);
                if s.occupant == Occupant::BarrelSlot {
                    shoulder_barrels += 1;
                }
            }
        }
        assert!(
            shoulder_barrels > 0,
            "seed {seed}: the road drew no barrels at all, so there is no \
             baseline to be a tier above"
        );
        let road_density =
            shoulder_barrels as f32 / (shoulder_cells as f32 * CELL_SIZE * CELL_SIZE);
        let ratio = site_density / road_density;
        if ratio < worst_site_over_road {
            worst_site_over_road = ratio;
            worst_seed = seed;
        }
        assert!(
            ratio > 1.0,
            "seed {seed}: a waystation pays {site_density:.5} containers/m² \
             against the road shoulder's {road_density:.5} — the middle tier \
             is not above the route it stands on, so the walk buys nothing"
        );
    }

    println!(
        "gradient: pad {:.5} > waystation {:.5} > shoulder (worst site/road \
         ratio {worst_site_over_road:.2}× on seed {worst_seed}); pad/site \
         {:.2}×",
        pad_density,
        site_density,
        pad_density / site_density
    );
}

/// **Every site carries its canopy, and the canopy is not in the road.**
///
/// The failure this exists for is measured, not hypothetical. The first draft
/// stood the canopy at the site centre on the argument that a two-anchor site
/// needs a middle; the centre of a waystation is the centre line of the coast
/// road, because every candidate `pick_minor` scores comes off the road ring,
/// and `tests/road.rs` read 2 slots on the carriageway at every seed probed.
/// Three suites went red together and none of them names the canopy, so this
/// is the one that does.
///
/// Four claims per site, all arithmetic: the canopy exists and stands at the
/// offset and bearing the site carries; its whole footprint is off the
/// carriageway, sampled at the two extremes across the road's width rather
/// than at its anchor alone; it is inside the exclusion zone that keeps
/// scatter off it; and it holds a scatter cell neither cache holds, because a
/// cell carries one occupant and a collision would silently delete one of the
/// two.
#[test]
fn every_site_carries_its_canopy_clear_of_the_road() {
    let c = terrain::ISLAND_SIZE * 0.5;
    let mut worst_edge = f32::MAX;

    for seed in SEEDS {
        let haven = terrain::haven(seed);
        for w in 0..WAYSTATIONS {
            let ws = haven.minor[w];
            if !ws.live {
                continue;
            }
            let (kx, kz, kyaw) = terrain::waystation_canopy(&ws);

            // It stands where the site says it does: `WAYSTATION_CANOPY_OFF_M`
            // out on the carried bearing, facing back in. The facing is
            // asserted as an ANGLE — the dot of the facing against the inward
            // direction — and not as a number, because `haven_shelter` shipped
            // an outward bearing into a facing field once and every number in
            // it was the right type.
            let dx = kx - ws.x;
            let dz = kz - ws.z;
            let r = (dx * dx + dz * dz).sqrt();
            let off = terrain::WAYSTATION_CANOPY_OFF_M;
            assert!(
                (r - off).max(off - r) < 1.0e-3,
                "seed {seed}: site {w}'s canopy stands {r:.3} m out against \
                 {off:.3} m"
            );
            let (fx, fz) = sim_core::yaw_dir((kyaw as u16) << 8);
            let dot = (fx * -dx + fz * -dz) / r;
            assert!(
                dot > 0.99,
                "seed {seed}: site {w}'s canopy faces {dot:.3} of the way back \
                 at its site — the yaw is an outward bearing in a facing field"
            );

            // Off the carriageway across the road's whole width. The road is
            // a ring about the island centre, so its normal at any point is
            // the radial; the extremes of the footprint across it are the
            // anchor ± the bounding radius along that vector.
            let (rx, rz) = (kx - c, kz - c);
            let d = (rx * rx + rz * rz).sqrt();
            let (ux, uz) = (rx / d, rz / d);
            let e = terrain::WAYSTATION_CANOPY_R_M;
            for (sx, sz) in [
                (kx, kz),
                (kx + ux * e, kz + uz * e),
                (kx - ux * e, kz - uz * e),
            ] {
                assert_ne!(
                    terrain::road_band(seed, sx, sz),
                    terrain::RoadBand::Carriageway,
                    "seed {seed}: site {w}'s canopy footprint touches the \
                     carriageway — the road surface is not clear, so the \
                     circulation loop is obstructed at the place built to \
                     draw players onto it"
                );
            }
            assert!(
                terrain::height(seed, kx, kz) >= LAND_MIN_H,
                "seed {seed}: site {w}'s canopy stands in the water"
            );

            // Inside the zone that keeps scatter off it — whole, not just its
            // anchor. This is the const block's `OFF + R < RADIUS` measured on
            // the shipped site rather than asserted about the constants.
            let edge = WAYSTATION_RADIUS_M - (r + e);
            assert!(
                edge > 0.0,
                "seed {seed}: site {w}'s canopy overhangs its exclusion zone \
                 by {:.2} m, so ordinary scatter grows through the deck",
                -edge
            );
            worst_edge = worst_edge.min(edge);

            // One cell, one occupant.
            let kcell = cell_of(kx, kz);
            for k in 0..WAYSTATION_CRATES {
                let (ax, az, _) = terrain::waystation_crate(&ws, k);
                assert_ne!(
                    cell_of(ax, az),
                    kcell,
                    "seed {seed}: site {w}'s canopy and cache {k} resolved to \
                     one scatter cell, so one of the two is silently gone"
                );
            }
        }
    }
    println!(
        "canopies: worst zone margin {worst_edge:.2} m, all clear of the \
         carriageway at both footprint extremes"
    );
}
