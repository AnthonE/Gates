//! The build ghost tells the truth about the piece it becomes.
//!
//! **What this asserts, and what it deliberately does not.** `render/ghost.rs`
//! and `render/structures.rs` both draw a doorway — since 2026-08-09 from ONE
//! emit site, `structures::shape_parts` (`NOW.md` §0u item 1). Asserting "the
//! ghost's table equals the kit's table" would be the byte-golden hole
//! `CLAUDE.md`'s trap list names — both could be wrong together and stay
//! green, which is how 27 Oxide payload fixes shipped under a pinned
//! `MSILHash`. Sharing the emit closes the drift *between* them; these tests
//! close the other half by checking the one table against the sim.
//!
//! So the assertion is against the **sim**: the posts the client draws stand
//! where `sim_core::collide::doorway_solid_at` says a player is blocked, and
//! the gap is where it says they are not. `RENDER.md` §8 states the stake — the
//! opening is 1.2 m x 2.1 m *because* that is what `edge_hit` refuses, and
//! "draw it elsewhere and the frame lies about where a player can walk".
//!
//! Headless, no GPU, no socket; it links `render` for the shared constants.

use client::render::structures::{
    door_opening_w, door_post_gap, shape_parts, LINTEL_DROP_M, LINTEL_H_M, MAX_PARTS, N_SHAPES,
    SEAM_M,
};
use sim_core::build::{BUILD_CELL_M, LEVEL_H_M};
use sim_core::collide::{doorway_solid_at, DOOR_POST_W_M};

/// Where the two drawn posts span, as `t` along the edge.
fn drawn_posts() -> [(f32, f32); 2] {
    let gap = door_post_gap();
    let mid = BUILD_CELL_M * 0.5;
    let half = DOOR_POST_W_M * 0.5;
    [
        (mid - gap - half, mid - gap + half),
        (mid + gap - half, mid + gap + half),
    ]
}

fn drawn_solid_at(t: f32) -> bool {
    drawn_posts().iter().any(|(a, b)| t >= *a && t <= *b)
}

/// **The measured deviation, pinned.** The drawn posts are inset from the
/// cell boundary by half the cosmetic seam — `SEAM_M / 2` = 2 cm — because
/// `door_post_gap` is computed over `BUILD_CELL_M - SEAM_M` so adjacent edge
/// pieces do not z-fight. The sim blocks to the boundary itself.
///
/// This is a deliberate cosmetic offset, not a defect, and it is pinned here
/// rather than tolerated silently: at 2 cm no player can see or exploit it,
/// and if it ever grows this test says so before anyone has to notice a
/// phantom post in play.
const EXPECTED_INSET_M: f32 = SEAM_M * 0.5;

#[test]
fn the_posts_are_inset_by_exactly_half_the_seam_and_no_more() {
    let [(a0, a1), (b0, b1)] = drawn_posts();
    // The outer face of each post against the cell's two ends.
    assert!(
        (a0 - EXPECTED_INSET_M).abs() < 1e-4,
        "near post starts at {a0}, expected {EXPECTED_INSET_M}"
    );
    assert!(
        ((BUILD_CELL_M - b1) - EXPECTED_INSET_M).abs() < 1e-4,
        "far post ends {} from the boundary, expected {EXPECTED_INSET_M}",
        BUILD_CELL_M - b1
    );
    // The inner faces, against the band the sim stops blocking at.
    assert!(
        (a1 - (DOOR_POST_W_M + EXPECTED_INSET_M)).abs() < 1e-4,
        "near post ends at {a1}"
    );
    assert!(
        (b0 - (BUILD_CELL_M - DOOR_POST_W_M - EXPECTED_INSET_M)).abs() < 1e-4,
        "far post starts at {b0}"
    );
    // A const assertion, because the bound is on a constant: clippy is right
    // that a runtime `assert!` over two literals is not a test. It still
    // belongs here rather than beside the constant — this is the file that
    // explains why 2 cm is tolerable and 5 cm is not.
    const {
        assert!(
            EXPECTED_INSET_M < 0.05,
            "the drawn/blocked disagreement has grown past what a player \
             cannot see: a post they can walk through"
        )
    };
}

