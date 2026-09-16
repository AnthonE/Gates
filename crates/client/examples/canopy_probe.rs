//! `canopy_probe` — a bench, not a gate: measure what a canopy IS, so a
//! complaint about how it LOOKS can be answered with arithmetic.
//!
//! ```text
//! cargo run --release -p client --features render --example canopy_probe
//! ```
//!
//! `tree_sweep` next door chooses a parameter block against a distribution of
//! seeds; this one describes the block that shipped. It exists because the
//! operator put four frames of our forest beside one of the reference's on
//! 2026-09-16 and said the trees need help, and every defect in those frames
//! turned out to have a number: a needle 20× too wide, a leaf 37 cm long, a
//! crown 6 % darker inside than out where foliage is several times darker, and
//! a canopy with no vertical normal anywhere in it. Nothing here asserts —
//! `tests/tree.rs` holds what this found.
//!
//! ⚠ **Every millimetre figure divides by the MEASURED card edge, never by
//! `LeafParams::size`.** That field is in the generator's units and
//! `fit_to_bounds` rescales the mesh afterwards — by 0.94 on the conifer and
//! 2.09 on the broadleaf — so reading it as metres is how a 38 cm leaf shipped
//! under a comment saying 0.60 m.

use bevy::mesh::{Mesh, VertexAttributeValues};
use client::render::tree::{canopy_shade, conifer, leaf_image, needle_image, SPECIES};

fn positions(m: &Mesh) -> Vec<[f32; 3]> {
    match m.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(v)) => v.clone(),
        _ => vec![],
    }
}
fn colors(m: &Mesh) -> Vec<[f32; 4]> {
    match m.attribute(Mesh::ATTRIBUTE_COLOR) {
        Some(VertexAttributeValues::Float32x4(v)) => v.clone(),
        _ => vec![],
    }
}

/// The MEASURED median world-space card edge for a variant, metres.
fn card_edge(variant: usize) -> f32 {
    let (_, needles) = conifer(variant);
    let p = positions(&needles);
    let mut e: Vec<f32> = p
        .chunks_exact(4)
        .map(|q| {
            let a = bevy::prelude::Vec3::from_array(q[0]);
            (bevy::prelude::Vec3::from_array(q[1]) - a)
                .length()
                .max((bevy::prelude::Vec3::from_array(q[3]) - a).length())
        })
        .collect();
    e.sort_by(|x, y| x.partial_cmp(y).unwrap());
    e[e.len() / 2]
}

/// Write one mask's level-0 ALPHA to a raw 8-bit file, so a person can look at
/// the card instead of at a coverage number.
///
/// Raw rather than PNG because this crate carries no encoder of its own and a
/// bench is not worth adding one; `ci/` has Pillow and turns these into
/// something viewable in two lines. **The card is the one part of a canopy that
/// can be judged without a GPU**, which is why it is worth writing at all —
/// `CLAUDE.md` says the visual gate is a person booting the game, and this is
/// the small part of that a headless box can still hand them.
fn dump_alpha(img: &bevy::prelude::Image, path: &str) {
    let w = img.texture_descriptor.size.width as usize;
    let d = img.data.as_ref().expect("mask has no data");
    let alpha: Vec<u8> = d[..w * w * 4].chunks_exact(4).map(|p| p[3]).collect();
    match std::fs::write(path, &alpha) {
        Ok(()) => println!("wrote {path} ({w}x{w}, 8-bit alpha, row 0 = card base)"),
        Err(e) => println!("could not write {path}: {e}"),
    }
}

