//! The grass card's gate. **Renderer tier**: it links Bevy for `Mesh`, but it
//! opens no window, needs no GPU and reads no pixel of a frame.
//!
//! `CLAUDE.md` forbids a visual gate here and this is not one — every
//! assertion is arithmetic that, violated, produces a specific named artefact:
//! a tuft planted upside down, a tuft stretched because the quad and the atlas
//! cell disagree about aspect, a card sampling across a cell boundary so two
//! tufts appear spliced, or a card that shades exactly like the dirt it grows
//! out of.
//!
//! **The atlas is read from disk, not described.** `assets/textures/
//! grass_card_albedo.png` is baked by `ci/bake_grass_atlas.py` from a source
//! that is gitignored, so the shipped file is the only artefact CI has — and a
//! constant in `clutter.rs` that disagrees with it is exactly the drift
//! `CLAUDE.md` warns about twice. Reading the PNG header is enough for the
//! layout claims and costs no dependency.
//!
//! The person who decides whether it looks good boots the game and looks.

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use client::render::clutter::{
    element_mesh, masked, CARDS_PER_TUFT, CARD_ASPECT, CARD_ATLAS, CARD_CELLS, CARD_COLS,
    CARD_ROWS, CARD_SINK, TUFT_H,
};
use sim_core::terrain::{Clutter, ClutterElem};

fn elem(kind: Clutter, yaw: u8, scale: f32) -> ClutterElem {
    ClutterElem {
        kind,
        x: -12.0,
        y: 3.0,
        z: 7.5,
        yaw,
        scale,
    }
}

fn attr(m: &Mesh, id: bevy::mesh::MeshVertexAttributeId) -> Vec<[f32; 3]> {
    match m.attribute(id) {
        Some(VertexAttributeValues::Float32x3(v)) => v.clone(),
        _ => panic!("missing attribute"),
    }
}

fn positions(m: &Mesh) -> Vec<Vec3> {
    attr(m, Mesh::ATTRIBUTE_POSITION.id)
        .into_iter()
        .map(Vec3::from)
        .collect()
}

fn uvs(m: &Mesh) -> Vec<[f32; 2]> {
    match m.attribute(Mesh::ATTRIBUTE_UV_0.id) {
        Some(VertexAttributeValues::Float32x2(v)) => v.clone(),
        _ => panic!("a card with no UVs cannot address the atlas"),
    }
}

/// Width and height of the shipped atlas, straight out of the PNG's IHDR.
fn atlas_size() -> (u32, u32) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets")
        .join(CARD_ATLAS);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("the shipped atlas {} is unreadable: {e}", path.display()));
    assert_eq!(
        &bytes[..8],
        b"\x89PNG\r\n\x1a\n",
        "the atlas is not a PNG; it must carry alpha, which JPEG cannot"
    );
    // IHDR is the first chunk: 8 magic + 4 length + 4 type, then w, h as BE u32.
    let w = u32::from_be_bytes(bytes[16..20].try_into().expect("IHDR width"));
    let h = u32::from_be_bytes(bytes[20..24].try_into().expect("IHDR height"));
    (w, h)
}

// ---------------------------------------------------------------------------
// The atlas and the constants that address it.
// ---------------------------------------------------------------------------

/// **The card's aspect is the cell's aspect or the tuft is stretched**, and a
/// stretched photograph of grass reads as bad grass rather than as a bug, so
/// nobody looks for it in a constant.
#[test]
fn the_quad_and_the_cell_agree_about_aspect() {
    let (w, h) = atlas_size();
    let cell = (w as f32 / CARD_COLS as f32) / (h as f32 / CARD_ROWS as f32);
    assert!(
        (cell - CARD_ASPECT).abs() < 1e-3,
        "the atlas cell is {cell:.3} wide per tall and the quad is drawn at \
         {CARD_ASPECT:.3} — every tuft is stretched by {:.1}%",
        (CARD_ASPECT / cell - 1.0).abs() * 100.0
    );
}

/// The atlas must be power-of-two on both sides or `render::mipmap::wants`
/// refuses it — and a cutout with no chain is the shimmer this whole slice
/// exists downstream of.
#[test]
fn the_atlas_can_carry_a_mip_chain() {
    let (w, h) = atlas_size();
    assert!(
        w.is_power_of_two() && h.is_power_of_two(),
        "the atlas is {w}x{h}; `mipmap::wants` skips a non-power-of-two, so it \
         would ship with one level and shimmer"
    );
    assert_eq!(
        CARD_CELLS,
        CARD_COLS * CARD_ROWS,
        "the cell count and the grid disagree, so a hash can address a cell \
         that is not there"
    );
}

// ---------------------------------------------------------------------------
// The mesh.
// ---------------------------------------------------------------------------

/// A tuft is cards now, and each card is two triangles.
#[test]
fn a_tuft_is_crossed_cards() {
    let m = element_mesh(&elem(Clutter::Tuft, 40, 1.0));
    assert_eq!(
        positions(&m).len() as u32,
        CARDS_PER_TUFT * 6,
        "a tuft should be {CARDS_PER_TUFT} quads of two triangles"
    );
    assert!(masked(Clutter::Tuft), "the tuft must draw through the cutout");
    for k in [Clutter::Pebble, Clutter::Twig, Clutter::Shard] {
        assert!(
            !masked(k),
            "{k:?} would be alpha-tested against a texture it has no UVs for"
        );
    }
}

