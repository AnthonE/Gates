//! The far treeline's card bake (`render/far_trees.rs`): each pool variant
//! rasterised side-on must come out as a tree — a solid-enough silhouette in
//! its own cell, standing on the cell's bottom edge, sized like the tree.

use client::render::far_trees::{bake_atlas, CARD_RES};
use client::render::tree;

#[test]
fn every_card_is_a_tree_standing_in_its_own_cell() {
    let (img, dims) = bake_atlas();
    assert_eq!(dims.len(), tree::CONIFER_POOL);
    let w = img.texture_descriptor.size.width as usize;
    let data = img.data.as_ref().expect("the atlas keeps its bytes");
    let r = CARD_RES as usize;
    for (v, d) in dims.iter().enumerate() {
        let sp = &tree::SPECIES[tree::species_of(v)];
        assert!(
            (d.h - sp.height_m).abs() < 0.5,
            "variant {v}: card is {} m tall, the tree {} m",
            d.h,
            sp.height_m
        );
        assert!(
            d.half_w > 0.5 && d.half_w <= sp.max_r_m + 0.5,
            "variant {v}: half-width {}",
            d.half_w
        );
        let (mut covered, mut bottom) = (0usize, 0usize);
        for y in 0..r {
            for x in 0..r {
                let a = data[(y * w + v * r + x) * 4 + 3];
                if a >= 128 {
                    covered += 1;
                    if y >= r - 4 {
                        bottom += 1;
                    }
                }
            }
        }
        let share = covered as f32 / (r * r) as f32;
        assert!(
            (0.08..0.9).contains(&share),
            "variant {v}: {share} of the cell is tree"
        );
        assert!(
            bottom > 0,
            "variant {v}: nothing stands on the cell's bottom edge"
        );
    }
}

/// The ring mask covers the window the rings stream in around the eye, one
/// bit per chunk a ring holds — and nothing outside it.
#[test]
fn the_ring_mask_marks_exactly_the_chunks_the_rings_hold() {
    use bevy::math::Vec3;
    use client::render::far_trees::{ring_mask, MASK_SIDE};
    use client::render::props::OUTER_RADIUS;
    use client::render::terrain_mesh::CHUNK_M;
    let eye = client::render::Eye {
        pos: Vec3::new(30.5 * CHUNK_M, 0.0, 12.5 * CHUNK_M),
        ..Default::default()
    };
    let held = [(30, 12), (26, 8), (34, 16), (27, 15), (40, 12)];
    let (window, bits) = ring_mask(&eye, held.iter().copied());
    assert_eq!(window.x, 30 - OUTER_RADIUS);
    assert_eq!(window.y, 12 - OUTER_RADIUS);
    assert_eq!(window.z, MASK_SIDE);
    let words = bits.to_array();
    let set = |x: i32, z: i32| {
        let (lx, lz) = (x - window.x, z - window.y);
        if lx < 0 || lz < 0 || lx >= MASK_SIDE || lz >= MASK_SIDE {
            return false;
        }
        let i = (lz * MASK_SIDE + lx) as usize;
        words[i / 32] >> (i % 32) & 1 == 1
    };
    for &(x, z) in &held[..4] {
        assert!(set(x, z), "a held chunk ({x}, {z}) is not masked");
    }
    let count: u32 = words.iter().map(|w| w.count_ones()).sum();
    assert_eq!(count, 4, "the chunk outside the window must not alias in");
    assert!(!set(31, 12));
}
