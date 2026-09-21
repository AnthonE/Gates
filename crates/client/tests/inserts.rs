//! Insert transforms keep the drawn solids inside the aperture the sim owns.
#![cfg(feature = "render")]

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use client::render::structures::{deploy_transform, insert_mesh, level_base_y};
use sim_core::build::{BUILD_CELL_M, LEVEL_H_M, LOC_EDGE_XLO, LOC_EDGE_ZLO};
use sim_core::collide::{DOOR_POST_W_M, FRAME_RIM_M, WINDOW_HEAD_M, WINDOW_SILL_M};
use sim_core::deploy::{ARCH_GARAGE_DOOR, ARCH_WINDOW_BARS};

#[test]
fn inserts_fit_both_edge_axes_at_raised_and_upper_storeys() {
    let seed = 20260731;
    let haven = sim_core::terrain::haven(seed);
    for arch in [ARCH_WINDOW_BARS, ARCH_GARAGE_DOOR] {
        let mesh = insert_mesh(arch);
        let Some(VertexAttributeValues::Float32x3(points)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("insert has no vertices")
        };
        for loc in [LOC_EDGE_XLO, LOC_EDGE_ZLO] {
            for level in [0, 2] {
                for plate in [0, 2] {
                    let addr = (341, 341, level, loc);
                    let base = level_base_y(seed, &haven, 341, 341, level, plate);
                    let t = deploy_transform(seed, &haven, addr, arch, false, plate);
                    let (mut low, mut high) =
                        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
                    for p in points {
                        let p = t.transform_point(Vec3::from(*p));
                        low = low.min(p);
                        high = high.max(p);
                    }
                    let (rim, sill, head) = if arch == ARCH_WINDOW_BARS {
                        (DOOR_POST_W_M, WINDOW_SILL_M, WINDOW_HEAD_M)
                    } else {
                        (FRAME_RIM_M, 0.0, LEVEL_H_M - FRAME_RIM_M)
                    };
                    assert!((low.y - base - sill).abs() < 0.001);
                    assert!((high.y - base - head).abs() < 0.001);
                    let (lo, hi) = if loc == LOC_EDGE_XLO {
                        (low.z, high.z)
                    } else {
                        (low.x, high.x)
                    };
                    assert!((lo - 341.0 * BUILD_CELL_M - rim).abs() < 0.001);
                    assert!((hi - 342.0 * BUILD_CELL_M + rim).abs() < 0.001);
                    if arch == ARCH_GARAGE_DOOR {
                        let open = deploy_transform(seed, &haven, addr, arch, true, plate);
                        for p in points {
                            let y = open.transform_point(Vec3::from(*p)).y;
                            assert!(y >= base + LEVEL_H_M - FRAME_RIM_M - 0.001);
                            assert!(y <= base + LEVEL_H_M + 0.001);
                        }
                    }
                }
            }
        }
    }
}
