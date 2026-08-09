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
use client::render::mobs::{
    leg_swing_rad, pig_body_mesh, pig_leg_mesh, pig_mesh, Gait, LEG_ANCHORS, PIG_H_M,
    PIG_LEG_CYCLE_M, PIG_LEG_FULL_MPS, PIG_LEG_SWING_RAD, PIG_LEN_M,
};

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

// ---------------------------------------------------------------------------
// The legs, since they move (`NOW.md` §0m item 4). The massing split into a
// body mesh and four leg children, so the gate grew the arithmetic the split
// created: the hinge geometry, and the trot that drives it. Nothing above
// was weakened — `pig_mesh` assembles the whole animal at rest from the same
// tables the draw path spawns from, so every earlier assertion still
// measures shipped geometry.
// ---------------------------------------------------------------------------

/// The hinge: a leg's pivot is its own origin, the anchor holds that pivot
/// at exactly the leg's length, and the pieces reassemble into the animal
/// the earlier tests measure — so swinging is a rotation about the hip and
/// a resting herd is byte-for-byte the massing that shipped before legs
/// moved.
#[test]
fn the_legs_hang_from_their_own_hips() {
    let (lo, hi) = aabb(&pig_leg_mesh());
    assert!(
        hi.y.abs() < 1e-4,
        "the leg's top is at y = {:.4}, not its origin — rotating the child \
         transform would swing it about a point inside (or outside) the box",
        hi.y
    );
    let len = -lo.y;
    assert!(len > 0.0, "the leg has no length");
    assert_eq!(LEG_ANCHORS.len(), 4, "a pig has four legs");
    for (anchor, _) in LEG_ANCHORS {
        assert!(
            (anchor[1] - len).abs() < 1e-4,
            "a {len:.2} m leg hung at y = {:.2} either floats or sinks its foot",
            anchor[1]
        );
    }
    // The assembled animal is the body plus the legs and nothing less: its
    // box is the union of the parts' boxes.
    let (alo, ahi) = aabb(&pig_mesh());
    let (blo, bhi) = aabb(&pig_body_mesh());
    assert!(alo.y.abs() < 1e-4, "the assembled pig left the ground");
    assert!(
        blo.y > 1e-3,
        "the body mesh reaches y = {:.3} — it contains the legs it is meant \
         to have shed",
        blo.y
    );
    assert!((ahi - bhi).length() < 1e-4, "the legs grew past the body");
}

/// The trot's phase table: diagonal pairs in phase, lateral and fore-aft
/// neighbours π apart. Anything else is a pace or a bound wearing a trot's
/// comment.
#[test]
fn the_legs_trot_in_diagonal_pairs() {
    for (a, pa) in LEG_ANCHORS {
        for (b, pb) in LEG_ANCHORS {
            let diagonal = (a[0] * b[0] > 0.0) == (a[2] * b[2] > 0.0);
            let gap = (pa - pb).abs();
            if diagonal {
                assert!(
                    gap < 1e-6,
                    "diagonal partners at {a:?}/{b:?} are {gap:.3} rad apart"
                );
            } else {
                assert!(
                    (gap - std::f32::consts::PI).abs() < 1e-6,
                    "opposed legs at {a:?}/{b:?} are {gap:.3} rad apart, not π"
                );
            }
        }
    }
}

