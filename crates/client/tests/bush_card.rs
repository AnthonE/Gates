//! The plants' leaf cards (`render/plants.rs`). **Renderer tier**: Bevy for
//! `Mesh`, no window, no GPU, no pixel of a frame read.
//!
//! Every assertion is arithmetic that, violated, produces a named artefact: a
//! leaf stretched because the quad and the cell disagree about aspect, a
//! cluster planted upside down, a card sampling across a cell boundary so two
//! bushes appear spliced, an atlas that cannot carry a mip chain and
//! therefore shimmers, or a hemp leaf hung by its tip.
//!
//! **The atlas is read from disk.** `ci/bake_bush_atlas.py` composes it from a
//! source that is gitignored, so the shipped PNG is the only artefact CI has,
//! and a constant in `props.rs` that disagrees with it is the drift
//! `CLAUDE.md` warns about twice.
//!
//! The person who decides whether it looks good boots the game and looks.

#![cfg(feature = "render")]
#![allow(clippy::assertions_on_constants)]

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use client::render::mipmap::MASK_CUT;
use client::render::plants::{
    berry_leaves, hemp, hemp_leaf_rgba, shrub_leaves, HEMP_LEAF_PX, PLANT_POOL,
};
use client::render::props::{
    BUSH_CARD_ALPHA_CUT, BUSH_CARD_ATLAS, BUSH_CARD_CELLS, BUSH_CARD_COLS, BUSH_CARD_ROWS,
};

/// Vertices per leaf card: a 3×3 grid, four quads of two triangles.
const CARD_VERTS: usize = 24;

fn positions(m: &Mesh) -> Vec<Vec3> {
    match m.attribute(Mesh::ATTRIBUTE_POSITION.id) {
        Some(VertexAttributeValues::Float32x3(v)) => v.iter().copied().map(Vec3::from).collect(),
        _ => panic!("no positions"),
    }
}

fn normals(m: &Mesh) -> Vec<Vec3> {
    match m.attribute(Mesh::ATTRIBUTE_NORMAL.id) {
        Some(VertexAttributeValues::Float32x3(v)) => v.iter().copied().map(Vec3::from).collect(),
        _ => panic!("no normals"),
    }
}

fn uvs(m: &Mesh) -> Vec<[f32; 2]> {
    match m.attribute(Mesh::ATTRIBUTE_UV_0.id) {
        Some(VertexAttributeValues::Float32x2(v)) => v.clone(),
        _ => panic!("a card with no UVs cannot address the atlas"),
    }
}

/// Every leaf mesh that wears the bush atlas.
fn atlas_leaves() -> Vec<(String, Mesh)> {
    let mut out = Vec::new();
    for v in 0..PLANT_POOL as u32 {
        out.push((format!("shrub {v}"), shrub_leaves(v, false)));
        out.push((format!("tall shrub {v}"), shrub_leaves(v, true)));
        out.push((format!("berry bush {v}"), berry_leaves(v)));
    }
    out
}

fn atlas_size() -> (u32, u32) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets")
        .join(BUSH_CARD_ATLAS);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("the shipped atlas {} is unreadable: {e}", path.display()));
    assert_eq!(
        &bytes[..8],
        b"\x89PNG\r\n\x1a\n",
        "the atlas is not a PNG; it must carry alpha, which JPEG cannot"
    );
    let w = u32::from_be_bytes(bytes[16..20].try_into().expect("IHDR width"));
    let h = u32::from_be_bytes(bytes[20..24].try_into().expect("IHDR height"));
    (w, h)
}

