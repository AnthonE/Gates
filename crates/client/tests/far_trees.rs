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
