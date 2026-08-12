//! Gate: the mesh the client draws fits the volume the sim blocks, for every
//! occupant — not just the tree.
//!
//! **This closes an admission `sim_core` makes about itself.**
//! `terrain::OCCUPANT_R_M`'s doc says, accurately, that "nothing in the Rust
//! workspace can see a triangle, so the asserts below prove only that this
//! file agrees with itself". Eight browser `.mjs` gates used to see the
//! triangles and went with the browser client; `tests/tree.rs` replaced one of
//! them (the conifer) and the rest were uncovered. This is the rest.
//!
//! It is the same class of failure `reference/MONUMENTS.md` §4 is about, with
//! the floats taken out of it. Their client and server disagreed about a rock
//! edge because two implementations computed it; ours disagree when two people
//! WRITE it — and until this pass we had exactly that, measured
//! (`TERRAIN.md` §7.1): the waystation canopy was nine rows to a 4.1 m finial
//! in the sim and six rows topping out at 2.09 m in the renderer, so a player
//! was stopped ~0.7 m outside posts they could see, with every gate green.
//!
//! Headless — no GPU, no window, no shard. Geometry is countable.

#![cfg(feature = "render")]

use client::render::props::{
    archetype_lift, archetype_mesh, authored, CANOPY_HEX, SHELTER_HEX, SINK_M,
};
use client::render::tree::{bounds, min_y};
use sim_core::terrain::{
    self, Occupant, OCCUPANT_R_M, OCCUPANT_TOP_M, SHELTER_BOXES, SHELTER_CORNER_R_M,
    SHELTER_PEAK_M, WAYSTATION_CANOPY_BOXES, WAYSTATION_CANOPY_PEAK_M, WAYSTATION_CANOPY_R_M,
};

/// Every occupant the sim can place. Written out rather than derived so §C can
/// prove the list is complete — a new variant that nobody adds here is caught
/// by the count assert, not by nothing.
const ALL: [Occupant; 12] = [
    Occupant::None,
    Occupant::Tree,
    Occupant::StoneNode,
    Occupant::MetalNode,
    Occupant::SulfurNode,
    Occupant::Bush,
    Occupant::Rock,
    Occupant::BarrelSlot,
    Occupant::CrateSlot,
    Occupant::CacheSlot,
    Occupant::HavenShelter,
    Occupant::WaystationCanopy,
];

/// `boxes_mesh` emits six faces a box, two triangles each, three unshared
/// vertices each — the faces carry their own value so the massing reads as a
/// solid rather than a silhouette, and nothing is welded. 36 is measured, not
/// assumed: the first draft of this gate guessed 24 (four corners a quad) and
/// the assertion caught the guess, which is the behaviour wanted from it.
const VERTS_PER_BOX: usize = 36;

fn verts(m: &bevy::prelude::Mesh) -> usize {
    m.count_vertices()
}

// ── §A · The authored pair IS the sim's table ──────────────────────────────

/// The unit conversion, alone. Full size in the sim, half extent in the mesh
/// builder — the exact hazard that let the two lists come apart while both
/// looked plausible, so it gets an assertion of its own rather than riding
/// along inside a bound check.
#[test]
fn the_authored_boxes_are_the_sim_rows_at_half_extent() {
    for (rows, parts, name) in [
        (
            &SHELTER_BOXES[..],
            authored(&SHELTER_BOXES, &SHELTER_HEX),
            "shelter",
        ),
        (
            &WAYSTATION_CANOPY_BOXES[..],
            authored(&WAYSTATION_CANOPY_BOXES, &CANOPY_HEX),
            "canopy",
        ),
    ] {
        assert_eq!(
            parts.len(),
            rows.len(),
            "{name}: the drawn box list is {} rows against the sim's {}",
            parts.len(),
            rows.len()
        );
        for (i, (r, (c, h, _))) in rows.iter().zip(parts.iter()).enumerate() {
            assert_eq!(
                (c[0].to_bits(), c[1].to_bits(), c[2].to_bits()),
                (r[0].to_bits(), r[1].to_bits(), r[2].to_bits()),
                "{name} row {i}: drawn centre {c:?} is not the sim's {:?}",
                &r[0..3]
            );
            for k in 0..3 {
                assert_eq!(
                    h[k].to_bits(),
                    (r[3 + k] * 0.5).to_bits(),
                    "{name} row {i} axis {k}: drawn half extent {} is not half the sim's \
                     full size {} — this is the units hazard TERRAIN.md §7.1 recorded",
                    h[k],
                    r[3 + k]
                );
            }
        }
    }
}

/// Every row reaches the mesh. A conversion that silently dropped the last
/// four boxes would satisfy the row-for-row test above only if it also
/// shortened the list, so this counts triangles instead of trusting `len`.
#[test]
fn every_authored_row_reaches_the_mesh() {
    for (o, rows, name) in [
        (Occupant::HavenShelter, SHELTER_BOXES.len(), "shelter"),
        (
            Occupant::WaystationCanopy,
            WAYSTATION_CANOPY_BOXES.len(),
            "canopy",
        ),
    ] {
        let m = archetype_mesh(o).expect("authored structures have one mesh");
        assert_eq!(
            verts(&m),
            rows * VERTS_PER_BOX,
            "{name}: {} vertices for {rows} boxes — expected {}",
            verts(&m),
            rows * VERTS_PER_BOX
        );
    }
}

