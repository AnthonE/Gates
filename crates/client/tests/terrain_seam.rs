//! Arithmetic over the drawn triangles, including a coarse chord above the
//! near surface. No GPU and no pixel score.
#![cfg(feature = "render")]

use bevy::math::DVec2;
use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use client::render::{terrain_mesh as ground, terrain_seam as seam};
use sim_core::terrain;

fn positions(mesh: &Mesh) -> &[[f32; 3]] {
    let Some(VertexAttributeValues::Float32x3(p)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else {
        panic!("positions");
    };
    p
}

fn indices(mesh: &Mesh) -> &[u32] {
    let Some(Indices::U32(i)) = mesh.indices() else {
        panic!("indices")
    };
    i
}

#[test]
fn joins_reach_both_drawn_surfaces_without_moving_the_walkable_ground() {
    let mut higher = 0;
    let mut lower = 0;
    for seed in [20260731, 7] {
        let haven = terrain::haven(seed);
        for (cx, cz) in [(24, 10), (23, 9), (16, 16), (0, 0)] {
            let (ox, oz) = (cx as f32 * ground::CHUNK_M, cz as f32 * ground::CHUNK_M);
            let original = ground::heightfield(seed, &haven, ox, oz, ground::NEAR_N, 1.0, 0.0);
            let mut joined = original.clone();
            seam::append(&mut joined, seed, &haven);
            let fine = positions(&original);
            let p = positions(&joined);
            for (attribute, values) in original.attributes() {
                let got = joined.attribute(attribute.id).unwrap();
                assert_eq!(
                    &got.get_bytes()[..values.get_bytes().len()],
                    values.get_bytes()
                );
                assert_eq!(got.len(), fine.len() + 4 * ground::NEAR_N * 2);
                if attribute.id != Mesh::ATTRIBUTE_POSITION.id {
                    let stride = values.get_bytes().len() / values.len();
                    for (i, pair) in p[fine.len()..].chunks_exact(2).enumerate() {
                        let source = fine.iter().position(|v| *v == pair[0]).unwrap();
                        for end in 0..2 {
                            let index = fine.len() + i * 2 + end;
                            assert_eq!(
                                &got.get_bytes()[index * stride..(index + 1) * stride],
                                &values.get_bytes()[source * stride..(source + 1) * stride]
                            );
                        }
                    }
                }
            }
            assert_eq!(
                &indices(&joined)[..indices(&original).len()],
                indices(&original)
            );
            // Independently mesh the far patch. Chunk edges coincide with
            // its grid lines, so interpolation along its actual edges is exact.
            let n = (ground::CHUNK_M / ground::FAR_STEP) as usize + 1;
            let far =
                ground::heightfield(seed, &haven, ox, oz, n, ground::FAR_STEP, ground::FAR_DROP);
            let coarse = positions(&far);
            for pair in p[fine.len()..].chunks_exact(2) {
                let [a, b] = [pair[0], pair[1]];
                assert!(fine.contains(&a), "join must start on the near mesh");
                assert_eq!((a[0], a[2]), (b[0], b[2]));
                let along_x = a[2] == oz || a[2] == oz + ground::CHUNK_M;
                let axis = if along_x { 0 } else { 2 };
                let other = 2 - axis;
                let edge: Vec<_> = coarse.iter().filter(|v| v[other] == a[other]).collect();
                let segment = edge
                    .windows(2)
                    .find(|v| v[0][axis] <= a[axis] && a[axis] <= v[1][axis])
                    .unwrap();
                let t = (a[axis] - segment[0][axis]) / (segment[1][axis] - segment[0][axis]);
                let expected = segment[0][1] * (1.0 - t) + segment[1][1] * t;
                assert!(
                    (b[1] - expected).abs() < 0.00002,
                    "{seed} ({cx},{cz}) {a:?}: {b:?} != {expected}"
                );
                higher += usize::from(b[1] > a[1]);
                lower += usize::from(b[1] < a[1]);
            }
            // Every exposed segment is closed and has both windings, even
            // when its heights change order. Attribute copies at each end
            // also preserve the exact splat/road interpolation on that join.
            let base = indices(&original).len();
            for triangles in indices(&joined)[base..].chunks_exact(12) {
                let first = &triangles[..6];
                let second = &triangles[6..];
                for (front, back) in first.chunks_exact(3).zip(second.chunks_exact(3)) {
                    assert_eq!([front[0], front[2], front[1]], back);
                }
                let a = first[0] as usize;
                let corners = &p[a..a + 4];
                let axis = if corners[0][0] != corners[2][0] { 0 } else { 2 };
                let project = |v: [f32; 3]| DVec2::new(v[axis] as f64, v[1] as f64);
                for t in [0.25, 0.5, 0.75] {
                    let near = project(corners[0]).lerp(project(corners[2]), t);
                    let far = project(corners[1]).lerp(project(corners[3]), t);
                    for rise in [0.25, 0.5, 0.75] {
                        let point = near.lerp(far, rise);
                        assert!(
                            first.chunks_exact(3).any(|tri| {
                                let [a, b, c] = [
                                    project(p[tri[0] as usize]),
                                    project(p[tri[1] as usize]),
                                    project(p[tri[2] as usize]),
                                ];
                                let cross = |u: DVec2, v: DVec2| u.x * v.y - u.y * v.x;
                                let sides = [
                                    cross(b - a, point - a),
                                    cross(c - b, point - b),
                                    cross(a - c, point - c),
                                ];
                                sides.iter().all(|s| *s >= -1e-9)
                                    || sides.iter().all(|s| *s <= 1e-9)
                            }),
                            "an open slit between {near:?} and {far:?}"
                        );
                    }
                }
            }
            let capacity = match joined.indices().unwrap() {
                Indices::U32(v) => v.capacity(),
                _ => unreachable!(),
            };
            for mask in 0..16 {
                seam::set_edges(&mut joined, mask);
                assert_eq!(
                    indices(&joined).len(),
                    base + mask.count_ones() as usize * (ground::NEAR_N - 1) * 12
                );
                let Indices::U32(v) = joined.indices().unwrap() else {
                    unreachable!()
                };
                assert_eq!(
                    v.capacity(),
                    capacity,
                    "streaming must reuse worker allocation"
                );
            }
        }
    }
    assert!(
        higher > 0 && lower > 0,
        "fixtures must cover both directions of the mismatch"
    );
}

#[test]
fn no_join_reaches_beyond_the_far_sheet() {
    assert_eq!(seam::exposed((0, 0), |_| false), 0b1010);
    assert_eq!(seam::exposed((31, 31), |_| false), 0b0101);
    for key in [(-1, 0), (0, -1), (32, 0), (0, 32)] {
        assert_eq!(seam::exposed(key, |_| false), 0);
    }
    assert_eq!(seam::exposed((16, 16), |_| true), 0);
}
