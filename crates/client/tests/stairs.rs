//! The actual tread mesh beside the smooth movement ramp. The older lattice
//! suite checks the ramp's frame; this suite reads triangles, including the
//! normalized mesh the placement preview scales back to real size.
#![cfg(feature = "render")]

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use client::render::structures::{
    base_transform, level_base_y, part_mesh, shape_parts, Part, PIECE_UV_PER_M, STAIR_RISERS,
};
use sim_core::build::{BUILD_CELL_M, LEVEL_H_M, LOC_RISER, SHAPE_STAIRS};
use sim_core::collide::{piece_ground, ColIndex};
use sim_core::terrain;

fn positions(mesh: &Mesh, transform: Transform) -> Vec<Vec3> {
    let Some(VertexAttributeValues::Float32x3(values)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("stairs need positions");
    };
    values
        .iter()
        .map(|p| transform.transform_point(Vec3::from(*p)))
        .collect()
}

fn top_at(points: &[Vec3], mesh: &Mesh, x: f32, z: f32) -> f32 {
    let indices: Vec<usize> = mesh.indices().unwrap().iter().collect();
    let mut top = f32::NEG_INFINITY;
    for face in indices.chunks_exact(3) {
        let [a, b, c] = [points[face[0]], points[face[1]], points[face[2]]];
        let normal = (b - a).cross(c - a);
        if normal.y <= 0.0 {
            continue;
        }
        let p = Vec2::new(x, z);
        let a2 = Vec2::new(a.x, a.z);
        let u = Vec2::new(b.x - a.x, b.z - a.z);
        let v = Vec2::new(c.x - a.x, c.z - a.z);
        let d = p - a2;
        let det = u.perp_dot(v);
        let s = d.perp_dot(v) / det;
        let t = u.perp_dot(d) / det;
        if s >= -1e-5 && t >= -1e-5 && s + t <= 1.0 + 1e-5 {
            top = top.max(a.y + s * (b.y - a.y) + t * (c.y - a.y));
        }
    }
    top
}

#[test]
fn preview_and_placed_flight_have_identical_vertices() {
    let (parts, n) = shape_parts(SHAPE_STAIRS);
    assert_eq!(n, 1, "a flight remains one mesh and one draw");
    let part = parts[0];
    let placed = part_mesh(&part);
    let preview = part_mesh(&Part {
        size: Vec3::ONE,
        ..part
    });
    let a = positions(&placed, part.transform());
    let b = positions(&preview, part.transform().with_scale(part.size));
    assert_eq!(a.len(), b.len());
    assert_eq!(placed.indices().unwrap(), preview.indices().unwrap());
    for (a, b) in a.iter().zip(b) {
        assert!(a.distance(b) < 1e-5, "preview {b:?} != placed {a:?}");
    }
}

#[test]
fn the_mesh_has_horizontal_treads_vertical_risers_and_outward_faces() {
    let part = shape_parts(SHAPE_STAIRS).0[0];
    let mesh = part_mesh(&part);
    let points = positions(&mesh, part.transform());
    let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        panic!("stairs need normals");
    };
    let Some(VertexAttributeValues::Float32x4(tangents)) = mesh.attribute(Mesh::ATTRIBUTE_TANGENT)
    else {
        panic!("stairs need tangents for the normal map");
    };
    assert!(tangents.iter().flatten().all(|v| v.is_finite()));
    let mut heights = Vec::new();
    let mut risers = 0;
    for (face, ns) in points.chunks_exact(4).zip(normals.chunks_exact(4)) {
        let normal = part.transform().rotation * Vec3::from(ns[0]);
        for triangle in [[0, 1, 2], [0, 2, 3]] {
            let [a, b, c] = triangle.map(|i| face[i]);
            assert!((b - a).cross(c - a).dot(normal) > 1e-7);
        }
        if normal.y > 0.9 {
            assert!(normal.distance(Vec3::Y) < 1e-5, "a tread is still pitched");
            assert!(face.iter().all(|p| (p.y - face[0].y).abs() < 1e-5));
            heights.push(face[0].y);
        } else if normal.z < -0.9 {
            risers += 1;
        }
    }
    heights.sort_by(f32::total_cmp);
    heights.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
    assert_eq!(heights.len(), STAIR_RISERS + 1);
    assert_eq!(risers, STAIR_RISERS + 1, "risers plus the foot cap");
    assert!(heights[0].abs() < 1e-5);
    assert!((heights[STAIR_RISERS] - LEVEL_H_M).abs() < 1e-5);
    for p in points {
        assert!(p.is_finite());
        assert!(p.x.abs() <= BUILD_CELL_M * 0.5 + 1e-5);
        assert!(p.z.abs() <= BUILD_CELL_M * 0.5 + 1e-5);
        assert!(p.y <= LEVEL_H_M + 1e-5);
    }
}

