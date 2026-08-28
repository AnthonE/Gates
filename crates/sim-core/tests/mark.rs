//! `test_mark` — the gate for **where a raid swing leaves its scuff**.
//!
//! A hatchet swung at a tree has marked the bark since 2026-08-25; a
//! hatchet swung at a *wall* marked nothing at all until 2026-08-28, which
//! is `NOW.md` §0mk item 1 and the merge-gate judge's second ranked gap on
//! the same day: the shard took the right hp off the right record for an
//! arrow, a bullet, four wall orientations and nine deployable archetypes,
//! and what reached the player was a number they could not see. A raider
//! could not tell whether the raid was working.
//!
//! `combat::piece_mark` is the answer and this file is what makes its one
//! claim true rather than aspirational: **every point it returns is a
//! point the struck piece actually occupies.** A mark that is 20 cm off
//! the plank draws on nothing, or on the plank behind it, and no other
//! gate in this repo can see that — `EV_IMPACT` is not in `state_hash`
//! (replay green), it moved no wire byte (golden green), and all three
//! fields are `u32` (clippy green). `tests/event_roles.rs` checks which
//! field is which; nothing checked whether the *value* was on the object.
//!
//! ## Why the law is rebuilt rather than called
//!
//! `CLAUDE.md`'s lattice entry: `tests/lattice.rs` wrote a "naive rebuild"
//! whose naive side called the function under test, so both sides carried
//! the mutant and ten assertions were green over a real defect. So nothing
//! below calls `piece_mark` to decide what `piece_mark` should have
//! returned. The surface is rebuilt from parts that are published for
//! other reasons and share no line with it:
//!
//! * `build::anchor` — the point reach is already measured to.
//! * `build::BUILD_CELL_M` / `build::LEVEL_H_M` — the cell and the storey.
//! * `build::column_floor_y` — where a column's level-0 floor sits, which
//!   is what `collide::col_base_y` resolves for an unbuilt column and the
//!   independent statement of the slab height.
//! * the `LOC_TRI_*` half definitions, restated from their own doc
//!   comments in `build.rs` — a different function's rule (`piece_mark`
//!   never tests a half; it returns a centroid).
//!
//! ## Anti-vacuity
//!
//! Every `loc` this repo has is walked from one table, and the table is
//! asserted complete against `LOC_DIAG_B` — the highest code — so a
//! *eleventh* `loc` cannot be added with this file quietly covering ten.
//! `raid`'s scan produces exactly these ten.

use sim_core::build::{
    anchor, column_floor_y, BUILD_CELL_M, LEVEL_H_M, LOC_DIAG_A, LOC_DIAG_B, LOC_EDGE_XLO,
    LOC_EDGE_ZLO, LOC_PLANE, LOC_RISER, LOC_TRI_XHI_ZHI, LOC_TRI_XHI_ZLO, LOC_TRI_XLO_ZHI,
    LOC_TRI_XLO_ZLO,
};
use sim_core::collide::PieceHit;
use sim_core::combat::piece_mark;

const SEED: u64 = 20260731;
/// **The two cell axes differ, and that is load-bearing.** The first cut
/// used (341, 341) with a symmetric stance ring, and a mutant that
/// swapped `mx` and `mz` in the plane arm passed all five tests — the
/// sharpest positional payload in the lane (`tests/event_roles.rs` on
/// `EV_IMPACT`) invisible to the file written to gate it, because
/// `x0 == z0` makes a transposition the identity. `event_roles.rs`'s
/// `distinct3` is the same discipline; this is its fixture form. Every
/// stance offset below is asymmetric for the same reason.
const CX: u16 = 341;
const CZ: u16 = 344;

fn hv() -> &'static sim_core::terrain::Haven {
    static HV: std::sync::OnceLock<sim_core::terrain::Haven> = std::sync::OnceLock::new();
    HV.get_or_init(|| sim_core::terrain::haven(SEED))
}

/// The column's level-0 floor for the fixture cell, off `build`'s own
/// published resolver rather than `collide::col_base_y` — an unbuilt
/// column takes its plate from the terrain band and nothing else.
/// The fixture cell's address at `loc` and `level` — one constructor, so
/// no case can transpose the four parts by hand.
fn addr(loc: u8, level: u8) -> PieceHit {
    PieceHit {
        cx: CX,
        cz: CZ,
        level,
        loc,
    }
}

fn base() -> f32 {
    column_floor_y(SEED, hv(), CX, CZ, 0)
}

/// The ten `loc` codes `combat::raid`'s scan can produce, with the shape
/// each one is: a **slab** lies at the storey's floor, a **wall** spans
/// the storey, and the **riser** ramps between them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Slab,
    Wall,
    Ramp,
}