/// Everywhere except within the seam of a boundary, the drawn doorway and the
/// sim agree about solid and open.
#[test]
fn no_sampled_point_disagrees_with_the_sim() {
    // The four boundaries the seam sits astride.
    let seams = [
        0.0,
        DOOR_POST_W_M,
        BUILD_CELL_M - DOOR_POST_W_M,
        BUILD_CELL_M,
    ];
    let mut checked = 0;
    let mut skipped = 0;
    for i in 0..=600 {
        let t = BUILD_CELL_M * (i as f32 / 600.0);
        if seams.iter().any(|s| (t - s).abs() <= SEAM_M) {
            skipped += 1;
            continue;
        }
        assert_eq!(
            drawn_solid_at(t),
            doorway_solid_at(t),
            "at t={t}: drawn-solid={} but sim-blocks={}",
            drawn_solid_at(t),
            doorway_solid_at(t)
        );
        checked += 1;
    }
    // The suite must not pass by checking nothing, and must not pass by
    // skipping everything (`CLAUDE.md`: a pass it did not earn is the worst
    // bug class). Both halves are asserted.
    assert!(checked > 500, "only {checked} samples were compared");
    assert!(skipped < 60, "{skipped} samples were skipped as seam");
}

/// The middle of the opening is walkable and the middle of each post is not —
/// the two claims a player actually cares about, stated without tolerance.
#[test]
fn the_opening_is_open_and_the_posts_are_solid() {
    let mid = BUILD_CELL_M * 0.5;
    assert!(!doorway_solid_at(mid), "the doorway's opening is blocked");
    assert!(!drawn_solid_at(mid), "a post is drawn in the opening");
    let gap = door_post_gap();
    for c in [mid - gap, mid + gap] {
        assert!(doorway_solid_at(c), "the sim lets a player through a post");
        assert!(drawn_solid_at(c), "no post is drawn where the sim blocks");
    }
}

/// The lintel's underside is the top of the opening, and a player is 1.8 m.
#[test]
fn the_lintel_clears_a_standing_player() {
    let centre = LEVEL_H_M * 0.5 + (LEVEL_H_M * 0.5 - LINTEL_DROP_M);
    let underside = centre - LINTEL_H_M * 0.5;
    assert!(
        underside >= 1.8,
        "the doorway is {underside} m clear — a 1.8 m player cannot walk through"
    );
    // ...and it does not float: the lintel's top is the piece's own top.
    let top = centre + LINTEL_H_M * 0.5;
    assert!(
        (top - LEVEL_H_M).abs() < 1e-4,
        "the lintel's top is {top}, not the piece's {LEVEL_H_M}"
    );
}

/// The lintel spans exactly the gap the posts leave — not a third number that
/// happens to look right.
#[test]
fn the_lintel_spans_exactly_what_the_posts_leave() {
    let between = 2.0 * door_post_gap() - DOOR_POST_W_M;
    assert!(
        (door_opening_w() - between).abs() < 1e-4,
        "lintel spans {} but the posts leave {between}",
        door_opening_w()
    );
}

// ---------------------------------------------------------------------------
// The shared parts table's own emit (`structures::shape_parts`).
//
// The tests above pin the CONSTANTS; these read what the table actually
// EMITS — the same parts the ghost scales a unit cube to and `spawn_piece`
// builds meshes from, from the one function both call. They exist because
// the constants were never the hole: on 2026-08-09 the ghost and the piece
// shared every number and the ghost still drew the doorway's lintel at waist
// height (centre 1.05 m against the piece's 2.55 m) and previewed stairs as
// a level plate against the piece's pitched ramp, because which parts exist
// and where they go was a second, ungated copy. `no_sampled_point_…` above
// could not see either, since `drawn_posts` restates the layout by hand.

/// Where the table's doorway posts span, as `t` along the edge — read off the
/// emitted parts rather than restated.
fn table_posts() -> Vec<(f32, f32)> {
    let (parts, n) = shape_parts(sim_core::build::SHAPE_DOORWAY);
    let mid = BUILD_CELL_M * 0.5;
    parts[..n]
        .iter()
        .filter(|p| (p.size.y - LEVEL_H_M).abs() < 1e-4) // full height = a post
        .map(|p| {
            let c = mid + p.offset.z;
            (c - p.size.z * 0.5, c + p.size.z * 0.5)
        })
        .collect()
}

