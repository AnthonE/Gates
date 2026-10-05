//! **The growing channels stand up, and the ones that do not grow stay flat.**
//!
//! `sim-core` publishes a density law that names two of the four ground
//! identities as living. `clutter_richness_at`'s own doc says *"Grass
//! (channel 1) and forest litter (channel 2) are the ground identities that
//! grow things; sand and rock do not thicken, they stay at the one element
//! the coverage stratum already put there"*, and the code does exactly that:
//! `let grow = w[1] as u32 + w[2] as u32`. So the sim thickens the
//! population on litter for the stated reason that litter grows things.
//!
//! The client then drew every one of those extra elements with `chip()` at
//! 16 × 2.2 × 3 cm. That is an aspect ratio of 7.3 and the flattest mesh in
//! `render/clutter.rs` — the same builder sand and rock get. One side of the
//! seam said *understory*; the other said *gravel*.
//!
//! It is not an abstract mismatch. `NOW.md` §0gp measured the capture camera
//! standing on **93 % litter / 7 % sand, with grass and granite at exactly
//! zero within 60 m**, so on the frames the visual judge actually scores,
//! essentially every clutter element was a 2.2 cm sliver. The judge read it
//! back as *"`4-near` is entirely close ground with flat twig decals and not
//! one 3D clutter mesh"* (`pass-20260815-042118-06-visual.md` gap 1).
//!
//! What this file gates is the correspondence itself, in both directions: a
//! channel `sim-core` calls growing must draw standing geometry, and a channel
//! it calls inert must not. That is a claim about arithmetic, not about a
//! picture — no frame was opened by the pass that wrote it, and none is
//! needed to check it.
//!
//! It does NOT gate that the result looks right. Nobody has seen it.

#![cfg(feature = "render")]

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use client::render::clutter::{element_mesh, fern_at, BRUSH_H, CARDS_PER_TUFT, FERN_H, TUFT_H};
use sim_core::terrain::{Clutter, ClutterElem};

/// The growing channels' standing kinds: channel 1's tuft (channel 2's litter
/// stands up as a fern where one grows — `litter_grid`).
const GROWING: [Clutter; 1] = [Clutter::Tuft];
/// The two it refuses to thicken: sand and rock.
const INERT: [Clutter; 2] = [Clutter::Pebble, Clutter::Shard];

fn elem(kind: Clutter, yaw: u8, scale: f32) -> ClutterElem {
    ClutterElem {
        kind,
        x: -12.0,
        y: 0.0,
        z: 41.0,
        yaw,
        scale,
    }
}

fn positions(m: &Mesh) -> Vec<Vec3> {
    match m.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(v)) => {
            v.iter().map(|p| Vec3::from_array(*p)).collect()
        }
        _ => panic!("clutter mesh has float3 positions"),
    }
}

/// How tall an element's whole mesh stands, ground plane to highest vertex.
///
/// Measured from the element's own placement height rather than from its
/// lowest vertex, because a chip sinks below the ground it is placed on
/// (`CHIP_SINK`) and burying a mesh deeper is not the same as standing it up.
fn stands(kind: Clutter, yaw: u8, scale: f32) -> f32 {
    let e = elem(kind, yaw, scale);
    let m = element_mesh(&e);
    positions(&m).iter().fold(0.0f32, |a, v| a.max(v.y - e.y))
}

/// A sweep across yaw and the element hash. Scale is held at 1.0 deliberately:
/// `ClutterElem::scale` spans [0.75, 1.25], so comparing a small tuft against a
/// large shard would measure the scale draw rather than the kind, and the claim
/// under test is about the KIND.
fn swept(kind: Clutter) -> Vec<f32> {
    (0..64).map(|i| stands(kind, (i * 4) as u8, 1.0)).collect()
}

/// Litter elements over a grid, split by whether they carry a fern.
fn litter_grid() -> (Vec<ClutterElem>, Vec<ClutterElem>) {
    let (mut fern, mut bare) = (Vec::new(), Vec::new());
    for i in 0..60 {
        for j in 0..60 {
            let e = ClutterElem {
                kind: Clutter::Twig,
                x: i as f32 * 1.37 - 40.0,
                y: 0.0,
                z: j as f32 * 1.21 + 10.0,
                yaw: ((i * 7 + j * 13) % 256) as u8,
                scale: 1.0,
            };
            if fern_at(&e) {
                fern.push(e);
            } else {
                bare.push(e);
            }
        }
    }
    (fern, bare)
}

fn height_of(e: &ClutterElem) -> f32 {
    positions(&element_mesh(e))
        .iter()
        .fold(0.0f32, |a, v| a.max(v.y - e.y))
}

