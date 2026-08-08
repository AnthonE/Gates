//! Gate: the pig the client draws is the animal the sim is moving.
//!
//! There is no visual gate in this repo and this is not one (`CLAUDE.md`:
//! a pixel statistic cannot see whether the frame is a picture of
//! anything, and ours proved it). What may be gated about a frame is
//! **arithmetic**, which is the shape `tests/tree.rs` establishes, and a
//! box massing has three arithmetic facts that are not taste:
//!
//! 1. **Its origin is its feet.** The wire's `qy` is the capsule's feet —
//!    `bodies.rs` records the metre of float that assuming otherwise cost
//!    once already — so a massing authored around its own centre would
//!    bury or levitate every pig on the island with every other gate green.
//! 2. **It faces +Z.** Yaw 0 is +Z in the sim (`yaw_lut.rs`), the heading
//!    the animal walks along is the yaw the snapshot carries, and a mesh
//!    modelled facing −Z or +X would walk backwards or sideways forever
//!    while looking, frame by frame, like a bug in the interpolator.
//! 3. **It is the size it claims.** `PIG_LEN_M`/`PIG_H_M` are prose until
//!    something measures the vertices, and a pig that grew to elk scale on
//!    a taste pass is a gameplay change (what you can see over, what a
//!    spook radius means) wearing an art change's clothes.
//!
//! Headless: geometry is countable, so no GPU, no window and no shard.

#![cfg(feature = "render")]

use bevy::prelude::*;
use client::render::mobs::{pig_mesh, PIG_H_M, PIG_LEN_M};

/// Every position in the mesh, as an axis-aligned box.
fn aabb(mesh: &Mesh) -> (Vec3, Vec3) {
    let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("the pig has no positions");
    };
    assert!(!p.is_empty(), "the pig has no vertices");
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for v in p {
        let v = Vec3::from_array(*v);
        lo = lo.min(v);
        hi = hi.max(v);
    }
    (lo, hi)
}

#[test]
fn the_pig_stands_on_its_own_origin() {
    let (lo, hi) = aabb(&pig_mesh());
    assert!(
        lo.y.abs() < 1e-4,
        "the massing's lowest vertex is at y = {:.4}, not 0 — the wire's y IS \
         the feet (`bodies.rs`), so this pig is {} the ground",
        lo.y,
        if lo.y > 0.0 {
            "floating above"
        } else {
            "sunk into"
        }
    );
    assert!(hi.y > 0.0, "the pig has no height");
}

#[test]
fn the_pig_is_the_size_it_claims() {
    let (lo, hi) = aabb(&pig_mesh());
    let len = hi.z - lo.z;
    let height = hi.y - lo.y;
    // 5 cm of tolerance on numbers authored to the centimetre: enough that
    // a snout tweak does not redden the gate, far too little to let the
    // animal change scale.
    assert!(
        (len - PIG_LEN_M).abs() < 0.05,
        "nose to tail measures {len:.3} m against a declared {PIG_LEN_M} m"
    );
    assert!(
        (height - PIG_H_M).abs() < 0.05,
        "shoulder measures {height:.3} m against a declared {PIG_H_M} m"
    );
    // A player's eye is at 1.6 m (`render::EYE_HEIGHT`). An animal you look
    // DOWN at is most of why this reads as an animal at all, and it is the
    // one relation between the two numbers that gameplay depends on.
    assert!(
        height < client::render::EYE_HEIGHT,
        "a {height:.2} m pig is taller than the player's eye"
    );
    // Wider than it is tall and longer than it is wide: a quadruped's
    // proportions, which is what stops a cube passing every other check
    // here.
    let width = hi.x - lo.x;
    assert!(
        len > width && width < height,
        "the massing is not a quadruped's box"
    );
}

#[test]
fn the_pig_faces_plus_z() {
    let (lo, hi) = aabb(&pig_mesh());
    // The snout is the front-most geometry and the tail the rear-most; the
    // body is centred, so the head's overhang past centre must be on +Z.
    assert!(
        hi.z > -lo.z,
        "the massing's mass runs toward −Z ({:.2} forward vs {:.2} back) — \
         yaw 0 is +Z in the sim, so this pig walks tail-first",
        hi.z,
        -lo.z
    );
    // And the head is narrow where the body is wide: measured as the
    // vertex spread in the front fifth against the middle fifth, which is
    // a fact about a snout and cannot be satisfied by a symmetric blob.
    let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) =
        pig_mesh().attribute(Mesh::ATTRIBUTE_POSITION).cloned()
    else {
        unreachable!("checked above");
    };
    let spread = |from: f32, to: f32| {
        p.iter()
            .filter(|v| v[2] >= from && v[2] <= to)
            .map(|v| v[0].abs())
            .fold(0.0f32, f32::max)
    };
    let nose = spread(hi.z - (hi.z - lo.z) * 0.2, hi.z);
    let middle = spread(-0.1, 0.1);
    assert!(
        nose < middle,
        "the front of the animal ({nose:.2} m half-width) is not narrower than \
         its middle ({middle:.2} m) — that is a barrel, not a head"
    );
}

/// The pig carries its own albedo, not a mean-1 modulation of a texture it
/// does not have.
///
/// **This is the one that shipped wrong.** `props::boxes_mesh` runs the
/// authored hex through `tint1`, which divides by its own luma so the
/// vertex value modulates a photograph — correct for the shelter, and on a
/// material with no map behind it it renders a **near-white** animal. It
/// took a capture to notice, because no gate in this repo looks at a
/// colour and no assertion above this one can tell a brown pig from a pale
/// one. This can: a mean-1 field averages to 1 by construction, so a mean
/// materially below 1 is proof the massing kept its own colour.
#[test]
fn the_pig_is_not_a_ghost() {
    let mesh = pig_mesh();
    let Some(bevy::mesh::VertexAttributeValues::Float32x4(c)) =
        mesh.attribute(Mesh::ATTRIBUTE_COLOR)
    else {
        panic!("the pig has no vertex colours at all");
    };
    let n = c.len() as f32;
    let mean: f32 = c.iter().map(|v| (v[0] + v[1] + v[2]) / 3.0).sum::<f32>() / n;
    assert!(
        mean < 0.5,
        "the pig's mean vertex colour is {mean:.3} — a mean-1 tint (`tint1`) \
         wearing no texture, which draws a white animal"
    );
    // And it is a warm brown rather than a grey: red above blue on every
    // vertex is what says "an animal" before any of the geometry does.
    assert!(
        c.iter().all(|v| v[0] > v[2]),
        "some of the massing is cooler than it is warm — this is a pig, not a rock"
    );
}