const LOCS: [(u8, &str, Kind); 10] = [
    (LOC_PLANE, "LOC_PLANE", Kind::Slab),
    (LOC_RISER, "LOC_RISER", Kind::Ramp),
    (LOC_EDGE_XLO, "LOC_EDGE_XLO", Kind::Wall),
    (LOC_EDGE_ZLO, "LOC_EDGE_ZLO", Kind::Wall),
    (LOC_TRI_XLO_ZLO, "LOC_TRI_XLO_ZLO", Kind::Slab),
    (LOC_TRI_XHI_ZLO, "LOC_TRI_XHI_ZLO", Kind::Slab),
    (LOC_TRI_XLO_ZHI, "LOC_TRI_XLO_ZHI", Kind::Slab),
    (LOC_TRI_XHI_ZHI, "LOC_TRI_XHI_ZHI", Kind::Slab),
    (LOC_DIAG_A, "LOC_DIAG_A", Kind::Wall),
    (LOC_DIAG_B, "LOC_DIAG_B", Kind::Wall),
];

/// A ring of stances around the cell — inside it, outside it on each axis,
/// and on both boundaries — so no assertion below is proven on the one
/// spot where a clamp is the identity. `tests/chip.rs`'s judged FAIL of
/// 2026-08-28 is the receipt: every hit fixture fired down the exact
/// centre, where the clamp does nothing, and four mutants ran green.
fn stances(level: u8) -> Vec<(f32, f32, f32, &'static str)> {
    let x0 = CX as f32 * BUILD_CELL_M;
    let z0 = CZ as f32 * BUILD_CELL_M;
    let feet = base() + level as f32 * LEVEL_H_M;
    [
        (x0 + 0.4, z0 + 2.1, "inside, -x +z"),
        (x0 + 2.6, z0 + 0.7, "inside, +x -z"),
        (x0 + 1.1, z0 + 1.9, "inside, off centre both ways"),
        (x0 - 2.0, z0 + 0.8, "outside on -x"),
        (x0 + 5.0, z0 + 2.2, "outside on +x"),
        (x0 + 0.8, z0 - 2.0, "outside on -z"),
        (x0 + 2.2, z0 + 5.0, "outside on +z"),
        (x0, z0, "the low corner exactly"),
        (
            x0 + BUILD_CELL_M,
            z0 + BUILD_CELL_M,
            "the high corner exactly",
        ),
    ]
    .iter()
    .map(|(x, z, w)| (*x, *z, feet, *w))
    .collect()
}

/// A melee strike lands at the swinger's eye height, and the origin is
/// derived from the arrow's rather than typed: `gather::EYE_M` is the
/// same expression, so the two cannot drift and no number is invented
/// here (`CLAUDE.md` §loop discipline — knobs are spoken, never invented).
const EYE_M: f32 = sim_core::ranged::ARROW_EYE_MM as f32 / 1000.0;

/// **The claim, for all ten `loc` arms and every stance: the mark is on
/// the piece.**
///
/// Three things are checked and each fails a different way:
///
/// 1. **Inside the cell footprint.** A raider standing 2 m outside the
///    cell must not drag the mark out there with them — the piece is 3 m
///    wide and its surface stops at the boundary.
/// 2. **At the piece's own height.** A slab is at the storey floor, a wall
///    spans the storey, a ramp is between them. A mark at eye height on a
///    *floor* hangs in the air; a mark at floor height on a *wall* two
///    storeys up is under the base.
/// 3. **On the pinned axis, exactly.** A straight wall has no thickness,
///    so its mark's x (or z) is the edge's own coordinate to the bit. This
///    is the one that catches a mark drawn on the cell centre instead of
///    the plank.
#[test]
fn every_mark_lands_on_the_piece_it_marks() {
    assert_eq!(
        LOCS.len(),
        LOC_DIAG_B as usize + 1,
        "a loc was added and this table did not grow — see the header"
    );
    let x0 = CX as f32 * BUILD_CELL_M;
    let z0 = CZ as f32 * BUILD_CELL_M;
    for level in 0..3u8 {
        let floor = base() + level as f32 * LEVEL_H_M;
        for (loc, name, kind) in LOCS {
            for (px, pz, feet, where_) in stances(level) {
                let (mx, my, mz) = piece_mark(&addr(loc, level), base(), px, pz, feet + EYE_M);

                // 1 — the footprint.
                assert!(
                    (x0..=x0 + BUILD_CELL_M).contains(&mx)
                        && (z0..=z0 + BUILD_CELL_M).contains(&mz),
                    "{name} at level {level}, raider {where_}: mark ({mx}, {mz}) \
                     is outside the cell [{x0}, {}] x [{z0}, {}]",
                    x0 + BUILD_CELL_M,
                    z0 + BUILD_CELL_M
                );

                // 2 — the height.
                match kind {
                    Kind::Slab => assert_eq!(
                        my, floor,
                        "{name} at level {level}, raider {where_}: a slab's surface \
                         is the storey floor {floor}, got {my}"
                    ),
                    Kind::Wall | Kind::Ramp => assert!(
                        (floor..=floor + LEVEL_H_M).contains(&my),
                        "{name} at level {level}, raider {where_}: {my} is outside \
                         the storey [{floor}, {}]",
                        floor + LEVEL_H_M
                    ),
                }

                // 3 — the pinned axis, to the bit.
                if loc == LOC_EDGE_XLO {
                    assert_eq!(
                        mx.to_bits(),
                        x0.to_bits(),
                        "{name} at level {level}, raider {where_}: not on the -x edge"
                    );
                }
                if loc == LOC_EDGE_ZLO {
                    assert_eq!(
                        mz.to_bits(),
                        z0.to_bits(),
                        "{name} at level {level}, raider {where_}: not on the -z edge"
                    );
                }
            }
        }
    }
}

