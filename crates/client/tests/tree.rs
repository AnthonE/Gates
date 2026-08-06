//! Gate: the generated conifer fits the volume the sim blocks, and the forest
//! it makes fits the frame's triangle budget.
//!
//! **The load-bearing assertion is the first one, and it is not about art.**
//! `world.rs` derives `SPAWN_CLEAR_M = 4.0` from a sentence about the widest
//! thing the client draws — "the tree cone at radius <= 1.7 m × 1.1 max scale
//! ≈ 1.9 m, plus the 0.4 m capsule". When the pine was authored by hand that
//! sentence was checked by reading the constants. It is now the output of a
//! GENERATOR: nobody authored these vertices, the shape moves when any of a
//! dozen parameters moves, and a canopy that grew past the ceiling would put
//! fresh spawns back inside trees with every other gate green. `ci/
//! pine_shape.mjs` closes exactly this arithmetic for the browser and cannot
//! see across the language boundary; this closes it on our side.
//!
//! It runs headless — no GPU, no window, no shard. The generator is a pure
//! function and geometry is countable, which is the same reason
//! `ci/pine_shape.mjs` is arithmetic rather than a screenshot.

#![cfg(feature = "render")]

use bevy::prelude::*;
use client::render::props::{PINE_H, PINE_MAX_R};
use client::render::tree::{
    bounds, conifer, min_y, needle_image, tris, CONIFER_MAX_TRIS, CONIFER_POOL,
};

/// Trees inside the client's 5×5×64 m prop ring, p90 over 100 eye positions
/// sampled across the island off `sim_core::terrain::scatter`. Recorded here
/// because the budget assertion below is meaningless without it.
const RING_TREES_P90: usize = 328;

#[test]
fn every_variant_fits_the_volume_the_sim_blocks() {
    for v in 0..CONIFER_POOL {
        let (bark, needles) = conifer(v);
        let (h, r) = bounds(&[&bark, &needles]);
        assert!(
            r <= PINE_MAX_R,
            "variant {v} canopy reaches {r:.3} m, past PINE_MAX_R {PINE_MAX_R} — \
             world.rs derives SPAWN_CLEAR_M from that ceiling, so this puts \
             fresh spawns inside trees"
        );
        // Height is normalised, so this is an equality within float slop and
        // not a range. A tree that is not PINE_H is a tree whose fell pivot,
        // wind band and stump lift are all measured against the wrong number.
        assert!(
            (h - PINE_H).abs() < 1e-3,
            "variant {v} is {h:.3} m tall, expected PINE_H {PINE_H}"
        );
    }
}

#[test]
fn every_variant_is_rooted_at_the_ground() {
    // `spawn_slot` places the mesh origin at the slot's surface y and applies
    // no lift to a tree, so the TRUNK's base is what has to be 0: a bark mesh
    // that starts anywhere else floats or sinks every tree in the world at
    // once, by exactly that much.
    //
    // **The needles are held to a different rule on purpose.** Foliage that
    // dips below the trunk's base is not a defect — `ART.md` rule 2 is that
    // nothing sits ON the ground and everything sits IN it, and the lowest
    // branch's cards grazing the surface is that rule, not a violation of it.
    // What would be a defect is a canopy hanging far enough under to bury
    // whole cards, which is geometry drawn for nobody. Measured at −0.0011 m
    // with the shipped parameters; the bound is one card's half-height.
    const NEEDLE_DIP_MAX_M: f32 = 0.35;
    for v in 0..CONIFER_POOL {
        let (bark, needles) = conifer(v);
        let trunk = min_y(&[&bark]);
        assert!(
            trunk.abs() < 1e-3,
            "variant {v}'s trunk starts at y={trunk:.4}, not 0 — every tree would float or sink"
        );
        let canopy = min_y(&[&needles]);
        assert!(
            canopy > -NEEDLE_DIP_MAX_M,
            "variant {v}'s canopy hangs {:.3} m below the trunk base, past \
             {NEEDLE_DIP_MAX_M} m — that is cards buried in the terrain",
            -canopy
        );
    }
}