fn main() {
    // How much of its own colour ramp each slice of the crown keeps after
    // `occlude_canopy`. The apex must read NO DARKER than mid-crown — it is
    // the most exposed part of the tree — and it is the one statistic that
    // separates a per-height radius profile from a single global maximum.
    println!("=== crown shade, as a fraction of `band`'s ramp ===");
    for v in [0usize, 3] {
        println!(
            "variant {v}: apex(0.85..1.0) {:.4}   mid(0.40..0.60) {:.4}   whole crown {:.4}",
            canopy_shade(v, 0.85, 1.0),
            canopy_shade(v, 0.40, 0.60),
            canopy_shade(v, 0.0, 1.0),
        );
    }
    for variant in [0usize, 3] {
        let sp = &SPECIES[variant / 3];
        let (bark, needles) = conifer(variant);
        let p = positions(&needles);
        let c = colors(&needles);
        println!(
            "\n=== variant {variant}  h={:.1}m  verts={} ===",
            sp.height_m,
            p.len()
        );

        // A leaf card is a quad: 4 verts. `Double` billboard = 2 quads.
        println!("cards (verts/4) = {}", p.len() / 4);

        // World-space card size: the max edge length of the first few quads.
        let mut edges: Vec<f32> = vec![];
        for q in p.chunks_exact(4).take(200) {
            let a = bevy::prelude::Vec3::from_array(q[0]);
            let b = bevy::prelude::Vec3::from_array(q[1]);
            let d = bevy::prelude::Vec3::from_array(q[3]);
            edges.push((b - a).length().max((d - a).length()));
        }
        edges.sort_by(|x, y| x.partial_cmp(y).unwrap());
        println!(
            "card edge m: min {:.3} med {:.3} max {:.3}",
            edges[0],
            edges[edges.len() / 2],
            edges[edges.len() - 1]
        );

        // Crown extent and the radial depth of every canopy vertex.
        let maxr = p
            .iter()
            .map(|v| (v[0] * v[0] + v[2] * v[2]).sqrt())
            .fold(0.0f32, f32::max);
        println!("crown max r = {maxr:.3} m");

        // THE QUESTION: does vertex colour vary with how deep inside the
        // crown a vertex sits? Bin by r/r_at_that_height and print luma.
        // `band` ramps on y ALONE, so the answer should be "not at all".
        let mut bins = [(0.0f64, 0usize); 5];
        for (v, col) in p.iter().zip(c.iter()) {
            let r = (v[0] * v[0] + v[2] * v[2]).sqrt();
            let f = (r / maxr).clamp(0.0, 0.999);
            let b = (f * 5.0) as usize;
            let luma = 0.2126 * col[0] + 0.7152 * col[1] + 0.0722 * col[2];
            bins[b].0 += luma as f64;
            bins[b].1 += 1;
        }
        println!("linear luma by radial depth (0 = trunk core, 4 = crown edge):");
        for (i, (s, n)) in bins.iter().enumerate() {
            if *n > 0 {
                println!("  r bin {i}: n={n:6}  luma={:.4}", s / *n as f64);
            }
        }

        // And by height, which is the axis `band` DOES ramp on.
        let maxy = p.iter().map(|v| v[1]).fold(0.0f32, f32::max);
        let mut yb = [(0.0f64, 0usize); 5];
        for (v, col) in p.iter().zip(c.iter()) {
            let f = (v[1] / maxy).clamp(0.0, 0.999);
            let luma = 0.2126 * col[0] + 0.7152 * col[1] + 0.0722 * col[2];
            yb[(f * 5.0) as usize].0 += luma as f64;
            yb[(f * 5.0) as usize].1 += 1;
        }
        println!("linear luma by height:");
        for (i, (s, n)) in yb.iter().enumerate() {
            if *n > 0 {
                println!("  y bin {i}: n={n:6}  luma={:.4}", s / *n as f64);
            }
        }
        let _ = bark;
    }

    // The cards themselves, for eyes rather than for statistics. One optional
    // argument: the directory to write them to.
    if let Some(dir) = std::env::args().nth(1) {
        dump_alpha(&needle_image(), &format!("{dir}/needle_mask.raw"));
        dump_alpha(&leaf_image(), &format!("{dir}/leaf_mask.raw"));
    }

    // The masks: coverage and feature width at level 0. The card size is the
    // MEASURED world edge, not `LeafParams::size` — that is in the generator's
    // own units and `fit_to_bounds` scales it.
    for (img, name, card_m) in [
        (needle_image(), "needle", card_edge(0)),
        (leaf_image(), "leaf", card_edge(3)),
    ] {
        let w = img.texture_descriptor.size.width as usize;
        let d = img.data.as_ref().unwrap();
        let lvl0 = &d[..w * w * 4];
        let hit = lvl0.chunks_exact(4).filter(|p| p[3] > 128).count();
        // Longest run of opaque texels along a row, as a proxy for how thick
        // one drawn element is.
        let mut runs: Vec<usize> = vec![];
        for row in 0..w {
            let mut run = 0usize;
            for col in 0..w {
                if lvl0[(row * w + col) * 4 + 3] > 128 {
                    run += 1;
                } else if run > 0 {
                    runs.push(run);
                    run = 0;
                }
            }
            if run > 0 {
                runs.push(run);
            }
        }
        runs.sort_unstable();
        let mm_per_texel = card_m * 1000.0 / w as f32;
        println!(
            "\n{name}: {w}²  coverage {:.3}  card {card_m} m  {:.1} mm/texel",
            hit as f32 / (w * w) as f32,
            mm_per_texel
        );
        println!(
            "  element width texels: med {} p90 {} max {}  →  med {:.0} mm, max {:.0} mm",
            runs[runs.len() / 2],
            runs[runs.len() * 9 / 10],
            runs[runs.len() - 1],
            runs[runs.len() / 2] as f32 * mm_per_texel,
            runs[runs.len() - 1] as f32 * mm_per_texel
        );
    }
}