/// **Every UV lands inside one cell, and the card's own four corners land
/// inside the SAME cell.** A card straddling a boundary splices two different
/// tufts down its middle — which reads as one strange plant rather than as an
/// addressing bug.
#[test]
fn every_card_stays_inside_one_cell() {
    let (du, dv) = (1.0 / CARD_COLS as f32, 1.0 / CARD_ROWS as f32);
    const EPS: f32 = 1e-5;
    for yaw in [0u8, 37, 128, 200, 255] {
        let m = element_mesh(&elem(Clutter::Tuft, yaw, 1.0));
        let uv = uvs(&m);
        // Six vertices per card: two triangles over one quad.
        for quad in uv.chunks(6) {
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
            // **Identified by the card's CENTRE, not by a corner.** A card's
            // edge UVs sit exactly on a cell boundary, where `floor` is
            // ambiguous by one — classifying per vertex makes the test's own
            // arithmetic the thing under test.
            let ci = (((u0 + u1) * 0.5) / du).floor();
            let ri = (((v0 + v1) * 0.5) / dv).floor();
            let (cu0, cu1) = (ci * du, (ci + 1.0) * du);
            let (cv0, cv1) = (ri * dv, (ri + 1.0) * dv);
            assert!(
                u0 >= cu0 - EPS && u1 <= cu1 + EPS && v0 >= cv0 - EPS && v1 <= cv1 + EPS,
                "a card spans u {u0}..{u1} v {v0}..{v1}, outside cell \
                 ({ci},{ri}) = u {cu0}..{cu1} v {cv0}..{cv1} — it is drawing \
                 half of one tuft and half of another"
            );
            // And it fills the cell it is in, rather than sampling a sliver.
            assert!(
                (u1 - u0 - du).abs() < EPS && (v1 - v0 - dv).abs() < EPS,
                "a card samples {}x{} of a {du}x{dv} cell — the tuft is cropped",
                u1 - u0,
                v1 - v0
            );
        }
    }
}

/// **Roots at the bottom of the cell, tips at the top.** V grows downward in
/// image space, so the card's highest vertex takes the cell's SMALLEST v.
/// Getting this backwards plants every tuft upside down — instantly obvious in
/// a frame, invisible in a vertex count, and the exact class of thing this
/// file exists to hold.
#[test]
fn the_tuft_is_not_planted_upside_down() {
    let m = element_mesh(&elem(Clutter::Tuft, 91, 1.0));
    let (p, uv) = (positions(&m), uvs(&m));
    let hi = p
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.y.total_cmp(&b.1.y))
        .expect("a card has vertices")
        .0;
    let lo = p
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.y.total_cmp(&b.1.y))
        .expect("a card has vertices")
        .0;
    assert!(
        uv[hi][1] < uv[lo][1],
        "the highest vertex samples v={} and the lowest v={} — the tuft is \
         upside down",
        uv[hi][1],
        uv[lo][1]
    );
}

/// The card is bedded, not resting on the surface (`ART.md` rule 2), and its
/// height is `TUFT_H` scaled — a card that ignores `scale` makes every tuft on
/// the island the same size, which is rule 7's identical instances.
#[test]
fn the_card_is_bedded_and_scales() {
    let ground = elem(Clutter::Tuft, 40, 1.0).y;
    let m = element_mesh(&elem(Clutter::Tuft, 40, 1.0));
    let y_min = positions(&m).iter().fold(f32::MAX, |a, v| a.min(v.y));
    assert!(
        y_min < ground,
        "the card's baseline sits at {y_min} on ground {ground} — nothing may \
         sit ON the ground"
    );
    assert!(
        ground - y_min <= TUFT_H * CARD_SINK * 1.5 + 1e-4,
        "the card is buried {} m, well past the {CARD_SINK} sink",
        ground - y_min
    );

    let big = positions(&element_mesh(&elem(Clutter::Tuft, 40, 2.0)));
    let small = positions(&element_mesh(&elem(Clutter::Tuft, 40, 1.0)));
    let span = |p: &[Vec3]| {
        p.iter().fold(f32::MIN, |a, v| a.max(v.y)) - p.iter().fold(f32::MAX, |a, v| a.min(v.y))
    };
    assert!(
        span(&big) > span(&small) * 1.5,
        "scale 2.0 gives {} against 1.0's {} — the card ignores its element",
        span(&big),
        span(&small)
    );
}

// The card's root-vs-tip normal law is NOT re-asserted here. `tests/contact.rs`
// owns it — `a_blade_separates_from_the_ground_it_grows_out_of` runs on
// `element_mesh(Clutter::Tuft)`, which is a card now, and its bounds (root
// > 0.99, tip < 0.985) are tighter than anything this file would have written.
// A second copy of one law is the drift `CLAUDE.md` warns about; the card's
// contribution is `CARD_BED`, which exists because that gate went red.
