//! Gate: ore nodes as they are mined (`render/ore.rs`), driven as arithmetic.

#![cfg(feature = "render")]

use bevy::math::{Quat, Vec3};
use bevy::prelude::{Handle, Transform};
use client::render::ore::{
    pose_transform, stage_pose, Debris, OreKind, OreNode, Throw, MESH_HALF_H_M,
};
use sim_core::gather::ORE_STAGES;

/// A whole node is drawn exactly as `props::spawn_slot` placed it: the stage
/// pose may not move a rock nobody has touched.
#[test]
fn a_whole_node_stands_where_it_spawned() {
    let (x, y, z, scale, yaw) = (10.0, 3.0, -4.0, 1.2, 0.7);
    let spawned = Transform {
        translation: Vec3::new(x, y, z),
        rotation: Quat::from_rotation_y(yaw),
        scale: Vec3::splat(scale),
    };
    let foot = spawned.translation - Vec3::Y * MESH_HALF_H_M * scale;
    let n = OreNode::new(7, OreKind::Stone, foot, scale, yaw);
    let t = pose_transform(&n, &stage_pose(7, 0.0), Quat::IDENTITY, 0.0, 1.0);
    assert!(t.translation.distance(spawned.translation) < 1e-5);
    assert!(t.rotation.angle_between(spawned.rotation) < 1e-5);
    assert!(t.scale.distance(spawned.scale) < 1e-5);
}

/// Each stage leaves less standing than the one before, and the same key
/// draws the same rock for everybody.
#[test]
fn each_stage_is_smaller_and_shared() {
    for key in [1u32, 99, 0xBEEF] {
        let mut last_top = f32::MAX;
        for s in 0..ORE_STAGES {
            let p = stage_pose(key, s as f32);
            let top = 2.0 * MESH_HALF_H_M * p.h - p.sink;
            assert!(top < last_top, "key {key} stage {s} is no smaller");
            last_top = top;
            assert_eq!(p, stage_pose(key, s as f32));
        }
    }
    // Two keys lean their worn stages differently.
    assert_ne!(stage_pose(1, 2.0).lean, stage_pose(2, 2.0).lean);
}

/// Thrown chunks come down onto the ground and lie there, and the pool never
/// grows past its cap however much is thrown.
#[test]
fn chunks_land_and_the_pool_is_bounded() {
    let mut d = Debris::with_cap(8);
    let (mesh, mat) = (Handle::default(), Handle::default());
    let throw = Throw {
        at: Vec3::new(0.0, 1.0, 0.0),
        away: Vec3::Y,
        speed: (2.0, 4.0),
        up: 2.0,
        size: (0.05, 0.1),
        life: 30.0,
        spread: 0.2,
        mesh: &mesh,
        mat: &mat,
    };
    d.throw(20, &throw, &|_, _| 0.0);
    assert_eq!(d.live(), 8);
    assert_eq!(d.thrown, 20);
    assert_eq!(d.stolen, 12);
    for _ in 0..600 {
        d.step(1.0 / 60.0);
    }
    for i in 0..8 {
        let (pos, _, size) = d.draw(i).expect("still lying there");
        assert!(pos.y > 0.0 && pos.y < 0.2, "chunk {i} at {pos:?}");
        assert!(size > 0.0);
    }
}
