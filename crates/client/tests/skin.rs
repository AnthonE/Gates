//! A blow lands on what is drawn (`render/skin.rs`): a contact on the sim's
//! fat cylinder moves onto the mesh inside it, and a mark fitted there lies on
//! that mesh rather than on the cylinder.

#![cfg(feature = "render")]

use bevy::math::primitives::Sphere;
use bevy::prelude::*;
use client::render::decal::{Kind, Marks, MESH_MARK_LIFT_M};
use client::render::impact::Matter;
use client::render::skin::{ray_mesh, snap, Patch, Skin};

/// A 0.6 m ball standing 0.5 m up at (10, 0, 10): a rock the sim blocks as a
/// 0.9 m cylinder.
fn rock() -> (Mesh, GlobalTransform) {
    let mesh = Sphere::new(0.6).mesh().ico(4).unwrap();
    let tf = GlobalTransform::from(Transform::from_xyz(10.0, 0.5, 10.0));
    (mesh, tf)
}

#[test]
fn a_ray_meets_the_drawn_ball_at_its_skin_facing_back() {
    let (mesh, tf) = rock();
    let o = Vec3::new(8.0, 0.5, 10.0);
    let h = ray_mesh(&mesh, &tf, o, Vec3::X, 5.0).expect("the ray crosses the ball");
    assert!((h.at.x - 9.4).abs() < 0.01, "entered at {}", h.at.x);
    assert!(h.normal.dot(-Vec3::X) > 0.98, "normal {:?}", h.normal);
    assert!(
        ray_mesh(&mesh, &tf, o, Vec3::X, 1.0).is_none(),
        "past max_t"
    );
    assert!(ray_mesh(&mesh, &tf, o, -Vec3::X, 5.0).is_none(), "behind");
}

#[test]
fn a_contact_on_the_cylinders_cap_moves_onto_the_ball() {
    let (mesh, tf) = rock();
    let skin = Skin { tree: false };
    let e = Entity::from_raw_u32(7).unwrap();
    // Where the sim's 0.9 m cylinder, 1.1 m tall, takes a blow from above at
    // its rim: half a metre of air over the ball's shoulder.
    let at = Vec3::new(10.7, 1.1, 10.0);
    let cands = [(e, &skin, &mesh, &tf)];
    let (got, h) = snap(at, None, cands.iter().copied()).expect("the ball is in reach");
    assert_eq!(got, e);
    let r = h.at.distance(Vec3::new(10.0, 0.5, 10.0));
    assert!((r - 0.6).abs() < 0.01, "snapped to {r} m from the centre");
    // From the swinger's eye the ray goes where they looked at the picture.
    let eye = Vec3::new(12.0, 1.6, 10.0);
    let (_, h) = snap(at, Some(eye), cands.iter().copied()).expect("seen");
    let r = h.at.distance(Vec3::new(10.0, 0.5, 10.0));
    assert!((r - 0.6).abs() < 0.01, "eye snap {r} m from the centre");
    // Something two metres off the ball is some other surface.
    assert!(snap(Vec3::new(12.6, 0.5, 10.0), None, cands.iter().copied()).is_none());
}

#[test]
fn a_fitted_mark_lies_on_the_ball_not_its_tangent_plane() {
    let (mesh, tf) = rock();
    let centre = Vec3::new(10.0, 0.5, 10.0);
    let at = centre + Vec3::new(-0.6, 0.0, 0.0);
    let mut pool = Marks::default();
    let ix = pool.place(at, -Vec3::X, Kind::ChipStone, 0.5, Matter::Stone, 0.0);
    let mut patch = Patch::default();
    patch.gather(&mesh, &tf, at, 1.0);
    pool.conform(ix, &mut |o, d, max| patch.cast(o, d, max));
    for (p, n, _, c, t) in pool.vertices(ix) {
        let p = Vec3::from_array(p);
        if c[3] == 0.0 {
            continue;
        }
        let r = p.distance(centre);
        assert!(
            (r - 0.6 - MESH_MARK_LIFT_M).abs() < 0.012,
            "a vertex {r} m out, the ball is 0.6"
        );
        let radial = (p - centre).normalize();
        assert!(
            Vec3::from_array(n).dot(radial) > 0.95,
            "normal off the ball"
        );
        assert!(Vec3::from_slice(&t[..3]).dot(Vec3::from_array(n)).abs() < 0.05);
    }
}
