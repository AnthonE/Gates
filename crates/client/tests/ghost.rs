//! The build ghost tells the truth about the piece it becomes.
//!
//! **What this asserts, and what it deliberately does not.** `render/ghost.rs`
//! and `render/structures.rs` both draw a doorway, from one shared set of
//! dimensions but two separate emit sites. Asserting "the ghost's table equals
//! the kit's table" would be the byte-golden hole `CLAUDE.md`'s trap list
//! names — both could be wrong together and stay green, which is how 27 Oxide
//! payload fixes shipped under a pinned `MSILHash`.
//!
//! So the assertion is against the **sim**: the posts the client draws stand
//! where `sim_core::collide::doorway_solid_at` says a player is blocked, and
//! the gap is where it says they are not. `RENDER.md` §8 states the stake — the
//! opening is 1.2 m x 2.1 m *because* that is what `edge_hit` refuses, and
//! "draw it elsewhere and the frame lies about where a player can walk".
//!
//! Headless, no GPU, no socket; it links `render` for the shared constants.

use client::render::structures::{
    door_opening_w, door_post_gap, LINTEL_DROP_M, LINTEL_H_M, SEAM_M,
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
