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
        // Each frame the flight is drawn in: its root, its storey's base,
        // the loc its walk is read through and what carries a drawn point to
        // the walk's own coordinates. A grid flight turns through the
        // stair locs in its cell; a triangle's is posed about the
        // triangle's centroid, its first side along +x (`posed_transform`),
        // the centroid an apothem off that side.
        let mut frames = Vec::new();
        if shape == SHAPE_STAIRS_TRI_SPIRAL {
            let h = BUILD_CELL_M * 0.5;
            frames.push((
                Transform::IDENTITY,
                0.0,
                LOC_RISER,
                Vec2::new(h, h / 3f32.sqrt()),
            ));
        } else {
            for loc in STAIR_LOCS {
                for level in [0, 8, 2] {
                    frames.push((
                        base_transform(seed, &haven, (682, 682, level, loc), 0),
                        column_floor_y(seed, &haven, 682, 682, 0) + level_y(level),
                        loc,
                        Vec2::splat(-2046.0),
                    ));
                }
            }
        }
        for (root, base, loc, to_walk) in frames {
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
                    circulation::surface(shape, loc, world.x + to_walk.x, world.z + to_walk.y)
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
    let mut tops = 0;
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
        tops += 1;
        let p =
            (Vec3::from(points[tri[0]]) + Vec3::from(points[tri[1]]) + Vec3::from(points[tri[2]]))
                / 3.0;
        // On the equilateral body's rim: inside it, and within a rim's
        // width of a side — `hull`'s triangle frame test, the drawn root at
        // the centroid an apothem off the first side.
        let h3 = 3f32.sqrt() * 0.5;
        let (x, z) = (p.x + 1.5, p.z + 1.5 / 3f32.sqrt());
        let d = [z, h3 * (3.0 - x) - 0.5 * z, h3 * x - 0.5 * z];
        assert!(
            d.iter().all(|&d| d >= -1e-4)
                && d.iter()
                    .any(|&d| d <= sim_core::collide::FRAME_RIM_M + 1e-4),
            "a top face at {p:?} is across the open centre"
        );
    }
    assert!(tops > 0, "the frame must draw a top surface");
}
