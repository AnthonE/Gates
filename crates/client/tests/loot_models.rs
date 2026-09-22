#![cfg(feature = "render")]

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use client::render::{heldgen, loot};

#[test]
fn loot_surface_matches_the_rendered_triangles() {
    use client::render::terrain_mesh::{heightfield, CHUNK_M, NEAR_N};
    let seed = 20260731;
    let haven = sim_core::terrain::haven(seed);
    let step = CHUNK_M / (NEAR_N - 1) as f32;
    let mesh = heightfield(seed, &haven, 1000.0, 1000.0, 4, step, 0.0);
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("no positions")
    };
    // Barycentric points in EVERY actual triangle, including both sides of
    // each quad's diagonal. The mesh's indices decide which heights to mix.
    let indices: Vec<_> = mesh.indices().unwrap().iter().collect();
    for tri in indices.chunks_exact(3) {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| Vec3::from(positions[i]));
        let p = a * 0.25 + b * 0.25 + c * 0.5;
        assert!((loot::surface_y(seed, &haven, p.x, p.z) - p.y).abs() < 1e-5);
    }
}

#[test]
fn resting_items_clear_flat_and_sloping_ground_at_every_rotation() {
    let models = [
        loot::sack_mesh([0.3, 0.24, 0.3]),
        heldgen::mesh("torch"),
        heldgen::mesh("revolver"),
    ];
    for (i, mesh) in models.iter().enumerate() {
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no positions")
        };
        for id in 0..4 {
            for slope in [0.0, 0.6, -0.6] {
                let ground = |x: f32, z: f32| (x - 30.0) * slope + (z - 40.0) * slope * 0.5;
                let t = loot::resting_transform(
                    mesh,
                    Vec3::new(30.0, -0.01, 40.0),
                    id,
                    1.0,
                    i != 0,
                    ground,
                );
                let mut bottom = f32::INFINITY;
                for p in positions {
                    let p = t.transform_point(Vec3::from(*p));
                    assert!(
                        p.y >= ground(p.x, p.z) - 1e-5,
                        "item {i}, rotation {id} is buried at {p:?}"
                    );
                    bottom = bottom.min(p.y);
                }
                if slope == 0.0 {
                    assert!(bottom.abs() < 1e-5, "floating on flat ground");
                }
            }
        }
    }
}
