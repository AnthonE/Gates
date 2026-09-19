//! Vertical joins from each near edge to the far mesh's actual edge.
//!
//! Chunk boundaries lie on the far lattice, so its edge is a straight line
//! between adjacent coarse samples (no bilinear/triangle approximation).
//! Both ends are derived; there is no guessed skirt depth. Internal joins
//! have no indices: when the coarse chord is higher, leaving one drawn would
//! put a fence through two neighbouring near chunks.

use bevy::mesh::{Indices, VertexAttributeValues};
use bevy::prelude::*;
use sim_core::terrain;

use super::terrain_mesh::{CHUNK_M, FAR_DROP, FAR_STEP, NEAR_N};

/// Left, right, bottom, top, in the same order as the appended edge vertices.
pub const NEIGHBORS: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 1)];
const SURFACE_INDICES: usize = (NEAR_N - 1) * (NEAR_N - 1) * 6;

fn edge_vertex(edge: usize, i: usize) -> usize {
    match edge {
        0 => i * NEAR_N,
        1 => i * NEAR_N + NEAR_N - 1,
        2 => i,
        3 => (NEAR_N - 1) * NEAR_N + i,
        _ => unreachable!(),
    }
}

/// Append paired near/far vertices off-thread, after road attributes exist.
/// The original surface vertices and indices remain byte-for-byte intact.
pub fn append(mesh: &mut Mesh, seed: u64, haven: &terrain::Haven) {
    for (_, values) in mesh.attributes_mut() {
        fn extend<const N: usize>(values: &mut Vec<[f32; N]>) {
            values.reserve_exact(4 * NEAR_N * 2);
            for edge in 0..4 {
                for i in 0..NEAR_N {
                    let value = values[edge_vertex(edge, i)];
                    values.extend_from_slice(&[value, value]);
                }
            }
        }
        match values {
            VertexAttributeValues::Float32x2(v) => extend(v),
            VertexAttributeValues::Float32x3(v) => extend(v),
            VertexAttributeValues::Float32x4(v) => extend(v),
            _ => panic!("unexpected ground attribute"),
        }
    }
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    else {
        unreachable!("ground has positions")
    };
    let mut lat = terrain::Lattice::new();
    let coarse_segments = (CHUNK_M / FAR_STEP) as usize;
    let fine_per_coarse = (NEAR_N - 1) / coarse_segments;
    for edge in 0..4 {
        for segment in 0..coarse_segments {
            let a = positions[edge_vertex(edge, segment * fine_per_coarse)];
            let b = positions[edge_vertex(edge, (segment + 1) * fine_per_coarse)];
            let ya = terrain::ground_memo(&mut lat, seed, haven, a[0], a[2]) - FAR_DROP;
            let yb = terrain::ground_memo(&mut lat, seed, haven, b[0], b[2]) - FAR_DROP;
            for j in 0..=fine_per_coarse {
                let i = segment * fine_per_coarse + j;
                let index = NEAR_N * NEAR_N + (edge * NEAR_N + i) * 2 + 1;
                positions[index][1] = ya + (yb - ya) * (j as f32 / fine_per_coarse as f32);
            }
        }
    }
    // Reserve the worst case on the worker. Later adjacency changes only
    // rewrite this tail and cannot grow its allocation.
    let Some(Indices::U32(indices)) = mesh.indices_mut() else {
        unreachable!("ground uses u32 indices")
    };
    indices.reserve_exact(4 * (NEAR_N - 1) * 12);
    set_edges(mesh, 0b1111);
}

/// Replace only the seam indices. A set bit means an exposed near/far edge.
pub fn set_edges(mesh: &mut Mesh, edges: u8) {
    let Some(Indices::U32(indices)) = mesh.indices_mut() else {
        unreachable!("ground uses u32 indices")
    };
    indices.truncate(SURFACE_INDICES);
    for edge in 0..4 {
        if edges & (1 << edge) == 0 {
            continue;
        }
        for i in 0..NEAR_N - 1 {
            let a = (NEAR_N * NEAR_N + (edge * NEAR_N + i) * 2) as u32;
            let (b, c, d) = (a + 1, a + 2, a + 3);
            // Either surface may be higher. Both windings close the join
            // from either side without changing the shared ground material.
            indices.extend_from_slice(&[a, c, b, b, c, d, a, b, c, b, d, c]);
        }
    }
}

/// There is no far sheet outside the island to join to.
pub fn exposed(key: (i32, i32), resident: impl Fn((i32, i32)) -> bool) -> u8 {
    let side = (terrain::ISLAND_SIZE / CHUNK_M) as i32;
    let inside = |(x, z)| x >= 0 && z >= 0 && x < side && z < side;
    if !inside(key) {
        return 0;
    }
    let mut edges = 0;
    for (edge, (dx, dz)) in NEIGHBORS.into_iter().enumerate() {
        let neighbor = (key.0 + dx, key.1 + dz);
        if inside(neighbor) && !resident(neighbor) {
            edges |= 1 << edge;
        }
    }
    edges
}
