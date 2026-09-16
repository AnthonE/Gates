//! Renderer geometry must stay inside the sim's depot collision kit.
#![cfg(feature = "render")]

use bevy::mesh::{Mesh, VertexAttributeValues};
use bevy::prelude::*;
use client::render::depot::{self, Surface};
use sim_core::depot::{PartKind, DEPOT_PARTS};

fn positions(mesh: &Mesh) -> &[[f32; 3]] {
    let Some(VertexAttributeValues::Float32x3(p)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
        panic!("missing positions")
    };
    p
}

#[test]
fn every_finished_part_stays_inside_its_authoritative_envelope() {
    let mut vertices = 0;
    for (index, part) in DEPOT_PARTS.iter().enumerate() {
        let groups = depot::part_meshes(part, index);
        assert!(!groups.is_empty());
        let mut low = [f32::INFINITY; 3];
        let mut high = [f32::NEG_INFINITY; 3];
        for (_, mesh) in groups {
            let p = positions(&mesh);
            let Some(VertexAttributeValues::Float32x4(tangents)) =
                mesh.attribute(Mesh::ATTRIBUTE_TANGENT)
            else {
                panic!("photographic normal map has no tangent frame")
            };
            let Some(VertexAttributeValues::Float32x3(normals)) =
                mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
            else {
                panic!("missing normals")
            };
            assert_eq!(p.len(), tangents.len());
            for ((p, tangent), normal) in p.iter().zip(tangents).zip(normals) {
                for axis in 0..3 {
                    assert!(
                        p[axis] >= part.bounds[axis] - 1e-5
                            && p[axis] <= part.bounds[axis + 3] + 1e-5,
                        "part {index}, {p:?} outside {:?}",
                        part.bounds
                    );
                    low[axis] = low[axis].min(p[axis]);
                    high[axis] = high[axis].max(p[axis]);
                }
                assert!(tangent.iter().all(|v| v.is_finite()));
                let n = Vec3::from_array(*normal);
                let t = Vec3::new(tangent[0], tangent[1], tangent[2]);
                assert!((n.length() - 1.0).abs() < 1e-4);
                assert!((t.length() - 1.0).abs() < 1e-4);
                assert!(n.dot(t).abs() < 1e-4);
            }
            vertices += p.len();
        }
        for axis in 0..3 {
            assert!((low[axis] - part.bounds[axis]).abs() < 1e-5);
            assert!((high[axis] - part.bounds[axis + 3]).abs() < 1e-5);
        }
    }
    eprintln!(
        "depot kit: {} parts, {vertices} vertices, {} triangles",
        DEPOT_PARTS.len(),
        vertices / 3
    );
}

#[test]
fn yard_is_at_the_walk_datum_and_decorations_keep_the_gate_and_door_open() {
    let meshes = depot::depot_meshes();
    assert_eq!(
        meshes.len(),
        depot::SURFACES.len(),
        "one draw group per material"
    );
    for (surface, mesh) in meshes {
        for &[x, y, z] in positions(&mesh) {
            if surface == Surface::Yard {
                assert!(y <= 0.0);
            } else {
                assert!(
                    !(x.abs() < sim_core::depot::GATE_HALF_W && y > 0.0 && z.abs() <= 22.0),
                    "through lane obstructed"
                );
                assert!(
                    !(x > -4.4 && x <= -4.0 && z.abs() < 3.0 && y > 0.0 && y < 4.0),
                    "warehouse doorway obstructed"
                );
            }
        }
    }
}

#[test]
fn sheet_ribs_run_vertically_and_sign_reads_from_outside() {
    let (index, wall) = DEPOT_PARTS
        .iter()
        .enumerate()
        .find(|(_, p)| p.kind == PartKind::Wall && p.bounds[1] == 0.0)
        .unwrap();
    for (surface, mesh) in depot::part_meshes(wall, index) {
        if surface != Surface::Sheet {
            continue;
        }
        let Some(VertexAttributeValues::Float32x2(uvs)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0)
        else {
            panic!("uv")
        };
        let Some(VertexAttributeValues::Float32x3(normals)) =
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            panic!("normal")
        };
        for ((p, uv), normal) in positions(&mesh).iter().zip(uvs).zip(normals) {
            if normal[1] == 0.0 {
                assert!((uv[0] - p[1] * depot::DEPOT_SHEET_TILES_PER_M).abs() < 1e-5);
            }
        }
    }
    let (index, header) = DEPOT_PARTS
        .iter()
        .enumerate()
        .find(|(_, p)| p.kind == PartKind::Wall && p.bounds[1] > 0.0)
        .unwrap();
    let paint = depot::part_meshes(header, index)
        .into_iter()
        .find(|(s, _)| *s == Surface::Paint)
        .unwrap()
        .1;
    let p = positions(&paint);
    // FREIGHT is emitted in reading order; a reader east of the header has
    // screen-right along -Z. The first F must precede the final T that way.
    assert!(p[0][2] > p[p.len() - 1][2]);
    let Some(VertexAttributeValues::Float32x3(normals)) = paint.attribute(Mesh::ATTRIBUTE_NORMAL)
    else {
        panic!("normal")
    };
    assert!(normals.iter().all(|n| *n == [1.0, 0.0, 0.0]));
}

#[test]
fn bevy_instances_match_sim_geometry_at_every_phase() {
    let mut site = sim_core::terrain::Waystation::NONE;
    site.x = 1234.25;
    site.z = 876.5;
    site.floor_y = 4.25;
    for phase in 0..=u8::MAX {
        site.phase = phase;
        let transform = depot::site_transform(&site);
        for part in DEPOT_PARTS {
            let b = part.bounds;
            for x in [b[0], b[3]] {
                for y in [b[1], b[4]] {
                    for z in [b[2], b[5]] {
                        let drawn = transform.transform_point(Vec3::new(x, y, z));
                        let (sx, sz) = sim_core::depot::to_world(&site, x, z);
                        let solid = Vec3::new(sx, site.floor_y + y, sz);
                        assert!(
                            drawn.distance(solid) < 0.0005,
                            "phase {phase}: {drawn} != {solid}"
                        );
                    }
                }
            }
        }
    }
}