/// The emitted doorway agrees with the sim about solid and open, sampled the
/// way `no_sampled_point_disagrees_with_the_sim` samples — but off the
/// table's emit instead of a hand-restated layout.
#[test]
fn the_tables_doorway_is_the_sims_doorway() {
    let posts = table_posts();
    assert_eq!(posts.len(), 2, "a doorway emits exactly two posts");
    let solid = |t: f32| posts.iter().any(|(a, b)| t >= *a && t <= *b);
    let seams = [
        0.0,
        DOOR_POST_W_M,
        BUILD_CELL_M - DOOR_POST_W_M,
        BUILD_CELL_M,
    ];
    let mut checked = 0;
    for i in 0..=600 {
        let t = BUILD_CELL_M * (i as f32 / 600.0);
        if seams.iter().any(|s| (t - s).abs() <= SEAM_M) {
            continue;
        }
        assert_eq!(
            solid(t),
            doorway_solid_at(t),
            "at t={t}: table-solid={} but sim-blocks={}",
            solid(t),
            doorway_solid_at(t)
        );
        checked += 1;
    }
    assert!(checked > 500, "only {checked} samples were compared");
}

/// The emitted lintel caps the opening: its underside is the 2.1 m the
/// opening derivation on `LINTEL_H_M` states, a 1.8 m player clears it, and
/// its top is the piece's own top. **This is the assertion the old ghost
/// failed** — its copy hung the lintel at waist height, underside 0.6 m.
#[test]
fn the_tables_lintel_caps_the_opening() {
    let (parts, n) = shape_parts(sim_core::build::SHAPE_DOORWAY);
    let lintels: Vec<_> = parts[..n]
        .iter()
        .filter(|p| (p.size.y - LINTEL_H_M).abs() < 1e-4)
        .collect();
    assert_eq!(lintels.len(), 1, "a doorway emits exactly one lintel");
    let lintel = lintels[0];
    let underside = lintel.offset.y - lintel.size.y * 0.5;
    assert!(
        (underside - (LEVEL_H_M - LINTEL_DROP_M - LINTEL_H_M * 0.5)).abs() < 1e-4,
        "the lintel's underside is {underside}, not the derived opening top"
    );
    assert!(
        underside >= 1.8,
        "the doorway is {underside} m clear — a 1.8 m player cannot walk through"
    );
    let top = lintel.offset.y + lintel.size.y * 0.5;
    assert!(
        (top - LEVEL_H_M).abs() < 1e-4,
        "the lintel's top is {top}, not the piece's {LEVEL_H_M}"
    );
    assert!(
        (lintel.size.z - door_opening_w()).abs() < 1e-4,
        "the lintel spans {}, not the opening's {}",
        lintel.size.z,
        door_opening_w()
    );
}

/// Every shape the wire can name emits something drawable: at least one part,
/// every part a real box, nothing above the piece's own top. Bounds, not
/// layout — the layout tests are the two above.
#[test]
fn every_shape_emits_a_drawable_part_set() {
    for shape in 0..=(N_SHAPES as u8) {
        let (parts, n) = shape_parts(shape);
        assert!(
            (1..=MAX_PARTS).contains(&n),
            "shape {shape} emits {n} parts"
        );
        for part in &parts[..n] {
            assert!(
                part.size.x > 0.0 && part.size.y > 0.0 && part.size.z > 0.0,
                "shape {shape} emits a degenerate box {:?}",
                part.size
            );
            let top = part.offset.y + part.size.y * 0.5;
            assert!(
                top <= LEVEL_H_M + 1e-4,
                "shape {shape} draws to {top}, above its own level"
            );
        }
    }
}

/// The stairs preview is pitched like the piece: the one part that carries a
/// non-zero pitch is the ramp, and every other shape is level. The old ghost
/// previewed the ramp as a horizontal plate — a preview of a different piece.
#[test]
fn only_the_stairs_carry_a_pitch_and_they_do_carry_one() {
    for shape in 0..(N_SHAPES as u8) {
        let (parts, n) = shape_parts(shape);
        for part in &parts[..n] {
            if shape == sim_core::build::SHAPE_STAIRS {
                assert!(
                    part.x_rot != 0.0,
                    "the stairs ramp is emitted level — the preview lies about the piece"
                );
            } else {
                assert_eq!(part.x_rot, 0.0, "shape {shape} is pitched and must not be");
            }
        }
    }
}