/// **The quad's aspect is the cell's aspect or every leaf is stretched.**
///
/// This is the one that already fired: the first cut drew a bush-shaped quad
/// (0.78 × 0.64) over a square cell, stretching the leaves 22% wide. A 22% fat
/// leaf reads as a different plant, not as a bug, so nothing but arithmetic
/// was ever going to catch it. The cards are square by construction (one
/// `half` per card), so the claim reduces to the cell being square too.
#[test]
fn the_quad_and_the_cell_agree_about_aspect() {
    let (w, h) = atlas_size();
    let cell = (w as f32 / BUSH_CARD_COLS as f32) / (h as f32 / BUSH_CARD_ROWS as f32);
    assert!(
        (cell - 1.0).abs() < 1e-3,
        "the atlas cell is {cell:.3} wide per tall and the quad is square — \
         every leaf is stretched by {:.1}%",
        (cell - 1.0).abs() * 100.0
    );
    for (name, m) in atlas_leaves() {
        for card in positions(&m).chunks(CARD_VERTS) {
            // The grid's corners are the first triangle's b0 and the last
            // triangle's t1; the four edges of a square card are equal.
            let lo = card[0];
            let hi = card[CARD_VERTS - 1];
            let (right, up) = (card[12] - lo, card[1] - lo);
            let (w, h) = (right.length(), up.length());
            assert!(
                (w - h).abs() < 1e-4
                    && ((hi - lo).length() - (w * w + h * h).sqrt() * 2.0).abs() < 1e-3,
                "{name}: a card is {w:.3} × {h:.3} per half — not square"
            );
        }
    }
}

/// Power-of-two both ways or `render::mipmap::wants` skips it, and a cutout
/// with one level shimmers at every distance the camera moves through.
#[test]
fn the_atlas_can_carry_a_mip_chain() {
    let (w, h) = atlas_size();
    assert!(
        w.is_power_of_two() && h.is_power_of_two(),
        "the atlas is {w}x{h}; `mipmap::wants` refuses a non-power-of-two"
    );
    assert_eq!(BUSH_CARD_CELLS, BUSH_CARD_COLS * BUSH_CARD_ROWS);
    assert!(HEMP_LEAF_PX.is_power_of_two());
}

/// The cutoff the chain preserves against and the one the frame tests with are
/// one number, or the leaves thin with distance by exactly the gap.
#[test]
fn the_cutoff_is_the_one_the_chain_preserves() {
    let as_byte = (BUSH_CARD_ALPHA_CUT * 255.0).round() as u8;
    assert_eq!(as_byte, MASK_CUT);
}

/// The pools hold distinct meshes — one shared mesh would make every plant on
/// the island identical, which is `ART.md` rule 7's forbidden case.
#[test]
fn the_pools_differ() {
    let shrubs: Vec<_> = (0..PLANT_POOL as u32)
        .map(|v| positions(&shrub_leaves(v, false)))
        .collect();
    let hemps: Vec<_> = (0..PLANT_POOL as u32)
        .map(|v| positions(&hemp(v).1))
        .collect();
    for pool in [&shrubs, &hemps] {
        for i in 1..pool.len() {
            assert!(
                pool[i] != pool[0],
                "variant {i} is geometrically identical to variant 0 — the pool \
                 is not buying any variation"
            );
        }
    }
}

/// A card stays inside one cell and fills it. Straddling a boundary splices
/// two different bushes down one quad's middle.
#[test]
fn every_card_stays_inside_one_cell() {
    let (du, dv) = (1.0 / BUSH_CARD_COLS as f32, 1.0 / BUSH_CARD_ROWS as f32);
    const EPS: f32 = 1e-5;
    for (name, m) in atlas_leaves() {
        for card in uvs(&m).chunks(CARD_VERTS) {
            let (mut u0, mut u1) = (f32::MAX, f32::MIN);
            let (mut v0, mut v1) = (f32::MAX, f32::MIN);
            for t in card {
                assert!(
                    (0.0..=1.0).contains(&t[0]) && (0.0..=1.0).contains(&t[1]),
                    "{name}: UV {t:?} is off the atlas"
                );
                u0 = u0.min(t[0]);
                u1 = u1.max(t[0]);
                v0 = v0.min(t[1]);
                v1 = v1.max(t[1]);
            }
            // Identified by the card's centre: its edge UVs sit exactly on a
            // cell boundary, where `floor` is ambiguous by one.
            let ci = (((u0 + u1) * 0.5) / du).floor();
            let ri = (((v0 + v1) * 0.5) / dv).floor();
            assert!(
                u0 >= ci * du - EPS
                    && u1 <= (ci + 1.0) * du + EPS
                    && v0 >= ri * dv - EPS
                    && v1 <= (ri + 1.0) * dv + EPS,
                "{name}: a card spans u {u0}..{u1} v {v0}..{v1}, outside cell ({ci},{ri})"
            );
            assert!(
                (u1 - u0 - du).abs() < EPS && (v1 - v0 - dv).abs() < EPS,
                "{name}: a card samples {}x{} of a {du}x{dv} cell — the cluster is cropped",
                u1 - u0,
                v1 - v0
            );
        }
    }
}

