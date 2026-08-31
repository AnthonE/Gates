//! The bush card's gate. **Renderer tier**: Bevy for `Mesh`, no window, no GPU,
//! no pixel of a frame read.
//!
//! Every assertion is arithmetic that, violated, produces a named artefact: a
//! leaf stretched because the quad and the cell disagree about aspect, a
//! cluster planted upside down, a card sampling across a cell boundary so two
//! bushes appear spliced, or an atlas that cannot carry a mip chain and
//! therefore shimmers.
//!
//! **The atlas is read from disk.** `ci/bake_bush_atlas.py` composes it from a
//! source that is gitignored, so the shipped PNG is the only artefact CI has,
//! and a constant in `props.rs` that disagrees with it is the drift
//! `CLAUDE.md` warns about twice.
//!
//! The person who decides whether it looks good boots the game and looks.
//!
//! **`assertions_on_constants` is allowed here for `tests/water.rs`'s reason**,
//! quoted because it is the whole justification: "a knob gate is a statement
//! about values somebody will edit, and a relation that folds to `true` today
//! is exactly the one that stops folding when they do". The cell count against
//! the grid is such a relation — fold it away and a hash can address a cell
//! that is not there.

#![allow(clippy::assertions_on_constants)]

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use client::render::mipmap::MASK_CUT;
use client::render::props::{
    bush_card_mesh, BUSH_CARDS, BUSH_CARD_ALPHA_CUT, BUSH_CARD_ATLAS, BUSH_CARD_CELLS,
    BUSH_CARD_COLS, BUSH_CARD_HALF, BUSH_CARD_POOL, BUSH_CARD_ROWS, BUSH_CARD_VOLUME,
};

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
/// was ever going to catch it. The bush is still wider than tall — the baked
/// CLUSTER is, which is where that shape belongs.
#[test]
fn the_quad_and_the_cell_agree_about_aspect() {
    let (w, h) = atlas_size();
    let cell = (w as f32 / BUSH_CARD_COLS as f32) / (h as f32 / BUSH_CARD_ROWS as f32);
    // The quad is square by construction (`BUSH_CARD_HALF` is one number), so
    // the claim reduces to the cell being square too.
    assert!(
        (cell - 1.0).abs() < 1e-3,
        "the atlas cell is {cell:.3} wide per tall and the quad is square — \
         every leaf is stretched by {:.1}%",
        (cell - 1.0).abs() * 100.0
    );
    assert!(BUSH_CARD_HALF > 0.0, "a card with no size");
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
}

/// The cutoff the chain preserves against and the one the frame tests with are
/// one number, or the leaves thin with distance by exactly the gap.
#[test]
fn the_cutoff_is_the_one_the_chain_preserves() {
    let as_byte = (BUSH_CARD_ALPHA_CUT * 255.0).round() as u8;
    assert_eq!(as_byte, MASK_CUT);
}

/// Every variant is [`BUSH_CARDS`] quads, and the pool holds distinct meshes —
/// one shared mesh would make every bush on the island identical, which is
/// `ART.md` rule 7's forbidden case.
#[test]
fn the_pool_is_crossed_cards_and_they_differ() {
    let mut seen: Vec<Vec<Vec3>> = Vec::new();
    for v in 0..BUSH_CARD_POOL as u32 {
        let m = bush_card_mesh(v);
        let p = positions(&m);
        assert_eq!(
            p.len() as u32,
            BUSH_CARDS * 6,
            "variant {v} should be {BUSH_CARDS} quads of two triangles"
        );
        seen.push(p);
    }
    for i in 1..seen.len() {
        assert!(
            seen[i] != seen[0],
            "variant {i} is geometrically identical to variant 0 — the pool is \
             not buying any variation"
        );
    }
}

