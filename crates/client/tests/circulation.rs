#![cfg(feature = "render")]
use bevy::{mesh::VertexAttributeValues, prelude::*};
use client::render::structures::{base_transform, part_mesh, shape_parts};
use sim_core::{build::*, circulation};
#[test]
fn circulation_mesh_tops_match_walk_surfaces_in_every_orientation() {
    let seed = 20260731;
    let haven = sim_core::terrain::haven(seed);
    for shape in SHAPE_FOUNDATION_STEPS..=SHAPE_STAIRS_TRI_SPIRAL {
        let (parts, n) = shape_parts(shape);
        assert_eq!(n, 1);
        let part = parts[0];
        let mesh = part_mesh(&part);
        let VertexAttributeValues::Float32x3(points) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
        else {
            panic!("positions")
        };
        let VertexAttributeValues::Float32x3(normals) =
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap()
        else {
            panic!("normals")
        };
        let indices = mesh.indices().unwrap().iter().collect::<Vec<_>>();
        for loc in STAIR_LOCS {
            for level in [0, 8, 2] {
                let root = base_transform(seed, &haven, (341, 341, level, loc), 0);
                let base = column_floor_y(seed, &haven, 341, 341, 0) + level_y(level);
                let mut tops = 0;
                for tri in indices.chunks_exact(3) {
                    if normals[tri[0]][1] < 0.5 {
                        continue;
                    }
                    let local = (Vec3::from(points[tri[0]])
                        + Vec3::from(points[tri[1]])
                        + Vec3::from(points[tri[2]]))
                        / 3.0;
                    let world = (root * part.transform()).transform_point(local);
                    let expected =
                        circulation::surface(shape, loc, world.x - 1023.0, world.z - 1023.0)
                            .expect("drawn top outside the walk footprint");
                    assert!(
                        (world.y - base - expected).abs() <= LEVEL_H_M / 20.0,
                        "shape {shape} loc {loc}: drawn {}, walk {}",
                        world.y - base,
                        expected
                    );
                    tops += 1;
                }
                assert!(tops > 0);
            }
        }
    }
}
#[test]
fn triangle_frame_mesh_has_no_face_across_the_open_centre() {
    let (parts, n) = shape_parts(SHAPE_TRI_FLOOR_FRAME);
    assert_eq!(n, 1);
    let mesh = part_mesh(&parts[0]);
    let VertexAttributeValues::Float32x3(points) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
    else {
        panic!("positions")
    };
    let VertexAttributeValues::Float32x3(normals) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap()
    else {
        panic!("normals")
    };
    for tri in mesh
        .indices()
        .unwrap()
        .iter()
        .collect::<Vec<_>>()
        .chunks_exact(3)
    {
        if normals[tri[0]][1] < 0.5 {
            continue;
        }
        let p =
            (Vec3::from(points[tri[0]]) + Vec3::from(points[tri[1]]) + Vec3::from(points[tri[2]]))
                / 3.0;
        assert!(sim_core::collide::tri_frame_solid(
            LOC_TRI_XLO_ZLO,
            p.x + 1.5,
            p.z + 1.5,
            0.0
        ));
    }
}
