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
    self, SiteKind, SiteLedger, ISLAND_SIZE, MAX_SITES, SITE_KINDS, SITE_SEP_M, WAYSTATIONS,
    WAYSTATION_MIN_SEP_M,
};

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
        let mut live = 0usize;
        for ws in hv.minor.iter().filter(|w| w.live) {
            assert!(
                led.clears(SiteKind::Waystation, ws.x, ws.z),
                "seed {seed:#x}: waystation at ({:.0},{:.0}) does not clear the \
                 roster it was chosen against",
                ws.x,
                ws.z
            );
            assert!(led.take(SiteKind::Waystation, ws.x, ws.z));
            live += 1;
        }
        assert_eq!(
            live, WAYSTATIONS,
            "seed {seed:#x}: {live} live waystations of {WAYSTATIONS} — a short \
             tier is a finding, not a silent degradation"
        );
        println!(
            "seed {seed:#x}: roster holds {} sites, all clear",
            led.len()
        );
    }
}
