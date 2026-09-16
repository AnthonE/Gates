//! Gate: the site roster is a mechanism, and landing it moved nothing.
//!
//! `reference/MONUMENTS.md` §9.3 names three things a third kind of authored
//! place needs — a pairwise separation rule, a reservation ledger, and an
//! explicit tier — and says of what we had: *"That is correct at two and is
//! §1's starvation shape at five."* `SiteKind`, `SITE_SEP_M` and `SiteLedger`
//! are those three. This file is what says they cost nothing on the way in.
//!
//! ⚠ **The bit-equality below is the proof, not the terrain golden.** The
//! golden would only notice a separation change if some real pair on some
//! gate seed happened to sit within an ulp of 600 m of the floor, and nothing
//! guarantees that — a digest that did not move is evidence about the seeds
//! it hashed, where `assert_eq!` on `to_bits()` is evidence about the table.
//! Both are run; only one of them is a proof.

// The measurements are this file's output, as in `tests/forest.rs`: the L5
// wall bans format/print in SIM code and a test harness is not sim code.
#![allow(clippy::disallowed_macros)]

use sim_core::terrain::{
    self, SiteKind, SiteLedger, INLAND_SITES, ISLAND_SIZE, MAX_SITES, SITE_KINDS, SITE_SEP_M,
    WAYSTATIONS, WAYSTATION_MIN_SEP_M,
};

/// A millimetre, `tests/carve.rs`'s tolerance and its reason: the claims
/// below are PHYSICAL — "this site is 300 m from the ring" — and the site's
/// radius is recovered from a position built as `centre + dir * r`, so it
/// comes back one ulp heavy. Measured over the sweep: the worst overshoot is
/// **61 microns** (seed 31337, one ulp of 300 m), which is sixteen times
/// under this bound and sixteen thousand times under anything this repo
/// measures in metres. A bit-exact compare here would be a gate that fails
/// on rounding.
const MM: f32 = 1.0e-3;