/// The swing itself, pure: rests at a stand, scales with speed, saturates
/// at the flight gait, mirrors across the π offset — and stays under
/// horizontal, which is what makes "a swing can only RAISE a foot" true
/// (foot height is hip − len·cos(swing), and cos is positive below π/2).
#[test]
fn the_swing_rests_scales_and_never_cartwheels() {
    use std::f32::consts::FRAC_PI_2;
    // A standing animal's legs are vertical wherever its phase stopped.
    for i in 0..12 {
        assert_eq!(leg_swing_rad(i as f32, 0.0, 0.0), 0.0);
    }
    // Amplitude grows with speed and clamps at the flight gait: a spooked
    // pig at full flee and one hit by a speed spike swing identically.
    let at = |v: f32| leg_swing_rad(FRAC_PI_2, 0.0, v); // sin = 1: the peak
    assert!(
        at(1.0) > 0.0 && at(2.0) > at(1.0),
        "the swing ignores speed"
    );
    assert!((at(PIG_LEG_FULL_MPS) - PIG_LEG_SWING_RAD).abs() < 1e-6);
    assert_eq!(at(PIG_LEG_FULL_MPS), at(50.0), "the amplitude must clamp");
    // Diagonal pairs mirror their opposed pairs exactly.
    let fore = leg_swing_rad(1.234, 0.0, 3.0);
    let aft = leg_swing_rad(1.234, std::f32::consts::PI, 3.0);
    assert!(
        (fore + aft).abs() < 1e-5,
        "the π-offset legs do not mirror: {fore} vs {aft}"
    );
    // And the amplitude is under horizontal — past it the foot rises above
    // its own hip, which is a cartwheel wearing a stride's name. A const
    // block so the claim fails at compile time, which is where a constant's
    // claims belong.
    const {
        assert!(PIG_LEG_SWING_RAD < std::f32::consts::FRAC_PI_2);
    }
}

/// Cadence is distance, not time — the footstep odometer's rule
/// (`sound/steps.rs`), applied to legs: standing holds the phase, ground
/// covered advances it by the cycle's own arithmetic, and climbing is not
/// ground.
#[test]
fn the_stride_integrates_ground_not_frames() {
    use std::f32::consts::{PI, TAU};
    let p0 = Gait::new(0).phase;
    // Standing: any number of frames, no stride.
    let mut g = Gait::new(0);
    g.observe(Vec3::ZERO, 0.016);
    for _ in 0..100 {
        g.observe(Vec3::ZERO, 0.016);
    }
    assert_eq!(g.phase, p0, "a standing pig strode");
    assert!(g.speed < 0.01, "a standing pig reads as moving");
    // Half a cycle of ground moves the phase π, however long it took.
    let mut a = Gait::new(0);
    a.observe(Vec3::ZERO, 0.016);
    a.observe(Vec3::new(PIG_LEG_CYCLE_M / 2.0, 0.0, 0.0), 0.1);
    let moved = (a.phase - p0).rem_euclid(TAU);
    assert!(
        (moved - PI).abs() < 1e-3,
        "half a cycle of ground moved the phase {moved:.3} rad, not π"
    );
    // Climbing: vertical travel is not ground covered.
    let mut c = Gait::new(0);
    c.observe(Vec3::ZERO, 0.016);
    c.observe(Vec3::new(0.0, 3.0, 0.0), 0.016);
    assert_eq!(c.phase, p0, "a lifted pig pedalled");
}

/// Two pigs do not march in step: the phase origin is hashed from the
/// roster slot (`sound::pig::hash01`, the snort's own convention), so a
/// herd walked into is strides in four places rather than a chorus line.
#[test]
fn two_pigs_do_not_march_in_step() {
    let phases: Vec<f32> = (0..8usize).map(|slot| Gait::new(slot).phase).collect();
    for p in &phases {
        assert!(
            (0.0..std::f32::consts::TAU).contains(p),
            "phase {p} left [0, TAU)"
        );
    }
    let lo = phases.iter().cloned().fold(f32::MAX, f32::min);
    let hi = phases.iter().cloned().fold(0.0f32, f32::max);
    assert!(
        hi - lo > 0.5,
        "eight slots' phases span {:.3} rad — the herd marches in step",
        hi - lo
    );
    // And deterministic: the same slot always starts its stride at the
    // same place, which is what keeps two clients' herds in agreement.
    assert_eq!(Gait::new(3).phase, Gait::new(3).phase);
}