#[test]
fn the_photograph_keeps_its_scale_and_continues_across_treads() {
    let part = shape_parts(SHAPE_STAIRS).0[0];
    let mesh = part_mesh(&part);
    let points = positions(&mesh, part.transform());
    let Some(VertexAttributeValues::Float32x2(uvs)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else {
        panic!("stairs need texture coordinates");
    };
    for (face, uv) in points.chunks_exact(4).zip(uvs.chunks_exact(4)) {
        for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
            let metres = face[a].distance(face[b]);
            let texels = Vec2::from(uv[a]).distance(Vec2::from(uv[b]));
            assert!((texels - metres * PIECE_UV_PER_M).abs() < 1e-5);
        }
        if face.iter().all(|p| (p.y - face[0].y).abs() < 1e-5) {
            for (p, uv) in face.iter().zip(uv) {
                assert!((uv[0] - (p.z + BUILD_CELL_M * 0.5) * PIECE_UV_PER_M).abs() < 1e-5);
            }
        }
    }
}

#[test]
fn treads_follow_collision_and_join_both_landings_at_every_plate() {
    let seed = 20260731;
    let haven = terrain::haven(seed);
    let (cx, cz) = (341, 341);
    let part = shape_parts(SHAPE_STAIRS).0[0];
    let mesh = part_mesh(&part);
    let rise = LEVEL_H_M / STAIR_RISERS as f32;
    for plate in [-3, 0, 3] {
        for level in [0, 2] {
            let mut cols = ColIndex::new();
            cols.add(cx, cz, level, LOC_RISER, SHAPE_STAIRS, plate);
            let base = level_base_y(seed, &haven, cx, cz, level, plate);
            let transform =
                base_transform(seed, &haven, (cx, cz, level, LOC_RISER), plate) * part.transform();
            let points = positions(&mesh, transform);
            for sample in 0..=STAIR_RISERS * 8 {
                let local_z = (sample as f32 * BUILD_CELL_M / (STAIR_RISERS * 8) as f32)
                    .clamp(0.001, BUILD_CELL_M - 0.001);
                let z = cz as f32 * BUILD_CELL_M + local_z;
                for fraction in [0.01, 0.5, 0.99] {
                    let x = (cx as f32 + fraction) * BUILD_CELL_M;
                    let walked = piece_ground(seed, &haven, &cols, x, z, base + LEVEL_H_M);
                    let drawn = top_at(&points, &mesh, x, z);
                    assert!(drawn.is_finite(), "hole in a tread at {x}, {z}");
                    assert!(
                        (walked - drawn).abs() <= rise * 0.5 + 0.001,
                        "plate {plate}, level {level}: walked {walked}, drawn {drawn}"
                    );
                    if sample == 0 {
                        assert!((drawn - base).abs() < 0.001);
                    } else if sample == STAIR_RISERS * 8 {
                        assert!((drawn - base - LEVEL_H_M).abs() < 0.001);
                    }
                }
            }
        }
    }
}