/// V grows downward in image space, so a card's highest vertex takes its
/// cell's smallest v. Backwards plants every cluster upside down.
#[test]
fn no_cluster_is_planted_upside_down() {
    for (name, m) in atlas_leaves() {
        let (p, uv) = (positions(&m), uvs(&m));
        for (pc, uc) in p.chunks(CARD_VERTS).zip(uv.chunks(CARD_VERTS)) {
            let hi = (0..pc.len()).max_by(|&a, &b| pc[a].y.total_cmp(&pc[b].y));
            let lo = (0..pc.len()).min_by(|&a, &b| pc[a].y.total_cmp(&pc[b].y));
            let (hi, lo) = (hi.expect("vertices"), lo.expect("vertices"));
            assert!(
                uc[hi][1] < uc[lo][1],
                "{name}: a card's highest vertex samples v={} and its lowest v={} — upside down",
                uc[hi][1],
                uc[lo][1]
            );
        }
    }
}

/// **A leaf mass scatters as a sphere, not as plates.** The normals are
/// pulled off each card's facet toward a dome inside the plant; without it
/// crossed quads take different sun cosines and the bush reads as folded foil
/// (the defect `clutter::BLADE_TIP_BLEND` names for grass).
#[test]
fn the_leaves_shade_as_a_mass() {
    for (name, m) in atlas_leaves() {
        let (p, n) = (positions(&m), normals(&m));
        let centre = p.iter().copied().sum::<Vec3>() / p.len() as f32;
        let mut radial = 0.0f32;
        for (v, nv) in p.iter().zip(&n) {
            radial += (*v - centre).normalize_or_zero().dot(*nv);
        }
        radial /= p.len() as f32;
        assert!(
            radial > 0.6,
            "{name}: the cards' normals average {radial:.3} of the outward \
             direction — the leaves are shading as plates"
        );
        assert!(
            radial < 0.999,
            "{name}: the normals are perfectly radial ({radial:.3}) — no facet \
             gets through, so no two cards can separate"
        );
    }
}

/// **A hemp leaf hangs by its stalk, not by its tip.** The card maps the
/// image's bottom-centre to the petiole on the stalk and v = 0 to the tip, so
/// the drawn leaf must have its petiole at the bottom-centre, its longest
/// leaflet reaching the top, and nothing in the top corners — upside down, the
/// leaves would grow out of the air and point at the stalk.
#[test]
fn the_hemp_leaf_grows_from_the_bottom_centre() {
    let n = HEMP_LEAF_PX;
    let px = hemp_leaf_rgba(n);
    let alpha = |x: u32, y: u32| px[((y * n + x) * 4 + 3) as usize];
    let at = |fx: f32, fy: f32| alpha((fx * n as f32) as u32, (fy * n as f32) as u32);
    assert!(at(0.5, 0.94) > 200, "no petiole at the bottom-centre");
    assert!(
        at(0.5, 0.1) > 200,
        "the middle leaflet does not reach the top"
    );
    for (fx, fy) in [(0.02, 0.02), (0.98, 0.02), (0.02, 0.98), (0.98, 0.98)] {
        assert_eq!(at(fx, fy), 0, "a corner at ({fx}, {fy}) is not empty");
    }
    // A palmate leaf is mostly air between its leaflets: a mask that filled
    // the card would be a green square, and one that drew nothing a stalk.
    let opaque = px.chunks(4).filter(|t| t[3] >= MASK_CUT).count();
    let share = opaque as f32 / (n * n) as f32;
    assert!(
        (0.08..0.4).contains(&share),
        "the leaf covers {:.0}% of its card",
        share * 100.0
    );
}