/// The published broad phase is the drawn bound, both directions.
///
/// **Containment is not enough here and that is the point.** `OCCUPANT_R_M`
/// and `OCCUPANT_TOP_M` do not approximate these two structures, they are
/// *defined* as the tables' own bounds (`SHELTER_CORNER_R_M`'s doc: the
/// plinth's half-diagonal, rounded up; `SHELTER_PEAK_M`: the tower-cap's top).
/// So a gap in either direction means the scalar and the table have come
/// apart, which is how the canopy shipped blocked at 4.1 m and drawn at 2.09.
#[test]
fn the_authored_pair_bounds_equal_what_the_sim_publishes() {
    for (o, r_pub, peak, name) in [
        (
            Occupant::HavenShelter,
            SHELTER_CORNER_R_M,
            SHELTER_PEAK_M,
            "shelter",
        ),
        (
            Occupant::WaystationCanopy,
            WAYSTATION_CANOPY_R_M,
            WAYSTATION_CANOPY_PEAK_M,
            "canopy",
        ),
    ] {
        let m = archetype_mesh(o).expect("authored structures have one mesh");
        let (h, r) = bounds(&[&m]);
        assert!(
            (h - peak).abs() < 1.0e-4,
            "{name}: drawn peak {h:.4} m against the sim's published {peak:.4} m"
        );
        // The radius is deliberately rounded OUTWARD (erring outward costs one
        // box-loop iteration; erring inward drops a wall), so the assertion is
        // "contains, and by less than a millimetre" rather than equality.
        assert!(
            r <= r_pub,
            "{name}: drawn radius {r:.4} m reaches past the sim's broad phase {r_pub:.4} m — \
             a body would pass through geometry it can see"
        );
        assert!(
            r_pub - r < 1.0e-3,
            "{name}: the sim's broad phase {r_pub:.4} m stands {:.4} m off the drawn \
             radius {r:.4} m — the scalar and the box table have come apart",
            r_pub - r
        );
    }
}

// ── §B · Every other archetype fits the volume the sim blocks ──────────────

/// The occupants whose sim volume is deliberately not their drawn bound, and
/// why. An entry here is an excuse that has to be written down; §C refuses a
/// variant that is neither measured nor excused.
fn excused(o: Occupant) -> Option<&'static str> {
    match o {
        Occupant::None => Some("not a thing; nothing is drawn"),
        // `tests/tree.rs` gates all CONIFER_POOL variants against PINE_MAX_R
        // and PINE_H, which is the same assertion at pool granularity.
        Occupant::Tree => Some("a generated pool, gated variant by variant in tests/tree.rs"),
        // `OCCUPANT_R_M`'s row 5 is 0.0 on purpose — you walk through a bush.
        Occupant::Bush => Some("deliberately passable: OCCUPANT_R_M and _TOP_M are 0"),
        _ => None,
    }
}

/// How much wider and taller the sim is allowed to block than the client
/// draws, in metres, at slot scale 1.0.
///
/// **Closed 2026-08-10 (operator), so this is now an equality gate.** It
/// shipped as a ratchet at the measured 0.39 m / 0.52 m, because the slack was
/// entirely the three generated blobs: `blob_mesh(r, jitter, …)` displaces
/// vertices INWARD from its nominal radius, so a boulder written into
/// `OCCUPANT_R_M` as "DodecahedronGeometry(1.5)" actually reached 1.1145 m and
/// blocked 0.39 m of ground nobody could see. Those rows are the measured
/// bounds now.
///
/// What is left is rounding, and it is deliberately outward:
/// `SHELTER_CORNER_R_M`'s convention, because erring outward costs a wasted
/// narrow-phase test while erring inward lets a body stand inside the mesh.
/// The two authored rows do not even round at the same place — 4.9498 at four
/// decimals, 3.96 at two, a 0.2 mm artefact — so **one millimetre**, which is
/// the threshold `the_authored_pair_bounds_equal_what_the_sim_publishes`
/// already uses a few tests up.
///
/// It is a rounding allowance, not a budget. A millimetre is below anything a
/// player can be stopped by and three orders under the 0.39 m skirt this
/// closed; raising it re-opens that skirt rather than relaxing a threshold.
const SLACK_R_M: f32 = 0.001;
const SLACK_TOP_M: f32 = 0.001;

