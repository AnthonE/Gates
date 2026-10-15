//! Painted skins' UV layouts are a public contract (`DECISIONS.md:432`,
//! `render/canvas.rs`): once a skin is painted for an item, moving its UVs
//! moves every painted skin off its mark. These pin each canvas's layout.
//!
//! **If a pin fails, the change moved a canvas.** If no skin is painted for
//! it yet, update the pin. If one is, the change is wrong: keep the layout
//! and change the mesh some other way.

#![cfg(feature = "render")]

use client::render::canvas::{self, CANVASES};

/// The pinned layouts: FNV-1a over each mesh's UVs (to 1e-6) and triangles.
const PINS: [(&str, u64); 2] = [("bat", 0xe4ab55ed216fdd78), ("tunic", 0xa9ec50d3accfe4f0)];

fn layout_hash(c: &canvas::Canvas) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |x: u32| {
        for b in x.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for m in canvas::meshes(c) {
        let f = canvas::flat(&m);
        eat(f.uv.len() as u32);
        for [u, v] in &f.uv {
            eat((u * 1e6).round() as i64 as u32);
            eat((v * 1e6).round() as i64 as u32);
        }
        for t in &f.tris {
            t.iter().for_each(|&i| eat(i));
        }
    }
    h
}

#[test]
fn canvas_layouts_are_pinned() {
    let moved: Vec<String> = CANVASES
        .iter()
        .filter_map(|c| {
            let pin = PINS.iter().find(|(k, _)| *k == c.key).map(|p| p.1);
            let got = layout_hash(c);
            (pin != Some(got)).then(|| format!("(\"{}\", {got:#018x})", c.key))
        })
        .collect();
    assert!(
        moved.is_empty(),
        "canvas layouts moved, and a moved layout moves every skin painted for it: {}",
        moved.join(", ")
    );
}

/// Every UV on the image, and no two triangles on the same texel: a texel
/// one part paints must not paint another.
#[test]
fn canvases_fill_the_image_without_overlap() {
    const N: usize = 256;
    for c in &CANVASES {
        let mut hits = vec![0u8; N * N];
        for m in canvas::meshes(c) {
            let f = canvas::flat(&m);
            assert!(!f.uv.is_empty(), "{} has UVs", c.key);
            for [u, v] in &f.uv {
                assert!(
                    (0.0..=1.0).contains(u) && (0.0..=1.0).contains(v),
                    "{} UV ({u}, {v}) off the image",
                    c.key
                );
            }
            for t in &f.tris {
                let [a, b, d] = t.map(|i| f.uv[i as usize]);
                let area = (b[0] - a[0]) * (d[1] - a[1]) - (d[0] - a[0]) * (b[1] - a[1]);
                if area.abs() < 1e-9 {
                    continue;
                }
                for y in 0..N {
                    for x in 0..N {
                        let p = [(x as f32 + 0.5) / N as f32, (y as f32 + 0.5) / N as f32];
                        let w0 =
                            ((b[0] - p[0]) * (d[1] - p[1]) - (d[0] - p[0]) * (b[1] - p[1])) / area;
                        let w1 =
                            ((d[0] - p[0]) * (a[1] - p[1]) - (a[0] - p[0]) * (d[1] - p[1])) / area;
                        let w2 = 1.0 - w0 - w1;
                        if w0 > 1e-4 && w1 > 1e-4 && w2 > 1e-4 {
                            hits[y * N + x] = hits[y * N + x].saturating_add(1);
                        }
                    }
                }
            }
        }
        let twice = hits.iter().filter(|&&h| h > 1).count();
        let used = hits.iter().filter(|&&h| h > 0).count();
        assert_eq!(
            twice, 0,
            "{}: {twice} texels painted by two triangles",
            c.key
        );
        assert!(
            used * 10 > N * N * 9,
            "{} uses {}% of its image",
            c.key,
            used * 100 / (N * N)
        );
    }
}

/// The content rule and the renderer agree on which items can be painted.
#[test]
fn content_and_renderer_name_the_same_canvases() {
    let mut ours: Vec<&str> = CANVASES.iter().map(|c| c.item).collect();
    let mut theirs: Vec<&str> = content::SKIN_CANVAS_ITEMS.to_vec();
    ours.sort_unstable();
    theirs.sort_unstable();
    assert_eq!(ours, theirs);
}