/// A card stays inside one cell and fills it. Straddling a boundary splices
/// two different bushes down one quad's middle.
#[test]
fn every_card_stays_inside_one_cell() {
    let (du, dv) = (
        1.0 / BUSH_CARD_COLS as f32,
        1.0 / BUSH_CARD_ROWS as f32,
    );
    const EPS: f32 = 1e-5;
    for v in 0..BUSH_CARD_POOL as u32 {
        let m = bush_card_mesh(v);
        for quad in uvs(&m).chunks(6) {
            let (mut u0, mut u1) = (f32::MAX, f32::MIN);
            let (mut v0, mut v1) = (f32::MAX, f32::MIN);
            for t in quad {
                assert!(
                    (0.0..=1.0).contains(&t[0]) && (0.0..=1.0).contains(&t[1]),
                    "UV {t:?} is off the atlas"
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
                "a card spans u {u0}..{u1} v {v0}..{v1}, outside cell ({ci},{ri})"
            );
            assert!(
                (u1 - u0 - du).abs() < EPS && (v1 - v0 - dv).abs() < EPS,
                "a card samples {}x{} of a {du}x{dv} cell — the cluster is cropped",
                u1 - u0,
                v1 - v0
            );
        }
    }
}

/// V grows downward in image space, so the card's highest vertex takes the
/// cell's smallest v. Backwards plants every bush upside down.
#[test]
fn the_cluster_is_not_planted_upside_down() {
    let m = bush_card_mesh(0);
    let (p, uv) = (positions(&m), uvs(&m));
    let hi = p
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.y.total_cmp(&b.1.y))
        .expect("vertices")
        .0;
    let lo = p
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.y.total_cmp(&b.1.y))
        .expect("vertices")
        .0;
    assert!(
        uv[hi][1] < uv[lo][1],
        "the highest vertex samples v={} and the lowest v={} — upside down",
        uv[hi][1],
        uv[lo][1]
    );
}

/// The cards are centred on the origin, which is `blob_mesh`'s own frame —
/// both children of a bush slot take the identical transform, so a card built
/// anywhere else floats beside its bush rather than around it.
#[test]
fn the_cards_are_centred_on_the_blob_they_wrap() {
    for v in 0..BUSH_CARD_POOL as u32 {
        let p = positions(&bush_card_mesh(v));
        let c = p.iter().copied().sum::<Vec3>() / p.len() as f32;
        assert!(
            c.length() < 1e-4,
            "variant {v}'s cards centre on {c:?}, not the origin"
        );
        let reach = p.iter().fold(0.0f32, |a, q| a.max(q.length()));
        assert!(
            reach <= BUSH_CARD_HALF * 1.15 * 2.0f32.sqrt() + 1e-3,
            "variant {v} reaches {reach} m, past what {BUSH_CARD_HALF} and its \
             jitter allow"
        );
    }
}

/// **A leaf mass scatters as a sphere, not as three plates.** The normals are
/// pulled off each quad's facet toward a dome centred on the bush; without it
/// three crossed quads take three different sun cosines and the bush reads as
/// folded foil (the defect `clutter::BLADE_TIP_BLEND` names for grass).
#[test]
fn the_leaves_shade_as_a_mass() {
    let m = bush_card_mesh(0);
    let (p, n) = (positions(&m), normals(&m));
    let mut radial = 0.0f32;
    for (v, nv) in p.iter().zip(&n) {
        // Every vertex is off-origin, so its outward direction is defined.
        let want = v.normalize();
        radial += want.dot(*nv);
    }
    radial /= p.len() as f32;
    // At blend 1.0 this is exactly 1. The facet share keeps it under that, and
    // the bound is what stops somebody quietly setting the blend to 0.
    assert!(
        radial > 0.80,
        "the cards' normals average {radial:.3} of the outward direction — at \
         {BUSH_CARD_VOLUME} blend they should be nearly radial, so the leaves \
         are shading as plates"
    );
    assert!(
        radial < 0.999,
        "the normals are perfectly radial ({radial:.3}) — no facet is getting \
         through, so the three cards cannot separate from each other at all"
    );
}