#[test]
fn every_drawn_archetype_fits_the_volume_the_sim_blocks() {
    let mut checked = 0usize;
    for o in ALL {
        if excused(o).is_some() {
            continue;
        }
        let m = archetype_mesh(o).unwrap_or_else(|| panic!("{o:?} is not excused and has no mesh"));
        let (h, r) = bounds(&[&m]);
        let lift = archetype_lift(o);
        let (r_pub, top_pub) = (OCCUPANT_R_M[o as usize], OCCUPANT_TOP_M[o as usize]);
        checked += 1;

        // The load-bearing direction: nothing may be drawn outside the volume
        // that stops a body, or the player sees geometry they pass through.
        assert!(
            r <= r_pub + 1.0e-4,
            "{o:?}: drawn radius {r:.4} m reaches past the blocked {r_pub:.4} m"
        );
        assert!(
            lift + h <= top_pub + 1.0e-4,
            "{o:?}: drawn top {:.4} m reaches past the blocked {top_pub:.4} m",
            lift + h
        );
        // The other direction, ratcheted.
        assert!(
            r_pub - r <= SLACK_R_M,
            "{o:?}: the sim blocks {:.4} m wider than the client draws (cap {SLACK_R_M}) — \
             an invisible collision skirt",
            r_pub - r
        );
        assert!(
            top_pub - (lift + h) <= SLACK_TOP_M,
            "{o:?}: the sim blocks {:.4} m taller than the client draws (cap {SLACK_TOP_M})",
            top_pub - (lift + h)
        );
        // `ART.md` rule 2, as arithmetic: nothing sits ON the ground. The
        // lowest vertex must land at or under the slot's surface once the
        // draw's own sink is applied — a prop whose base floats reads as a
        // sticker, and it is the defect the clutter skirt exists to hide.
        //
        // Deep is FINE and deliberate: the shelter's plinth spans y ∈
        // [−1.4, 0.2] on purpose (`SHELTER_FLOOR_IX`), so this is one-sided.
        // The first draft asserted a floor here and the plinth reddened it,
        // which is the buried foundation doing its job.
        let lo = min_y(&[&m]);
        assert!(
            lift + lo - SINK_M <= 1.0e-4,
            "{o:?}: the mesh base floats {:.4} m above the slot's ground",
            lift + lo - SINK_M
        );
    }
    assert!(
        checked >= 8,
        "only {checked} archetypes measured — the sweep is not reaching the table"
    );
}

// ── §C · Nothing is unchecked ──────────────────────────────────────────────

/// A new occupant arrives measured or excused, never silently uncovered.
///
/// The `event_roles.rs` discipline applied to geometry: the failure this file
/// exists to stop is not a wrong number, it is a row nobody looked at.
#[test]
fn every_occupant_is_measured_or_excused() {
    assert_eq!(
        ALL.len(),
        OCCUPANT_R_M.len() - 1,
        "the occupant list here is {} long against {} sim rows (one is the client's \
         stump, index 8, which is not a sim occupant) — a variant was added without \
         a geometry check",
        ALL.len(),
        OCCUPANT_R_M.len()
    );
    for o in ALL {
        let has_mesh = archetype_mesh(o).is_some();
        assert!(
            has_mesh || excused(o).is_some(),
            "{o:?} draws nothing and carries no excuse"
        );
    }
    // And the sim's own tables stay the same length as each other.
    assert_eq!(OCCUPANT_R_M.len(), OCCUPANT_TOP_M.len());
    assert_eq!(terrain::SHELTER_BOXES.len(), SHELTER_HEX.len());
    assert_eq!(terrain::WAYSTATION_CANOPY_BOXES.len(), CANOPY_HEX.len());
}

// ── §D · Deployables: the drawn box is the blocked box ─────────────────────

/// Every solid deploy archetype blocks exactly the volume the client draws,
/// and every walk-over archetype is walk-over on purpose.
///
/// This is the comparison `NOW.md` §0n2 said could not exist — "the greybox
/// gate cannot catch it because there is nothing on the sim side to compare
/// against". The sim side exists now (`sim_core::deploy::DEPLOY_VOL`, deploy
/// collision v0), and the contract is exact equality with
/// `structures::deploy_size`'s authored rows: both are real-world dimensions
/// typed by a person, so a drift is a typo and not a tolerance question.
#[test]
fn every_solid_deploy_blocks_what_it_draws() {
    use client::render::structures::deploy_size;
    use sim_core::deploy::{solid_vol, DEPLOY_VOL};

    assert_eq!(
        DEPLOY_VOL.len(),
        10,
        "the sim volume table and the archetype space drifted"
    );
    for (arch, [w, h, d]) in DEPLOY_VOL.iter().enumerate() {
        let drawn = deploy_size(arch);
        match solid_vol(arch as u8) {
            Some((hw, sh, hd)) => {
                assert_eq!(
                    (drawn.x, drawn.y, drawn.z),
                    (*w, *h, *d),
                    "arch {arch}: the sim blocks a different box than the client draws"
                );
                assert_eq!((hw * 2.0, sh, hd * 2.0), (*w, *h, *d));
            }
            None => {
                // Walk-over rows, by index: bag, fire, door, lock. The door
                // blocks as an edge (shut bits) and the lock never mints a
                // record; the bag and the fire are the two deliberate
                // walk-overs (`DEPLOY_VOL`'s doc).
                assert!(
                    matches!(arch, 0 | 3 | 6 | 7),
                    "arch {arch} became walk-over without the doc's excuse list"
                );
            }
        }
    }
}