/// The sixteen `tests/haven.rs` sweeps, so a seed interesting to the pad is
/// interesting to the roster.
const SWEEP: [u64; 16] = [
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

/// Every pair the shipped tree actually places must be floored at exactly the
/// constant that floored it before the table existed.
///
/// **`to_bits()`, not `==`**, for the reason `client/tests/ground.rs` gives
/// about its own comparisons: a float equality that passes on a value one ulp
/// away is a gate that admits the change it was written to refuse, and one
/// ulp of 600 m is 6.1e-5 m — far under any distance this repo measures, and
/// exactly the size of edit that would slip through a tolerance.
///
/// **Mutant, run, and the result is the argument for this file.** Moving
/// EVERY entry of `SITE_SEP_M` by one ulp — 600.0 to 600.00006, 61 microns on
/// a 600 m floor — reddens this test and is invisible to **twenty** others:
/// `terrain_golden` (9), `replay` (1), `haven` (3) and `waystation` (7) all
/// pass. Not one of them is a bad gate; they hash and measure where sites
/// LANDED, and a 61-micron change to the floor moves no site on any seed
/// anyone has run. That is exactly why the rule needs a gate of its own.
#[test]
fn the_shipped_pairs_are_floored_at_the_constant_they_always_were() {
    let want = WAYSTATION_MIN_SEP_M.to_bits();
    for (a, b, why) in [
        (
            SiteKind::Haven,
            SiteKind::Waystation,
            "the pad to a waystation",
        ),
        (
            SiteKind::Waystation,
            SiteKind::Haven,
            "a waystation to the pad",
        ),
        (
            SiteKind::Waystation,
            SiteKind::Waystation,
            "one waystation to another",
        ),
    ] {
        let got = SITE_SEP_M[a as usize][b as usize];
        assert_eq!(
            got.to_bits(),
            want,
            "{why} is floored at {got} where it was floored at \
             {WAYSTATION_MIN_SEP_M} before the roster existed. The table is \
             supposed to START as the constant it replaces, so the mechanism \
             lands without moving a site — a value here that is merely CLOSE \
             has already moved one."
        );
    }
    println!(
        "site roster: {SITE_KINDS} kinds, {MAX_SITES} slots, shipped pairs all at \
         {WAYSTATION_MIN_SEP_M} m"
    );
}

/// The table is symmetric and satisfiable. A const block asserts the same
/// thing at compile time; this runs it over the whole table and PRINTS it, so
/// a reader of the suite sees what is actually shipped rather than trusting
/// that a `const _` somewhere agreed.
#[test]
fn the_separation_table_is_symmetric_and_satisfiable() {
    let kinds = [SiteKind::Haven, SiteKind::Waystation, SiteKind::Inland];
    for (a, row) in SITE_SEP_M.iter().enumerate() {
        for (b, sep) in row.iter().enumerate() {
            assert_eq!(
                sep.to_bits(),
                SITE_SEP_M[b][a].to_bits(),
                "SITE_SEP_M[{a}][{b}] and [{b}][{a}] disagree — \"A is far \
                 enough from B\" and \"B is far enough from A\" are the same \
                 sentence, and a table that disagrees with itself makes the \
                 answer depend on which site was placed first"
            );
            assert!(
                *sep > 0.0 && *sep < ISLAND_SIZE,
                "SITE_SEP_M[{a}][{b}] is {sep} — a floor at or under zero is \
                 no floor, and one past the island's own diagonal can never \
                 be met by any pair at all"
            );
            assert!(
                *sep > terrain::site_footprint(kinds[a]).blend_m
                    + terrain::site_footprint(kinds[b]).blend_m,
                "site tiers {a} and {b} can overlap their graded ground"
            );
        }
    }
    println!("separation table (m): {SITE_SEP_M:?}");
}

/// The ledger refuses past its cap and says so — wall 4's stated overflow
/// policy, which for this path is REFUSE rather than drop.
///
/// A site that vanished without a word is the failure mode
/// `tests/waystation.rs` exists to make audible ("a short tier is a finding
/// rather than a silent degradation"), so the ledger must not introduce a
/// quieter version of it one layer down.
#[test]
fn the_ledger_refuses_past_its_cap_rather_than_dropping() {
    let mut led = SiteLedger::new();
    assert!(led.is_empty());
    // Spread them far enough apart that nothing is refused for separation —
    // this test is about the CAP, and a site refused for the other reason
    // would make it pass for the wrong one.
    let step = ISLAND_SIZE / (MAX_SITES + 2) as f32;
    for i in 0..MAX_SITES {
        assert!(
            led.take(SiteKind::Waystation, i as f32 * step, 0.0),
            "the ledger refused entry {i} of its stated {MAX_SITES}"
        );
    }
    assert_eq!(led.len(), MAX_SITES);
    assert!(
        !led.take(SiteKind::Waystation, ISLAND_SIZE, ISLAND_SIZE),
        "the ledger accepted a {}th site into {MAX_SITES} slots",
        MAX_SITES + 1
    );
    assert_eq!(led.len(), MAX_SITES, "a refused take still grew the roster");
    println!("ledger: {MAX_SITES} slots, the {}th refused", MAX_SITES + 1);
}

/// `clears` is the inline pairwise test `pick_minor` used to carry, and this
/// rebuilds that test from the published table rather than calling the thing
/// under test.
///
/// **Non-vacuous by construction**, which is the half that matters: a roster
/// with one entry is asked about a point inside its floor AND a point outside
/// it, and both answers are asserted. `CLAUDE.md`'s lattice entry is about a
/// gate that agreed with itself on cases that never reached the feature.
#[test]
fn clears_is_the_pairwise_rule_and_it_actually_refuses() {
    let mut led = SiteLedger::new();
    led.take(SiteKind::Haven, 1000.0, 1000.0);
    let floor = SITE_SEP_M[SiteKind::Waystation as usize][SiteKind::Haven as usize];

    // Just inside the floor: refused.
    let inside = 1000.0 + floor * 0.5;
    assert!(
        !led.clears(SiteKind::Waystation, inside, 1000.0),
        "a waystation {:.0} m from the pad cleared a {floor} m floor",
        inside - 1000.0
    );
    // Comfortably outside: allowed.
    let outside = 1000.0 + floor * 1.5;
    assert!(
        led.clears(SiteKind::Waystation, outside, 1000.0),
        "a waystation {:.0} m from the pad was refused by a {floor} m floor",
        outside - 1000.0
    );
    // And the rule is a DISTANCE, not an axis: the same radius on the
    // diagonal has to answer the same way.
    let diag = floor * 1.5 / core::f32::consts::SQRT_2;
    assert!(
        led.clears(SiteKind::Waystation, 1000.0 + diag, 1000.0 + diag),
        "the floor is being applied per-axis rather than as a radius"
    );
    let diag_in = floor * 0.5 / core::f32::consts::SQRT_2;
    assert!(
        !led.clears(SiteKind::Waystation, 1000.0 + diag_in, 1000.0 + diag_in),
        "the floor is being applied per-axis rather than as a radius"
    );
    println!("clears: refuses inside {floor} m, allows outside, on both axes and the diagonal");
}

/// And the island the tree actually ships still satisfies the roster it is
/// now described by — every placed pair clears the table.
///
/// This is the end-to-end half: the three tests above are about the
/// mechanism, and this is about the islands. It would catch a `pick_minor`
/// that stopped consulting the ledger at all, which none of them would.
#[test]
fn every_shipped_island_satisfies_its_own_roster() {
    for seed in [0u64, 1, 7, 42, 12345, 20_260_731, 20_260_804, 0xDEAD_BEEF] {
        let hv = terrain::haven(seed);
        let mut led = SiteLedger::new();
        assert!(led.take(SiteKind::Haven, hv.x, hv.z));
        // Each site re-tested under its OWN tier's row of the table. That is
        // the thing the matrix buys and a single constant could not: the
        // ledger is asked the question `pick_minor` asked, not a question
        // that happens to have the same answer while every entry is equal.
        let mut live = [0usize; SITE_KINDS];
        for ws in hv.minor.iter().filter(|w| w.live) {
            assert!(
                led.clears(ws.kind, ws.x, ws.z),
                "seed {seed:#x}: {:?} at ({:.0},{:.0}) does not clear the \
                 roster it was chosen against",
                ws.kind,
                ws.x,
                ws.z
            );
            assert!(led.take(ws.kind, ws.x, ws.z));
            live[ws.kind as usize] += 1;
        }
        assert_eq!(
            live[SiteKind::Waystation as usize],
            WAYSTATIONS,
            "seed {seed:#x}: {} live waystations of {WAYSTATIONS} — a short \
             tier is a finding, not a silent degradation",
            live[SiteKind::Waystation as usize]
        );
        assert_eq!(
            live[SiteKind::Inland as usize],
            INLAND_SITES,
            "seed {seed:#x}: {} live inland sites of {INLAND_SITES} — the \
             interior lattice found nowhere clearing the roster, which is the \
             same finding one tier over",
            live[SiteKind::Inland as usize]
        );
        // And the pad is still exactly one, which is what makes the two
        // counts above a partition of `minor` rather than two filters that
        // could both be satisfied while a slot went unclassified.
        assert_eq!(led.len(), 1 + WAYSTATIONS + INLAND_SITES);
        println!(
            "seed {seed:#x}: roster holds {} sites, all clear",
            led.len()
        );
    }
}

// ───────────────────────── the inland tier ────────────────────────────────

/// **An inland site is off the road, and "off" is measured rather than
/// assumed.**
///
/// The tier's whole definition is the negative one — it is the first site the
/// road ring is not the reference curve for — and a negative definition is
/// the easy kind to ship broken, because the thing that would be wrong is the
/// ABSENCE of a road and no count can see an absence. So this asks the road
/// itself: `road_band` over the site's own footprint, at the eight bearings
/// the carve blends across, must answer `Off` everywhere.
///
/// **Mutant, run.** Restoring the first draft's bracket — `INLAND_R_MAX` as
/// the geometric limit, `ROAD_R_MIN - ROAD_SHOULDER_HALF_W -
/// WAYSTATION_RADIUS_M` = 579.99 m — does NOT redden this: a site at 580 m is
/// still clear of the band by 15 m, which is the whole reason that number
/// looked right. What reddens is the second assert below, and that is the
/// point of having both.
#[test]
fn an_inland_site_stands_clear_of_the_road_it_is_defined_as_being_off() {
    for seed in SWEEP {
        let hv = terrain::haven(seed);
        for ws in hv
            .minor
            .iter()
            .filter(|w| w.live && w.kind == SiteKind::Inland)
        {
            let c = ISLAND_SIZE * 0.5;
            let r = ((ws.x - c) * (ws.x - c) + (ws.z - c) * (ws.z - c)).sqrt();
            assert!(
                r <= terrain::INLAND_R_MAX + MM,
                "seed {seed:#x}: an inland site stands at r = {r:.1} m, past \
                 INLAND_R_MAX = {:.1} m",
                terrain::INLAND_R_MAX
            );
            // The blend radius, not the site radius: the carve reaches that
            // far and `carve.rs` §C holds it there, so that is the disc the
            // road may not be inside.
            let fp = terrain::site_footprint(ws.kind);
            for b in 0..8u16 {
                let (dx, dz) = sim_core::yaw_dir((b * 32) << 8);
                let (px, pz) = (ws.x + dx * fp.blend_m, ws.z + dz * fp.blend_m);
                assert_eq!(
                    terrain::ring_band(seed, px, pz),
                    terrain::RoadBand::Off,
                    "seed {seed:#x}: an inland site's carve reaches the road at \
                     ({px:.0}, {pz:.0}) — the tier is defined as the one the \
                     ring is not the reference curve for"
                );
            }
        }
    }
}

/// **And it stands where the ring does not reach**, which is the claim
/// `INLAND_SITES` is justified by and the one the first bracket failed.
///
/// `ROAD_REACH_M` is how far off a road the land stops being served
/// (`examples/second_road.rs`: 38% of walkable land is over 300 m of walking
/// from any road). A site inside that band opens nothing — its side road
/// would be laid through ground the ring already serves — so the distance
/// from an inland site to the NEAREST POINT the road ring can occupy is the
/// number that says the tier is doing its job.
///
/// The ring's innermost radius is `ROAD_R_MIN`, so that distance is
/// `ROAD_R_MIN - r` for a site at radius `r` about the island centre, and
/// asserting it is the same statement as the bracket — deliberately, because
/// a bracket nobody checks against its reason drifts back to the geometric
/// limit, which is exactly what it was.
///
/// **Mutant, run.** With `INLAND_R_MAX` restored to 579.99 m this fails on
/// **7 of 16** seeds — 42, 1337, 4294967291, 999999937, 0xDEADBEEF and
/// 0x0BADC0DE all place at r = 580 m, **20 m from the ring**, and 31337 at
/// 435 m. Those are the sites that made the tier look built while opening
/// nothing, and the other gate in this file stays green over every one of
/// them, which is why both exist.
#[test]
fn an_inland_site_is_further_from_the_ring_than_the_ring_reaches() {
    let mut worst = f32::INFINITY;
    for seed in SWEEP {
        let hv = terrain::haven(seed);
        for ws in hv
            .minor
            .iter()
            .filter(|w| w.live && w.kind == SiteKind::Inland)
        {
            let c = ISLAND_SIZE * 0.5;
            let r = ((ws.x - c) * (ws.x - c) + (ws.z - c) * (ws.z - c)).sqrt();
            let to_ring = terrain::ROAD_R_MIN - r;
            assert!(
                to_ring >= terrain::ROAD_REACH_M - MM,
                "seed {seed:#x}: an inland site sits {to_ring:.1} m inside the \
                 ring's innermost radius, within ROAD_REACH_M = {:.0} m of \
                 ground the ring already serves — the tier opens nothing",
                terrain::ROAD_REACH_M
            );
            worst = worst.min(to_ring);
        }
    }
    println!(
        "the closest inland site on {} seeds stands {worst:.0} m inside the \
         ring's innermost radius (floor {:.0} m)",
        SWEEP.len(),
        terrain::ROAD_REACH_M
    );
}

/// **The tier that pays nothing stands nothing to rob**, asserted at the
/// object rather than at the constant.
///
/// `INLAND_CRATES = 0` and the const block ties the guard roster to it, but
/// both of those are claims about numbers. This walks the site's own scatter
/// cells and counts what `scatter` actually emitted: one canopy, and not one
/// container of either kind.
///
/// **Mutant, run.** Pointing `scatter`'s crate loop back at
/// `WAYSTATION_CRATES` instead of `site_crates(ws.kind)` reddens this with 2
/// caches on every seed, and is invisible to the const block — which is the
/// whole distance between a rule and a gate.
#[test]
fn an_inland_site_is_a_depot_without_scattered_canopy_or_rewards() {
    let table = terrain::ScatterTable::alpha_default();
    for seed in SWEEP {
        let hv = terrain::haven(seed);
        for ws in hv
            .minor
            .iter()
            .filter(|w| w.live && w.kind == SiteKind::Inland)
        {
            let (mut canopies, mut containers) = (0i32, 0i32);
            let (wcx, wcz) = (
                (ws.x * (1.0 / terrain::CELL_SIZE)) as i32,
                (ws.z * (1.0 / terrain::CELL_SIZE)) as i32,
            );
            for cz in wcz - 2..=wcz + 2 {
                for cx in wcx - 2..=wcx + 2 {
                    match terrain::scatter(seed, &table, &hv, cx, cz).occupant {
                        terrain::Occupant::WaystationCanopy => canopies += 1,
                        terrain::Occupant::CacheSlot | terrain::Occupant::CrateSlot => {
                            containers += 1
                        }
                        _ => {}
                    }
                }
            }
            assert_eq!(
                canopies, 0,
                "seed {seed:#x}: an inland site stands {canopies} canopies — \
                 the depot kit replaces the old canopy"
            );
            assert_eq!(
                containers,
                0,
                "seed {seed:#x}: an inland site stands {containers} \
                 container(s). The ladder has one crate of headroom \
                 (WAYSTATIONS * WAYSTATION_CRATES = {} against HAVEN_CRATES = \
                 {}), so arming this tier is a spoken re-pricing, not an edit",
                WAYSTATIONS as i32 * terrain::WAYSTATION_CRATES,
                terrain::HAVEN_CRATES
            );
        }
    }
}

/// **An empty roster says which tier is empty.**
///
/// `Waystation::NONE` is one value, so a roster built from it alone claims
/// every dead slot is a waystation — and `server::boot`'s refusal reads
/// exactly that field to tell an operator whether the ROAD RING or the
/// INTERIOR came up short, which are two different things to do about it.
/// `empty_minor` is the fix and this is what holds it.
///
/// **Mutant, run.** Replacing `empty_minor()`'s body with
/// `[Waystation::NONE; MINOR_SITES]` reddens this and exactly one other gate
/// in the tree — `server::boot`'s `an_empty_tier_counts_the_pad_alone`, whose
/// message comes back reading *"3 of 2 waystations … 0 of 1 inland sites"*,
/// which is the misattribution itself. Every other suite stays green, because
/// on every shipped seed the array is overwritten by live sites before
/// anything reads it: the value only matters on the branch no seed reaches,
/// which is the branch `boot.rs` was written for.
#[test]
fn an_empty_roster_carries_the_tier_that_owns_each_slot() {
    let empty = terrain::empty_minor();
    let mut per_kind = [0usize; SITE_KINDS];
    for ws in empty.iter() {
        assert!(!ws.live, "empty_minor handed back a live site");
        per_kind[ws.kind as usize] += 1;
    }
    assert_eq!(
        per_kind[SiteKind::Waystation as usize],
        WAYSTATIONS,
        "the empty roster claims {} ring slots of {WAYSTATIONS}",
        per_kind[SiteKind::Waystation as usize]
    );
    assert_eq!(
        per_kind[SiteKind::Inland as usize],
        INLAND_SITES,
        "the empty roster claims {} interior slots of {INLAND_SITES} — a short \
         interior would be reported as a short road ring",
        per_kind[SiteKind::Inland as usize]
    );
    assert_eq!(
        per_kind[SiteKind::Haven as usize],
        0,
        "the pad is not a slot"
    );
    // And the live roster agrees with it slot for slot, which is what makes
    // the dead value usable: a reader may group by `kind` without knowing
    // where either tier's block starts.
    let live = terrain::haven(20_260_731).minor;
    for (i, (a, b)) in empty.iter().zip(live.iter()).enumerate() {
        assert_eq!(
            a.kind, b.kind,
            "slot {i}: the empty roster calls it {:?} and the solver fills it \
             with {:?} — a dead slot would name the wrong tier",
            a.kind, b.kind
        );
    }
}