/// **A triangle's mark is inside the half the triangle occupies**, and a
/// diagonal's is where the two diagonals cross.
///
/// This is the arm that cannot take the raider's clamped stance: a
/// rectangle's clamp lands anywhere in the cell, and half of the cell is
/// air for a triangle. The half tests are restated from the `LOC_TRI_*`
/// doc comments in `build.rs` — a rule `piece_mark` never evaluates.
#[test]
fn a_triangle_is_marked_on_its_own_half_and_a_diagonal_where_they_cross() {
    let x0 = CX as f32 * BUILD_CELL_M;
    let z0 = CZ as f32 * BUILD_CELL_M;
    for (px, pz, feet, where_) in stances(0) {
        for loc in [
            LOC_TRI_XLO_ZLO,
            LOC_TRI_XHI_ZLO,
            LOC_TRI_XLO_ZHI,
            LOC_TRI_XHI_ZHI,
        ] {
            let (mx, my, mz) = piece_mark(&addr(loc, 0), base(), px, pz, feet + EYE_M);
            let (dx, dz) = (mx - x0, mz - z0);
            let inside = match loc {
                LOC_TRI_XLO_ZLO => dx + dz <= BUILD_CELL_M,
                LOC_TRI_XHI_ZHI => dx + dz >= BUILD_CELL_M,
                LOC_TRI_XHI_ZLO => dz <= dx,
                _ => dz >= dx,
            };
            assert!(
                inside,
                "loc {loc}, raider {where_}: mark ({dx}, {dz}) into the cell is \
                 on the empty half of the triangle"
            );
            let _ = my;
        }
        // The diagonals cross at the cell centre and `anchor` says so; a
        // mark anywhere else is on one wall or the other, never both.
        for loc in [LOC_DIAG_A, LOC_DIAG_B] {
            let (mx, _, mz) = piece_mark(&addr(loc, 0), base(), px, pz, feet + EYE_M);
            let (ax, az) = anchor(CX, CZ, loc);
            assert_eq!(
                (mx.to_bits(), mz.to_bits()),
                (ax.to_bits(), az.to_bits()),
                "loc {loc}, raider {where_}: a diagonal is marked where the two cross"
            );
        }
    }
}

