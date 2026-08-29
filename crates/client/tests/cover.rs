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
use client::render::clutter::{element_mesh, FRONDS_PER_CLUMP, FROND_H, FROND_TIP_GAIN, TUFT_H};
use client::render::terrain_mesh::GROUND_ALBEDO;
use sim_core::terrain::{Clutter, ClutterElem};

/// The channels `clutter_richness_at` counts into `grow`, in its own order:
/// splat channel 1 is grass and 2 is forest litter, and `Clutter`'s
/// discriminants are `kind as usize - 1` into the same weights.
const GROWING: [Clutter; 2] = [Clutter::Tuft, Clutter::Twig];
/// The two it refuses to thicken: sand and rock.
const INERT: [Clutter; 2] = [Clutter::Pebble, Clutter::Shard];

/// The litter identity's index into `GROUND_ALBEDO` / the splat.
const LITTER: usize = 2;

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

fn colors(m: &Mesh) -> Vec<[f32; 4]> {
    match m.attribute(Mesh::ATTRIBUTE_COLOR) {
        Some(VertexAttributeValues::Float32x4(v)) => v.clone(),
        _ => panic!("clutter mesh has float4 colours"),
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

/// **The gate this file exists for.** Every element of a channel `sim-core`
/// thickens must stand taller than every element of a channel it does not.
///
/// Stated as a separation rather than as an absolute height on purpose: there
/// is no measured litter height to hold anyone to. `ART.md` §1's 20–40 cm band
/// is quoted about grass and §3 has no litter row at all, so an absolute floor
/// here would be a number invented in a test and then defended as if it had
/// been measured. The separation is not invented — `sim-core` already decided
/// which channels grow, and this only asks the meshes to agree.
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
             {tallest_inert:.4} m for the tallest thing sand or rock draws — \
             `clutter_richness_at` thickens this channel because it GROWS, and \
             the mesh it thickens is no taller than gravel"
        );
    }

    // And the other direction, so the gate cannot be satisfied by standing
    // everything up: a channel the sim refuses to thicken must stay flat.
    for kind in INERT {
        let tallest = swept(kind).iter().copied().fold(0.0f32, f32::max);
        assert!(
            tallest < FROND_H * 0.5,
            "{kind:?} stands {tallest:.4} m — sand and rock do not grow things \
             and must not sprout"
        );
    }
}

/// The clump keeps its fallen half. `Clutter::Twig`'s definition in `sim-core`
/// is "fallen needles, sticks, cones" and that reading was never wrong — a
/// forest floor IS mostly fallen matter. What it lacked was anything standing
/// in the fallen matter, so the fix adds and does not replace.
///
/// The stick is identified the way `tests/contact.rs` identifies it: the first
/// four triangles, sunk below the ground plane by `CHIP_SINK`.
#[test]
fn the_litter_clump_still_has_the_stick_it_always_had() {
    let e = elem(Clutter::Twig, 77, 1.0);
    let m = element_mesh(&e);
    let p = positions(&m);
    assert_eq!(
        p.len(),
        12 + FRONDS_PER_CLUMP as usize * 6,
        "a litter clump is a chip plus its standing stalks"
    );
    let sunk = p[..12].iter().filter(|v| v.y < e.y).count();
    assert!(
        sunk >= 4,
        "only {sunk} of the stick's vertices sit below the ground it was \
         placed on — the fallen half stopped being a chip"
    );
    let stalk_lo = p[12..].iter().fold(f32::INFINITY, |a, v| a.min(v.y));
    assert!(
        stalk_lo >= e.y - 1e-6,
        "a stalk roots {:.4} m below the ground plane — standing litter grows \
         out of the surface, it is not pressed into it like the stick",
        e.y - stalk_lo
    );
}

