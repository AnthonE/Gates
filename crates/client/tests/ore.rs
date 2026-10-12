//! Gate: ore nodes as they are mined (`render/ore.rs`), driven as arithmetic.

#![cfg(feature = "render")]

use bevy::math::{Quat, Vec3};
use bevy::prelude::Mesh;
use bevy::prelude::{Handle, Transform};
use client::render::ore::{
    cut_mesh, fresh_tint, pose_transform, stage_planes, stage_pose, stage_top_m, Debris, OreKind,
    OreNode, Throw, MESH_HALF_H_M,
};
use sim_core::gather::{ORE_STAGES, ORE_STAGE_TOP_PM};
use sim_core::terrain::{occupant_volume, Occupant};

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
    let n = OreNode::new(7, OreKind::Stone, foot, scale, yaw, Handle::default());
    for cut in [false, true] {
        let p = stage_pose(OreKind::Stone, 7, 0.0);
        let t = pose_transform(&n, &p, Quat::IDENTITY, 0.0, 1.0, cut);
        assert!(t.translation.distance(spawned.translation) < 1e-5);
        assert!(t.rotation.angle_between(spawned.rotation) < 1e-5);
        assert!(t.scale.distance(spawned.scale) < 1e-5);
    }
}

/// The drawn node's top at each stage is the sim's top for that stage, from
/// the same table: what you see is what a swing strikes.
#[test]
fn the_drawn_top_is_the_sims_top() {
    let (_, top) = occupant_volume(Occupant::StoneNode);
    let bury = stage_top_m(OreKind::Stone, 0.0) - top;
    for (s, pm) in ORE_STAGE_TOP_PM.iter().enumerate() {
        let want = bury + top * *pm as f32 * 0.001;
        assert!((stage_top_m(OreKind::Stone, s as f32) - want).abs() < 1e-5);
    }
}

/// A cut leaves nothing past its planes, gives the chip its plane's normal and
/// the fresh tint, and leaves the rest of the rock alone.
#[test]
fn a_cut_shears_the_rock_flat_past_its_planes() {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues as V};
    // A tall column of points, half above the stage-3 cap and half below.
    let pos: Vec<[f32; 3]> = (0..20)
        .map(|i| [0.1, -0.6 + i as f32 * 0.06, 0.05])
        .collect();
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos.clone());
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[1.0f32, 0.0, 0.0]; pos.len()]);
    m.insert_indices(Indices::U32((0..18).collect()));
    let (planes, n) = stage_planes(OreKind::Stone, 0, ORE_STAGES as usize - 1);
    let out = cut_mesh(&m, &planes[..n], fresh_tint(OreKind::Stone)).unwrap();
    let Some(V::Float32x3(p)) = out.attribute(Mesh::ATTRIBUTE_POSITION) else {
        panic!("no positions");
    };
    let Some(V::Float32x4(c)) = out.attribute(Mesh::ATTRIBUTE_COLOR) else {
        panic!("no tint");
    };
    let mut fresh = 0;
    for (i, v) in p.iter().enumerate() {
        let v = Vec3::from(*v);
        for &(pn, d) in &planes[..n] {
            assert!(pn.dot(v) <= d + 1e-4, "vertex {i} left past a plane");
        }
        if c[i] != [1.0; 4] {
            fresh += 1;
        } else {
            assert_eq!(v, Vec3::from(pos[i]), "an uncut vertex moved");
        }
    }
    assert!(
        fresh > 0 && fresh < pos.len(),
        "{fresh} of {} cut",
        pos.len()
    );
    // The tallest point left is under the stage's top.
    let max_y = p.iter().map(|v| v[1]).fold(f32::MIN, f32::max);
    let top_local = stage_top_m(OreKind::Stone, (ORE_STAGES - 1) as f32) - MESH_HALF_H_M;
    assert!(max_y < top_local, "{max_y} stands past {top_local}");
}

/// Each stage leaves less standing than the one before, and the same key
/// draws the same rock for everybody.
#[test]
fn each_stage_is_smaller_and_shared() {
    for key in [1u32, 99, 0xBEEF] {
        let mut last_top = f32::MAX;
        for s in 0..ORE_STAGES {
            let p = stage_pose(OreKind::Metal, key, s as f32);
            let top = 2.0 * MESH_HALF_H_M * p.h;
            assert!(top < last_top, "key {key} stage {s} is no smaller");
            last_top = top;
            assert_eq!(p, stage_pose(OreKind::Metal, key, s as f32));
        }
    }
    // Two keys lean their worn stages differently.
    assert_ne!(
        stage_pose(OreKind::Stone, 1, 2.0).lean,
        stage_pose(OreKind::Stone, 2, 2.0).lean
    );
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