#[test]
fn the_pool_is_three_distinct_silhouettes() {
    // `ART.md` rule 7: no two identical instances adjacent. Yaw and scale
    // vary per slot, but at the measured p90 of 328 trees in the draw ring one
    // silhouette repeated 328 times reads as one silhouette repeated.
    let shapes: Vec<(f32, f32)> = (0..CONIFER_POOL)
        .map(|v| {
            let (bark, needles) = conifer(v);
            bounds(&[&bark, &needles])
        })
        .collect();
    for i in 0..shapes.len() {
        for j in (i + 1)..shapes.len() {
            assert!(
                (shapes[i].1 - shapes[j].1).abs() > 1e-4,
                "variants {i} and {j} have the same canopy radius {:.5} — \
                 the pool is not seeding distinct trees",
                shapes[i].1
            );
        }
    }
}

#[test]
fn one_tree_stays_inside_its_triangle_ceiling() {
    for v in 0..CONIFER_POOL {
        let (bark, needles) = conifer(v);
        let t = tris(&bark) + tris(&needles);
        assert!(
            t <= CONIFER_MAX_TRIS,
            "variant {v} is {t} tris, past CONIFER_MAX_TRIS {CONIFER_MAX_TRIS}"
        );
    }
}

/// **This test states a known-over-budget condition rather than asserting it
/// away, and that is deliberate.**
///
/// `DESIGN.md` §9 allows 1.5 M triangles a frame. A full-detail conifer at the
/// ring's p90 does not fit and no parameter change makes it fit — measured, by
/// distance band: 40 m holds 21 trees (~89 k tris), 80 m holds 82 (~350 k),
/// 120 m holds 168 (~709 k), 160 m holds 288 (~1.22 M). The answer is the
/// billboard LOD `TERRAIN.md` §4 queues, which this slice does not build.
///
/// So the gate asserts the thing that is actually true today — the near band
/// is affordable — and prints the number that is not, so the debt cannot be
/// forgotten by anyone reading a green suite.
#[test]
fn the_near_band_is_affordable_and_the_ring_is_not() {
    let (bark, needles) = conifer(0);
    let per_tree = tris(&bark) + tris(&needles);

    // The near band: 80 m, p90 82 trees. This is what ships today.
    let near = per_tree * 82;
    assert!(
        near < 600_000,
        "the 80 m band costs {near} tris at {per_tree}/tree — past the ~600 k \
         a forest can have of DESIGN §9's 1.5 M"
    );

    let ring = per_tree * RING_TREES_P90;
    println!(
        "conifer {per_tree} tris/tree · 80 m band {near} tris (OK) · \
         full {RING_TREES_P90}-tree ring {ring} tris (OVER 1.5 M — needs the \
         billboard LOD, TERRAIN.md §4)"
    );
    assert!(
        ring > 1_000_000,
        "the ring got cheap ({ring} tris) — if that is real, re-measure the \
         budget claim in tree.rs and RENDER.md rather than deleting this"
    );
}

#[test]
fn the_needle_card_is_actually_cut_out() {
    // An `AlphaMode::Mask` material with a fully opaque map is an opaque quad,
    // which is the hull `props.js` spent three passes rejecting — and it would
    // look like a solid green square, not like an error. So: the map must have
    // real transparency, and it must have real coverage.
    let img = needle_image();
    let data = img.data.as_ref().expect("needle image has no data");
    let alphas: Vec<u8> = data.chunks_exact(4).map(|p| p[3]).collect();
    let opaque = alphas.iter().filter(|&&a| a > 128).count();
    let clear = alphas.iter().filter(|&&a| a < 16).count();
    let total = alphas.len();

    assert!(
        clear * 100 / total > 30,
        "only {}% of the needle card is transparent — that is a quad, not a sprig",
        clear * 100 / total
    );
    assert!(
        opaque * 100 / total > 5,
        "only {}% of the needle card is opaque — the canopy would be invisible",
        opaque * 100 / total
    );
}