/// **The gate this file exists for.** What a growing channel stands up must
/// stand taller than anything sand or rock draws: every tuft, and every fern
/// the litter grows — and the inert channels stay flat.
///
/// Stated as a separation rather than as an absolute height on purpose: there
/// is no measured litter height to hold anyone to.
#[test]
fn every_growing_channel_stands_and_every_inert_one_does_not() {
    let tallest_inert = INERT.iter().flat_map(|k| swept(*k)).fold(0.0f32, f32::max);
    assert!(
        tallest_inert > 0.0,
        "the inert kinds drew nothing at all — this gate would pass vacuously"
    );

    for kind in GROWING {
        let s = swept(kind);
        let shortest = s.iter().copied().fold(f32::INFINITY, f32::min);
        assert!(
            shortest > tallest_inert,
            "{kind:?}: its shortest element stands {shortest:.4} m against \
             {tallest_inert:.4} m for the tallest thing sand or rock draws"
        );
    }
    let (ferns, _) = litter_grid();
    let shortest_fern = ferns.iter().map(height_of).fold(f32::INFINITY, f32::min);
    assert!(
        shortest_fern > tallest_inert,
        "the shortest fern stands {shortest_fern:.4} m against {tallest_inert:.4} m \
         for the tallest thing sand or rock draws"
    );

    for kind in INERT {
        let tallest = swept(kind).iter().copied().fold(0.0f32, f32::max);
        assert!(
            tallest < TUFT_H * 0.25,
            "{kind:?} stands {tallest:.4} m — sand and rock do not grow things \
             and must not sprout"
        );
    }
}

/// Ferns grow in colonies: some litter carries one and most does not, and the
/// rest is the fallen stick `Clutter::Twig` always was — flat, never a spike.
#[test]
fn ferns_grow_in_colonies_and_the_rest_of_the_litter_lies_down() {
    let (ferns, bare) = litter_grid();
    let share = ferns.len() as f32 / (ferns.len() + bare.len()) as f32;
    assert!(
        (0.05..=0.5).contains(&share),
        "{:.1}% of litter carries a fern — outside 5–50% it is either no \
         understory or a fern carpet",
        share * 100.0
    );
    for e in bare.iter().take(200) {
        let h = height_of(e);
        assert!(
            h < TUFT_H * 0.25,
            "a bare litter element stands {h:.3} m — the standing stalks were \
             the brown spikes in every frame"
        );
    }
}

/// The litter keeps its fallen half, fern or not: the first four triangles are
/// the stick, sunk below the ground plane by `CHIP_SINK`, and a fern stands on
/// the ground rather than in it.
#[test]
fn the_litter_clump_still_has_the_stick_it_always_had() {
    let (ferns, bare) = litter_grid();
    for e in [&ferns[0], &bare[0]] {
        let p = positions(&element_mesh(e));
        let want = if fern_at(e) {
            12 + CARDS_PER_TUFT as usize * 6
        } else {
            12
        };
        assert_eq!(p.len(), want, "a litter element is a chip plus its fern");
        let sunk = p[..12].iter().filter(|v| v.y < e.y).count();
        assert!(
            sunk >= 4,
            "only {sunk} of the stick's vertices sit below the ground it was \
             placed on — the fallen half stopped being a chip"
        );
    }
}

#[test]
fn a_fern_is_understory_and_not_a_shrub() {
    const {
        assert!(
            FERN_H > TUFT_H * 0.5 && FERN_H < BRUSH_H,
            "a fern clump stands between the turf and the brush layer"
        )
    };
}

/// Wall 4, at this seam: the growing channels must stay bounded against the
/// budget a near-ground element was accepted at.
///
/// **42, and where it comes from**: `BLADES_PER_TUFT` was 7 and a blade is two
/// triangles — the blade path is deleted, so it is written out here.
const BLADE_TUFT_VERTS: usize = 42;

#[test]
fn the_growing_channels_stay_under_the_budget_that_was_accepted() {
    let (ferns, _) = litter_grid();
    let tuft = positions(&element_mesh(&elem(Clutter::Tuft, 33, 1.0))).len();
    let clump = positions(&element_mesh(&ferns[0])).len();
    assert!(
        clump <= BLADE_TUFT_VERTS,
        "a litter clump with its fern is {clump} vertices against the \
         {BLADE_TUFT_VERTS} accepted for a near-ground element"
    );
    assert!(
        tuft < BLADE_TUFT_VERTS,
        "a tuft is {tuft} vertices against the {BLADE_TUFT_VERTS} the blades \
         cost"
    );
}