/// **The mark moves with the raider.** Two people hitting opposite ends of
/// one plank leave two scuffs, not one.
///
/// The mutant this kills is the obvious cheap implementation — return
/// `build::anchor` for everything — which satisfies every assertion in the
/// first test above, because an anchor is always on its piece. What it
/// costs is the whole point of the slice: a wall a raider has been hitting
/// for thirty seconds would carry exactly one mark, in the middle,
/// wherever they stood.
///
/// The free axis is asserted rather than "some coordinate differs", so a
/// mark that moved along the *pinned* axis — off the plank — cannot pass
/// this by being different.
#[test]
fn a_mark_moves_along_the_face_with_the_stance() {
    let x0 = CX as f32 * BUILD_CELL_M;
    let z0 = CZ as f32 * BUILD_CELL_M;
    let feet = base();
    let near = |loc, px, pz| piece_mark(&addr(loc, 0), base(), px, pz, feet + EYE_M);

    // A wall on the -x edge runs along z: standing at the two ends of it
    // marks the two ends of it.
    let lo = near(LOC_EDGE_XLO, x0 - 1.0, z0 + 0.2);
    let hi = near(LOC_EDGE_XLO, x0 - 1.0, z0 + 2.8);
    assert!(
        hi.2 - lo.2 > 2.0,
        "LOC_EDGE_XLO: the mark did not travel along the plank ({} -> {})",
        lo.2,
        hi.2
    );
    assert_eq!(lo.0.to_bits(), hi.0.to_bits(), "and it stayed on the face");

    // The -z edge runs along x.
    let lo = near(LOC_EDGE_ZLO, x0 + 0.2, z0 - 1.0);
    let hi = near(LOC_EDGE_ZLO, x0 + 2.8, z0 - 1.0);
    assert!(
        hi.0 - lo.0 > 2.0,
        "LOC_EDGE_ZLO: the mark did not travel along the plank"
    );
    assert_eq!(lo.2.to_bits(), hi.2.to_bits(), "and it stayed on the face");

    // A floor is surface in both axes, so both move.
    let lo = near(LOC_PLANE, x0 + 0.2, z0 + 0.4);
    let hi = near(LOC_PLANE, x0 + 2.8, z0 + 2.7);
    assert!(
        hi.0 - lo.0 > 2.0 && hi.2 - lo.2 > 2.0,
        "LOC_PLANE: a floor is marked under the raider"
    );
    assert_eq!(lo.1.to_bits(), hi.1.to_bits(), "at one height, the slab's");
}

/// **A wall's mark climbs with the raider and stops at the storey.**
///
/// `combat::raid`'s `storey_ok` is the movement collider's overlap test, so
/// a 1.7 m capsule reaches a level its eye is not inside: standing on the
/// ground floor you can swing at the wall above you. The clamp is what
/// keeps that mark on the wall instead of under the base, and this is the
/// case that makes it load-bearing rather than decorative — under a mutant
/// that drops either rail the mark leaves the plank.
#[test]
fn a_walls_mark_is_clamped_into_its_own_storey() {
    let x0 = CX as f32 * BUILD_CELL_M;
    let z0 = CZ as f32 * BUILD_CELL_M;
    let feet = base();
    let eye = feet + EYE_M;

    // Level 0: the eye is inside the storey, so the mark is at eye height
    // exactly — neither rail binds and the clamp is the identity.
    let (_, my, _) = piece_mark(&addr(LOC_EDGE_XLO, 0), base(), x0 - 1.0, z0 + 1.5, eye);
    assert_eq!(my, eye, "inside its own storey the eye is the mark");

    // Level 1: the eye is BELOW the wall. The floor rail binds.
    let up = base() + LEVEL_H_M;
    let (_, my, _) = piece_mark(&addr(LOC_EDGE_XLO, 1), base(), x0 - 1.0, z0 + 1.5, eye);
    assert_eq!(
        my, up,
        "a wall a storey up is marked at its foot, not below it"
    );

    // Standing a storey up swinging DOWN at the ground-floor wall: the
    // ceiling rail binds, and the mark is at the top of the plank.
    let (_, my, _) = piece_mark(
        &addr(LOC_EDGE_XLO, 0),
        base(),
        x0 - 1.0,
        z0 + 1.5,
        up + EYE_M,
    );
    assert_eq!(
        my,
        base() + LEVEL_H_M,
        "a wall below is marked at its head, not above it"
    );
}

/// **A riser is marked on the tread**, which rises toward +Z across the
/// storey (`collide::piece_ground`'s ramp, in words).
///
/// Asserted as an ORDER rather than as the formula restated: a stance
/// further along +Z marks higher, the low end is the storey floor and the
/// high end is the storey ceiling. A rebuild of the expression here would
/// be the lattice trap with two authors instead of one.
#[test]
fn a_riser_is_marked_on_the_tread_it_rises_along() {
    let x0 = CX as f32 * BUILD_CELL_M;
    let z0 = CZ as f32 * BUILD_CELL_M;
    let feet = base();
    let at = |pz| piece_mark(&addr(LOC_RISER, 0), base(), x0 + 1.5, pz, feet + EYE_M).1;

    let bottom = at(z0 - 1.0); // clamped to the low end
    let mid = at(z0 + 1.5);
    let top = at(z0 + BUILD_CELL_M + 1.0); // clamped to the high end
    assert_eq!(bottom, base(), "the foot of the ramp is the storey floor");
    assert_eq!(
        top,
        base() + LEVEL_H_M,
        "the head of the ramp is the storey ceiling"
    );
    assert!(
        bottom < mid && mid < top,
        "the tread rises monotonically toward +Z: {bottom} / {mid} / {top}"
    );
}