/// **Standing litter is the colour of the ground it grew out of.**
///
/// Every other clutter colour in `clutter.rs` is a hex authored beside the
/// ground rather than from it (`PEBBLE_C`, `TWIG_C`, `SHARD_C`, the tuft ramp),
/// so a shard rolled off the rock channel is painted `0x7d7a73` while the rock
/// under it is `GROUND_ALBEDO[3]` — and nothing in the tree measures that gap.
/// `terrain.rs` argues the population cannot drift from its surface because
/// "THE MIX IS THE SPLAT"; that argument covers WHICH kind is drawn where and
/// does not reach what colour it is painted.
///
/// The stalk closes the seam for one kind by construction, and this holds it
/// closed. It is also why no new hex was authored: `ART.md` §3 has no litter
/// row to author one against, and `GROUND_ALBEDO[2]` already carries that
/// identity's hue and saturation under `tests/ground_identity.rs`.
#[test]
fn a_standing_stalk_is_rooted_in_the_grounds_own_litter_albedo() {
    let e = elem(Clutter::Twig, 12, 1.0);
    let m = element_mesh(&e);
    let (p, c) = (positions(&m), colors(&m));
    let want = GROUND_ALBEDO[LITTER];

    // The root vertices of the stalks: the quad corners sitting on the ground
    // plane. Their ramp parameter is 0, so their colour is the ramp's low end
    // times that stalk's own value jitter — the DIRECTION must be the ground's
    // litter albedo exactly, and only the magnitude may vary.
    let mut checked = 0;
    for i in 12..p.len() {
        if (p[i].y - e.y).abs() > 1e-6 {
            continue;
        }
        let got = [c[i][0], c[i][1], c[i][2]];
        // Scale-invariant comparison: the jitter is one scalar on all three
        // channels, so the ratios between channels must survive it exactly.
        let k = got[1] / want[1];
        for ch in 0..3 {
            let d = (got[ch] - want[ch] * k).abs();
            assert!(
                d < 1e-5,
                "stalk root vertex {i} is {got:?}, which is not {want:?} times \
                 any scalar (channel {ch} off by {d:e}) — the standing litter \
                 has been given a colour of its own and can now drift from the \
                 ground it grows out of"
            );
        }
        checked += 1;
    }
    assert!(
        checked >= FRONDS_PER_CLUMP as usize * 2,
        "only {checked} stalk root vertices found — the sample is too thin to \
         hold the claim"
    );
}

/// The two shape constants, held against the one measured neighbour they have.
///
/// A `const` block, so this refuses at COMPILE time — the idiom
/// `tests/contact.rs` already uses for `CHIP_VOLUME_BLEND`, and the stronger
/// gate: the constants cannot be edited out of range in any build, including
/// one nobody runs.
#[test]
fn standing_litter_is_understory_and_not_a_second_lawn() {
    const {
        assert!(
            FROND_H > TUFT_H * 0.25 && FROND_H < TUFT_H,
            "standing litter is shorter than turf — it is debris under a \
             canopy, not grass — but a quarter of a blade is a sliver again"
        )
    };
    const {
        assert!(
            FRONDS_PER_CLUMP > 0,
            "a litter clump with no stalks is the flat chip this file exists \
             to retire"
        )
    };
    // `ART.md` §5's `ALBEDO_LUMA_BAND = [0.05, 0.55]` linear, applied to the
    // brightest thing the stalk ramp can produce. The root is `GROUND_ALBEDO`,
    // which `tests/ground_identity.rs` already holds inside the band; the tip
    // is the root times the gain and nothing was checking it.
    let root = GROUND_ALBEDO[LITTER];
    let tip = 0.2126 * root[0] * FROND_TIP_GAIN
        + 0.7152 * root[1] * FROND_TIP_GAIN
        + 0.0722 * root[2] * FROND_TIP_GAIN;
    assert!(
        (0.05..=0.55).contains(&tip),
        "a stalk tip's albedo luma is {tip:.4}, outside ART.md §5's \
         ALBEDO_LUMA_BAND [0.05, 0.55]"
    );
    const {
        assert!(
            FROND_TIP_GAIN > 1.0,
            "the tip must be lighter than the root — a stalk that is one value \
             is rule 1's flat surface at stalk scale"
        )
    };
}

/// Wall 4, at this seam: the growing channels must stay bounded against the
/// budget the tile already carried.
///
/// A tile is capped at `CLUTTER_TILE_CAP` elements and baked into ONE mesh, so
/// the cost of standing the litter channel up is (elements × the vertices each
/// one adds). Grass has always been the expensive channel — seven blades — and
/// the bound that matters is that litter did not quietly become worse than the
/// thing that was already the ceiling.
#[test]
fn a_litter_clump_costs_less_than_the_tuft_that_was_already_the_ceiling() {
    let n = |k: Clutter| positions(&element_mesh(&elem(k, 33, 1.0))).len();
    let (tuft, clump) = (n(Clutter::Tuft), n(Clutter::Twig));
    assert!(
        clump < tuft,
        "a litter clump is {clump} vertices against a tuft's {tuft} — the \
         channel that covers 93% of the capture spawn is now the most \
         expensive one on the island"
    );
    // And it is not free either, which is the whole point of the change.
    assert!(
        clump > n(Clutter::Pebble),
        "a litter clump costs no more than a pebble, so nothing was added"
    );
}
